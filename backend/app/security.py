"""Security, Argon2id password hashing, AES-256-GCM credential encryption, JWT, and TOTP."""

from __future__ import annotations

import base64
import hashlib
import hmac
import os
import secrets
import struct
import time
from datetime import datetime, timedelta, timezone
from typing import Any, Optional

import jwt
import nacl.exceptions
import nacl.pwhash.argon2id
from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from cryptography.hazmat.primitives.kdf.hkdf import HKDF

from backend.app.config import get_settings


# ---------------------------------------------------------------------------
# 1. Argon2id Password Hashing (libsodium / PyNaCl)
# ---------------------------------------------------------------------------
def hash_password(plain_password: str) -> str:
    """Hash a password using Argon2id (libsodium MODERATE parameters)."""
    if not plain_password or len(plain_password) < 8:
        raise ValueError("Password must be at least 8 characters long")
    hashed = nacl.pwhash.argon2id.str(
        plain_password.encode("utf-8"),
        opslimit=nacl.pwhash.argon2id.OPSLIMIT_INTERACTIVE,
        memlimit=nacl.pwhash.argon2id.MEMLIMIT_INTERACTIVE,
    )
    return hashed.decode("ascii")


def verify_password(plain_password: str, password_hash: str) -> bool:
    """Verify a password against an Argon2id hash in constant time."""
    if not plain_password or not password_hash:
        return False
    try:
        return nacl.pwhash.argon2id.verify(
            password_hash.encode("ascii"),
            plain_password.encode("utf-8"),
        )
    except (nacl.exceptions.InvalidkeyError, ValueError, TypeError):
        return False


# ---------------------------------------------------------------------------
# 2. Authenticated Encryption at Rest (AES-256-GCM + HKDF-SHA256 + Tenant AAD)
# ---------------------------------------------------------------------------
class CredentialVault:
    """Encrypts and decrypts trading account secrets using AES-256-GCM.

    Every ciphertext binds the owning `user_id` and `trading_account_id` in both
    the HKDF info parameter and the AES-GCM Additional Authenticated Data (AAD),
    preventing cross-tenant or cross-account ciphertext substitution even if
    database rows were tampered with.
    """

    def __init__(
        self,
        master_key: Optional[str] = None,
        previous_master_key: Optional[str] = None,
    ) -> None:
        settings = get_settings()
        self._master_key = (master_key or settings.master_encryption_key).encode("utf-8")
        prev = previous_master_key if previous_master_key is not None else settings.master_encryption_key_previous
        self._previous_master_key = prev.encode("utf-8") if prev else None
        if len(self._master_key) < 16:
            raise ValueError("MASTER_ENCRYPTION_KEY must be at least 16 bytes")

    @staticmethod
    def _derive_aead_key(master: bytes, salt: bytes, context_info: bytes) -> bytes:
        hkdf = HKDF(
            algorithm=hashes.SHA256(),
            length=32,
            salt=salt,
            info=b"pommarket-credential-vault-v1:" + context_info,
        )
        return hkdf.derive(master)

    @staticmethod
    def _build_aad(user_id: int, trading_account_id: int) -> bytes:
        return f"tenant:{int(user_id)}:account:{int(trading_account_id)}".encode("ascii")

    def encrypt_secret(
        self,
        plaintext: str,
        *,
        user_id: int,
        trading_account_id: int,
        key_version: int = 1,
    ) -> str:
        if not plaintext:
            raise ValueError("Cannot encrypt empty secret")
        salt = os.urandom(16)
        nonce = os.urandom(12)
        aad = self._build_aad(user_id, trading_account_id)
        derived_key = self._derive_aead_key(self._master_key, salt, aad)
        aesgcm = AESGCM(derived_key)
        ciphertext = aesgcm.encrypt(nonce, plaintext.encode("utf-8"), aad)
        envelope = salt + nonce + ciphertext
        b64_payload = base64.urlsafe_b64encode(envelope).decode("ascii")
        return f"v{int(key_version)}:{b64_payload}"

    def decrypt_secret(
        self,
        token: str,
        *,
        user_id: int,
        trading_account_id: int,
    ) -> str:
        if not token or ":" not in token:
            raise ValueError("Malformed encrypted secret envelope")
        _version_prefix, b64_payload = token.split(":", 1)
        raw = base64.urlsafe_b64decode(b64_payload.encode("ascii"))
        if len(raw) < 16 + 12 + 16:
            raise ValueError("Truncated encrypted secret payload")
        salt = raw[:16]
        nonce = raw[16:28]
        ciphertext = raw[28:]
        aad = self._build_aad(user_id, trading_account_id)

        candidate_keys = [self._master_key]
        if self._previous_master_key:
            candidate_keys.append(self._previous_master_key)

        last_err: Optional[Exception] = None
        for master in candidate_keys:
            try:
                derived_key = self._derive_aead_key(master, salt, aad)
                aesgcm = AESGCM(derived_key)
                plain_bytes = aesgcm.decrypt(nonce, ciphertext, aad)
                return plain_bytes.decode("utf-8")
            except Exception as exc:
                last_err = exc
        raise PermissionError("Failed to decrypt secret: invalid key or tenant/account mismatch") from last_err


