"""FastAPI Application Entrypoint for Multi-Tenant Polymarket Copy-Trading Platform."""

from __future__ import annotations

from datetime import datetime, timezone
from typing import Any, Optional

from fastapi import Depends, FastAPI, Header, HTTPException, Request, Response, WebSocket, WebSocketDisconnect, status
from fastapi.middleware.cors import CORSMiddleware
import jwt
from sqlalchemy import select, text
from sqlalchemy.orm import Session

from backend.app.config import get_settings
from backend.app.db import get_db, set_tenant_context
from backend.app.models import RoleName, SystemSetting, User, UserSession
from backend.app.schemas import (
    AdminSuspendUserRequest,
    AuthTokensResponse,
    CopyStrategyResponse,
    CopyStrategyUpdateRequest,
    CredentialRotateRequest,
    EmailVerifyRequest,
    EmergencyStopRequest,
    GlobalEmergencyStopRequest,
    OperatingModeChangeRequest,
    PasswordResetConfirmRequest,
    PasswordResetInitiateRequest,
    RefreshTokenRequest,
    RiskLimitResponse,
    RiskLimitUpdateRequest,
    SourceTraderAssignRequest,
    SourceTraderMappingResponse,
    SourceTraderMappingUpdateRequest,
    TotpSetupResponse,
    TotpVerifyRequest,
    TradingAccountCreateRequest,
    TradingAccountResponse,
    UserLoginRequest,
    UserRegisterRequest,
    UserResponse,
)
from backend.app.security import decode_access_token
from backend.app.services.account_service import (
    AccountValidationError,
    TenantAccessDeniedError,
    TradingAccountService,
)
from backend.app.services.auth_service import AuthError, AuthService, _to_user_response
from backend.app.services.dashboard_service import DashboardService
from backend.app.services.metrics_service import GLOBAL_METRICS
from backend.app.services.source_trader_service import SourceTraderService

app = FastAPI(
    title="Pom-Market Multi-Tenant Copy-Trading Platform API",
    version="1.0.0",
    description="Self-hosted multi-tenant Polymarket copy-trading platform with strict tenant isolation.",
)

_settings = get_settings()

app.add_middleware(
    CORSMiddleware,
    allow_origins=_settings.cors_allowed_origins,
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)

auth_service = AuthService()
account_service = TradingAccountService()
source_trader_service = SourceTraderService(account_service)
dashboard_service = DashboardService()


# ---------------------------------------------------------------------------
# Authentication & Tenant Context Dependencies
# ---------------------------------------------------------------------------
def get_current_user(
    authorization: Optional[str] = Header(default=None),
    db: Session = Depends(get_db),
) -> User:
    if not authorization or not authorization.lower().startswith("bearer "):
        raise HTTPException(status_code=status.HTTP_401_UNAUTHORIZED, detail="Missing Bearer token")
    raw_token = authorization.split(" ", 1)[1].strip()
    try:
        payload = decode_access_token(raw_token)
        user_id = int(payload["sub"])
        session_id = payload.get("sid")
    except (jwt.PyJWTError, KeyError, ValueError) as exc:
        raise HTTPException(status_code=status.HTTP_401_UNAUTHORIZED, detail="Invalid or expired access token") from exc

    # Use super_admin context briefly to load User, Roles, and Session row, then scope session to tenant
    set_tenant_context(db, user_id=None, is_super_admin=True)
    user = db.get(User, user_id)
    if user is None or not user.is_active or user.is_suspended:
        raise HTTPException(status_code=status.HTTP_403_FORBIDDEN, detail="User is inactive or suspended")

    # Fix S-02: Validate JWT session ID against UserSession (reject revoked or expired sessions)
    if session_id is None:
        raise HTTPException(status_code=status.HTTP_401_UNAUTHORIZED, detail="Access token missing session ID")
    user_session = db.get(UserSession, int(session_id))
    now = datetime.now(timezone.utc)
    if user_session is None or user_session.user_id != user.id or user_session.is_revoked:
        raise HTTPException(status_code=status.HTTP_401_UNAUTHORIZED, detail="Session has been revoked or logged out")
    exp = user_session.expires_at
    if exp.tzinfo is None:
        exp = exp.replace(tzinfo=timezone.utc)
    if exp <= now:
        user_session.is_revoked = True
        db.commit()
        raise HTTPException(status_code=status.HTTP_401_UNAUTHORIZED, detail="Session has expired")

    is_super = any(r.role_name == RoleName.SUPER_ADMIN.value for r in user.roles)
    set_tenant_context(db, user_id=user.id, is_super_admin=is_super)
    db.info["active_session_id"] = int(session_id)
    return user


