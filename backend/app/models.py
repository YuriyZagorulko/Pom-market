"""Multi-tenant SQLAlchemy 2.0 ORM models for the Polymarket copy-trading platform."""

from __future__ import annotations

from datetime import datetime, timezone
from decimal import Decimal
from enum import Enum
from typing import Optional

from sqlalchemy import (
    BigInteger,
    Boolean,
    CheckConstraint,
    DateTime,
    ForeignKey,
    Index,
    Integer,
    Numeric,
    String,
    Text,
    UniqueConstraint,
    JSON,
)
from sqlalchemy.orm import Mapped, mapped_column, relationship

from backend.app.db import Base


def utcnow() -> datetime:
    return datetime.now(timezone.utc)


class RoleName(str, Enum):
    SUPER_ADMIN = "SUPER_ADMIN"
    ADMIN_OPS = "ADMIN_OPS"
    USER = "USER"


class OperatingMode(str, Enum):
    MONITORING = "MONITORING"
    PAPER_TRADING = "PAPER_TRADING"
    LIVE_TRADING = "LIVE_TRADING"


class SignatureType(str, Enum):
    EOA = "EOA"
    POLY_PROXY = "POLY_PROXY"
    POLY_GNOSIS_SAFE = "POLY_GNOSIS_SAFE"
    POLY_1271 = "POLY_1271"


class ExecutionMode(str, Enum):
    FIXED_AMOUNT = "FIXED_AMOUNT"
    FIXED_PERCENTAGE = "FIXED_PERCENTAGE"
    PROPORTIONAL_BALANCE = "PROPORTIONAL_BALANCE"
    MAX_ORDER_SIZE = "MAX_ORDER_SIZE"


class TradeSide(str, Enum):
    BUY = "BUY"
    SELL = "SELL"


class OrderStatus(str, Enum):
    PREPARED = "PREPARED"
    SUBMITTING = "SUBMITTING"
    PENDING = "PENDING"
    SUBMITTED = "SUBMITTED"
    ACKNOWLEDGED = "ACKNOWLEDGED"
    PARTIAL = "PARTIAL"
    FILLED = "FILLED"
    REJECTED = "REJECTED"
    CANCELLED = "CANCELLED"
    AMBIGUOUS = "AMBIGUOUS"


class ExecutionJobStatus(str, Enum):
    QUEUED = "QUEUED"
    PROCESSING = "PROCESSING"
    COMPLETED = "COMPLETED"
    SKIPPED = "SKIPPED"
    FAILED = "FAILED"
    AMBIGUOUS = "AMBIGUOUS"


# ---------------------------------------------------------------------------
# 1. users
# ---------------------------------------------------------------------------
class User(Base):
    __tablename__ = "users"

    id: Mapped[int] = mapped_column(BigInteger().with_variant(Integer, "sqlite"), primary_key=True, autoincrement=True)
    email: Mapped[str] = mapped_column(String(255), unique=True, nullable=False, index=True)
    password_hash: Mapped[str] = mapped_column(String(512), nullable=False)
    full_name: Mapped[Optional[str]] = mapped_column(String(255), nullable=True)

    is_active: Mapped[bool] = mapped_column(Boolean, default=True, nullable=False)
    is_suspended: Mapped[bool] = mapped_column(Boolean, default=False, nullable=False)
    suspension_reason: Mapped[Optional[str]] = mapped_column(String(512), nullable=True)

    email_verified: Mapped[bool] = mapped_column(Boolean, default=False, nullable=False)
    email_verification_token_hash: Mapped[Optional[str]] = mapped_column(String(128), nullable=True)
    email_verification_expires_at: Mapped[Optional[datetime]] = mapped_column(DateTime(timezone=True), nullable=True)

    password_reset_token_hash: Mapped[Optional[str]] = mapped_column(String(128), nullable=True)
    password_reset_expires_at: Mapped[Optional[datetime]] = mapped_column(DateTime(timezone=True), nullable=True)

    totp_enabled: Mapped[bool] = mapped_column(Boolean, default=False, nullable=False)
    totp_secret_encrypted: Mapped[Optional[str]] = mapped_column(Text, nullable=True)

    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow, nullable=False)
    updated_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow, onupdate=utcnow, nullable=False)

    roles: Mapped[list["UserRole"]] = relationship("UserRole", back_populates="user", cascade="all, delete-orphan")
    sessions: Mapped[list["UserSession"]] = relationship("UserSession", back_populates="user", cascade="all, delete-orphan")
    trading_accounts: Mapped[list["TradingAccount"]] = relationship(
        "TradingAccount", back_populates="user", cascade="all, delete-orphan"
    )