# ---------------------------------------------------------------------------
# 3. TOTP Two-Factor Authentication (RFC 6238)
# ---------------------------------------------------------------------------
def generate_totp_secret() -> str:
    """Generate a 160-bit Base32-encoded TOTP secret."""
    return base64.b32encode(secrets.token_bytes(20)).decode("ascii").rstrip("=")


def compute_totp_code(secret_b32: str, for_time: Optional[int] = None, step: int = 30, digits: int = 6) -> str:
    """Compute a 6-digit RFC 6238 TOTP code."""
    padded = secret_b32.strip().upper()
    missing_padding = len(padded) % 8
    if missing_padding:
        padded += "=" * (8 - missing_padding)
    key = base64.b32decode(padded, casefold=True)
    ts = int(time.time()) if for_time is None else int(for_time)
    counter = ts // step
    msg = struct.pack(">Q", counter)
    digest = hmac.new(key, msg, hashlib.sha1).digest()
    offset = digest[-1] & 0x0F
    code_int = (struct.unpack(">I", digest[offset : offset + 4])[0] & 0x7FFFFFFF) % (10**digits)
    return f"{code_int:0{digits}d}"


def verify_totp_code(secret_b32: str, code: str, at_time: Optional[int] = None, window: int = 1) -> bool:
    """Verify a TOTP code allowing +/- `window` 30-second steps."""
    if not code or not code.isdigit() or len(code) != 6:
        return False
    ts = int(time.time()) if at_time is None else int(at_time)
    for delta in range(-window, window + 1):
        expected = compute_totp_code(secret_b32, for_time=ts + delta * 30)
        if hmac.compare_digest(expected, code):
            return True
    return False


# ---------------------------------------------------------------------------
# 4. JWT Access Tokens & Opaque Refresh / Verification Tokens
# ---------------------------------------------------------------------------
def hash_opaque_token(raw_token: str) -> str:
    return hashlib.sha256(raw_token.encode("utf-8")).hexdigest()


def generate_opaque_token() -> tuple[str, str]:
    """Return `(raw_token, sha256_hex_hash)`."""
    raw = secrets.token_urlsafe(32)
    return raw, hash_opaque_token(raw)


def create_access_token(
    *,
    user_id: int,
    email: str,
    roles: list[str],
    session_id: Optional[int] = None,
    expires_delta: Optional[timedelta] = None,
) -> str:
    settings = get_settings()
    now = datetime.now(timezone.utc)
    exp = now + (expires_delta or timedelta(minutes=settings.access_token_expire_minutes))
    payload: dict[str, Any] = {
        "sub": str(int(user_id)),
        "email": email,
        "roles": roles,
        "type": "access",
        "iat": int(now.timestamp()),
        "exp": int(exp.timestamp()),
    }
    if session_id is not None:
        payload["sid"] = int(session_id)
    return jwt.encode(payload, settings.jwt_secret_key, algorithm=settings.jwt_algorithm)


def decode_access_token(token: str) -> dict[str, Any]:
    settings = get_settings()
    payload = jwt.decode(token, settings.jwt_secret_key, algorithms=[settings.jwt_algorithm])
    if payload.get("type") != "access":
        raise jwt.InvalidTokenError("Invalid token type")
    return payload


def redact_sensitive_dict(data: dict[str, Any]) -> dict[str, Any]:
    """Ensure private keys, secrets, passphrases, and signatures are never logged."""
    redacted_keys = {
        "private_key",
        "encrypted_private_key",
        "api_secret",
        "encrypted_api_secret",
        "api_passphrase",
        "passphrase",
        "encrypted_api_passphrase",
        "signature",
        "password",
        "password_hash",
        "refresh_token",
        "totp_secret",
    }
    sanitized: dict[str, Any] = {}
    for k, v in data.items():
        if k.lower() in redacted_keys or "secret" in k.lower() or "private_key" in k.lower():
            sanitized[k] = "***REDACTED***"
        elif isinstance(v, dict):
            sanitized[k] = redact_sensitive_dict(v)
        else:
            sanitized[k] = v
    return sanitized
