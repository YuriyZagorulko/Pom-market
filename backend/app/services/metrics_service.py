"""Prometheus-compatible metrics & 7-stage latency telemetry tracker."""

from __future__ import annotations

import threading
from dataclasses import dataclass, field
from datetime import datetime
from typing import Any, Optional


@dataclass
class LatencySample:
    detection_latency_ms: float
    normalization_latency_ms: float
    routing_latency_ms: float
    submission_latency_ms: Optional[float] = None
    ack_latency_ms: Optional[float] = None
    fill_latency_ms: Optional[float] = None
    end_to_end_latency_ms: Optional[float] = None


class PlatformMetricsRegistry:
    """Thread-safe metrics collector tracking event pipeline latency and counters."""

    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._counters: dict[str, int] = {
            "events_received_total": 0,
            "events_normalized_total": 0,
            "events_deduplicated_total": 0,
            "events_routed_total": 0,
            "jobs_created_total": 0,
            "jobs_monitoring_total": 0,
            "jobs_paper_total": 0,
            "jobs_live_total": 0,
            "jobs_skipped_total": 0,
            "orders_submitted_total": 0,
            "orders_filled_total": 0,
            "orders_partial_total": 0,
            "orders_rejected_total": 0,
            "orders_ambiguous_total": 0,
            "reconciliations_run_total": 0,
            "reconciliations_resolved_total": 0,
            "collector_reconnects_total": 0,
        }
        self._latency_samples: list[LatencySample] = field(default_factory=list)
        self._samples: list[LatencySample] = []

    def inc(self, metric_name: str, amount: int = 1) -> None:
        with self._lock:
            self._counters[metric_name] = self._counters.get(metric_name, 0) + amount

    def record_latency(
        self,
        *,
        source_event_ts: datetime,
        event_received_ts: datetime,
        event_normalized_ts: datetime,
        event_routed_ts: datetime,
        order_submitted_ts: Optional[datetime] = None,
        order_ack_ts: Optional[datetime] = None,
        fill_ts: Optional[datetime] = None,
    ) -> LatencySample:
        def _diff_ms(t1: datetime, t2: datetime) -> float:
            return max(0.0, (t2 - t1).total_seconds() * 1000.0)

        det_ms = _diff_ms(source_event_ts, event_received_ts)
        norm_ms = _diff_ms(event_received_ts, event_normalized_ts)
        route_ms = _diff_ms(event_normalized_ts, event_routed_ts)
        sub_ms = _diff_ms(event_routed_ts, order_submitted_ts) if order_submitted_ts else None
        ack_ms = (
            _diff_ms(order_submitted_ts, order_ack_ts)
            if (order_submitted_ts and order_ack_ts)
            else None
        )
        fill_ms = (
            _diff_ms(order_submitted_ts, fill_ts)
            if (order_submitted_ts and fill_ts)
            else None
        )
        e2e_ms = _diff_ms(event_received_ts, fill_ts) if fill_ts else None

        sample = LatencySample(
            detection_latency_ms=det_ms,
            normalization_latency_ms=norm_ms,
            routing_latency_ms=route_ms,
            submission_latency_ms=sub_ms,
            ack_latency_ms=ack_ms,
            fill_latency_ms=fill_ms,
            end_to_end_latency_ms=e2e_ms,
        )
        with self._lock:
            if len(self._samples) >= 5000:
                self._samples.pop(0)
            self._samples.append(sample)
        return sample

    def snapshot(self) -> dict[str, Any]:
        with self._lock:
            counters = dict(self._counters)
            samples = list(self._samples)

        if not samples:
            return {
                "counters": counters,
                "latency_ms": {
                    "sample_count": 0,
                    "avg_detection_ms": 0.0,
                    "avg_normalization_ms": 0.0,
                    "avg_routing_ms": 0.0,
                    "avg_execution_ms": 0.0,
                },
            }

        n = len(samples)
        avg_det = sum(s.detection_latency_ms for s in samples) / n
        avg_norm = sum(s.normalization_latency_ms for s in samples) / n
        avg_route = sum(s.routing_latency_ms for s in samples) / n
        exec_samples = [s.fill_latency_ms for s in samples if s.fill_latency_ms is not None]
        avg_exec = sum(exec_samples) / len(exec_samples) if exec_samples else 0.0

        return {
            "counters": counters,
            "latency_ms": {
                "sample_count": n,
                "avg_detection_ms": round(avg_det, 3),
                "avg_normalization_ms": round(avg_norm, 3),
                "avg_routing_ms": round(avg_route, 3),
                "avg_execution_ms": round(avg_exec, 3),
            },
        }

    def render_prometheus_text(self) -> str:
        snap = self.snapshot()
        lines: list[str] = []
        for k, v in snap["counters"].items():
            lines.append(f"# TYPE pommarket_{k} counter")
            lines.append(f"pommarket_{k} {v}")
        lat = snap["latency_ms"]
        for k, v in lat.items():
            lines.append(f"# TYPE pommarket_{k} gauge")
            lines.append(f"pommarket_{k} {v}")
        return "\n".join(lines) + "\n"


GLOBAL_METRICS = PlatformMetricsRegistry()