def require_super_admin(current_user: User = Depends(get_current_user)) -> User:
    roles = {r.role_name for r in current_user.roles}
    if RoleName.SUPER_ADMIN.value not in roles:
        raise HTTPException(
            status_code=status.HTTP_403_FORBIDDEN,
            detail="SUPER_ADMIN role required",
        )
    return current_user


# ---------------------------------------------------------------------------
# Health, Metrics & Real-Time WebSocket Stream
# ---------------------------------------------------------------------------
@app.get("/health")
def health_check(db: Session = Depends(get_db)) -> dict[str, Any]:
    settings = get_settings()
    db_ok = False
    try:
        db.execute(text("SELECT 1"))
        db_ok = True
    except Exception:
        db_ok = False
    return {
        "status": "ok" if db_ok else "degraded",
        "service": "pom-market-api",
        "database_connected": db_ok,
        "redis_url": settings.redis_url,
        "nats_url": settings.nats_url,
        "rust_engine_url": settings.rust_engine_url,
        "timestamp": datetime.now(timezone.utc).isoformat(),
    }


@app.websocket("/ws/activity")
async def websocket_activity_stream(websocket: WebSocket) -> None:
    """Real-time trading activity WebSocket endpoint (`ws://localhost:4320/ws/activity`)."""
    await websocket.accept()
    try:
        await websocket.send_json(
            {
                "type": "connected",
                "service": "pom-market-ws",
                "timestamp": datetime.now(timezone.utc).isoformat(),
            }
        )
        while True:
            msg = await websocket.receive_text()
            if msg.strip().lower() == "ping":
                await websocket.send_json(
                    {
                        "type": "pong",
                        "metrics": GLOBAL_METRICS.snapshot(),
                        "timestamp": datetime.now(timezone.utc).isoformat(),
                    }
                )
            else:
                await websocket.send_json(
                    {
                        "type": "activity_snapshot",
                        "echo": msg,
                        "metrics": GLOBAL_METRICS.snapshot(),
                        "timestamp": datetime.now(timezone.utc).isoformat(),
                    }
                )
    except WebSocketDisconnect:
        return


@app.get("/metrics")
def prometheus_metrics() -> Response:
    return Response(
        content=GLOBAL_METRICS.render_prometheus_text(),
        media_type="text/plain; version=0.0.4",
    )


# ---------------------------------------------------------------------------
# 1. Authentication & User Management Endpoints
# ---------------------------------------------------------------------------
@app.post("/api/v1/auth/register", response_model=AuthTokensResponse, status_code=status.HTTP_201_CREATED)
def register(
    req: UserRegisterRequest,
    request: Request,
    db: Session = Depends(get_db),
) -> AuthTokensResponse:
    set_tenant_context(db, user_id=None, is_super_admin=True)
    try:
        res = auth_service.register_user(
            db,
            req,
            ip_address=request.client.host if request.client else None,
            user_agent=request.headers.get("user-agent"),
        )
        db.commit()
        return res
    except AuthError as exc:
        db.rollback()
        raise HTTPException(status_code=exc.status_code, detail=exc.message) from exc


@app.post("/api/v1/auth/login", response_model=AuthTokensResponse)
def login(
    req: UserLoginRequest,
    request: Request,
    db: Session = Depends(get_db),
) -> AuthTokensResponse:
    set_tenant_context(db, user_id=None, is_super_admin=True)
    try:
        res = auth_service.login(
            db,
            req,
            ip_address=request.client.host if request.client else None,
            user_agent=request.headers.get("user-agent"),
        )
        db.commit()
        return res
    except AuthError as exc:
        db.rollback()
        raise HTTPException(status_code=exc.status_code, detail=exc.message) from exc


