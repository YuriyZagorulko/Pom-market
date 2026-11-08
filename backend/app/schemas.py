"""Pydantic v2 request and response schemas with strict secret exclusion."""

from __future__ import annotations

from datetime import datetime
from decimal import Decimal
from typing import Any, Optional

from pydantic import BaseModel, ConfigDict, EmailStr, Field, field_validator

from backend.app.models import (
    ExecutionMode,
    OperatingMode,
    RoleName,
    SignatureType,
    TradeSide,
)


# ---------------------------------------------------------------------------
# Auth & User Schemas
# ---------------------------------------------------------------------------
class UserRegisterRequest(BaseModel):
    email: EmailStr
    password: str = Field(min_length=8, max_length=128)
    full_name: Optional[str] = Field(default=None, max_length=255)


class UserLoginRequest(BaseModel):
    email: EmailStr
    password: str
    totp_code: Optional[str] = Field(default=None, min_length=6, max_length=6)


class RefreshTokenRequest(BaseModel):
    refresh_token: str


class EmailVerifyRequest(BaseModel):
    token: str


class PasswordResetInitiateRequest(BaseModel):
    email: EmailStr


class PasswordResetConfirmRequest(BaseModel):
    token: str
    new_password: str = Field(min_length=8, max_length=128)


class TotpSetupResponse(BaseModel):
    secret: str
    otpauth_uri: str


class TotpVerifyRequest(BaseModel):
    totp_code: str = Field(min_length=6, max_length=6)


