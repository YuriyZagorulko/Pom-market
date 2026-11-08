"""Multi-user authentication, registration, session management, RBAC, and TOTP service."""

from __future__ import annotations

from datetime import datetime, timedelta, timezone
from typing import Optional

from sqlalchemy import select
from sqlalchemy.orm import Session

from backend.app.config import get_settings
from backend.app.services.email_service import DEV_MAILBOX, DevMailboxSink
from backend.app.models import (
    AccountActivityLog,
    RoleName,
    User,
    UserRole,
    UserSession,
)
from backend.app.schemas import (
    AuthTokensResponse,
    TotpSetupResponse,
    UserLoginRequest,
    UserRegisterRequest,
    UserResponse,
)
from backend.app.security import (
    CredentialVault,
    create_access_token,
    generate_opaque_token,
    generate_totp_secret,
    hash_opaque_token,
    hash_password,
    verify_password,
    verify_totp_code,
)


class AuthError(Exception):
    def __init__(self, message: str, status_code: int = 400) -> None:
        super().__init__(message)
        self.message = message
        self.status_code = status_code


def _to_user_response(user: User) -> UserResponse:
    role_names = sorted({r.role_name for r in user.roles})
    return UserResponse(
        id=user.id,
        email=user.email,
        full_name=user.full_name,
        is_active=user.is_active,
        is_suspended=user.is_suspended,
        email_verified=user.email_verified,
        totp_enabled=user.totp_enabled,
        roles=role_names,
        created_at=user.created_at,
    )