# ---------------------------------------------------------------------------
# 2. user_roles
# ---------------------------------------------------------------------------
class UserRole(Base):
    __tablename__ = "user_roles"
    __table_args__ = (UniqueConstraint("user_id", "role_name", name="uq_user_roles_user_role"),)

    id: Mapped[int] = mapped_column(BigInteger().with_variant(Integer, "sqlite"), primary_key=True, autoincrement=True)
    user_id: Mapped[int] = mapped_column(
        BigInteger().with_variant(Integer, "sqlite"),
        ForeignKey("users.id", ondelete="CASCADE"),
        nullable=False,
        index=True,
    )
    role_name: Mapped[str] = mapped_column(String(64), nullable=False, default=RoleName.USER.value)
    granted_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow, nullable=False)

    user: Mapped["User"] = relationship("User", back_populates="roles")


# ---------------------------------------------------------------------------
# 3. user_sessions
# ---------------------------------------------------------------------------
class UserSession(Base):
    __tablename__ = "user_sessions"

    id: Mapped[int] = mapped_column(BigInteger().with_variant(Integer, "sqlite"), primary_key=True, autoincrement=True)
    user_id: Mapped[int] = mapped_column(
        BigInteger().with_variant(Integer, "sqlite"),
        ForeignKey("users.id", ondelete="CASCADE"),
        nullable=False,
        index=True,
    )
    refresh_token_hash: Mapped[str] = mapped_column(String(128), unique=True, nullable=False, index=True)
    user_agent: Mapped[Optional[str]] = mapped_column(String(512), nullable=True)
    ip_address: Mapped[Optional[str]] = mapped_column(String(64), nullable=True)
    is_revoked: Mapped[bool] = mapped_column(Boolean, default=False, nullable=False)
    expires_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), nullable=False)
    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow, nullable=False)
    last_used_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow, nullable=False)

    user: Mapped["User"] = relationship("User", back_populates="sessions")


# ---------------------------------------------------------------------------
# 4. trading_accounts
# ---------------------------------------------------------------------------
class TradingAccount(Base):
    __tablename__ = "trading_accounts"
    __table_args__ = (
        Index("ix_trading_accounts_user_status", "user_id", "trading_status"),
    )

    id: Mapped[int] = mapped_column(BigInteger().with_variant(Integer, "sqlite"), primary_key=True, autoincrement=True)
    user_id: Mapped[int] = mapped_column(
        BigInteger().with_variant(Integer, "sqlite"),
        ForeignKey("users.id", ondelete="CASCADE"),
        nullable=False,
        index=True,
    )
    display_name: Mapped[str] = mapped_column(String(255), nullable=False)
    funder_wallet_address: Mapped[Optional[str]] = mapped_column(String(64), nullable=True, index=True)
    signer_wallet_address: Mapped[str] = mapped_column(String(64), nullable=False, index=True)
    signature_type: Mapped[str] = mapped_column(String(32), nullable=False, default=SignatureType.POLY_GNOSIS_SAFE.value)

    operating_mode: Mapped[str] = mapped_column(String(32), nullable=False, default=OperatingMode.MONITORING.value)
    connection_status: Mapped[str] = mapped_column(String(32), nullable=False, default="CONNECTED")
    authorization_status: Mapped[str] = mapped_column(String(32), nullable=False, default="AUTHORIZED")
    trading_status: Mapped[str] = mapped_column(String(32), nullable=False, default="ACTIVE")

    live_trading_Confirmed_at: Mapped[Optional[datetime]] = mapped_column(DateTime(timezone=True), nullable=True)
    emergency_stop: Mapped[bool] = mapped_column(Boolean, default=False, nullable=False)
    emergency_stop_reason: Mapped[Optional[str]] = mapped_column(String(512), nullable=True)
    consecutive_errors: Mapped[int] = mapped_column(Integer, default=0, nullable=False)
    circuit_breaker_tripped: Mapped[bool] = mapped_column(Boolean, default=False, nullable=False)

    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow, nullable=False)
    updated_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow, onupdate=utcnow, nullable=False)

    user: Mapped["User"] = relationship("User", back_populates="trading_accounts")
    credentials: Mapped[Optional["TradingAccountCredential"]] = relationship(
        "TradingAccountCredential", back_populates="trading_account", uselist=False, cascade="all, delete-orphan"
    )
    risk_limits: Mapped[Optional["RiskLimit"]] = relationship(
        "RiskLimit", back_populates="trading_account", uselist=False, cascade="all, delete-orphan"
    )
    copy_strategies: Mapped[list["CopyStrategy"]] = relationship(
        "CopyStrategy", back_populates="trading_account", cascade="all, delete-orphan"
    )
    source_mappings: Mapped[list["AccountSourceTrader"]] = relationship(
        "AccountSourceTrader", back_populates="trading_account", cascade="all, delete-orphan"
    )