@app.post("/api/v1/auth/refresh", response_model=AuthTokensResponse)
def refresh_tokens(
    req: RefreshTokenRequest,
    request: Request,
    db: Session = Depends(get_db),
) -> AuthTokensResponse:
    set_tenant_context(db, user_id=None, is_super_admin=True)
    try:
        res = auth_service.refresh_session(
            db,
            req.refresh_token,
            ip_address=request.client.host if request.client else None,
            user_agent=request.headers.get("user-agent"),
        )
        db.commit()
        return res
    except AuthError as exc:
        db.rollback()
        raise HTTPException(status_code=exc.status_code, detail=exc.message) from exc


@app.post("/api/v1/auth/logout")
def logout(
    req: Optional[RefreshTokenRequest] = None,
    current_user: User = Depends(get_current_user),
    db: Session = Depends(get_db),
) -> dict[str, str]:
    active_sid = db.info.get("active_session_id")
    auth_service.logout(
        db,
        current_user.id,
        req.refresh_token if req else None,
        session_id=int(active_sid) if active_sid is not None else None,
    )
    db.commit()
    return {"status": "logged_out"}


@app.post("/api/v1/auth/verify-email", response_model=UserResponse)
def verify_email(req: EmailVerifyRequest, db: Session = Depends(get_db)) -> UserResponse:
    set_tenant_context(db, user_id=None, is_super_admin=True)
    try:
        res = auth_service.verify_email(db, req.token)
        db.commit()
        return res
    except AuthError as exc:
        db.rollback()
        raise HTTPException(status_code=exc.status_code, detail=exc.message) from exc


@app.post("/api/v1/auth/password-reset/initiate")
def initiate_password_reset(req: PasswordResetInitiateRequest, db: Session = Depends(get_db)) -> dict[str, Any]:
    set_tenant_context(db, user_id=None, is_super_admin=True)
    auth_service.initiate_password_reset(db, req.email)
    db.commit()
    # Fix S-03: Never expose password reset tokens in HTTP responses in any environment.
    return {
        "status": "reset_initiated",
        "message": "If an account with that email exists, password reset instructions have been sent.",
    }


@app.post("/api/v1/auth/password-reset/confirm", response_model=UserResponse)
def confirm_password_reset(req: PasswordResetConfirmRequest, db: Session = Depends(get_db)) -> UserResponse:
    set_tenant_context(db, user_id=None, is_super_admin=True)
    try:
        res = auth_service.confirm_password_reset(db, req.token, req.new_password)
        db.commit()
        return res
    except AuthError as exc:
        db.rollback()
        raise HTTPException(status_code=exc.status_code, detail=exc.message) from exc


@app.post("/api/v1/auth/totp/setup", response_model=TotpSetupResponse)
def setup_totp(
    current_user: User = Depends(get_current_user),
    db: Session = Depends(get_db),
) -> TotpSetupResponse:
    res = auth_service.setup_totp(db, current_user.id)
    db.commit()
    return res


@app.post("/api/v1/auth/totp/enable", response_model=UserResponse)
def enable_totp(
    req: TotpVerifyRequest,
    current_user: User = Depends(get_current_user),
    db: Session = Depends(get_db),
) -> UserResponse:
    try:
        res = auth_service.enable_totp(db, current_user.id, req.totp_code)
        db.commit()
        return res
    except AuthError as exc:
        db.rollback()
        raise HTTPException(status_code=exc.status_code, detail=exc.message) from exc


@app.get("/api/v1/auth/me", response_model=UserResponse)
def get_me(current_user: User = Depends(get_current_user)) -> UserResponse:
    return _to_user_response(current_user)


# ---------------------------------------------------------------------------
# 2. Polymarket Trading Account Endpoints (Tenant-Isolated)
# ---------------------------------------------------------------------------
@app.get("/api/v1/accounts", response_model=list[TradingAccountResponse])
def list_accounts(
    current_user: User = Depends(get_current_user),
    db: Session = Depends(get_db),
) -> list[TradingAccountResponse]:
    return account_service.list_user_accounts(db, user_id=current_user.id)