class UserResponse(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    id: int
    email: str
    full_name: Optional[str]
    is_active: bool
    is_suspended: bool
    email_verified: bool
    totp_enabled: bool
    roles: list[str]
    created_at: datetime


class AuthTokensResponse(BaseModel):
    access_token: str
    refresh_token: str
    token_type: str = "bearer"
    expires_in: int
    user: UserResponse
    verification_token_preview: Optional[str] = None


# ---------------------------------------------------------------------------
# Trading Account & Credential Schemas (NEVER return secrets in responses)
# ---------------------------------------------------------------------------
class TradingAccountCreateRequest(BaseModel):
    display_name: str = Field(min_length=1, max_length=255)
    signer_wallet_address: str = Field(min_length=10, max_length=64)
    funder_wallet_address: Optional[str] = Field(default=None, max_length=64)
    signature_type: SignatureType = SignatureType.POLY_GNOSIS_SAFE

    # Optional sensitive credentials encrypted immediately at rest
    private_key: Optional[str] = Field(default=None)
    api_key: Optional[str] = Field(default=None)
    api_secret: Optional[str] = Field(default=None)
    api_passphrase: Optional[str] = Field(default=None)

    initial_paper_balance_usd: Decimal = Field(default=Decimal("1000.0"), ge=Decimal("0"))

    @field_validator("signer_wallet_address")
    @classmethod
    def validate_signer_address(cls, v: str) -> str:
        cleaned = v.strip().lower()
        if not cleaned.startswith("0x") or len(cleaned) != 42:
            raise ValueError("signer_wallet_address must be a valid 42-char 0x Ethereum address")
        int(cleaned[2:], 16)
        return cleaned

    @field_validator("funder_wallet_address")
    @classmethod
    def validate_funder_address(cls, v: Optional[str]) -> Optional[str]:
        if v is None or v.strip() == "":
            return None
        cleaned = v.strip().lower()
        if not cleaned.startswith("0x") or len(cleaned) != 42:
            raise ValueError("funder_wallet_address must be a valid 42-char 0x Ethereum address")
        int(cleaned[2:], 16)
        return cleaned


class CredentialRotateRequest(BaseModel):
    private_key: Optional[str] = None
    api_key: Optional[str] = None
    api_secret: Optional[str] = None
    api_passphrase: Optional[str] = None


class OperatingModeChangeRequest(BaseModel):
    operating_mode: OperatingMode
    explicit_confirmation: Optional[str] = Field(
        default=None,
        description="Required when switching to LIVE_TRADING: must equal 'CONFIRM_LIVE_TRADING'",
    )


class EmergencyStopRequest(BaseModel):
    emergency_stop: bool
    reason: Optional[str] = None


class RiskLimitUpdateRequest(BaseModel):
    max_order_size_usd: Optional[Decimal] = Field(default=None, gt=Decimal("0"))
    min_order_size_usd: Optional[Decimal] = Field(default=None, gt=Decimal("0"))
    max_market_exposure_usd: Optional[Decimal] = Field(default=None, ge=Decimal("0"))
    max_source_trader_exposure_usd: Optional[Decimal] = Field(default=None, ge=Decimal("0"))
    max_total_exposure_usd: Optional[Decimal] = Field(default=None, ge=Decimal("0"))
    max_daily_loss_usd: Optional[Decimal] = Field(default=None, ge=Decimal("0"))
    max_slippage_pct: Optional[Decimal] = Field(default=None, ge=Decimal("0"), le=Decimal("1"))
    max_executions_per_interval: Optional[int] = Field(default=None, ge=0)
    execution_interval_seconds: Optional[int] = Field(default=None, ge=1)
    min_market_liquidity_usd: Optional[Decimal] = Field(default=None, ge=Decimal("0"))
    max_stale_data_seconds: Optional[int] = Field(default=None, ge=1)
    circuit_breaker_error_threshold: Optional[int] = Field(default=None, ge=1)


class CopyStrategyUpdateRequest(BaseModel):
    execution_mode: Optional[ExecutionMode] = None
    fixed_amount_usd: Optional[Decimal] = Field(default=None, gt=Decimal("0"))
    source_size_percentage: Optional[Decimal] = Field(default=None, gt=Decimal("0"), le=Decimal("10"))
    copy_multiplier: Optional[Decimal] = Field(default=None, gt=Decimal("0"), le=Decimal("100"))
    max_order_size_usd: Optional[Decimal] = Field(default=None, gt=Decimal("0"))
    min_order_size_usd: Optional[Decimal] = Field(default=None, gt=Decimal("0"))
    max_slippage_pct: Optional[Decimal] = Field(default=None, ge=Decimal("0"), le=Decimal("1"))
    buy_enabled: Optional[bool] = None
    sell_enabled: Optional[bool] = None
    slice_collapse_window_secs: Optional[int] = Field(default=None, ge=0)
    allowed_markets: Optional[list[str]] = None


class RiskLimitResponse(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    max_order_size_usd: Decimal
    min_order_size_usd: Decimal
    max_market_exposure_usd: Decimal
    max_source_trader_exposure_usd: Decimal
    max_total_exposure_usd: Decimal
    max_daily_loss_usd: Decimal
    max_slippage_pct: Decimal
    max_executions_per_interval: int
    execution_interval_seconds: int
    min_market_liquidity_usd: Decimal
    max_stale_data_seconds: int
    circuit_breaker_error_threshold: int
    daily_loss_stop_latched: bool


class CopyStrategyResponse(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    id: int
    name: str
    is_active: bool
    execution_mode: str
    fixed_amount_usd: Decimal
    source_size_percentage: Decimal
    copy_multiplier: Decimal
    max_order_size_usd: Decimal
    min_order_size_usd: Decimal
    max_slippage_pct: Decimal
    buy_enabled: bool
    sell_enabled: bool
    slice_collapse_window_secs: int
    allowed_markets: Optional[list[str]]


class TradingAccountResponse(BaseModel):
    """Public representation of a trading account. Secrets are strictly excluded."""

    model_config = ConfigDict(from_attributes=True)

    id: int
    user_id: int
    display_name: str
    funder_wallet_address: Optional[str]
    signer_wallet_address: str
    signature_type: str
    operating_mode: str
    connection_status: str
    authorization_status: str
    trading_status: str
    emergency_stop: bool
    emergency_stop_reason: Optional[str]
    consecutive_errors: int
    circuit_breaker_tripped: bool
    has_signing_key: bool
    has_api_credentials: bool
    key_version: Optional[int]
    created_at: datetime
    updated_at: datetime


# ---------------------------------------------------------------------------
# Source Trader Schemas
# ---------------------------------------------------------------------------
class SourceTraderAssignRequest(BaseModel):
    wallet_address: str = Field(min_length=10, max_length=64)
    custom_name: Optional[str] = Field(default=None, max_length=255)
    is_enabled: bool = True
    execution_mode: ExecutionMode = ExecutionMode.FIXED_AMOUNT
    fixed_amount_usd: Decimal = Field(default=Decimal("10.0"), gt=Decimal("0"))
    copy_percentage: Decimal = Field(default=Decimal("0.10"), gt=Decimal("0"), le=Decimal("10.0"))
    copy_multiplier: Decimal = Field(default=Decimal("1.0"), gt=Decimal("0"), le=Decimal("100.0"))
    max_order_size_usd: Decimal = Field(default=Decimal("100.0"), gt=Decimal("0"))
    min_order_size_usd: Decimal = Field(default=Decimal("1.0"), gt=Decimal("0"))
    max_exposure_usd: Decimal = Field(default=Decimal("500.0"), ge=Decimal("0"))
    max_slippage_pct: Decimal = Field(default=Decimal("0.05"), ge=Decimal("0"), le=Decimal("1.0"))
    copy_buy_enabled: bool = True
    copy_sell_enabled: bool = True

    @field_validator("wallet_address")
    @classmethod
    def validate_wallet(cls, v: str) -> str:
        cleaned = v.strip().lower()
        if not cleaned.startswith("0x") or len(cleaned) != 42:
            raise ValueError("wallet_address must be a valid 42-char 0x Ethereum address")
        int(cleaned[2:], 16)
        return cleaned


class SourceTraderMappingUpdateRequest(BaseModel):
    custom_name: Optional[str] = Field(default=None, max_length=255)
    is_enabled: Optional[bool] = None
    execution_mode: Optional[ExecutionMode] = None
    fixed_amount_usd: Optional[Decimal] = Field(default=None, gt=Decimal("0"))
    copy_percentage: Optional[Decimal] = Field(default=None, gt=Decimal("0"), le=Decimal("10.0"))
    copy_multiplier: Optional[Decimal] = Field(default=None, gt=Decimal("0"), le=Decimal("100.0"))
    max_order_size_usd: Optional[Decimal] = Field(default=None, gt=Decimal("0"))
    min_order_size_usd: Optional[Decimal] = Field(default=None, gt=Decimal("0"))
    max_exposure_usd: Optional[Decimal] = Field(default=None, ge=Decimal("0"))
    max_slippage_pct: Optional[Decimal] = Field(default=None, ge=Decimal("0"), le=Decimal("1.0"))
    copy_buy_enabled: Optional[bool] = None
    copy_sell_enabled: Optional[bool] = None


class SourceTraderImportRequest(BaseModel):
    wallets: list[SourceTraderAssignRequest]


class SourceTraderMappingResponse(BaseModel):
    id: int
    user_id: int
    trading_account_id: int
    source_trader_id: int
    wallet_address: str
    custom_name: Optional[str]
    is_enabled: bool
    execution_mode: str
    fixed_amount_usd: Decimal
    copy_percentage: Decimal
    copy_multiplier: Decimal
    max_order_size_usd: Decimal
    min_order_size_usd: Decimal
    max_exposure_usd: Decimal
    max_slippage_pct: Decimal
    copy_buy_enabled: bool
    copy_sell_enabled: bool
    total_observed_trades: int
    total_observed_volume_usd: Decimal
    win_rate_pct: Optional[Decimal]
    realized_roi_pct: Optional[Decimal]
    created_at: datetime


# ---------------------------------------------------------------------------
# Admin Schemas
# ---------------------------------------------------------------------------
class AdminSuspendUserRequest(BaseModel):
    is_suspended: bool
    reason: Optional[str] = None


class GlobalEmergencyStopRequest(BaseModel):
    enabled: bool
    reason: str = Field(min_length=3, max_length=512)