# ---------------------------------------------------------------------------
# 5. trading_account_credentials
# ---------------------------------------------------------------------------
class TradingAccountCredential(Base):
    __tablename__ = "trading_account_credentials"

    id: Mapped[int] = mapped_column(BigInteger().with_variant(Integer, "sqlite"), primary_key=True, autoincrement=True)
    user_id: Mapped[int] = mapped_column(
        BigInteger().with_variant(Integer, "sqlite"),
        ForeignKey("users.id", ondelete="CASCADE"),
        nullable=False,
        index=True,
    )
    trading_account_id: Mapped[int] = mapped_column(
        BigInteger().with_variant(Integer, "sqlite"),
        ForeignKey("trading_accounts.id", ondelete="CASCADE"),
        unique=True,
        nullable=False,
        index=True,
    )
    encrypted_private_key: Mapped[Optional[str]] = mapped_column(Text, nullable=True)
    encrypted_api_key: Mapped[Optional[str]] = mapped_column(Text, nullable=True)
    encrypted_api_secret: Mapped[Optional[str]] = mapped_column(Text, nullable=True)
    encrypted_api_passphrase: Mapped[Optional[str]] = mapped_column(Text, nullable=True)
    key_version: Mapped[int] = mapped_column(Integer, default=1, nullable=False)
    is_connected: Mapped[bool] = mapped_column(Boolean, default=True, nullable=False)
    rotated_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow, nullable=False)
    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow, nullable=False)

    trading_account: Mapped["TradingAccount"] = relationship("TradingAccount", back_populates="credentials")


# ---------------------------------------------------------------------------
# 6. source_traders
# ---------------------------------------------------------------------------
class SourceTrader(Base):
    __tablename__ = "source_traders"

    id: Mapped[int] = mapped_column(BigInteger().with_variant(Integer, "sqlite"), primary_key=True, autoincrement=True)
    wallet_address: Mapped[str] = mapped_column(String(64), unique=True, nullable=False, index=True)
    default_label: Mapped[Optional[str]] = mapped_column(String(255), nullable=True)
    is_monitored: Mapped[bool] = mapped_column(Boolean, default=True, nullable=False, index=True)
    total_observed_trades: Mapped[int] = mapped_column(Integer, default=0, nullable=False)
    total_observed_volume_usd: Mapped[Decimal] = mapped_column(Numeric(20, 6), default=Decimal("0"), nullable=False)
    estimated_balance_usd: Mapped[Decimal] = mapped_column(Numeric(20, 6), default=Decimal("10000"), nullable=False)
    win_rate_pct: Mapped[Optional[Decimal]] = mapped_column(Numeric(8, 4), nullable=True)
    realized_roi_pct: Mapped[Optional[Decimal]] = mapped_column(Numeric(10, 4), nullable=True)
    last_trade_at: Mapped[Optional[datetime]] = mapped_column(DateTime(timezone=True), nullable=True)
    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow, nullable=False)

    account_mappings: Mapped[list["AccountSourceTrader"]] = relationship(
        "AccountSourceTrader", back_populates="source_trader", cascade="all, delete-orphan"
    )


# ---------------------------------------------------------------------------
# 7. account_source_traders (Many-to-Many: 100,000+ relationships supported)
# ---------------------------------------------------------------------------
class AccountSourceTrader(Base):
    __tablename__ = "account_source_traders"
    __table_args__ = (
        UniqueConstraint("trading_account_id", "source_trader_id", name="uq_account_source_trader"),
        Index("ix_ast_source_trader_enabled", "source_trader_id", "is_enabled"),
        Index("ix_ast_user_account", "user_id", "trading_account_id"),
    )

    id: Mapped[int] = mapped_column(BigInteger().with_variant(Integer, "sqlite"), primary_key=True, autoincrement=True)
    user_id: Mapped[int] = mapped_column(
        BigInteger().with_variant(Integer, "sqlite"),
        ForeignKey("users.id", ondelete="CASCADE"),
        nullable=False,
        index=True,
    )
    trading_account_id: Mapped[int] = mapped_column(
        BigInteger().with_variant(Integer, "sqlite"),
        ForeignKey("trading_accounts.id", ondelete="CASCADE"),
        nullable=False,
        index=True,
    )
    source_trader_id: Mapped[int] = mapped_column(
        BigInteger().with_variant(Integer, "sqlite"),
        ForeignKey("source_traders.id", ondelete="CASCADE"),
        nullable=False,
        index=True,
    )
    custom_name: Mapped[Optional[str]] = mapped_column(String(255), nullable=True)
    is_enabled: Mapped[bool] = mapped_column(Boolean, default=True, nullable=False)

    # Per-source-trader override parameters (overrides account-level copy_strategies when set)
    execution_mode: Mapped[str] = mapped_column(String(32), default=ExecutionMode.FIXED_AMOUNT.value, nullable=False)
    fixed_amount_usd: Mapped[Decimal] = mapped_column(Numeric(18, 6), default=Decimal("10.0"), nullable=False)
    copy_percentage: Mapped[Decimal] = mapped_column(Numeric(8, 4), default=Decimal("0.10"), nullable=False)
    copy_multiplier: Mapped[Decimal] = mapped_column(Numeric(8, 4), default=Decimal("1.0"), nullable=False)
    max_order_size_usd: Mapped[Decimal] = mapped_column(Numeric(18, 6), default=Decimal("100.0"), nullable=False)
    min_order_size_usd: Mapped[Decimal] = mapped_column(Numeric(18, 6), default=Decimal("1.0"), nullable=False)
    max_exposure_usd: Mapped[Decimal] = mapped_column(Numeric(18, 6), default=Decimal("500.0"), nullable=False)
    max_slippage_pct: Mapped[Decimal] = mapped_column(Numeric(8, 4), default=Decimal("0.05"), nullable=False)
    copy_buy_enabled: Mapped[bool] = mapped_column(Boolean, default=True, nullable=False)
    copy_sell_enabled: Mapped[bool] = mapped_column(Boolean, default=True, nullable=False)

    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow, nullable=False)
    updated_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow, onupdate=utcnow, nullable=False)

    trading_account: Mapped["TradingAccount"] = relationship("TradingAccount", back_populates="source_mappings")
    source_trader: Mapped["SourceTrader"] = relationship("SourceTrader", back_populates="account_mappings")