class AuthService:
    def __init__(
        self,
        vault: Optional[CredentialVault] = None,
        mailbox: Optional[DevMailboxSink] = None,
    ) -> None:
        self._vault = vault or CredentialVault()
        self._mailbox = mailbox or DEV_MAILBOX

    def register_user(
        self,
        session: Session,
        req: UserRegisterRequest,
        *,
        grant_super_admin: bool = False,
        ip_address: Optional[str] = None,
        user_agent: Optional[str] = None,
    ) -> AuthTokensResponse:
        normalized_email = req.email.strip().lower()
        existing = session.scalar(select(User).where(User.email == normalized_email))
        if existing is not None:
            raise AuthError("A user with this email is already registered", status_code=409)

        raw_verify_token, verify_token_hash = generate_opaque_token()
        now = datetime.now(timezone.utc)

        user = User(
            email=normalized_email,
            password_hash=hash_password(req.password),
            full_name=req.full_name,
            is_active=True,
            is_suspended=False,
            email_verified=False,
            email_verification_token_hash=verify_token_hash,
            email_verification_expires_at=now + timedelta(hours=24),
        )
        session.add(user)
        session.flush()

        session.add(UserRole(user_id=user.id, role_name=RoleName.USER.value))
        if grant_super_admin:
            session.add(UserRole(user_id=user.id, role_name=RoleName.SUPER_ADMIN.value))
        session.flush()
        session.refresh(user)

        session.add(
            AccountActivityLog(
                user_id=user.id,
                action="USER_REGISTERED",
                details_json={"email": user.email},
                ip_address=ip_address,
            )
        )

        tokens = self._issue_session_tokens(session, user, ip_address=ip_address, user_agent=user_agent)
        # Deliver verification token via out-of-band email sink (never in HTTP API responses)
        self._mailbox.send_token_email(
            recipient_email=user.email,
            subject="Verify your Pom-Market email",
            kind="email_verification",
            token=raw_verify_token,
        )
        return tokens

    def login(
        self,
        session: Session,
        req: UserLoginRequest,
        *,
        ip_address: Optional[str] = None,
        user_agent: Optional[str] = None,
    ) -> AuthTokensResponse:
        normalized_email = req.email.strip().lower()
        user = session.scalar(select(User).where(User.email == normalized_email))
        if user is None or not verify_password(req.password, user.password_hash):
            raise AuthError("Invalid email or password", status_code=401)

        if not user.is_active or user.is_suspended:
            reason = user.suspension_reason or "Account is suspended"
            raise AuthError(f"Account access denied: {reason}", status_code=403)

        if user.totp_enabled:
            if not req.totp_code:
                raise AuthError("TOTP two-factor authentication code is required", status_code=401)
            if not user.totp_secret_encrypted:
                raise AuthError("TOTP configuration corrupted", status_code=500)
            totp_secret = self._vault.decrypt_secret(
                user.totp_secret_encrypted,
                user_id=user.id,
                trading_account_id=0,
            )
            if not verify_totp_code(totp_secret, req.totp_code):
                raise AuthError("Invalid TOTP code", status_code=401)

        session.add(
            AccountActivityLog(
                user_id=user.id,
                action="USER_LOGIN",
                details_json={"totp_used": user.totp_enabled},
                ip_address=ip_address,
            )
        )
        return self._issue_session_tokens(session, user, ip_address=ip_address, user_agent=user_agent)

    def refresh_session(
        self,
        session: Session,
        raw_refresh_token: str,
        *,
        ip_address: Optional[str] = None,
        user_agent: Optional[str] = None,
    ) -> AuthTokensResponse:
        token_hash = hash_opaque_token(raw_refresh_token)
        user_session = session.scalar(
            select(UserSession).where(UserSession.refresh_token_hash == token_hash)
        )
        now = datetime.now(timezone.utc)
        if user_session is None or user_session.is_revoked:
            raise AuthError("Invalid or revoked refresh token", status_code=401)
        exp = user_session.expires_at
        if exp.tzinfo is None:
            exp = exp.replace(tzinfo=timezone.utc)
        if exp <= now:
            user_session.is_revoked = True
            raise AuthError("Refresh token has expired", status_code=401)

        user = session.get(User, user_session.user_id)
        if user is None or not user.is_active or user.is_suspended:
            user_session.is_revoked = True
            raise AuthError("User account is suspended or inactive", status_code=403)

        # Rotate refresh token on every use
        user_session.is_revoked = True
        return self._issue_session_tokens(session, user, ip_address=ip_address, user_agent=user_agent)

    def logout(
        self,
        session: Session,
        user_id: int,
        raw_refresh_token: Optional[str] = None,
        *,
        session_id: Optional[int] = None,
    ) -> None:
        if session_id is not None:
            sess_by_id = session.scalar(
                select(UserSession).where(
                    UserSession.id == session_id,
                    UserSession.user_id == user_id,
                )
            )
            if sess_by_id is not None:
                sess_by_id.is_revoked = True

        if raw_refresh_token:
            token_hash = hash_opaque_token(raw_refresh_token)
            sess = session.scalar(
                select(UserSession).where(
                    UserSession.user_id == user_id,
                    UserSession.refresh_token_hash == token_hash,
                )
            )
            if sess is not None:
                sess.is_revoked = True
        elif session_id is None:
            sessions = session.scalars(
                select(UserSession).where(UserSession.user_id == user_id, UserSession.is_revoked.is_(False))
            ).all()
            for s in sessions:
                s.is_revoked = True

        session.add(
            AccountActivityLog(
                user_id=user_id,
                action="USER_LOGOUT",
                details_json={"session_id": session_id},
            )
        )
        session.flush()

    def verify_email(self, session: Session, raw_token: str) -> UserResponse:
        token_hash = hash_opaque_token(raw_token)
        user = session.scalar(
            select(User).where(User.email_verification_token_hash == token_hash)
        )
        now = datetime.now(timezone.utc)
        if user is None or user.email_verification_expires_at is None:
            raise AuthError("Invalid email verification token", status_code=400)
        exp = user.email_verification_expires_at
        if exp.tzinfo is None:
            exp = exp.replace(tzinfo=timezone.utc)
        if exp < now:
            raise AuthError("Email verification token has expired", status_code=400)

        user.email_verified = True
        user.email_verification_token_hash = None
        user.email_verification_expires_at = None
        session.flush()
        return _to_user_response(user)

    def initiate_password_reset(self, session: Session, email: str) -> Optional[str]:
        normalized_email = email.strip().lower()
        user = session.scalar(select(User).where(User.email == normalized_email))
        if user is None:
            return None
        raw_token, token_hash = generate_opaque_token()
        user.password_reset_token_hash = token_hash
        user.password_reset_expires_at = datetime.now(timezone.utc) + timedelta(hours=1)
        session.flush()
        self._mailbox.send_token_email(
            recipient_email=user.email,
            subject="Pom-Market Password Reset",
            kind="password_reset",
            token=raw_token,
        )
        return raw_token

    def confirm_password_reset(self, session: Session, raw_token: str, new_password: str) -> UserResponse:
        token_hash = hash_opaque_token(raw_token)
        user = session.scalar(select(User).where(User.password_reset_token_hash == token_hash))
        now = datetime.now(timezone.utc)
        if user is None or user.password_reset_expires_at is None:
            raise AuthError("Invalid password reset token", status_code=400)
        exp = user.password_reset_expires_at
        if exp.tzinfo is None:
            exp = exp.replace(tzinfo=timezone.utc)
        if exp < now:
            raise AuthError("Password reset token has expired", status_code=400)

        user.password_hash = hash_password(new_password)
        user.password_reset_token_hash = None
        user.password_reset_expires_at = None

        # Revoke all existing sessions upon password change
        for s in user.sessions:
            s.is_revoked = True

        session.add(
            AccountActivityLog(
                user_id=user.id,
                action="PASSWORD_RESET_COMPLETED",
                details_json={},
            )
        )
        session.flush()
        return _to_user_response(user)

    def setup_totp(self, session: Session, user_id: int) -> TotpSetupResponse:
        user = session.get(User, user_id)
        if user is None:
            raise AuthError("User not found", status_code=404)
        secret_b32 = generate_totp_secret()
        user.totp_secret_encrypted = self._vault.encrypt_secret(
            secret_b32,
            user_id=user.id,
            trading_account_id=0,
        )
        user.totp_enabled = False
        session.flush()
        uri = f"otpauth://totp/PomMarket:{user.email}?secret={secret_b32}&issuer=PomMarket"
        return TotpSetupResponse(secret=secret_b32, otpauth_uri=uri)

    def enable_totp(self, session: Session, user_id: int, totp_code: str) -> UserResponse:
        user = session.get(User, user_id)
        if user is None or not user.totp_secret_encrypted:
            raise AuthError("TOTP setup not initiated", status_code=400)
        secret_b32 = self._vault.decrypt_secret(
            user.totp_secret_encrypted,
            user_id=user.id,
            trading_account_id=0,
        )
        if not verify_totp_code(secret_b32, totp_code):
            raise AuthError("Invalid TOTP verification code", status_code=400)
        user.totp_enabled = True
        session.add(
            AccountActivityLog(
                user_id=user.id,
                action="TOTP_ENABLED",
                details_json={},
            )
        )
        session.flush()
        return _to_user_response(user)

    def set_user_suspension(
        self,
        session: Session,
        *,
        admin_user_id: int,
        target_user_id: int,
        is_suspended: bool,
        reason: Optional[str] = None,
    ) -> UserResponse:
        user = session.get(User, target_user_id)
        if user is None:
            raise AuthError("Target user not found", status_code=404)
        user.is_suspended = is_suspended
        user.suspension_reason = reason if is_suspended else None
        if is_suspended:
            for s in user.sessions:
                s.is_revoked = True
        session.add(
            AccountActivityLog(
                user_id=target_user_id,
                action="USER_SUSPENDED" if is_suspended else "USER_REACTIVATED",
                details_json={"by_admin_id": admin_user_id, "reason": reason},
            )
        )
        session.flush()
        return _to_user_response(user)

    def _issue_session_tokens(
        self,
        session: Session,
        user: User,
        *,
        ip_address: Optional[str] = None,
        user_agent: Optional[str] = None,
    ) -> AuthTokensResponse:
        settings = get_settings()
        raw_refresh, refresh_hash = generate_opaque_token()
        now = datetime.now(timezone.utc)
        user_session = UserSession(
            user_id=user.id,
            refresh_token_hash=refresh_hash,
            ip_address=ip_address,
            user_agent=user_agent,
            is_revoked=False,
            expires_at=now + timedelta(days=settings.refresh_token_expire_days),
        )
        session.add(user_session)
        session.flush()

        role_names = sorted({r.role_name for r in user.roles})
        access_token = create_access_token(
            user_id=user.id,
            email=user.email,
            roles=role_names,
            session_id=user_session.id,
        )
        return AuthTokensResponse(
            access_token=access_token,
            refresh_token=raw_refresh,
            expires_in=settings.access_token_expire_minutes * 60,
            user=_to_user_response(user),
        )