@app.post("/api/v1/accounts", response_model=TradingAccountResponse, status_code=status.HTTP_201_CREATED)
def create_account(
    req: TradingAccountCreateRequest,
    current_user: User = Depends(get_current_user),
    db: Session = Depends(get_db),
) -> TradingAccountResponse:
    try:
        res = account_service.create_trading_account(db, user_id=current_user.id, req=req)
        db.commit()
        return res
    except AccountValidationError as exc:
        db.rollback()
        raise HTTPException(status_code=status.HTTP_400_BAD_REQUEST, detail=str(exc)) from exc


@app.get("/api/v1/accounts/{account_id}", response_model=TradingAccountResponse)
def get_account(
    account_id: int,
    current_user: User = Depends(get_current_user),
    db: Session = Depends(get_db),
) -> TradingAccountResponse:
    try:
        return account_service.get_account_response(db, user_id=current_user.id, account_id=account_id)
    except TenantAccessDeniedError as exc:
        raise HTTPException(status_code=status.HTTP_404_NOT_FOUND, detail="Trading account not found") from exc


@app.get("/api/v1/accounts/{account_id}/detail")
def get_account_detail(
    account_id: int,
    current_user: User = Depends(get_current_user),
    db: Session = Depends(get_db),
) -> dict[str, Any]:
    try:
        return dashboard_service.get_trading_account_page_detail(
            db, user_id=current_user.id, account_id=account_id
        )
    except TenantAccessDeniedError as exc:
        raise HTTPException(status_code=status.HTTP_404_NOT_FOUND, detail="Trading account not found") from exc


@app.post("/api/v1/accounts/{account_id}/credentials/rotate", response_model=TradingAccountResponse)
def rotate_account_credentials(
    account_id: int,
    req: CredentialRotateRequest,
    current_user: User = Depends(get_current_user),
    db: Session = Depends(get_db),
) -> TradingAccountResponse:
    try:
        res = account_service.rotate_credentials(db, user_id=current_user.id, account_id=account_id, req=req)
        db.commit()
        return res
    except TenantAccessDeniedError as exc:
        db.rollback()
        raise HTTPException(status_code=status.HTTP_404_NOT_FOUND, detail="Trading account not found") from exc


@app.post("/api/v1/accounts/{account_id}/disconnect", response_model=TradingAccountResponse)
def disconnect_account(
    account_id: int,
    current_user: User = Depends(get_current_user),
    db: Session = Depends(get_db),
) -> TradingAccountResponse:
    try:
        res = account_service.disconnect_account(db, user_id=current_user.id, account_id=account_id)
        db.commit()
        return res
    except TenantAccessDeniedError as exc:
        db.rollback()
        raise HTTPException(status_code=status.HTTP_404_NOT_FOUND, detail="Trading account not found") from exc


@app.post("/api/v1/accounts/{account_id}/mode", response_model=TradingAccountResponse)
def change_account_operating_mode(
    account_id: int,
    req: OperatingModeChangeRequest,
    current_user: User = Depends(get_current_user),
    db: Session = Depends(get_db),
) -> TradingAccountResponse:
    try:
        res = account_service.change_operating_mode(db, user_id=current_user.id, account_id=account_id, req=req)
        db.commit()
        return res
    except TenantAccessDeniedError as exc:
        db.rollback()
        raise HTTPException(status_code=status.HTTP_404_NOT_FOUND, detail="Trading account not found") from exc
    except AccountValidationError as exc:
        db.rollback()
        raise HTTPException(status_code=status.HTTP_400_BAD_REQUEST, detail=str(exc)) from exc


@app.post("/api/v1/accounts/{account_id}/emergency-stop", response_model=TradingAccountResponse)
def set_account_emergency_stop(
    account_id: int,
    req: EmergencyStopRequest,
    current_user: User = Depends(get_current_user),
    db: Session = Depends(get_db),
) -> TradingAccountResponse:
    try:
        res = account_service.set_emergency_stop(db, user_id=current_user.id, account_id=account_id, req=req)
        db.commit()
        return res
    except TenantAccessDeniedError as exc:
        db.rollback()
        raise HTTPException(status_code=status.HTTP_404_NOT_FOUND, detail="Trading account not found") from exc