# ---------------------------------------------------------------------------
# 8. copy_strategies (Account-level copy-trading strategy configuration)
# ---------------------------------------------------------------------------
class CopyStrategy(Base):
    __tablename__ = "copy_strategies"
    __table_args__ = (
        UniqueConstraint("trading_account_id", "name", name="uq_copy_strategies_account_name"),
    )

    id: Mapped[int] = mapped_column(BigInteger().with_variant(Integer, "sqlite"), primary_key=True, autoincrement=True)
    user_id: Mapped[int] = mapped_column(
        BigInteger().with_variant(Integer, "sqlite"),
        ForeignKey("users.id", ondelete="CASCADE"),
        nullable=False,
        index=True,
    )
    trading_account_id: Mapped[int] = mapped_column(
        BigInteger().with_variant(Integer, "sqlite"),
        ForeignKey("trading_accounts.id", ondelete="CASCADE"),
        nullable=False,
        index=True,
    )
    name: Mapped[str] = mapped_column(String(128), default="Default Strategy", nullable=False)
    is_active: Mapped[bool] = mapped_column(Boolean, default=True, nullable=False)

    execution_mode: Mapped[str] = mapped_column(String(32), default=ExecutionMode.FIXED_AMOUNT.value, nullable=False)
    fixed_amount_usd: Mapped[Decimal] = mapped_column(Numeric(18, 6), default=Decimal("10.0"), nullable=False)
    source_size_percentage: Mapped[Decimal] = mapped_column(Numeric(8, 4), default=Decimal("0.10"), nullable=False)
    copy_multiplier: Mapped[Decimal] = mapped_column(Numeric(8, 4), default=Decimal("1.0"), nullable=False)
    max_order_size_usd: Mapped[Decimal] = mapped_column(Numeric(18, 6), default=Decimal("100.0"), nullable=False)
    min_order_size_usd: Mapped[Decimal] = mapped_column(Numeric(18, 6), default=Decimal("1.0"), nullable=False)
    max_slippage_pct: Mapped[Decimal] = mapped_column(Numeric(8, 4), default=Decimal("0.05"), nullable=False)

    buy_enabled: Mapped[bool] = mapped_column(Boolean, default=True, nullable=False)
    sell_enabled: Mapped[bool] = mapped_column(Boolean, default=True, nullable=False)
    slice_collapse_window_secs: Mapped[int] = mapped_column(Integer, default=300, nullable=False)
    allowed_markets: Mapped[Optional[list[str]]] = mapped_column(JSON, nullable=True)

    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow, nullable=False)
    updated_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow, onupdate=utcnow, nullable=False)

    trading_account: Mapped["TradingAccount"] = relationship("TradingAccount", back_populates="copy_strategies")


