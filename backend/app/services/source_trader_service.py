"""Source trader management service supporting many-to-many account mappings and import/export."""

from __future__ import annotations

import csv
import io
from decimal import Decimal
from typing import Any, Optional

from sqlalchemy import select
from sqlalchemy.orm import Session

from backend.app.models import (
    AccountActivityLog,
    AccountSourceTrader,
    SourceTradeEvent,
    SourceTrader,
    TradingAccount,
)
from backend.app.schemas import (
    SourceTraderAssignRequest,
    SourceTraderMappingResponse,
    SourceTraderMappingUpdateRequest,
)
from backend.app.services.account_service import TradingAccountService


def _to_mapping_response(mapping: AccountSourceTrader, trader: SourceTrader) -> SourceTraderMappingResponse:
    return SourceTraderMappingResponse(
        id=mapping.id,
        user_id=mapping.user_id,
        trading_account_id=mapping.trading_account_id,
        source_trader_id=trader.id,
        wallet_address=trader.wallet_address,
        custom_name=mapping.custom_name,
        is_enabled=mapping.is_enabled,
        execution_mode=mapping.execution_mode,
        fixed_amount_usd=mapping.fixed_amount_usd,
        copy_percentage=mapping.copy_percentage,
        copy_multiplier=mapping.copy_multiplier,
        max_order_size_usd=mapping.max_order_size_usd,
        min_order_size_usd=mapping.min_order_size_usd,
        max_exposure_usd=mapping.max_exposure_usd,
        max_slippage_pct=mapping.max_slippage_pct,
        copy_buy_enabled=mapping.copy_buy_enabled,
        copy_sell_enabled=mapping.copy_sell_enabled,
        total_observed_trades=trader.total_observed_trades,
        total_observed_volume_usd=trader.total_observed_volume_usd,
        win_rate_pct=trader.win_rate_pct,
        realized_roi_pct=trader.realized_roi_pct,
        created_at=mapping.created_at,
    )


