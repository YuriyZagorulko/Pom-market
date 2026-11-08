"""Event Processing Service: deduplicates normalized source trades, routes to destination accounts, and creates ExecutionJobs."""

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timezone
from decimal import Decimal
import threading
from typing import Optional

from sqlalchemy import select
from sqlalchemy.exc import IntegrityError
from sqlalchemy.orm import Session

from backend.app.models import (
    ExecutionJob,
    ExecutionJobStatus,
    OperatingMode,
    SourceTradeEvent,
    SourceTrader,
    SystemSetting,
)
from backend.app.services.account_router import AccountRouterIndex, RouteTarget
from backend.app.services.collector_service import NormalizedTradeEvent
from backend.app.services.metrics_service import GLOBAL_METRICS, PlatformMetricsRegistry


@dataclass
class ProcessedEventResult:
    source_trade_event_id: Optional[int]
    was_duplicate: bool
    matched_targets_count: int
    created_jobs: list[ExecutionJob]


class EventProcessingService:
    """Validates, deduplicates, stores, and routes normalized source trader events."""

    def __init__(
        self,
        router: AccountRouterIndex,
        metrics: Optional[PlatformMetricsRegistry] = None,
    ) -> None:
        self._router = router
        self._metrics = metrics or GLOBAL_METRICS
        self._lock = threading.Lock()
        self._seen_keys: set[tuple[str, str, str, str]] = set()

    def is_global_emergency_stop_active(self, session: Session) -> bool:
        setting = session.scalar(
            select(SystemSetting).where(SystemSetting.key == "global_emergency_stop")
        )
        if setting and isinstance(setting.value_json, dict):
            return bool(setting.value_json.get("enabled", False))
        return False

    def process_event(
        self,
        session: Session,
        event: NormalizedTradeEvent,
    ) -> ProcessedEventResult:
        """Deduplicate a normalized trade event and route it to all subscribed destination accounts."""
        # Fast O(1) filter: ignore wallets that no active trading account is monitoring
        targets = self._router.match_targets(event.source_wallet, side=event.side)
        if not targets:
            # Also check if wallet is monitored regardless of side
            if not self._router.is_wallet_monitored(event.source_wallet):
                return ProcessedEventResult(
                    source_trade_event_id=None,
                    was_duplicate=False,
                    matched_targets_count=0,
                    created_jobs=[],
                )

        dedup_key = event.dedup_key
        with self._lock:
            if dedup_key in self._seen_keys:
                self._metrics.inc("events_deduplicated_total")
                return ProcessedEventResult(
                    source_trade_event_id=None,
                    was_duplicate=True,
                    matched_targets_count=0,
                    created_jobs=[],
                )

        # Check database deduplication constraint `(tx_hash, source_wallet, token_id, side)`
        existing = session.scalar(
            select(SourceTradeEvent).where(
                SourceTradeEvent.tx_hash == event.tx_hash.lower(),
                SourceTradeEvent.source_wallet == event.source_wallet.lower(),
                SourceTradeEvent.token_id == event.token_id,
                SourceTradeEvent.side == event.side.upper(),
            )
        )
        if existing is not None:
            with self._lock:
                self._seen_keys.add(dedup_key)
            self._metrics.inc("events_deduplicated_total")
            return ProcessedEventResult(
                source_trade_event_id=existing.id,
                was_duplicate=True,
                matched_targets_count=0,
                created_jobs=[],
            )

        routed_ts = datetime.now(timezone.utc)
        trader_id = targets[0].source_trader_id if targets else None
        if trader_id is None:
            trader = session.scalar(
                select(SourceTrader).where(SourceTrader.wallet_address == event.source_wallet.lower())
            )
            trader_id = trader.id if trader else None

        db_event = SourceTradeEvent(
            source_trader_id=trader_id,
            source_wallet=event.source_wallet.lower(),
            market_condition_id=event.market_condition_id,
            market_question=event.market_question,
            outcome_label=event.outcome_label,
            token_id=event.token_id,
            side=event.side.upper(),
            price=event.price,
            size=event.size,
            notional_usd=event.notional_usd,
            tx_hash=event.tx_hash.lower(),
            feed_source=event.feed_source,
            source_event_ts=event.source_event_ts,
            event_received_ts=event.event_received_ts,
            event_normalized_ts=event.event_normalized_ts,
            event_routed_ts=routed_ts,
        )

        try:
            with session.begin_nested():
                session.add(db_event)
                session.flush()
        except IntegrityError:
            with self._lock:
                self._seen_keys.add(dedup_key)
            self._metrics.inc("events_deduplicated_total")
            return ProcessedEventResult(
                source_trade_event_id=None,
                was_duplicate=True,
                matched_targets_count=0,
                created_jobs=[],
            )

        with self._lock:
            if len(self._seen_keys) > 200_000:
                self._seen_keys.clear()
            self._seen_keys.add(dedup_key)

        # Update source trader aggregate telemetry
        if trader_id is not None:
            trader = session.get(SourceTrader, trader_id)
            if trader is not None:
                trader.total_observed_trades += 1
                trader.total_observed_volume_usd = (
                    Decimal(str(trader.total_observed_volume_usd)) + event.notional_usd
                )
                trader.last_trade_at = event.source_event_ts

        self._metrics.inc("events_routed_total")
        self._metrics.record_latency(
            source_event_ts=event.source_event_ts,
            event_received_ts=event.event_received_ts,
            event_normalized_ts=event.event_normalized_ts,
            event_routed_ts=routed_ts,
        )

        created_jobs: list[ExecutionJob] = []
        for target in targets:
            job = self._create_job_for_target(session, db_event, target, routed_ts=routed_ts)
            if job is not None:
                created_jobs.append(job)

        session.flush()
        return ProcessedEventResult(
            source_trade_event_id=db_event.id,
            was_duplicate=False,
            matched_targets_count=len(targets),
            created_jobs=created_jobs,
        )

    def _create_job_for_target(
        self,
        session: Session,
        db_event: SourceTradeEvent,
        target: RouteTarget,
        *,
        routed_ts: datetime,
    ) -> Optional[ExecutionJob]:
        # Rule (Invariant 25): Never retroactively copy trades that occurred before the source wallet was assigned
        mapping_ts = target.mapping_created_at
        if mapping_ts.tzinfo is None:
            mapping_ts = mapping_ts.replace(tzinfo=timezone.utc)
        event_ts = db_event.source_event_ts
        if event_ts.tzinfo is None:
            event_ts = event_ts.replace(tzinfo=timezone.utc)

        # Compare at 1-second resolution (since RTDS timestamps are in whole seconds)
        if int(event_ts.timestamp()) < int(mapping_ts.timestamp()):
            return None

        idempotency_key = f"job:acct:{target.trading_account_id}:ev:{db_event.id}"
        existing_job = session.scalar(
            select(ExecutionJob).where(ExecutionJob.idempotency_key == idempotency_key)
        )
        if existing_job is not None:
            return None

        # In MONITORING mode, record the routed activity without queueing an order
        if target.operating_mode == OperatingMode.MONITORING.value:
            status = ExecutionJobStatus.COMPLETED.value
            reason = "monitoring_mode_no_order"
            completed_at = routed_ts
            self._metrics.inc("jobs_monitoring_total")
        else:
            status = ExecutionJobStatus.QUEUED.value
            reason = None
            completed_at = None
            if target.operating_mode == OperatingMode.PAPER_TRADING.value:
                self._metrics.inc("jobs_paper_total")
            else:
                self._metrics.inc("jobs_live_total")

        job = ExecutionJob(
            user_id=target.user_id,
            trading_account_id=target.trading_account_id,
            source_trader_id=target.source_trader_id,
            source_trade_event_id=db_event.id,
            idempotency_key=idempotency_key,
            operating_mode=target.operating_mode,
            status=status,
            skip_or_error_reason=reason,
            target_notional_usd=Decimal("0"),
            target_shares=Decimal("0"),
            limit_price=db_event.price,
            attempts=0,
            routed_at=routed_ts,
            completed_at=completed_at,
        )
        session.add(job)
        self._metrics.inc("jobs_created_total")
        return job