# ---------------------------------------------------------------------------
# 9. risk_limits (Independent risk configuration per trading account)
# ---------------------------------------------------------------------------
class RiskLimit(Base):
    __tablename__ = "risk_limits"

    id: Mapped[int] = mapped_column(BigInteger().with_variant(Integer, "sqlite"), primary_key=True, autoincrement=True)
    user_id: Mapped[int] = mapped_column(
        BigInteger().with_variant(Integer, "sqlite"),
        ForeignKey("users.id", ondelete="CASCADE"),
        nullable=False,
        index=True,
    )
    trading_account_id: Mapped[int] = mapped_column(
        BigInteger().with_variant(Integer, "sqlite"),
        ForeignKey("trading_accounts.id", ondelete="CASCADE"),
        unique=True,
        nullable=False,
        index=True,
    )
    max_order_size_usd: Mapped[Decimal] = mapped_column(Numeric(18, 6), default=Decimal("100.0"), nullable=False)
    min_order_size_usd: Mapped[Decimal] = mapped_column(Numeric(18, 6), default=Decimal("1.0"), nullable=False)
    max_market_exposure_usd: Mapped[Decimal] = mapped_column(Numeric(18, 6), default=Decimal("500.0"), nullable=False)
    max_source_trader_exposure_usd: Mapped[Decimal] = mapped_column(Numeric(18, 6), default=Decimal("500.0"), nullable=False)
    max_total_exposure_usd: Mapped[Decimal] = mapped_column(Numeric(18, 6), default=Decimal("2500.0"), nullable=False)
    max_daily_loss_usd: Mapped[Decimal] = mapped_column(Numeric(18, 6), default=Decimal("200.0"), nullable=False)
    max_slippage_pct: Mapped[Decimal] = mapped_column(Numeric(8, 4), default=Decimal("0.05"), nullable=False)
    max_executions_per_interval: Mapped[int] = mapped_column(Integer, default=10, nullable=False)
    execution_interval_seconds: Mapped[int] = mapped_column(Integer, default=60, nullable=False)
    min_market_liquidity_usd: Mapped[Decimal] = mapped_column(Numeric(18, 6), default=Decimal("5.0"), nullable=False)
    max_stale_data_seconds: Mapped[int] = mapped_column(Integer, default=60, nullable=False)
    circuit_breaker_error_threshold: Mapped[int] = mapped_column(Integer, default=5, nullable=False)
    daily_loss_stop_latched: Mapped[bool] = mapped_column(Boolean, default=False, nullable=False)
    daily_loss_latched_date: Mapped[Optional[str]] = mapped_column(String(16), nullable=True)

    updated_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow, onupdate=utcnow, nullable=False)

    trading_account: Mapped["TradingAccount"] = relationship("TradingAccount", back_populates="risk_limits")


# ---------------------------------------------------------------------------
# 10. source_trade_events (Normalized public source trades with dedup constraint)
# ---------------------------------------------------------------------------
class SourceTradeEvent(Base):
    __tablename__ = "source_trade_events"
    __table_args__ = (
        UniqueConstraint(
            "tx_hash",
            "source_wallet",
            "token_id",
            "side",
            name="uq_source_trade_events_dedup",
        ),
        Index("ix_source_trade_events_wallet_ts", "source_wallet", "source_event_ts"),
    )

    id: Mapped[int] = mapped_column(BigInteger().with_variant(Integer, "sqlite"), primary_key=True, autoincrement=True)
    source_trader_id: Mapped[Optional[int]] = mapped_column(
        BigInteger().with_variant(Integer, "sqlite"),
        ForeignKey("source_traders.id", ondelete="SET NULL"),
        nullable=True,
        index=True,
    )
    source_wallet: Mapped[str] = mapped_column(String(64), nullable=False, index=True)
    market_condition_id: Mapped[str] = mapped_column(String(128), nullable=False, index=True)
    market_question: Mapped[Optional[str]] = mapped_column(String(512), nullable=True)
    outcome_label: Mapped[Optional[str]] = mapped_column(String(128), nullable=True)
    token_id: Mapped[str] = mapped_column(String(128), nullable=False, index=True)
    side: Mapped[str] = mapped_column(String(16), nullable=False)
    price: Mapped[Decimal] = mapped_column(Numeric(12, 6), nullable=False)
    size: Mapped[Decimal] = mapped_column(Numeric(20, 6), nullable=False)
    notional_usd: Mapped[Decimal] = mapped_column(Numeric(20, 6), nullable=False)
    tx_hash: Mapped[str] = mapped_column(String(128), nullable=False)
    feed_source: Mapped[str] = mapped_column(String(32), default="rtds", nullable=False)

    # Latency timestamps (Stages 1 - 4)
    source_event_ts: Mapped[datetime] = mapped_column(DateTime(timezone=True), nullable=False)
    event_received_ts: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow, nullable=False)
    event_normalized_ts: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow, nullable=False)
    event_routed_ts: Mapped[Optional[datetime]] = mapped_column(DateTime(timezone=True), nullable=True)


