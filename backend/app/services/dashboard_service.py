"""Dashboard and Administrative aggregation service with strict tenant isolation."""

from __future__ import annotations

from datetime import datetime, timezone
from decimal import Decimal
import os
import resource
from typing import Any

from sqlalchemy import func, select
from sqlalchemy.orm import Session

from backend.app.models import (
    AccountActivityLog,
    AccountBalance,
    AccountSourceTrader,
    CopyStrategy,
    ExecutionJob,
    ExecutionJobStatus,
    Fill,
    OperatingMode,
    Order,
    OrderStatus,
    RiskLimit,
    SourceTradeEvent,
    SourceTrader,
    SystemSetting,
    TradingAccount,
    User,
    VirtualPosition,
)
from backend.app.services.account_service import TradingAccountService
from backend.app.services.metrics_service import GLOBAL_METRICS


class DashboardService:
    def __init__(self) -> None:
        self._account_service = TradingAccountService()

    def get_user_dashboard_summary(self, session: Session, *, user_id: int) -> dict[str, Any]:
        """Compute tenant-isolated user dashboard overview across all of the user's trading accounts."""
        accounts = session.scalars(
            select(TradingAccount)
            .where(TradingAccount.user_id == user_id)
            .order_by(TradingAccount.id.asc())
        ).all()

        total_accounts = len(accounts)
        connected_accounts = sum(1 for a in accounts if a.connection_status == "CONNECTED")
        disconnected_accounts = total_accounts - connected_accounts
        active_copy_accounts = sum(
            1
            for a in accounts
            if a.trading_status == "ACTIVE"
            and not a.emergency_stop
            and a.operating_mode in (OperatingMode.PAPER_TRADING.value, OperatingMode.LIVE_TRADING.value)
        )

        monitored_traders_count = (
            session.scalar(
                select(func.count(func.distinct(AccountSourceTrader.source_trader_id))).where(
                    AccountSourceTrader.user_id == user_id,
                    AccountSourceTrader.is_enabled.is_(True),
                )
            )
            or 0
        )

        total_copied_trades = (
            session.scalar(
                select(func.count(Fill.id)).where(Fill.user_id == user_id)
            )
            or 0
        )

        # Aggregate active mode balances per account
        daily_pnl = Decimal("0")
        weekly_pnl = Decimal("0")
        monthly_pnl = Decimal("0")
        current_exposure = Decimal("0")
        account_performance: list[dict[str, Any]] = []

        for acct in accounts:
            bal = session.scalar(
                select(AccountBalance).where(
                    AccountBalance.user_id == user_id,
                    AccountBalance.trading_account_id == acct.id,
                    AccountBalance.operating_mode == acct.operating_mode,
                )
            )
            acct_exp = (
                session.scalar(
                    select(func.coalesce(func.sum(VirtualPosition.cost_basis_usd), 0)).where(
                        VirtualPosition.user_id == user_id,
                        VirtualPosition.trading_account_id == acct.id,
                        VirtualPosition.operating_mode == acct.operating_mode,
                        VirtualPosition.open_shares > 0,
                    )
                )
                or Decimal("0")
            )
            acct_exp = Decimal(str(acct_exp))
            current_exposure += acct_exp

            d_pnl = Decimal(str(bal.daily_realized_pnl_usd)) if bal else Decimal("0")
            w_pnl = Decimal(str(bal.weekly_realized_pnl_usd)) if bal else Decimal("0")
            m_pnl = Decimal(str(bal.monthly_realized_pnl_usd)) if bal else Decimal("0")
            cash = Decimal(str(bal.available_cash_usd)) if bal else Decimal("0")
            equity = Decimal(str(bal.total_equity_usd)) if bal else Decimal("0")

            daily_pnl += d_pnl
            weekly_pnl += w_pnl
            monthly_pnl += m_pnl

            account_performance.append(
                {
                    "account_id": acct.id,
                    "display_name": acct.display_name,
                    "operating_mode": acct.operating_mode,
                    "connection_status": acct.connection_status,
                    "trading_status": acct.trading_status,
                    "emergency_stop": acct.emergency_stop,
                    "available_cash_usd": str(cash),
                    "current_exposure_usd": str(acct_exp),
                    "total_equity_usd": str(equity),
                    "daily_pnl_usd": str(d_pnl),
                    "weekly_pnl_usd": str(w_pnl),
                    "monthly_pnl_usd": str(m_pnl),
                }
            )

        recent_orders = session.scalars(
            select(Order)
            .where(Order.user_id == user_id)
            .order_by(Order.order_submitted_ts.desc())
            .limit(20)
        ).all()

        recent_fills = session.scalars(
            select(Fill)
            .where(Fill.user_id == user_id)
            .order_by(Fill.fill_ts.desc())
            .limit(20)
        ).all()

        execution_errors = session.scalars(
            select(ExecutionJob)
            .where(
                ExecutionJob.user_id == user_id,
                ExecutionJob.status.in_(
                    [
                        ExecutionJobStatus.FAILED.value,
                        ExecutionJobStatus.AMBIGUOUS.value,
                        ExecutionJobStatus.SKIPPED.value,
                    ]
                ),
            )
            .order_by(ExecutionJob.created_at.desc())
            .limit(20)
        ).all()

        return {
            "total_trading_accounts": total_accounts,
            "connected_accounts": connected_accounts,
            "disconnected_accounts": disconnected_accounts,
            "active_copy_trading_accounts": active_copy_accounts,
            "monitored_source_traders": monitored_traders_count,
            "total_copied_trades": total_copied_trades,
            "daily_pnl_usd": str(daily_pnl),
            "weekly_pnl_usd": str(weekly_pnl),
            "monthly_pnl_usd": str(monthly_pnl),
            "current_exposure_usd": str(current_exposure),
            "account_performance": account_performance,
            "recent_orders": [
                {
                    "id": o.id,
                    "trading_account_id": o.trading_account_id,
                    "operating_mode": o.operating_mode,
                    "token_id": o.token_id,
                    "side": o.side,
                    "limit_price": str(o.limit_price),
                    "requested_size_shares": str(o.requested_size_shares),
                    "filled_size_shares": str(o.filled_size_shares),
                    "status": o.status,
                    "error_message": o.error_message,
                    "order_submitted_ts": o.order_submitted_ts.isoformat(),
                }
                for o in recent_orders
            ],
            "recent_fills": [
                {
                    "id": f.id,
                    "order_id": f.order_id,
                    "trading_account_id": f.trading_account_id,
                    "operating_mode": f.operating_mode,
                    "token_id": f.token_id,
                    "side": f.side,
                    "filled_shares": str(f.filled_shares),
                    "avg_fill_price": str(f.avg_fill_price),
                    "notional_usd": str(f.notional_usd),
                    "fee_usd": str(f.fee_usd),
                    "realized_pnl_usd": str(f.realized_pnl_usd),
                    "fill_ts": f.fill_ts.isoformat(),
                }
                for f in recent_fills
            ],
            "execution_errors": [
                {
                    "job_id": j.id,
                    "trading_account_id": j.trading_account_id,
                    "operating_mode": j.operating_mode,
                    "status": j.status,
                    "reason": j.skip_or_error_reason,
                    "created_at": j.created_at.isoformat(),
                }
                for j in execution_errors
            ],
        }

    def get_trading_account_page_detail(
        self,
        session: Session,
        *,
        user_id: int,
        account_id: int,
    ) -> dict[str, Any]:
        """Return full details for a single trading account page (strictly tenant-isolated, no secrets)."""
        acct = self._account_service.get_owned_account(session, user_id=user_id, account_id=account_id)
        acct_resp = self._account_service.get_account_response(session, user_id=user_id, account_id=account_id)

        balances = session.scalars(
            select(AccountBalance).where(
                AccountBalance.user_id == user_id,
                AccountBalance.trading_account_id == account_id,
            )
        ).all()

        positions = session.scalars(
            select(VirtualPosition)
            .where(
                VirtualPosition.user_id == user_id,
                VirtualPosition.trading_account_id == account_id,
            )
            .order_by(VirtualPosition.updated_at.desc())
        ).all()

        orders = session.scalars(
            select(Order)
            .where(
                Order.user_id == user_id,
                Order.trading_account_id == account_id,
            )
            .order_by(Order.order_submitted_ts.desc())
            .limit(50)
        ).all()

        fills = session.scalars(
            select(Fill)
            .where(
                Fill.user_id == user_id,
                Fill.trading_account_id == account_id,
            )
            .order_by(Fill.fill_ts.desc())
            .limit(50)
        ).all()

        source_mappings = session.execute(
            select(AccountSourceTrader, SourceTrader)
            .join(SourceTrader, AccountSourceTrader.source_trader_id == SourceTrader.id)
            .where(
                AccountSourceTrader.user_id == user_id,
                AccountSourceTrader.trading_account_id == account_id,
            )
        ).all()

        strategy = acct.copy_strategies[0] if acct.copy_strategies else None
        risk = acct.risk_limits

        return {
            "account": acct_resp.model_dump(mode="json"),
            "balances": [
                {
                    "operating_mode": b.operating_mode,
                    "available_cash_usd": str(b.available_cash_usd),
                    "locked_cash_usd": str(b.locked_cash_usd),
                    "positions_market_value_usd": str(b.positions_market_value_usd),
                    "total_equity_usd": str(b.total_equity_usd),
                    "daily_realized_pnl_usd": str(b.daily_realized_pnl_usd),
                    "weekly_realized_pnl_usd": str(b.weekly_realized_pnl_usd),
                    "monthly_realized_pnl_usd": str(b.monthly_realized_pnl_usd),
                    "all_time_realized_pnl_usd": str(b.all_time_realized_pnl_usd),
                }
                for b in balances
            ],
            "positions": [
                {
                    "id": p.id,
                    "source_wallet": p.source_wallet,
                    "market_condition_id": p.market_condition_id,
                    "token_id": p.token_id,
                    "operating_mode": p.operating_mode,
                    "open_shares": str(p.open_shares),
                    "avg_entry_price": str(p.avg_entry_price),
                    "cost_basis_usd": str(p.cost_basis_usd),
                    "realized_pnl_usd": str(p.realized_pnl_usd),
                    "total_fees_usd": str(p.total_fees_usd),
                    "source_observed_shares": str(p.source_observed_shares),
                    "pending_exit_shares": str(p.pending_exit_shares),
                }
                for p in positions
            ],
            "orders": [
                {
                    "id": o.id,
                    "exchange_order_id": o.exchange_order_id,
                    "operating_mode": o.operating_mode,
                    "token_id": o.token_id,
                    "side": o.side,
                    "limit_price": str(o.limit_price),
                    "requested_size_shares": str(o.requested_size_shares),
                    "filled_size_shares": str(o.filled_size_shares),
                    "status": o.status,
                    "error_message": o.error_message,
                    "order_submitted_ts": o.order_submitted_ts.isoformat(),
                }
                for o in orders
            ],
            "trade_history": [
                {
                    "id": f.id,
                    "order_id": f.order_id,
                    "token_id": f.token_id,
                    "side": f.side,
                    "filled_shares": str(f.filled_shares),
                    "avg_fill_price": str(f.avg_fill_price),
                    "notional_usd": str(f.notional_usd),
                    "fee_usd": str(f.fee_usd),
                    "realized_pnl_usd": str(f.realized_pnl_usd),
                    "fill_ts": f.fill_ts.isoformat(),
                }
                for f in fills
            ],
            "source_traders": [
                {
                    "source_trader_id": t.id,
                    "wallet_address": t.wallet_address,
                    "custom_name": m.custom_name,
                    "is_enabled": m.is_enabled,
                    "execution_mode": m.execution_mode,
                    "fixed_amount_usd": str(m.fixed_amount_usd),
                    "copy_percentage": str(m.copy_percentage),
                    "copy_multiplier": str(m.copy_multiplier),
                    "max_order_size_usd": str(m.max_order_size_usd),
                    "max_slippage_pct": str(m.max_slippage_pct),
                }
                for m, t in source_mappings
            ],
            "copy_configuration": (
                {
                    "execution_mode": strategy.execution_mode,
                    "fixed_amount_usd": str(strategy.fixed_amount_usd),
                    "source_size_percentage": str(strategy.source_size_percentage),
                    "copy_multiplier": str(strategy.copy_multiplier),
                    "max_order_size_usd": str(strategy.max_order_size_usd),
                    "min_order_size_usd": str(strategy.min_order_size_usd),
                    "max_slippage_pct": str(strategy.max_slippage_pct),
                    "buy_enabled": strategy.buy_enabled,
                    "sell_enabled": strategy.sell_enabled,
                    "slice_collapse_window_secs": strategy.slice_collapse_window_secs,
                    "allowed_markets": strategy.allowed_markets,
                }
                if strategy
                else None
            ),
            "risk_limits": (
                {
                    "max_order_size_usd": str(risk.max_order_size_usd),
                    "min_order_size_usd": str(risk.min_order_size_usd),
                    "max_market_exposure_usd": str(risk.max_market_exposure_usd),
                    "max_source_trader_exposure_usd": str(risk.max_source_trader_exposure_usd),
                    "max_total_exposure_usd": str(risk.max_total_exposure_usd),
                    "max_daily_loss_usd": str(risk.max_daily_loss_usd),
                    "max_slippage_pct": str(risk.max_slippage_pct),
                    "max_executions_per_interval": risk.max_executions_per_interval,
                    "execution_interval_seconds": risk.execution_interval_seconds,
                    "daily_loss_stop_latched": risk.daily_loss_stop_latched,
                }
                if risk
                else None
            ),
        }

    def get_admin_platform_overview(self, session: Session) -> dict[str, Any]:
        """Return platform-wide administrative health, queues, errors, and users WITHOUT any signing secrets."""
        total_users = session.scalar(select(func.count(User.id))) or 0
        suspended_users = session.scalar(select(func.count(User.id)).where(User.is_suspended.is_(True))) or 0
        total_accounts = session.scalar(select(func.count(TradingAccount.id))) or 0
        total_source_traders = session.scalar(select(func.count(SourceTrader.id))) or 0
        total_mappings = session.scalar(select(func.count(AccountSourceTrader.id))) or 0
        total_events = session.scalar(select(func.count(SourceTradeEvent.id))) or 0
        queued_jobs = (
            session.scalar(
                select(func.count(ExecutionJob.id)).where(
                    ExecutionJob.status == ExecutionJobStatus.QUEUED.value
                )
            )
            or 0
        )
        ambiguous_orders = (
            session.scalar(
                select(func.count(Order.id)).where(Order.status == OrderStatus.AMBIGUOUS.value)
            )
            or 0
        )
        failed_jobs = (
            session.scalar(
                select(func.count(ExecutionJob.id)).where(
                    ExecutionJob.status == ExecutionJobStatus.FAILED.value
                )
            )
            or 0
        )

        global_stop_setting = session.scalar(
            select(SystemSetting).where(SystemSetting.key == "global_emergency_stop")
        )
        global_stop_active = bool(
            global_stop_setting
            and isinstance(global_stop_setting.value_json, dict)
            and global_stop_setting.value_json.get("enabled")
        )

        rusage = resource.getrusage(resource.RUSAGE_SELF)
        rss_mb = round(rusage.ru_maxrss / 1024.0, 2)

        users = session.scalars(select(User).order_by(User.id.asc()).limit(100)).all()
        recent_errors = session.scalars(
            select(ExecutionJob)
            .where(ExecutionJob.status.in_([ExecutionJobStatus.FAILED.value, ExecutionJobStatus.AMBIGUOUS.value]))
            .order_by(ExecutionJob.created_at.desc())
            .limit(25)
        ).all()

        return {
            "platform_health": "DEGRADED_EMERGENCY_STOP" if global_stop_active else "HEALTHY",
            "global_emergency_stop": global_stop_active,
            "services": {
                "api_backend": "ONLINE",
                "market_data_collector": "ONLINE",
                "event_processor": "ONLINE",
                "account_router": "ONLINE",
                "execution_workers": "ONLINE",
                "reconciliation_service": "ONLINE",
            },
            "aggregates": {
                "registered_users": total_users,
                "suspended_users": suspended_users,
                "trading_accounts": total_accounts,
                "monitored_source_traders": total_source_traders,
                "account_source_mappings": total_mappings,
                "source_events_processed": total_events,
                "queued_execution_jobs": queued_jobs,
                "ambiguous_orders_in_flight": ambiguous_orders,
                "failed_execution_jobs": failed_jobs,
            },
            "resource_usage": {
                "pid": os.getpid(),
                "max_rss_mb": rss_mb,
                "user_cpu_seconds": round(rusage.ru_utime, 3),
                "system_cpu_seconds": round(rusage.ru_stime, 3),
            },
            "metrics": GLOBAL_METRICS.snapshot(),
            "registered_users": [
                {
                    "id": u.id,
                    "email": u.email,
                    "full_name": u.full_name,
                    "is_active": u.is_active,
                    "is_suspended": u.is_suspended,
                    "email_verified": u.email_verified,
                    "totp_enabled": u.totp_enabled,
                    "roles": sorted({r.role_name for r in u.roles}),
                    "created_at": u.created_at.isoformat(),
                }
                for u in users
            ],
            "recent_errors": [
                {
                    "job_id": j.id,
                    "user_id": j.user_id,
                    "trading_account_id": j.trading_account_id,
                    "status": j.status,
                    "reason": j.skip_or_error_reason,
                    "created_at": j.created_at.isoformat(),
                }
                for j in recent_errors
            ],
        }
