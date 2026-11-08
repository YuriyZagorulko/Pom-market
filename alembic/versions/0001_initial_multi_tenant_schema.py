"""Initial multi-tenant schema and PostgreSQL Row-Level Security (RLS) policies.

Revision ID: 0001_initial_schema
Revises: None
Create Date: 2026-11-08 15:15:00

"""
from typing import Sequence, Union

from alembic import op
import sqlalchemy as sa

revision: str = "0001_initial_schema"
down_revision: Union[str, None] = None
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None

TENANT_TABLES = [
    "trading_accounts",
    "trading_account_credentials",
    "account_source_traders",
    "copy_strategies",
    "risk_limits",
    "execution_jobs",
    "orders",
    "fills",
    "virtual_positions",
    "account_balances",
    "account_activity_logs",
]


def upgrade() -> None:
    # 1. users
    op.create_table(
        "users",
        sa.Column("id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), primary_key=True, autoincrement=True),
        sa.Column("email", sa.String(255), nullable=False),
        sa.Column("password_hash", sa.String(512), nullable=False),
        sa.Column("full_name", sa.String(255), nullable=True),
        sa.Column("is_active", sa.Boolean(), nullable=False, server_default=sa.text("true")),
        sa.Column("is_suspended", sa.Boolean(), nullable=False, server_default=sa.text("false")),
        sa.Column("suspension_reason", sa.String(512), nullable=True),
        sa.Column("email_verified", sa.Boolean(), nullable=False, server_default=sa.text("false")),
        sa.Column("email_verification_token_hash", sa.String(128), nullable=True),
        sa.Column("email_verification_expires_at", sa.DateTime(timezone=True), nullable=True),
        sa.Column("password_reset_token_hash", sa.String(128), nullable=True),
        sa.Column("password_reset_expires_at", sa.DateTime(timezone=True), nullable=True),
        sa.Column("totp_enabled", sa.Boolean(), nullable=False, server_default=sa.text("false")),
        sa.Column("totp_secret_encrypted", sa.Text(), nullable=True),
        sa.Column("created_at", sa.DateTime(timezone=True), nullable=False),
        sa.Column("updated_at", sa.DateTime(timezone=True), nullable=False),
        sa.UniqueConstraint("email", name="uq_users_email"),
    )
    op.create_index("ix_users_email", "users", ["email"])

    # 2. user_roles
    op.create_table(
        "user_roles",
        sa.Column("id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), primary_key=True, autoincrement=True),
        sa.Column("user_id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), sa.ForeignKey("users.id", ondelete="CASCADE"), nullable=False),
        sa.Column("role_name", sa.String(64), nullable=False),
        sa.Column("granted_at", sa.DateTime(timezone=True), nullable=False),
        sa.UniqueConstraint("user_id", "role_name", name="uq_user_roles_user_role"),
    )
    op.create_index("ix_user_roles_user_id", "user_roles", ["user_id"])

    # 3. user_sessions
    op.create_table(
        "user_sessions",
        sa.Column("id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), primary_key=True, autoincrement=True),
        sa.Column("user_id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), sa.ForeignKey("users.id", ondelete="CASCADE"), nullable=False),
        sa.Column("refresh_token_hash", sa.String(128), nullable=False),
        sa.Column("user_agent", sa.String(512), nullable=True),
        sa.Column("ip_address", sa.String(64), nullable=True),
        sa.Column("is_revoked", sa.Boolean(), nullable=False, server_default=sa.text("false")),
        sa.Column("expires_at", sa.DateTime(timezone=True), nullable=False),
        sa.Column("created_at", sa.DateTime(timezone=True), nullable=False),
        sa.Column("last_used_at", sa.DateTime(timezone=True), nullable=False),
        sa.UniqueConstraint("refresh_token_hash", name="uq_user_sessions_refresh_token_hash"),
    )
    op.create_index("ix_user_sessions_user_id", "user_sessions", ["user_id"])
    op.create_index("ix_user_sessions_refresh_token_hash", "user_sessions", ["refresh_token_hash"])

    # 4. trading_accounts
    op.create_table(
        "trading_accounts",
        sa.Column("id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), primary_key=True, autoincrement=True),
        sa.Column("user_id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), sa.ForeignKey("users.id", ondelete="CASCADE"), nullable=False),
        sa.Column("display_name", sa.String(255), nullable=False),
        sa.Column("funder_wallet_address", sa.String(64), nullable=True),
        sa.Column("signer_wallet_address", sa.String(64), nullable=False),
        sa.Column("signature_type", sa.String(32), nullable=False),
        sa.Column("operating_mode", sa.String(32), nullable=False),
        sa.Column("connection_status", sa.String(32), nullable=False),
        sa.Column("authorization_status", sa.String(32), nullable=False),
        sa.Column("trading_status", sa.String(32), nullable=False),
        sa.Column("live_trading_Confirmed_at", sa.DateTime(timezone=True), nullable=True),
        sa.Column("emergency_stop", sa.Boolean(), nullable=False, server_default=sa.text("false")),
        sa.Column("emergency_stop_reason", sa.String(512), nullable=True),
        sa.Column("consecutive_errors", sa.Integer(), nullable=False, server_default="0"),
        sa.Column("circuit_breaker_tripped", sa.Boolean(), nullable=False, server_default=sa.text("false")),
        sa.Column("created_at", sa.DateTime(timezone=True), nullable=False),
        sa.Column("updated_at", sa.DateTime(timezone=True), nullable=False),
    )
    op.create_index("ix_trading_accounts_user_id", "trading_accounts", ["user_id"])
    op.create_index("ix_trading_accounts_funder_wallet_address", "trading_accounts", ["funder_wallet_address"])
    op.create_index("ix_trading_accounts_signer_wallet_address", "trading_accounts", ["signer_wallet_address"])
    op.create_index("ix_trading_accounts_user_status", "trading_accounts", ["user_id", "trading_status"])

    # 5. trading_account_credentials
    op.create_table(
        "trading_account_credentials",
        sa.Column("id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), primary_key=True, autoincrement=True),
        sa.Column("user_id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), sa.ForeignKey("users.id", ondelete="CASCADE"), nullable=False),
        sa.Column("trading_account_id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), sa.ForeignKey("trading_accounts.id", ondelete="CASCADE"), nullable=False),
        sa.Column("encrypted_private_key", sa.Text(), nullable=True),
        sa.Column("encrypted_api_key", sa.Text(), nullable=True),
        sa.Column("encrypted_api_secret", sa.Text(), nullable=True),
        sa.Column("encrypted_api_passphrase", sa.Text(), nullable=True),
        sa.Column("key_version", sa.Integer(), nullable=False, server_default="1"),
        sa.Column("is_connected", sa.Boolean(), nullable=False, server_default=sa.text("true")),
        sa.Column("rotated_at", sa.DateTime(timezone=True), nullable=False),
        sa.Column("created_at", sa.DateTime(timezone=True), nullable=False),
        sa.UniqueConstraint("trading_account_id", name="uq_trading_account_credentials_account"),
    )
    op.create_index("ix_trading_account_credentials_user_id", "trading_account_credentials", ["user_id"])
    op.create_index("ix_trading_account_credentials_trading_account_id", "trading_account_credentials", ["trading_account_id"])

    # 6. source_traders
    op.create_table(
        "source_traders",
        sa.Column("id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), primary_key=True, autoincrement=True),
        sa.Column("wallet_address", sa.String(64), nullable=False),
        sa.Column("default_label", sa.String(255), nullable=True),
        sa.Column("is_monitored", sa.Boolean(), nullable=False, server_default=sa.text("true")),
        sa.Column("total_observed_trades", sa.Integer(), nullable=False, server_default="0"),
        sa.Column("total_observed_volume_usd", sa.Numeric(20, 6), nullable=False, server_default="0"),
        sa.Column("estimated_balance_usd", sa.Numeric(20, 6), nullable=False, server_default="10000"),
        sa.Column("win_rate_pct", sa.Numeric(8, 4), nullable=True),
        sa.Column("realized_roi_pct", sa.Numeric(10, 4), nullable=True),
        sa.Column("last_trade_at", sa.DateTime(timezone=True), nullable=True),
        sa.Column("created_at", sa.DateTime(timezone=True), nullable=False),
        sa.UniqueConstraint("wallet_address", name="uq_source_traders_wallet_address"),
    )
    op.create_index("ix_source_traders_wallet_address", "source_traders", ["wallet_address"])
    op.create_index("ix_source_traders_is_monitored", "source_traders", ["is_monitored"])

    # 7. account_source_traders
    op.create_table(
        "account_source_traders",
        sa.Column("id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), primary_key=True, autoincrement=True),
        sa.Column("user_id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), sa.ForeignKey("users.id", ondelete="CASCADE"), nullable=False),
        sa.Column("trading_account_id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), sa.ForeignKey("trading_accounts.id", ondelete="CASCADE"), nullable=False),
        sa.Column("source_trader_id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), sa.ForeignKey("source_traders.id", ondelete="CASCADE"), nullable=False),
        sa.Column("custom_name", sa.String(255), nullable=True),
        sa.Column("is_enabled", sa.Boolean(), nullable=False, server_default=sa.text("true")),
        sa.Column("execution_mode", sa.String(32), nullable=False),
        sa.Column("fixed_amount_usd", sa.Numeric(18, 6), nullable=False),
        sa.Column("copy_percentage", sa.Numeric(8, 4), nullable=False),
        sa.Column("copy_multiplier", sa.Numeric(8, 4), nullable=False),
        sa.Column("max_order_size_usd", sa.Numeric(18, 6), nullable=False),
        sa.Column("min_order_size_usd", sa.Numeric(18, 6), nullable=False),
        sa.Column("max_exposure_usd", sa.Numeric(18, 6), nullable=False),
        sa.Column("max_slippage_pct", sa.Numeric(8, 4), nullable=False),
        sa.Column("copy_buy_enabled", sa.Boolean(), nullable=False, server_default=sa.text("true")),
        sa.Column("copy_sell_enabled", sa.Boolean(), nullable=False, server_default=sa.text("true")),
        sa.Column("created_at", sa.DateTime(timezone=True), nullable=False),
        sa.Column("updated_at", sa.DateTime(timezone=True), nullable=False),
        sa.UniqueConstraint("trading_account_id", "source_trader_id", name="uq_account_source_trader"),
    )
    op.create_index("ix_account_source_traders_user_id", "account_source_traders", ["user_id"])
    op.create_index("ix_account_source_traders_trading_account_id", "account_source_traders", ["trading_account_id"])
    op.create_index("ix_account_source_traders_source_trader_id", "account_source_traders", ["source_trader_id"])
    op.create_index("ix_ast_source_trader_enabled", "account_source_traders", ["source_trader_id", "is_enabled"])
    op.create_index("ix_ast_user_account", "account_source_traders", ["user_id", "trading_account_id"])

    # 8. copy_strategies
    op.create_table(
        "copy_strategies",
        sa.Column("id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), primary_key=True, autoincrement=True),
        sa.Column("user_id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), sa.ForeignKey("users.id", ondelete="CASCADE"), nullable=False),
        sa.Column("trading_account_id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), sa.ForeignKey("trading_accounts.id", ondelete="CASCADE"), nullable=False),
        sa.Column("name", sa.String(128), nullable=False),
        sa.Column("is_active", sa.Boolean(), nullable=False, server_default=sa.text("true")),
        sa.Column("execution_mode", sa.String(32), nullable=False),
        sa.Column("fixed_amount_usd", sa.Numeric(18, 6), nullable=False),
        sa.Column("source_size_percentage", sa.Numeric(8, 4), nullable=False),
        sa.Column("copy_multiplier", sa.Numeric(8, 4), nullable=False),
        sa.Column("max_order_size_usd", sa.Numeric(18, 6), nullable=False),
        sa.Column("min_order_size_usd", sa.Numeric(18, 6), nullable=False),
        sa.Column("max_slippage_pct", sa.Numeric(8, 4), nullable=False),
        sa.Column("buy_enabled", sa.Boolean(), nullable=False, server_default=sa.text("true")),
        sa.Column("sell_enabled", sa.Boolean(), nullable=False, server_default=sa.text("true")),
        sa.Column("slice_collapse_window_secs", sa.Integer(), nullable=False, server_default="300"),
        sa.Column("allowed_markets", sa.JSON(), nullable=True),
        sa.Column("created_at", sa.DateTime(timezone=True), nullable=False),
        sa.Column("updated_at", sa.DateTime(timezone=True), nullable=False),
        sa.UniqueConstraint("trading_account_id", "name", name="uq_copy_strategies_account_name"),
    )
    op.create_index("ix_copy_strategies_user_id", "copy_strategies", ["user_id"])
    op.create_index("ix_copy_strategies_trading_account_id", "copy_strategies", ["trading_account_id"])

    # 9. risk_limits
    op.create_table(
        "risk_limits",
        sa.Column("id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), primary_key=True, autoincrement=True),
        sa.Column("user_id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), sa.ForeignKey("users.id", ondelete="CASCADE"), nullable=False),
        sa.Column("trading_account_id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), sa.ForeignKey("trading_accounts.id", ondelete="CASCADE"), nullable=False),
        sa.Column("max_order_size_usd", sa.Numeric(18, 6), nullable=False),
        sa.Column("min_order_size_usd", sa.Numeric(18, 6), nullable=False),
        sa.Column("max_market_exposure_usd", sa.Numeric(18, 6), nullable=False),
        sa.Column("max_source_trader_exposure_usd", sa.Numeric(18, 6), nullable=False),
        sa.Column("max_total_exposure_usd", sa.Numeric(18, 6), nullable=False),
        sa.Column("max_daily_loss_usd", sa.Numeric(18, 6), nullable=False),
        sa.Column("max_slippage_pct", sa.Numeric(8, 4), nullable=False),
        sa.Column("max_executions_per_interval", sa.Integer(), nullable=False),
        sa.Column("execution_interval_seconds", sa.Integer(), nullable=False),
        sa.Column("min_market_liquidity_usd", sa.Numeric(18, 6), nullable=False),
        sa.Column("max_stale_data_seconds", sa.Integer(), nullable=False),
        sa.Column("circuit_breaker_error_threshold", sa.Integer(), nullable=False),
        sa.Column("daily_loss_stop_latched", sa.Boolean(), nullable=False, server_default=sa.text("false")),
        sa.Column("daily_loss_latched_date", sa.String(16), nullable=True),
        sa.Column("updated_at", sa.DateTime(timezone=True), nullable=False),
        sa.UniqueConstraint("trading_account_id", name="uq_risk_limits_trading_account_id"),
    )
    op.create_index("ix_risk_limits_user_id", "risk_limits", ["user_id"])
    op.create_index("ix_risk_limits_trading_account_id", "risk_limits", ["trading_account_id"])

    # 10. source_trade_events
    op.create_table(
        "source_trade_events",
        sa.Column("id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), primary_key=True, autoincrement=True),
        sa.Column("source_trader_id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), sa.ForeignKey("source_traders.id", ondelete="SET NULL"), nullable=True),
        sa.Column("source_wallet", sa.String(64), nullable=False),
        sa.Column("market_condition_id", sa.String(128), nullable=False),
        sa.Column("market_question", sa.String(512), nullable=True),
        sa.Column("outcome_label", sa.String(128), nullable=True),
        sa.Column("token_id", sa.String(128), nullable=False),
        sa.Column("side", sa.String(16), nullable=False),
        sa.Column("price", sa.Numeric(12, 6), nullable=False),
        sa.Column("size", sa.Numeric(20, 6), nullable=False),
        sa.Column("notional_usd", sa.Numeric(20, 6), nullable=False),
        sa.Column("tx_hash", sa.String(128), nullable=False),
        sa.Column("feed_source", sa.String(32), nullable=False),
        sa.Column("source_event_ts", sa.DateTime(timezone=True), nullable=False),
        sa.Column("event_received_ts", sa.DateTime(timezone=True), nullable=False),
        sa.Column("event_normalized_ts", sa.DateTime(timezone=True), nullable=False),
        sa.Column("event_routed_ts", sa.DateTime(timezone=True), nullable=True),
        sa.UniqueConstraint("tx_hash", "source_wallet", "token_id", "side", name="uq_source_trade_events_dedup"),
    )
    op.create_index("ix_source_trade_events_source_trader_id", "source_trade_events", ["source_trader_id"])
    op.create_index("ix_source_trade_events_source_wallet", "source_trade_events", ["source_wallet"])
    op.create_index("ix_source_trade_events_market_condition_id", "source_trade_events", ["market_condition_id"])
    op.create_index("ix_source_trade_events_token_id", "source_trade_events", ["token_id"])
    op.create_index("ix_source_trade_events_wallet_ts", "source_trade_events", ["source_wallet", "source_event_ts"])

    # 11. execution_jobs
    op.create_table(
        "execution_jobs",
        sa.Column("id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), primary_key=True, autoincrement=True),
        sa.Column("user_id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), sa.ForeignKey("users.id", ondelete="CASCADE"), nullable=False),
        sa.Column("trading_account_id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), sa.ForeignKey("trading_accounts.id", ondelete="CASCADE"), nullable=False),
        sa.Column("source_trader_id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), sa.ForeignKey("source_traders.id", ondelete="CASCADE"), nullable=False),
        sa.Column("source_trade_event_id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), sa.ForeignKey("source_trade_events.id", ondelete="CASCADE"), nullable=False),
        sa.Column("idempotency_key", sa.String(128), nullable=False),
        sa.Column("operating_mode", sa.String(32), nullable=False),
        sa.Column("status", sa.String(32), nullable=False),
        sa.Column("skip_or_error_reason", sa.String(512), nullable=True),
        sa.Column("target_notional_usd", sa.Numeric(18, 6), nullable=False),
        sa.Column("target_shares", sa.Numeric(18, 6), nullable=False),
        sa.Column("limit_price", sa.Numeric(12, 6), nullable=True),
        sa.Column("attempts", sa.Integer(), nullable=False, server_default="0"),
        sa.Column("routed_at", sa.DateTime(timezone=True), nullable=False),
        sa.Column("started_at", sa.DateTime(timezone=True), nullable=True),
        sa.Column("completed_at", sa.DateTime(timezone=True), nullable=True),
        sa.Column("created_at", sa.DateTime(timezone=True), nullable=False),
        sa.UniqueConstraint("trading_account_id", "source_trade_event_id", name="uq_execution_jobs_account_event"),
        sa.UniqueConstraint("idempotency_key", name="uq_execution_jobs_idempotency_key"),
    )
    op.create_index("ix_execution_jobs_user_id", "execution_jobs", ["user_id"])
    op.create_index("ix_execution_jobs_trading_account_id", "execution_jobs", ["trading_account_id"])
    op.create_index("ix_execution_jobs_source_trader_id", "execution_jobs", ["source_trader_id"])
    op.create_index("ix_execution_jobs_source_trade_event_id", "execution_jobs", ["source_trade_event_id"])
    op.create_index("ix_execution_jobs_idempotency_key", "execution_jobs", ["idempotency_key"])
    op.create_index("ix_execution_jobs_user_status", "execution_jobs", ["user_id", "status"])
    op.create_index("ix_execution_jobs_account_created", "execution_jobs", ["trading_account_id", "created_at"])

    # 12. orders
    op.create_table(
        "orders",
        sa.Column("id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), primary_key=True, autoincrement=True),
        sa.Column("user_id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), sa.ForeignKey("users.id", ondelete="CASCADE"), nullable=False),
        sa.Column("trading_account_id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), sa.ForeignKey("trading_accounts.id", ondelete="CASCADE"), nullable=False),
        sa.Column("source_trader_id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), sa.ForeignKey("source_traders.id", ondelete="SET NULL"), nullable=True),
        sa.Column("execution_job_id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), sa.ForeignKey("execution_jobs.id", ondelete="SET NULL"), nullable=True),
        sa.Column("exchange_order_id", sa.String(128), nullable=True),
        sa.Column("idempotency_key", sa.String(128), nullable=False),
        sa.Column("operating_mode", sa.String(32), nullable=False),
        sa.Column("market_condition_id", sa.String(128), nullable=False),
        sa.Column("token_id", sa.String(128), nullable=False),
        sa.Column("side", sa.String(16), nullable=False),
        sa.Column("order_type", sa.String(16), nullable=False),
        sa.Column("limit_price", sa.Numeric(12, 6), nullable=False),
        sa.Column("requested_size_shares", sa.Numeric(20, 6), nullable=False),
        sa.Column("requested_notional_usd", sa.Numeric(20, 6), nullable=False),
        sa.Column("filled_size_shares", sa.Numeric(20, 6), nullable=False, server_default="0"),
        sa.Column("status", sa.String(32), nullable=False),
        sa.Column("error_message", sa.String(512), nullable=True),
        sa.Column("tx_hash", sa.String(128), nullable=True),
        sa.Column("order_submitted_ts", sa.DateTime(timezone=True), nullable=False),
        sa.Column("order_ack_ts", sa.DateTime(timezone=True), nullable=True),
        sa.Column("reconciled_at", sa.DateTime(timezone=True), nullable=True),
        sa.UniqueConstraint("idempotency_key", name="uq_orders_idempotency_key"),
    )
    op.create_index("ix_orders_user_id", "orders", ["user_id"])
    op.create_index("ix_orders_trading_account_id", "orders", ["trading_account_id"])
    op.create_index("ix_orders_source_trader_id", "orders", ["source_trader_id"])
    op.create_index("ix_orders_execution_job_id", "orders", ["execution_job_id"])
    op.create_index("ix_orders_exchange_order_id", "orders", ["exchange_order_id"])
    op.create_index("ix_orders_idempotency_key", "orders", ["idempotency_key"])
    op.create_index("ix_orders_market_condition_id", "orders", ["market_condition_id"])
    op.create_index("ix_orders_token_id", "orders", ["token_id"])
    op.create_index("ix_orders_user_account_status", "orders", ["user_id", "trading_account_id", "status"])

    # 13. fills
    op.create_table(
        "fills",
        sa.Column("id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), primary_key=True, autoincrement=True),
        sa.Column("user_id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), sa.ForeignKey("users.id", ondelete="CASCADE"), nullable=False),
        sa.Column("trading_account_id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), sa.ForeignKey("trading_accounts.id", ondelete="CASCADE"), nullable=False),
        sa.Column("order_id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), sa.ForeignKey("orders.id", ondelete="CASCADE"), nullable=False),
        sa.Column("source_trader_id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), sa.ForeignKey("source_traders.id", ondelete="SET NULL"), nullable=True),
        sa.Column("exchange_trade_id", sa.String(128), nullable=True),
        sa.Column("operating_mode", sa.String(32), nullable=False),
        sa.Column("token_id", sa.String(128), nullable=False),
        sa.Column("side", sa.String(16), nullable=False),
        sa.Column("filled_shares", sa.Numeric(20, 6), nullable=False),
        sa.Column("avg_fill_price", sa.Numeric(12, 6), nullable=False),
        sa.Column("notional_usd", sa.Numeric(20, 6), nullable=False),
        sa.Column("fee_usd", sa.Numeric(20, 6), nullable=False, server_default="0"),
        sa.Column("realized_pnl_usd", sa.Numeric(20, 6), nullable=False, server_default="0"),
        sa.Column("fill_ts", sa.DateTime(timezone=True), nullable=False),
    )
    op.create_index("ix_fills_user_id", "fills", ["user_id"])
    op.create_index("ix_fills_trading_account_id", "fills", ["trading_account_id"])
    op.create_index("ix_fills_order_id", "fills", ["order_id"])
    op.create_index("ix_fills_source_trader_id", "fills", ["source_trader_id"])
    op.create_index("ix_fills_exchange_trade_id", "fills", ["exchange_trade_id"])
    op.create_index("ix_fills_token_id", "fills", ["token_id"])
    op.create_index("ix_fills_user_account_ts", "fills", ["user_id", "trading_account_id", "fill_ts"])

    # 14. virtual_positions
    op.create_table(
        "virtual_positions",
        sa.Column("id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), primary_key=True, autoincrement=True),
        sa.Column("user_id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), sa.ForeignKey("users.id", ondelete="CASCADE"), nullable=False),
        sa.Column("trading_account_id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), sa.ForeignKey("trading_accounts.id", ondelete="CASCADE"), nullable=False),
        sa.Column("source_trader_id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), sa.ForeignKey("source_traders.id", ondelete="CASCADE"), nullable=False),
        sa.Column("source_wallet", sa.String(64), nullable=False),
        sa.Column("market_condition_id", sa.String(128), nullable=False),
        sa.Column("token_id", sa.String(128), nullable=False),
        sa.Column("operating_mode", sa.String(32), nullable=False),
        sa.Column("open_shares", sa.Numeric(20, 6), nullable=False, server_default="0"),
        sa.Column("total_bought_shares", sa.Numeric(20, 6), nullable=False, server_default="0"),
        sa.Column("total_sold_shares", sa.Numeric(20, 6), nullable=False, server_default="0"),
        sa.Column("avg_entry_price", sa.Numeric(12, 6), nullable=False, server_default="0"),
        sa.Column("cost_basis_usd", sa.Numeric(20, 6), nullable=False, server_default="0"),
        sa.Column("realized_pnl_usd", sa.Numeric(20, 6), nullable=False, server_default="0"),
        sa.Column("total_fees_usd", sa.Numeric(20, 6), nullable=False, server_default="0"),
        sa.Column("source_observed_shares", sa.Numeric(20, 6), nullable=False, server_default="0"),
        sa.Column("pending_exit_shares", sa.Numeric(20, 6), nullable=False, server_default="0"),
        sa.Column("opened_at", sa.DateTime(timezone=True), nullable=False),
        sa.Column("updated_at", sa.DateTime(timezone=True), nullable=False),
        sa.Column("closed_at", sa.DateTime(timezone=True), nullable=True),
        sa.UniqueConstraint(
            "trading_account_id",
            "source_wallet",
            "token_id",
            "operating_mode",
            name="uq_virtual_positions_attribution_lot",
        ),
        sa.CheckConstraint("open_shares >= 0", name="ck_virtual_positions_non_negative_shares"),
    )
    op.create_index("ix_virtual_positions_user_id", "virtual_positions", ["user_id"])
    op.create_index("ix_virtual_positions_trading_account_id", "virtual_positions", ["trading_account_id"])
    op.create_index("ix_virtual_positions_source_trader_id", "virtual_positions", ["source_trader_id"])
    op.create_index("ix_virtual_positions_source_wallet", "virtual_positions", ["source_wallet"])
    op.create_index("ix_virtual_positions_market_condition_id", "virtual_positions", ["market_condition_id"])
    op.create_index("ix_virtual_positions_token_id", "virtual_positions", ["token_id"])
    op.create_index("ix_virtual_positions_account_mode", "virtual_positions", ["trading_account_id", "operating_mode"])

    # 15. account_balances
    op.create_table(
        "account_balances",
        sa.Column("id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), primary_key=True, autoincrement=True),
        sa.Column("user_id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), sa.ForeignKey("users.id", ondelete="CASCADE"), nullable=False),
        sa.Column("trading_account_id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), sa.ForeignKey("trading_accounts.id", ondelete="CASCADE"), nullable=False),
        sa.Column("operating_mode", sa.String(32), nullable=False),
        sa.Column("available_cash_usd", sa.Numeric(20, 6), nullable=False, server_default="1000"),
        sa.Column("locked_cash_usd", sa.Numeric(20, 6), nullable=False, server_default="0"),
        sa.Column("positions_market_value_usd", sa.Numeric(20, 6), nullable=False, server_default="0"),
        sa.Column("total_equity_usd", sa.Numeric(20, 6), nullable=False, server_default="1000"),
        sa.Column("daily_realized_pnl_usd", sa.Numeric(20, 6), nullable=False, server_default="0"),
        sa.Column("weekly_realized_pnl_usd", sa.Numeric(20, 6), nullable=False, server_default="0"),
        sa.Column("monthly_realized_pnl_usd", sa.Numeric(20, 6), nullable=False, server_default="0"),
        sa.Column("all_time_realized_pnl_usd", sa.Numeric(20, 6), nullable=False, server_default="0"),
        sa.Column("updated_at", sa.DateTime(timezone=True), nullable=False),
        sa.UniqueConstraint("trading_account_id", "operating_mode", name="uq_account_balances_account_mode"),
    )
    op.create_index("ix_account_balances_user_id", "account_balances", ["user_id"])
    op.create_index("ix_account_balances_trading_account_id", "account_balances", ["trading_account_id"])

    # 16. account_activity_logs
    op.create_table(
        "account_activity_logs",
        sa.Column("id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), primary_key=True, autoincrement=True),
        sa.Column("user_id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), sa.ForeignKey("users.id", ondelete="CASCADE"), nullable=False),
        sa.Column("trading_account_id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), sa.ForeignKey("trading_accounts.id", ondelete="SET NULL"), nullable=True),
        sa.Column("action", sa.String(128), nullable=False),
        sa.Column("details_json", sa.JSON(), nullable=True),
        sa.Column("ip_address", sa.String(64), nullable=True),
        sa.Column("created_at", sa.DateTime(timezone=True), nullable=False),
    )
    op.create_index("ix_account_activity_logs_user_id", "account_activity_logs", ["user_id"])
    op.create_index("ix_account_activity_logs_trading_account_id", "account_activity_logs", ["trading_account_id"])
    op.create_index("ix_activity_logs_user_created", "account_activity_logs", ["user_id", "created_at"])

    # 17. system_settings
    op.create_table(
        "system_settings",
        sa.Column("id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), primary_key=True, autoincrement=True),
        sa.Column("key", sa.String(128), nullable=False),
        sa.Column("value_json", sa.JSON(), nullable=False),
        sa.Column("description", sa.String(512), nullable=True),
        sa.Column("updated_by_user_id", sa.BigInteger().with_variant(sa.Integer(), "sqlite"), sa.ForeignKey("users.id", ondelete="SET NULL"), nullable=True),
        sa.Column("updated_at", sa.DateTime(timezone=True), nullable=False),
        sa.UniqueConstraint("key", name="uq_system_settings_key"),
    )
    op.create_index("ix_system_settings_key", "system_settings", ["key"])

    # PostgreSQL Row-Level Security (RLS) policies on all tenant-owned tables
    bind = op.get_bind()
    if bind.dialect.name == "postgresql":
        op.execute(
            """
            DO $$
            BEGIN
                IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'pommarket_rls') THEN
                    CREATE ROLE pommarket_rls NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS NOLOGIN;
                END IF;
            END
            $$;
            GRANT USAGE ON SCHEMA public TO pommarket_rls;
            GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO pommarket_rls;
            GRANT USAGE, SELECT, UPDATE ON ALL SEQUENCES IN SCHEMA public TO pommarket_rls;
            """
        )
        for table in TENANT_TABLES:
            op.execute(f"ALTER TABLE {table} ENABLE ROW LEVEL SECURITY;")
            op.execute(f"ALTER TABLE {table} FORCE ROW LEVEL SECURITY;")
            op.execute(
                f"""
                CREATE POLICY tenant_isolation_policy ON {table}
                USING (
                    current_setting('app.is_super_admin', true) = 'true'
                    OR (
                        current_setting('app.current_user_id', true) IS NOT NULL
                        AND current_setting('app.current_user_id', true) <> ''
                        AND user_id::text = current_setting('app.current_user_id', true)
                    )
                )
                WITH CHECK (
                    current_setting('app.is_super_admin', true) = 'true'
                    OR (
                        current_setting('app.current_user_id', true) IS NOT NULL
                        AND current_setting('app.current_user_id', true) <> ''
                        AND user_id::text = current_setting('app.current_user_id', true)
                    )
                );
                """
            )


def downgrade() -> None:
    bind = op.get_bind()
    if bind.dialect.name == "postgresql":
        for table in reversed(TENANT_TABLES):
            op.execute(f"DROP POLICY IF EXISTS tenant_isolation_policy ON {table};")
            op.execute(f"ALTER TABLE {table} DISABLE ROW LEVEL SECURITY;")

    for table in [
        "system_settings",
        "account_activity_logs",
        "account_balances",
        "virtual_positions",
        "fills",
        "orders",
        "execution_jobs",
        "source_trade_events",
        "risk_limits",
        "copy_strategies",
        "account_source_traders",
        "source_traders",
        "trading_account_credentials",
        "trading_accounts",
        "user_sessions",
        "user_roles",
        "users",
    ]:
        op.drop_table(table)