# ---------------------------------------------------------------------------
# 11. execution_jobs (Per-destination-account routed copy-trading job)
# ---------------------------------------------------------------------------
class ExecutionJob(Base):
    __tablename__ = "execution_jobs"
    __table_args__ = (
        UniqueConstraint(
            "trading_account_id",
            "source_trade_event_id",
            name="uq_execution_jobs_account_event",
        ),
        Index("ix_execution_jobs_user_status", "user_id", "status"),
        Index("ix_execution_jobs_account_created", "trading_account_id", "created_at"),
    )

    id: Mapped[int] = mapped_column(BigInteger().with_variant(Integer, "sqlite"), primary_key=True, autoincrement=True)
    user_id: Mapped[int] = mapped_column(
        BigInteger().with_variant(Integer, "sqlite"),
        ForeignKey("users.id", ondelete="CASCADE"),
        nullable=False,
        index=True,
    )
    trading_account_id: Mapped[int] = mapped_column(
        BigInteger().with_variant(Integer, "sqlite"),
        ForeignKey("trading_accounts.id", ondelete="CASCADE"),
        nullable=False,
        index=True,
    )
    source_trader_id: Mapped[int] = mapped_column(
        BigInteger().with_variant(Integer, "sqlite"),
        ForeignKey("source_traders.id", ondelete="CASCADE"),
        nullable=False,
        index=True,
    )
    source_trade_event_id: Mapped[int] = mapped_column(
        BigInteger().with_variant(Integer, "sqlite"),
        ForeignKey("source_trade_events.id", ondelete="CASCADE"),
        nullable=False,
        index=True,
    )
    idempotency_key: Mapped[str] = mapped_column(String(128), unique=True, nullable=False, index=True)
    operating_mode: Mapped[str] = mapped_column(String(32), nullable=False)
    status: Mapped[str] = mapped_column(String(32), default=ExecutionJobStatus.QUEUED.value, nullable=False)
    skip_or_error_reason: Mapped[Optional[str]] = mapped_column(String(512), nullable=True)

    target_notional_usd: Mapped[Decimal] = mapped_column(Numeric(18, 6), default=Decimal("0"), nullable=False)
    target_shares: Mapped[Decimal] = mapped_column(Numeric(18, 6), default=Decimal("0"), nullable=False)
    limit_price: Mapped[Optional[Decimal]] = mapped_column(Numeric(12, 6), nullable=True)
    attempts: Mapped[int] = mapped_column(Integer, default=0, nullable=False)

    routed_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow, nullable=False)
    started_at: Mapped[Optional[datetime]] = mapped_column(DateTime(timezone=True), nullable=True)
    completed_at: Mapped[Optional[datetime]] = mapped_column(DateTime(timezone=True), nullable=True)
    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow, nullable=False)


# ---------------------------------------------------------------------------
# 12. orders
# ---------------------------------------------------------------------------
class Order(Base):
    __tablename__ = "orders"
    __table_args__ = (
        Index("ix_orders_user_account_status", "user_id", "trading_account_id", "status"),
    )

    id: Mapped[int] = mapped_column(BigInteger().with_variant(Integer, "sqlite"), primary_key=True, autoincrement=True)
    user_id: Mapped[int] = mapped_column(
        BigInteger().with_variant(Integer, "sqlite"),
        ForeignKey("users.id", ondelete="CASCADE"),
        nullable=False,
        index=True,
    )
    trading_account_id: Mapped[int] = mapped_column(
        BigInteger().with_variant(Integer, "sqlite"),
        ForeignKey("trading_accounts.id", ondelete="CASCADE"),
        nullable=False,
        index=True,
    )
    source_trader_id: Mapped[Optional[int]] = mapped_column(
        BigInteger().with_variant(Integer, "sqlite"),
        ForeignKey("source_traders.id", ondelete="SET NULL"),
        nullable=True,
        index=True,
    )
    execution_job_id: Mapped[Optional[int]] = mapped_column(
        BigInteger().with_variant(Integer, "sqlite"),
        ForeignKey("execution_jobs.id", ondelete="SET NULL"),
        nullable=True,
        index=True,
    )
    exchange_order_id: Mapped[Optional[str]] = mapped_column(String(128), nullable=True, index=True)
    client_order_id: Mapped[Optional[str]] = mapped_column(String(128), unique=True, nullable=True, index=True)
    eip712_order_hash: Mapped[Optional[str]] = mapped_column(String(128), nullable=True, index=True)
    signed_payload_json: Mapped[Optional[dict]] = mapped_column(JSON, nullable=True)
    idempotency_key: Mapped[str] = mapped_column(String(128), unique=True, nullable=False, index=True)

    operating_mode: Mapped[str] = mapped_column(String(32), nullable=False)
    market_condition_id: Mapped[str] = mapped_column(String(128), nullable=False, index=True)
    token_id: Mapped[str] = mapped_column(String(128), nullable=False, index=True)
    side: Mapped[str] = mapped_column(String(16), nullable=False)
    order_type: Mapped[str] = mapped_column(String(16), default="FAK", nullable=False)

    limit_price: Mapped[Decimal] = mapped_column(Numeric(12, 6), nullable=False)
    requested_size_shares: Mapped[Decimal] = mapped_column(Numeric(20, 6), nullable=False)
    requested_notional_usd: Mapped[Decimal] = mapped_column(Numeric(20, 6), nullable=False)
    filled_size_shares: Mapped[Decimal] = mapped_column(Numeric(20, 6), default=Decimal("0"), nullable=False)

    status: Mapped[str] = mapped_column(String(32), default=OrderStatus.PENDING.value, nullable=False)
    error_message: Mapped[Optional[str]] = mapped_column(String(512), nullable=True)
    tx_hash: Mapped[Optional[str]] = mapped_column(String(128), nullable=True)

    # Latency timestamps (Stages 5 - 6)
    order_submitted_ts: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow, nullable=False)
    order_ack_ts: Mapped[Optional[datetime]] = mapped_column(DateTime(timezone=True), nullable=True)
    reconciled_at: Mapped[Optional[datetime]] = mapped_column(DateTime(timezone=True), nullable=True)