@app.patch("/api/v1/accounts/{account_id}/risk-limits", response_model=RiskLimitResponse)
def update_account_risk_limits(
    account_id: int,
    req: RiskLimitUpdateRequest,
    current_user: User = Depends(get_current_user),
    db: Session = Depends(get_db),
) -> RiskLimitResponse:
    try:
        res = account_service.update_risk_limits(db, user_id=current_user.id, account_id=account_id, req=req)
        db.commit()
        return res
    except TenantAccessDeniedError as exc:
        db.rollback()
        raise HTTPException(status_code=status.HTTP_404_NOT_FOUND, detail="Trading account not found") from exc


@app.patch("/api/v1/accounts/{account_id}/copy-strategy", response_model=CopyStrategyResponse)
def update_account_copy_strategy(
    account_id: int,
    req: CopyStrategyUpdateRequest,
    current_user: User = Depends(get_current_user),
    db: Session = Depends(get_db),
) -> CopyStrategyResponse:
    try:
        res = account_service.update_copy_strategy(db, user_id=current_user.id, account_id=account_id, req=req)
        db.commit()
        return res
    except TenantAccessDeniedError as exc:
        db.rollback()
        raise HTTPException(status_code=status.HTTP_404_NOT_FOUND, detail="Trading account not found") from exc


# ---------------------------------------------------------------------------
# 3. Source Trader Management Endpoints (Tenant-Isolated)
# ---------------------------------------------------------------------------
@app.get("/api/v1/accounts/{account_id}/source-traders", response_model=list[SourceTraderMappingResponse])
def list_account_source_traders(
    account_id: int,
    current_user: User = Depends(get_current_user),
    db: Session = Depends(get_db),
) -> list[SourceTraderMappingResponse]:
    try:
        return source_trader_service.list_account_source_traders(
            db, user_id=current_user.id, account_id=account_id
        )
    except TenantAccessDeniedError as exc:
        raise HTTPException(status_code=status.HTTP_404_NOT_FOUND, detail="Trading account not found") from exc


@app.post(
    "/api/v1/accounts/{account_id}/source-traders",
    response_model=SourceTraderMappingResponse,
    status_code=status.HTTP_201_CREATED,
)
def assign_source_trader(
    account_id: int,
    req: SourceTraderAssignRequest,
    current_user: User = Depends(get_current_user),
    db: Session = Depends(get_db),
) -> SourceTraderMappingResponse:
    try:
        res = source_trader_service.add_source_trader_to_account(
            db, user_id=current_user.id, account_id=account_id, req=req
        )
        db.commit()
        return res
    except TenantAccessDeniedError as exc:
        db.rollback()
        raise HTTPException(status_code=status.HTTP_404_NOT_FOUND, detail="Trading account not found") from exc


@app.patch(
    "/api/v1/accounts/{account_id}/source-traders/{source_trader_id}",
    response_model=SourceTraderMappingResponse,
)
def update_source_trader_mapping(
    account_id: int,
    source_trader_id: int,
    req: SourceTraderMappingUpdateRequest,
    current_user: User = Depends(get_current_user),
    db: Session = Depends(get_db),
) -> SourceTraderMappingResponse:
    try:
        res = source_trader_service.update_source_trader_mapping(
            db,
            user_id=current_user.id,
            account_id=account_id,
            source_trader_id=source_trader_id,
            req=req,
        )
        db.commit()
        return res
    except TenantAccessDeniedError as exc:
        db.rollback()
        raise HTTPException(status_code=status.HTTP_404_NOT_FOUND, detail="Trading account not found") from exc
    except ValueError as exc:
        db.rollback()
        raise HTTPException(status_code=status.HTTP_404_NOT_FOUND, detail=str(exc)) from exc


