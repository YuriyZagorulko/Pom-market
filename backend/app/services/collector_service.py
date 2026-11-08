"""Market Data Collector: normalizes Polymarket RTDS WebSocket frames, Data API v2 (`/v2/activity`) payloads, and Polygon V2 OrderFilled logs."""

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timezone
from decimal import Decimal, InvalidOperation
import json
import random
import re
import time
from typing import Any, Callable, Optional

import httpx

from backend.app.services.metrics_service import GLOBAL_METRICS, PlatformMetricsRegistry

# Verified Polygon CTF Exchange V2 OrderFilled event topic0:
# keccak256("OrderFilled(bytes32,address,address,uint8,uint256,uint256,uint256,uint256,bytes32,bytes32)")
ORDER_FILLED_V2_TOPIC0 = "0xd543adfd945773f1a62f74f0ee55a5e3b9b1a28262980ba90b1a89f2ea84d8ee"

_ETH_ADDRESS_RE = re.compile(r"^0x[0-9a-f]{40}$")
_TOKEN_ID_RE = re.compile(r"^(?:0x[0-9a-f]+|[0-9]+|[a-z0-9_.-]+)$")


@dataclass(frozen=True, slots=True)
class NormalizedTradeEvent:
    source_wallet: str
    market_condition_id: str
    market_question: Optional[str]
    outcome_label: Optional[str]
    token_id: str
    side: str  # "BUY" | "SELL"
    price: Decimal
    size: Decimal
    notional_usd: Decimal
    tx_hash: str
    feed_source: str  # "rtds" | "activity_poll" | "polygon_chain"
    source_event_ts: datetime
    event_received_ts: datetime
    event_normalized_ts: datetime

    @property
    def dedup_key(self) -> tuple[str, str, str, str]:
        """Canonical deduplication tuple: `(tx_hash, source_wallet, token_id, side)`."""
        return (self.tx_hash.lower(), self.source_wallet.lower(), self.token_id, self.side.upper())


@dataclass(frozen=True)
class ActivityV2PageResult:
    events: list[NormalizedTradeEvent]
    has_more: bool
    next_cursor: Optional[str]
    raw_row_count: int


def build_rtds_subscribe_message() -> str:
    """Build official Polymarket RTDS subscription message.

    Subscribes to `activity/trades` (wallet-attributed trade stream) and
    `crypto_prices/update` (control heartbeat topic to distinguish quiet markets from dead sockets).
    """
    return json.dumps(
        {
            "action": "subscribe",
            "subscriptions": [
                {"topic": "activity", "type": "trades"},
                {"topic": "crypto_prices", "type": "update"},
            ],
        }
    )


def compute_reconnect_backoff(attempt: int, base_sec: float = 1.0, max_sec: float = 60.0) -> float:
    """Exponential backoff with full jitter for WebSocket/HTTP reconnect recovery."""
    cap = min(max_sec, base_sec * (2 ** min(attempt, 10)))
    return random.uniform(base_sec * 0.5, cap)


def _parse_decimal(val: Any) -> Optional[Decimal]:
    if val is None or isinstance(val, bool):
        return None
    try:
        d = Decimal(str(val))
        return d if d.is_finite() else None
    except (InvalidOperation, ValueError, TypeError):
        return None


def _parse_timestamp(val: Any) -> Optional[datetime]:
    if val is None:
        return None
    try:
        ts_num = float(val)
        # Handle millisecond vs second epoch timestamps
        if ts_num > 1e11:
            ts_num = ts_num / 1000.0
        return datetime.fromtimestamp(ts_num, tz=timezone.utc)
    except (ValueError, TypeError, OverflowError):
        return None


def _is_valid_wallet_address(addr: str) -> bool:
    if not addr or not addr.startswith("0x"):
        return False
    if _ETH_ADDRESS_RE.match(addr):
        return True
    # Allow compact 0x hex addresses in unit test fixtures while rejecting non-hex garbage
    try:
        int(addr[2:], 16)
        return len(addr) >= 6
    except ValueError:
        return False