# ---------------------------------------------------------------------------
# 13. fills
# ---------------------------------------------------------------------------
class Fill(Base):
    __tablename__ = "fills"
    __table_args__ = (
        Index("ix_fills_user_account_ts", "user_id", "trading_account_id", "fill_ts"),
    )

    id: Mapped[int] = mapped_column(BigInteger().with_variant(Integer, "sqlite"), primary_key=True, autoincrement=True)
    user_id: Mapped[int] = mapped_column(
        BigInteger().with_variant(Integer, "sqlite"),
        ForeignKey("users.id", ondelete="CASCADE"),
        nullable=False,
        index=True,
    )
    trading_account_id: Mapped[int] = mapped_column(
        BigInteger().with_variant(Integer, "sqlite"),
        ForeignKey("trading_accounts.id", ondelete="CASCADE"),
        nullable=False,
        index=True,
    )
    order_id: Mapped[int] = mapped_column(
        BigInteger().with_variant(Integer, "sqlite"),
        ForeignKey("orders.id", ondelete="CASCADE"),
        nullable=False,
        index=True,
    )
    source_trader_id: Mapped[Optional[int]] = mapped_column(
        BigInteger().with_variant(Integer, "sqlite"),
        ForeignKey("source_traders.id", ondelete="SET NULL"),
        nullable=True,
        index=True,
    )
    exchange_trade_id: Mapped[Optional[str]] = mapped_column(String(128), nullable=True, index=True)
    operating_mode: Mapped[str] = mapped_column(String(32), nullable=False)
    token_id: Mapped[str] = mapped_column(String(128), nullable=False, index=True)
    side: Mapped[str] = mapped_column(String(16), nullable=False)

    filled_shares: Mapped[Decimal] = mapped_column(Numeric(20, 6), nullable=False)
    avg_fill_price: Mapped[Decimal] = mapped_column(Numeric(12, 6), nullable=False)
    notional_usd: Mapped[Decimal] = mapped_column(Numeric(20, 6), nullable=False)
    fee_usd: Mapped[Decimal] = mapped_column(Numeric(20, 6), default=Decimal("0"), nullable=False)
    realized_pnl_usd: Mapped[Decimal] = mapped_column(Numeric(20, 6), default=Decimal("0"), nullable=False)

    # Latency timestamp (Stage 7)
    fill_ts: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow, nullable=False)


# ---------------------------------------------------------------------------
# 14. virtual_positions (Strict source-attributed lots: account + source_wallet + token + mode)
# ---------------------------------------------------------------------------
class VirtualPosition(Base):
    __tablename__ = "virtual_positions"
    __table_args__ = (
        UniqueConstraint(
            "trading_account_id",
            "source_wallet",
            "token_id",
            "operating_mode",
            name="uq_virtual_positions_attribution_lot",
        ),
        CheckConstraint("open_shares >= 0", name="ck_virtual_positions_non_negative_shares"),
        Index("ix_virtual_positions_account_mode", "trading_account_id", "operating_mode"),
    )

    id: Mapped[int] = mapped_column(BigInteger().with_variant(Integer, "sqlite"), primary_key=True, autoincrement=True)
    user_id: Mapped[int] = mapped_column(
        BigInteger().with_variant(Integer, "sqlite"),
        ForeignKey("users.id", ondelete="CASCADE"),
        nullable=False,
        index=True,
    )
    trading_account_id: Mapped[int] = mapped_column(
        BigInteger().with_variant(Integer, "sqlite"),
        ForeignKey("trading_accounts.id", ondelete="CASCADE"),
        nullable=False,
        index=True,
    )
    source_trader_id: Mapped[int] = mapped_column(
        BigInteger().with_variant(Integer, "sqlite"),
        ForeignKey("source_traders.id", ondelete="CASCADE"),
        nullable=False,
        index=True,
    )
    source_wallet: Mapped[str] = mapped_column(String(64), nullable=False, index=True)
    market_condition_id: Mapped[str] = mapped_column(String(128), nullable=False, index=True)
    token_id: Mapped[str] = mapped_column(String(128), nullable=False, index=True)
    operating_mode: Mapped[str] = mapped_column(String(32), nullable=False)

    open_shares: Mapped[Decimal] = mapped_column(Numeric(20, 6), default=Decimal("0"), nullable=False)
    total_bought_shares: Mapped[Decimal] = mapped_column(Numeric(20, 6), default=Decimal("0"), nullable=False)
    total_sold_shares: Mapped[Decimal] = mapped_column(Numeric(20, 6), default=Decimal("0"), nullable=False)
    avg_entry_price: Mapped[Decimal] = mapped_column(Numeric(12, 6), default=Decimal("0"), nullable=False)
    cost_basis_usd: Mapped[Decimal] = mapped_column(Numeric(20, 6), default=Decimal("0"), nullable=False)
    realized_pnl_usd: Mapped[Decimal] = mapped_column(Numeric(20, 6), default=Decimal("0"), nullable=False)
    total_fees_usd: Mapped[Decimal] = mapped_column(Numeric(20, 6), default=Decimal("0"), nullable=False)
    source_observed_shares: Mapped[Decimal] = mapped_column(Numeric(20, 6), default=Decimal("0"), nullable=False)
    pending_exit_shares: Mapped[Decimal] = mapped_column(Numeric(20, 6), default=Decimal("0"), nullable=False)

    opened_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow, nullable=False)
    updated_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow, onupdate=utcnow, nullable=False)
    closed_at: Mapped[Optional[datetime]] = mapped_column(DateTime(timezone=True), nullable=True)