class SourceTraderService:
    def __init__(self, account_service: Optional[TradingAccountService] = None) -> None:
        self._account_service = account_service or TradingAccountService()

    def add_source_trader_to_account(
        self,
        session: Session,
        *,
        user_id: int,
        account_id: int,
        req: SourceTraderAssignRequest,
    ) -> SourceTraderMappingResponse:
        account = self._account_service.get_owned_account(session, user_id=user_id, account_id=account_id)
        wallet = req.wallet_address.strip().lower()

        trader = session.scalar(select(SourceTrader).where(SourceTrader.wallet_address == wallet))
        if trader is None:
            trader = SourceTrader(
                wallet_address=wallet,
                default_label=req.custom_name,
                is_monitored=True,
            )
            session.add(trader)
            session.flush()
        else:
            trader.is_monitored = True

        existing = session.scalar(
            select(AccountSourceTrader).where(
                AccountSourceTrader.trading_account_id == account.id,
                AccountSourceTrader.source_trader_id == trader.id,
            )
        )
        if existing is not None:
            existing.custom_name = req.custom_name
            existing.is_enabled = req.is_enabled
            existing.execution_mode = req.execution_mode.value
            existing.fixed_amount_usd = req.fixed_amount_usd
            existing.copy_percentage = req.copy_percentage
            existing.copy_multiplier = req.copy_multiplier
            existing.max_order_size_usd = req.max_order_size_usd
            existing.min_order_size_usd = req.min_order_size_usd
            existing.max_exposure_usd = req.max_exposure_usd
            existing.max_slippage_pct = req.max_slippage_pct
            existing.copy_buy_enabled = req.copy_buy_enabled
            existing.copy_sell_enabled = req.copy_sell_enabled
            mapping = existing
        else:
            mapping = AccountSourceTrader(
                user_id=user_id,
                trading_account_id=account.id,
                source_trader_id=trader.id,
                custom_name=req.custom_name,
                is_enabled=req.is_enabled,
                execution_mode=req.execution_mode.value,
                fixed_amount_usd=req.fixed_amount_usd,
                copy_percentage=req.copy_percentage,
                copy_multiplier=req.copy_multiplier,
                max_order_size_usd=req.max_order_size_usd,
                min_order_size_usd=req.min_order_size_usd,
                max_exposure_usd=req.max_exposure_usd,
                max_slippage_pct=req.max_slippage_pct,
                copy_buy_enabled=req.copy_buy_enabled,
                copy_sell_enabled=req.copy_sell_enabled,
            )
            session.add(mapping)

        session.add(
            AccountActivityLog(
                user_id=user_id,
                trading_account_id=account.id,
                action="SOURCE_TRADER_ASSIGNED",
                details_json={"wallet_address": wallet, "custom_name": req.custom_name},
            )
        )
        session.flush()
        session.refresh(mapping)
        return _to_mapping_response(mapping, trader)

    def update_source_trader_mapping(
        self,
        session: Session,
        *,
        user_id: int,
        account_id: int,
        source_trader_id: int,
        req: SourceTraderMappingUpdateRequest,
    ) -> SourceTraderMappingResponse:
        self._account_service.get_owned_account(session, user_id=user_id, account_id=account_id)
        mapping = session.scalar(
            select(AccountSourceTrader).where(
                AccountSourceTrader.user_id == user_id,
                AccountSourceTrader.trading_account_id == account_id,
                AccountSourceTrader.source_trader_id == source_trader_id,
            )
        )
        if mapping is None:
            raise ValueError(f"Source trader {source_trader_id} is not mapped to account {account_id}")

        for field_name, val in req.model_dump(exclude_unset=True).items():
            if val is not None:
                if field_name == "execution_mode" and hasattr(val, "value"):
                    setattr(mapping, field_name, val.value)
                else:
                    setattr(mapping, field_name, val)

        session.flush()
        session.refresh(mapping)
        return _to_mapping_response(mapping, mapping.source_trader)

    def remove_source_trader_from_account(
        self,
        session: Session,
        *,
        user_id: int,
        account_id: int,
        source_trader_id: int,
    ) -> None:
        self._account_service.get_owned_account(session, user_id=user_id, account_id=account_id)
        mapping = session.scalar(
            select(AccountSourceTrader).where(
                AccountSourceTrader.user_id == user_id,
                AccountSourceTrader.trading_account_id == account_id,
                AccountSourceTrader.source_trader_id == source_trader_id,
            )
        )
        if mapping is None:
            raise ValueError(f"Source trader {source_trader_id} is not mapped to account {account_id}")
        wallet = mapping.source_trader.wallet_address
        session.delete(mapping)
        session.add(
            AccountActivityLog(
                user_id=user_id,
                trading_account_id=account_id,
                action="SOURCE_TRADER_REMOVED",
                details_json={"wallet_address": wallet, "source_trader_id": source_trader_id},
            )
        )
        session.flush()

    def list_account_source_traders(
        self,
        session: Session,
        *,
        user_id: int,
        account_id: int,
    ) -> list[SourceTraderMappingResponse]:
        self._account_service.get_owned_account(session, user_id=user_id, account_id=account_id)
        rows = session.execute(
            select(AccountSourceTrader, SourceTrader)
            .join(SourceTrader, AccountSourceTrader.source_trader_id == SourceTrader.id)
            .where(
                AccountSourceTrader.user_id == user_id,
                AccountSourceTrader.trading_account_id == account_id,
            )
            .order_by(AccountSourceTrader.id.asc())
        ).all()
        return [_to_mapping_response(m, t) for m, t in rows]

    def get_source_trader_details_for_user(
        self,
        session: Session,
        *,
        user_id: int,
        wallet_address: str,
        limit_trades: int = 50,
    ) -> dict[str, Any]:
        """Return source trader profile, recent trades, markets, and ONLY the caller's copying accounts."""
        wallet = wallet_address.strip().lower()
        trader = session.scalar(select(SourceTrader).where(SourceTrader.wallet_address == wallet))
        if trader is None:
            raise ValueError(f"Source trader {wallet} not found")

        user_mappings = session.execute(
            select(AccountSourceTrader, TradingAccount)
            .join(TradingAccount, AccountSourceTrader.trading_account_id == TradingAccount.id)
            .where(
                AccountSourceTrader.user_id == user_id,
                AccountSourceTrader.source_trader_id == trader.id,
            )
        ).all()

        recent_events = session.scalars(
            select(SourceTradeEvent)
            .where(SourceTradeEvent.source_wallet == wallet)
            .order_by(SourceTradeEvent.source_event_ts.desc())
            .limit(limit_trades)
        ).all()

        markets = sorted({e.market_condition_id for e in recent_events if e.market_condition_id})
        return {
            "source_trader_id": trader.id,
            "wallet_address": trader.wallet_address,
            "default_label": trader.default_label,
            "total_observed_trades": trader.total_observed_trades,
            "total_observed_volume_usd": str(trader.total_observed_volume_usd),
            "estimated_balance_usd": str(trader.estimated_balance_usd),
            "win_rate_pct": str(trader.win_rate_pct) if trader.win_rate_pct is not None else None,
            "realized_roi_pct": str(trader.realized_roi_pct) if trader.realized_roi_pct is not None else None,
            "markets": markets,
            "recent_trades": [
                {
                    "id": e.id,
                    "tx_hash": e.tx_hash,
                    "market_condition_id": e.market_condition_id,
                    "market_question": e.market_question,
                    "outcome_label": e.outcome_label,
                    "token_id": e.token_id,
                    "side": e.side,
                    "price": str(e.price),
                    "size": str(e.size),
                    "notional_usd": str(e.notional_usd),
                    "source_event_ts": e.source_event_ts.isoformat(),
                }
                for e in recent_events
            ],
            "copying_destination_accounts": [
                {
                    "trading_account_id": acct.id,
                    "display_name": acct.display_name,
                    "operating_mode": acct.operating_mode,
                    "custom_name": m.custom_name,
                    "is_enabled": m.is_enabled,
                    "execution_mode": m.execution_mode,
                    "fixed_amount_usd": str(m.fixed_amount_usd),
                    "copy_percentage": str(m.copy_percentage),
                    "copy_multiplier": str(m.copy_multiplier),
                    "max_order_size_usd": str(m.max_order_size_usd),
                    "max_slippage_pct": str(m.max_slippage_pct),
                }
                for m, acct in user_mappings
            ],
        }

    def export_source_wallets_csv(
        self,
        session: Session,
        *,
        user_id: int,
        account_id: int,
    ) -> str:
        items = self.list_account_source_traders(session, user_id=user_id, account_id=account_id)
        buf = io.StringIO()
        writer = csv.DictWriter(
            buf,
            fieldnames=[
                "wallet_address",
                "custom_name",
                "is_enabled",
                "execution_mode",
                "fixed_amount_usd",
                "copy_percentage",
                "copy_multiplier",
                "max_order_size_usd",
                "min_order_size_usd",
                "max_exposure_usd",
                "max_slippage_pct",
                "copy_buy_enabled",
                "copy_sell_enabled",
            ],
        )
        writer.writeheader()
        for item in items:
            writer.writerow(
                {
                    "wallet_address": item.wallet_address,
                    "custom_name": item.custom_name or "",
                    "is_enabled": str(item.is_enabled).lower(),
                    "execution_mode": item.execution_mode,
                    "fixed_amount_usd": str(item.fixed_amount_usd),
                    "copy_percentage": str(item.copy_percentage),
                    "copy_multiplier": str(item.copy_multiplier),
                    "max_order_size_usd": str(item.max_order_size_usd),
                    "min_order_size_usd": str(item.min_order_size_usd),
                    "max_exposure_usd": str(item.max_exposure_usd),
                    "max_slippage_pct": str(item.max_slippage_pct),
                    "copy_buy_enabled": str(item.copy_buy_enabled).lower(),
                    "copy_sell_enabled": str(item.copy_sell_enabled).lower(),
                }
            )
        return buf.getvalue()

    def import_source_wallets_csv(
        self,
        session: Session,
        *,
        user_id: int,
        account_id: int,
        csv_content: str,
    ) -> list[SourceTraderMappingResponse]:
        reader = csv.DictReader(io.StringIO(csv_content))
        results: list[SourceTraderMappingResponse] = []
        for row in reader:
            req = SourceTraderAssignRequest(
                wallet_address=row["wallet_address"],
                custom_name=row.get("custom_name") or None,
                is_enabled=row.get("is_enabled", "true").lower() == "true",
                execution_mode=row.get("execution_mode", "FIXED_AMOUNT"),
                fixed_amount_usd=Decimal(row.get("fixed_amount_usd", "10.0")),
                copy_percentage=Decimal(row.get("copy_percentage", "0.10")),
                copy_multiplier=Decimal(row.get("copy_multiplier", "1.0")),
                max_order_size_usd=Decimal(row.get("max_order_size_usd", "100.0")),
                min_order_size_usd=Decimal(row.get("min_order_size_usd", "1.0")),
                max_exposure_usd=Decimal(row.get("max_exposure_usd", "500.0")),
                max_slippage_pct=Decimal(row.get("max_slippage_pct", "0.05")),
                copy_buy_enabled=row.get("copy_buy_enabled", "true").lower() == "true",
                copy_sell_enabled=row.get("copy_sell_enabled", "true").lower() == "true",
            )
            results.append(
                self.add_source_trader_to_account(
                    session,
                    user_id=user_id,
                    account_id=account_id,
                    req=req,
                )
            )
        return results