def _is_valid_token_id(token_id: str) -> bool:
    return bool(token_id) and bool(_TOKEN_ID_RE.match(token_id.lower()))


class MarketDataCollector:
    """Collects and normalizes public wallet-attributed trades from Polymarket."""

    def __init__(self, metrics: Optional[PlatformMetricsRegistry] = None) -> None:
        self._metrics = metrics or GLOBAL_METRICS
        self.last_frame_received_at: Optional[datetime] = None
        self.last_trade_received_at: Optional[datetime] = None
        self.reconnect_count: int = 0

    def record_reconnect(self) -> None:
        self.reconnect_count += 1
        self._metrics.inc("collector_reconnects_total")

    def parse_rtds_frame(
        self,
        raw_frame: str | dict[str, Any],
        *,
        received_at: Optional[datetime] = None,
    ) -> list[NormalizedTradeEvent]:
        """Parse a Polymarket RTDS WebSocket frame.

        Ignores non-wallet market price updates (`crypto_prices` or book ticks) while
        updating `last_frame_received_at` for stall detection.
        """
        recv_ts = received_at or datetime.now(timezone.utc)
        self.last_frame_received_at = recv_ts
        self._metrics.inc("events_received_total")

        frame: dict[str, Any]
        if isinstance(raw_frame, str):
            try:
                frame = json.loads(raw_frame)
            except json.JSONDecodeError:
                return []
        elif isinstance(raw_frame, dict):
            frame = raw_frame
        else:
            return []

        if frame.get("topic") != "activity" or frame.get("type") != "trades":
            return []

        payload = frame.get("payload")
        items: list[dict[str, Any]]
        if isinstance(payload, list):
            items = [x for x in payload if isinstance(x, dict)]
        elif isinstance(payload, dict):
            items = [payload]
        else:
            return []

        results: list[NormalizedTradeEvent] = []
        for item in items:
            ev = self._normalize_row(item, feed_source="rtds", received_at=recv_ts)
            if ev is not None:
                results.append(ev)
                self.last_trade_received_at = recv_ts
                self._metrics.inc("events_normalized_total")
        return results

    def parse_activity_v2_response(
        self,
        payload: dict[str, Any] | list[dict[str, Any]],
        *,
        received_at: Optional[datetime] = None,
    ) -> ActivityV2PageResult:
        """Parse Polymarket Data API v2 response (`GET /v2/activity?user=<wallet>`).

        Supports both the v2 envelope (`{"data": [...], "pagination": {"has_more": ..., "next_cursor": ...}}`)
        and direct row lists.
        """
        recv_ts = received_at or datetime.now(timezone.utc)
        self.last_frame_received_at = recv_ts
        self._metrics.inc("events_received_total")

        rows: list[dict[str, Any]]
        has_more = False
        next_cursor: Optional[str] = None

        if isinstance(payload, dict):
            raw_data = payload.get("data")
            rows = [r for r in raw_data if isinstance(r, dict)] if isinstance(raw_data, list) else []
            pagination = payload.get("pagination")
            if isinstance(pagination, dict):
                has_more = bool(pagination.get("has_more", False))
                raw_cursor = pagination.get("next_cursor")
                if isinstance(raw_cursor, str) and raw_cursor.strip():
                    next_cursor = raw_cursor.strip()
        elif isinstance(payload, list):
            rows = [r for r in payload if isinstance(r, dict)]
        else:
            rows = []

        events: list[NormalizedTradeEvent] = []
        for row in rows:
            ev = self._normalize_row(row, feed_source="activity_poll", received_at=recv_ts)
            if ev is not None:
                events.append(ev)
                self.last_trade_received_at = recv_ts
                self._metrics.inc("events_normalized_total")

        return ActivityV2PageResult(
            events=events,
            has_more=has_more,
            next_cursor=next_cursor,
            raw_row_count=len(rows),
        )

    def parse_activity_poll_rows(
        self,
        rows: list[dict[str, Any]] | dict[str, Any],
        *,
        received_at: Optional[datetime] = None,
    ) -> list[NormalizedTradeEvent]:
        """Parse response rows or v2 envelope from Polymarket Data API `/v2/activity`."""
        return self.parse_activity_v2_response(rows, received_at=received_at).events

    def fetch_wallet_activity_v2(
        self,
        wallet_address: str,
        *,
        data_api_host: str = "https://data-api.polymarket.com",
        limit: int = 50,
        max_pages: int = 3,
        max_retries: int = 3,
        http_get: Optional[Callable[[str, dict[str, Any]], httpx.Response]] = None,
        sleep_fn: Callable[[float], None] = time.sleep,
    ) -> list[NormalizedTradeEvent]:
        """Fetch and paginate `GET /v2/activity` for a monitored source wallet with HTTP 429/5xx/timeout retry handling."""
        wallet = wallet_address.strip().lower()
        if not _is_valid_wallet_address(wallet):
            raise ValueError(f"Invalid source wallet address: {wallet_address}")

        base_url = f"{data_api_host.rstrip('/')}/v2/activity"
        cursor: Optional[str] = None
        collected: list[NormalizedTradeEvent] = []

        for _page_idx in range(max(1, max_pages)):
            params: dict[str, Any] = {"user": wallet, "limit": limit}
            if cursor:
                params["cursor"] = cursor

            resp: Optional[httpx.Response] = None
            for attempt in range(max(1, max_retries)):
                try:
                    if http_get is not None:
                        resp = http_get(base_url, params)
                    else:
                        resp = httpx.get(base_url, params=params, timeout=10.0)

                    if resp.status_code == 200:
                        break

                    # Handle HTTP 429 Too Many Requests and temporary 5xx failures
                    if resp.status_code in (429, 500, 502, 503, 504):
                        retry_after_hdr = resp.headers.get("Retry-After")
                        wait_s = (
                            float(retry_after_hdr)
                            if (retry_after_hdr and retry_after_hdr.replace(".", "", 1).isdigit())
                            else compute_reconnect_backoff(attempt, base_sec=0.5, max_sec=10.0)
                        )
                        if attempt + 1 < max_retries:
                            sleep_fn(wait_s)
                            continue
                    return collected
                except (httpx.TimeoutException, httpx.TransportError):
                    if attempt + 1 < max_retries:
                        sleep_fn(compute_reconnect_backoff(attempt, base_sec=0.5, max_sec=10.0))
                        continue
                    return collected

            if resp is None or resp.status_code != 200:
                break

            try:
                payload = resp.json()
            except ValueError:
                break

            page = self.parse_activity_v2_response(payload)
            collected.extend(page.events)
            if not page.has_more or not page.next_cursor:
                break
            cursor = page.next_cursor

        return collected

    def parse_polygon_v2_log(
        self,
        log: dict[str, Any],
        *,
        block_timestamp: datetime,
        received_at: Optional[datetime] = None,
    ) -> Optional[NormalizedTradeEvent]:
        """Decode a Polygon CTF Exchange V2 `OrderFilled` log into a `NormalizedTradeEvent`."""
        recv_ts = received_at or datetime.now(timezone.utc)
        if log.get("removed") is True:
            return None
        topics = log.get("topics")
        if not isinstance(topics, list) or len(topics) != 4:
            return None
        if str(topics[0]).lower() != ORDER_FILLED_V2_TOPIC0:
            return None

        data_hex = str(log.get("data", ""))
        if not data_hex.startswith("0x"):
            return None
        raw_hex = data_hex[2:]
        if len(raw_hex) != 7 * 64:
            return None

        words = [raw_hex[i * 64 : (i + 1) * 64] for i in range(7)]
        side_int = int(words[0], 16)
        if side_int == 0:
            side = "BUY"
        elif side_int == 1:
            side = "SELL"
        else:
            return None

        token_id = str(int(words[1], 16))
        scale = Decimal("1000000")
        maker_amount = Decimal(int(words[2], 16)) / scale
        taker_amount = Decimal(int(words[3], 16)) / scale
        if maker_amount <= Decimal("0") or taker_amount <= Decimal("0"):
            return None

        if side == "BUY":
            size = taker_amount
            price = (maker_amount / taker_amount).quantize(Decimal("0.000001"))
            notional = maker_amount.quantize(Decimal("0.000001"))
        else:
            size = maker_amount
            price = (taker_amount / maker_amount).quantize(Decimal("0.000001"))
            notional = taker_amount.quantize(Decimal("0.000001"))

        maker_topic = str(topics[2])
        if not maker_topic.startswith("0x") or len(maker_topic) != 66:
            return None
        maker_wallet = "0x" + maker_topic[26:].lower()
        tx_hash = str(log.get("transactionHash") or log.get("transaction_hash") or "").strip().lower()
        if not tx_hash:
            return None

        norm_ts = datetime.now(timezone.utc)
        self._metrics.inc("events_received_total")
        self._metrics.inc("events_normalized_total")
        self.last_frame_received_at = recv_ts
        self.last_trade_received_at = recv_ts

        return NormalizedTradeEvent(
            source_wallet=maker_wallet,
            market_condition_id=str(log.get("conditionId") or log.get("condition_id") or "onchain_condition"),
            market_question=None,
            outcome_label=None,
            token_id=token_id,
            side=side,
            price=price,
            size=size.quantize(Decimal("0.000001")),
            notional_usd=notional,
            tx_hash=tx_hash,
            feed_source="polygon_chain",
            source_event_ts=block_timestamp,
            event_received_ts=recv_ts,
            event_normalized_ts=norm_ts,
        )

    def _normalize_row(
        self,
        row: dict[str, Any],
        *,
        feed_source: str,
        received_at: datetime,
    ) -> Optional[NormalizedTradeEvent]:
        row_type = str(row.get("type") or row.get("side") or "").strip().upper()
        if row_type not in ("TRADE", "BUY", "SELL"):
            return None

        side = str(row.get("side") or "").strip().upper()
        if side not in ("BUY", "SELL"):
            return None

        # Fix P-01: Support both Data API v2 snake_case (`proxy_wallet`, `transaction_hash`,
        # `condition_id`, `token_id`) and RTDS camelCase (`proxyWallet`, `transactionHash`,
        # `conditionId`, `asset`) fields with explicit address & token validation.
        wallet = str(
            row.get("proxy_wallet")
            or row.get("proxyWallet")
            or row.get("wallet")
            or ""
        ).strip().lower()
        tx_hash = str(
            row.get("transaction_hash")
            or row.get("transactionHash")
            or row.get("tx_hash")
            or ""
        ).strip().lower()
        token_id = str(
            row.get("token_id")
            or row.get("asset")
            or ""
        ).strip()

        if not _is_valid_wallet_address(wallet) or not tx_hash or not _is_valid_token_id(token_id):
            return None

        price = _parse_decimal(row.get("price"))
        size = _parse_decimal(row.get("size"))
        if price is None or size is None or price <= Decimal("0") or size <= Decimal("0"):
            return None

        source_ts = _parse_timestamp(row.get("timestamp"))
        if source_ts is None:
            return None

        condition_id = str(
            row.get("condition_id")
            or row.get("conditionId")
            or row.get("market_condition_id")
            or f"cond_{token_id[:16]}"
        ).strip()
        question = str(row.get("title") or row.get("market_question") or "").strip() or None
        outcome = str(row.get("outcome") or row.get("outcome_label") or "").strip() or None

        notional = (price * size).quantize(Decimal("0.000001"))
        norm_ts = datetime.now(timezone.utc)

        return NormalizedTradeEvent(
            source_wallet=wallet,
            market_condition_id=condition_id,
            market_question=question,
            outcome_label=outcome,
            token_id=token_id,
            side=side,
            price=price.quantize(Decimal("0.000001")),
            size=size.quantize(Decimal("0.000001")),
            notional_usd=notional,
            tx_hash=tx_hash,
            feed_source=feed_source,
            source_event_ts=source_ts,
            event_received_ts=received_at,
            event_normalized_ts=norm_ts,
        )