# ---------------------------------------------------------------------------
# 15. account_balances
# ---------------------------------------------------------------------------
class AccountBalance(Base):
    __tablename__ = "account_balances"
    __table_args__ = (
        UniqueConstraint("trading_account_id", "operating_mode", name="uq_account_balances_account_mode"),
    )

    id: Mapped[int] = mapped_column(BigInteger().with_variant(Integer, "sqlite"), primary_key=True, autoincrement=True)
    user_id: Mapped[int] = mapped_column(
        BigInteger().with_variant(Integer, "sqlite"),
        ForeignKey("users.id", ondelete="CASCADE"),
        nullable=False,
        index=True,
    )
    trading_account_id: Mapped[int] = mapped_column(
        BigInteger().with_variant(Integer, "sqlite"),
        ForeignKey("trading_accounts.id", ondelete="CASCADE"),
        nullable=False,
        index=True,
    )
    operating_mode: Mapped[str] = mapped_column(String(32), nullable=False)
    available_cash_usd: Mapped[Decimal] = mapped_column(Numeric(20, 6), default=Decimal("1000.0"), nullable=False)
    locked_cash_usd: Mapped[Decimal] = mapped_column(Numeric(20, 6), default=Decimal("0.0"), nullable=False)
    positions_market_value_usd: Mapped[Decimal] = mapped_column(Numeric(20, 6), default=Decimal("0.0"), nullable=False)
    total_equity_usd: Mapped[Decimal] = mapped_column(Numeric(20, 6), default=Decimal("1000.0"), nullable=False)
    daily_realized_pnl_usd: Mapped[Decimal] = mapped_column(Numeric(20, 6), default=Decimal("0.0"), nullable=False)
    weekly_realized_pnl_usd: Mapped[Decimal] = mapped_column(Numeric(20, 6), default=Decimal("0.0"), nullable=False)
    monthly_realized_pnl_usd: Mapped[Decimal] = mapped_column(Numeric(20, 6), default=Decimal("0.0"), nullable=False)
    all_time_realized_pnl_usd: Mapped[Decimal] = mapped_column(Numeric(20, 6), default=Decimal("0.0"), nullable=False)
    updated_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow, onupdate=utcnow, nullable=False)


# ---------------------------------------------------------------------------
# 16. account_activity_logs
# ---------------------------------------------------------------------------
class AccountActivityLog(Base):
    __tablename__ = "account_activity_logs"
    __table_args__ = (
        Index("ix_activity_logs_user_created", "user_id", "created_at"),
    )

    id: Mapped[int] = mapped_column(BigInteger().with_variant(Integer, "sqlite"), primary_key=True, autoincrement=True)
    user_id: Mapped[int] = mapped_column(
        BigInteger().with_variant(Integer, "sqlite"),
        ForeignKey("users.id", ondelete="CASCADE"),
        nullable=False,
        index=True,
    )
    trading_account_id: Mapped[Optional[int]] = mapped_column(
        BigInteger().with_variant(Integer, "sqlite"),
        ForeignKey("trading_accounts.id", ondelete="SET NULL"),
        nullable=True,
        index=True,
    )
    action: Mapped[str] = mapped_column(String(128), nullable=False)
    details_json: Mapped[Optional[dict]] = mapped_column(JSON, nullable=True)
    ip_address: Mapped[Optional[str]] = mapped_column(String(64), nullable=True)
    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow, nullable=False)


# ---------------------------------------------------------------------------
# 17. system_settings
# ---------------------------------------------------------------------------
class SystemSetting(Base):
    __tablename__ = "system_settings"

    id: Mapped[int] = mapped_column(BigInteger().with_variant(Integer, "sqlite"), primary_key=True, autoincrement=True)
    key: Mapped[str] = mapped_column(String(128), unique=True, nullable=False, index=True)
    value_json: Mapped[dict] = mapped_column(JSON, nullable=False)
    description: Mapped[Optional[str]] = mapped_column(String(512), nullable=True)
    updated_by_user_id: Mapped[Optional[int]] = mapped_column(
        BigInteger().with_variant(Integer, "sqlite"),
        ForeignKey("users.id", ondelete="SET NULL"),
        nullable=True,
    )
    updated_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow, onupdate=utcnow, nullable=False)