@app.delete("/api/v1/accounts/{account_id}/source-traders/{source_trader_id}")
def delete_source_trader_mapping(
    account_id: int,
    source_trader_id: int,
    current_user: User = Depends(get_current_user),
    db: Session = Depends(get_db),
) -> dict[str, str]:
    try:
        source_trader_service.remove_source_trader_from_account(
            db,
            user_id=current_user.id,
            account_id=account_id,
            source_trader_id=source_trader_id,
        )
        db.commit()
        return {"status": "removed"}
    except TenantAccessDeniedError as exc:
        db.rollback()
        raise HTTPException(status_code=status.HTTP_404_NOT_FOUND, detail="Trading account not found") from exc
    except ValueError as exc:
        db.rollback()
        raise HTTPException(status_code=status.HTTP_404_NOT_FOUND, detail=str(exc)) from exc


@app.get("/api/v1/accounts/{account_id}/source-traders/export")
def export_source_traders_csv(
    account_id: int,
    current_user: User = Depends(get_current_user),
    db: Session = Depends(get_db),
) -> Response:
    try:
        csv_text = source_trader_service.export_source_wallets_csv(
            db, user_id=current_user.id, account_id=account_id
        )
        return Response(content=csv_text, media_type="text/csv")
    except TenantAccessDeniedError as exc:
        raise HTTPException(status_code=status.HTTP_404_NOT_FOUND, detail="Trading account not found") from exc


@app.get("/api/v1/source-traders/{wallet_address}")
def get_source_trader_page(
    wallet_address: str,
    current_user: User = Depends(get_current_user),
    db: Session = Depends(get_db),
) -> dict[str, Any]:
    try:
        return source_trader_service.get_source_trader_details_for_user(
            db, user_id=current_user.id, wallet_address=wallet_address
        )
    except ValueError as exc:
        raise HTTPException(status_code=status.HTTP_404_NOT_FOUND, detail=str(exc)) from exc


# ---------------------------------------------------------------------------
# 4. User Dashboard Endpoint
# ---------------------------------------------------------------------------
@app.get("/api/v1/dashboard/summary")
def get_dashboard_summary(
    current_user: User = Depends(get_current_user),
    db: Session = Depends(get_db),
) -> dict[str, Any]:
    return dashboard_service.get_user_dashboard_summary(db, user_id=current_user.id)


# ---------------------------------------------------------------------------
# 5. Administrative Endpoints (`SUPER_ADMIN` role only; zero secret access)
# ---------------------------------------------------------------------------
@app.get("/api/v1/admin/overview")
def admin_overview(
    _admin: User = Depends(require_super_admin),
    db: Session = Depends(get_db),
) -> dict[str, Any]:
    set_tenant_context(db, user_id=None, is_super_admin=True)
    return dashboard_service.get_admin_platform_overview(db)


@app.post("/api/v1/admin/users/{target_user_id}/suspend", response_model=UserResponse)
def admin_suspend_user(
    target_user_id: int,
    req: AdminSuspendUserRequest,
    admin: User = Depends(require_super_admin),
    db: Session = Depends(get_db),
) -> UserResponse:
    set_tenant_context(db, user_id=None, is_super_admin=True)
    try:
        res = auth_service.set_user_suspension(
            db,
            admin_user_id=admin.id,
            target_user_id=target_user_id,
            is_suspended=req.is_suspended,
            reason=req.reason,
        )
        db.commit()
        return res
    except AuthError as exc:
        db.rollback()
        raise HTTPException(status_code=exc.status_code, detail=exc.message) from exc


@app.post("/api/v1/admin/emergency-stop")
def admin_set_global_emergency_stop(
    req: GlobalEmergencyStopRequest,
    admin: User = Depends(require_super_admin),
    db: Session = Depends(get_db),
) -> dict[str, Any]:
    set_tenant_context(db, user_id=None, is_super_admin=True)
    setting = db.scalar(select(SystemSetting).where(SystemSetting.key == "global_emergency_stop"))
    payload = {
        "enabled": req.enabled,
        "reason": req.reason,
        "updated_at": datetime.now(timezone.utc).isoformat(),
        "updated_by_admin_id": admin.id,
    }
    if setting is None:
        setting = SystemSetting(
            key="global_emergency_stop",
            value_json=payload,
            description="Platform-wide emergency kill switch",
            updated_by_user_id=admin.id,
        )
        db.add(setting)
    else:
        setting.value_json = payload
        setting.updated_by_user_id = admin.id
    db.commit()
    return {"status": "updated", "global_emergency_stop": payload}
