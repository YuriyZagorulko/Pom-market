//! When to treat a source trade as stale for copy decisions (distinct from exchange risk).
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct StaleEvaluation {
    pub stale: bool,
    /// `event_ts` or `received_at` — which clock was used for the age test.
    pub reference_field: &'static str,
    pub age_seconds: i64,
    /// `received_at - event_ts` when both are known (ingestion lag).
    pub ingestion_lag_seconds: i64,
    pub max_stale_seconds: i32,
    /// Human-readable rationale for audit logs / copy_decision.checks.
    pub reason: &'static str,
}

/// Decide whether a source event is too old to copy.
///
/// - **RTDS / realtime**: age is measured from `event_ts` (trade time on chain).
/// - **Data API poll**: when ingestion lag is small, use `event_ts` (leader trade time).
///   When lag exceeds `max_stale`, the event was backfilled after sitting on the exchange;
///   still measure from `event_ts` so ancient trades are not copied.
/// - **Clock sanity**: if `event_ts` is in the future relative to `now`, use `received_at`.
pub fn evaluate_stale_data(
    event_ts: DateTime<Utc>,
    received_at: DateTime<Utc>,
    origin: &str,
    max_stale_seconds: i32,
    now: DateTime<Utc>,
) -> StaleEvaluation {
    let max_dur = Duration::seconds(max_stale_seconds as i64);
    let lag = received_at.signed_duration_since(event_ts);
    let ingestion_lag_seconds = lag.num_seconds().max(0);

    let (reference_ts, reference_field, reason) = if event_ts > now + Duration::seconds(30) {
        (received_at, "received_at", "event_ts_in_future")
    } else if origin == "data_api"
        && ingestion_lag_seconds > 0
        && ingestion_lag_seconds <= max_stale_seconds as i64
    {
        // Just ingested from Data API; processing delay should not reject fresh trades
        // that were missing from /v2/activity (common for high-volume wallets).
        (received_at, "received_at", "data_api_fresh_ingest")
    } else {
        (event_ts, "event_ts", "trade_time")
    };

    let age = now.signed_duration_since(reference_ts);
    let age_seconds = age.num_seconds().max(0);
    let stale = age > max_dur;

    StaleEvaluation {
        stale,
        reference_field,
        age_seconds,
        ingestion_lag_seconds,
        max_stale_seconds: max_stale_seconds,
        reason: if stale {
            "exceeded_max_stale"
        } else {
            reason
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rtds_uses_event_time() {
        let now = Utc::now();
        let event_ts = now - Duration::seconds(10);
        let received = now - Duration::seconds(2);
        let ev = evaluate_stale_data(event_ts, received, "rtds", 300, now);
        assert!(!ev.stale);
        assert_eq!(ev.reference_field, "event_ts");
        assert_eq!(ev.age_seconds, 10);
    }

    #[test]
    fn old_backfill_stays_stale() {
        let now = Utc::now();
        let event_ts = now - Duration::hours(2);
        let received = now;
        let ev = evaluate_stale_data(event_ts, received, "data_api", 300, now);
        assert!(ev.stale);
        assert_eq!(ev.reference_field, "event_ts");
        assert_eq!(ev.reason, "exceeded_max_stale");
    }

    #[test]
    fn data_api_just_fetched_uses_received_at() {
        let now = Utc::now();
        let event_ts = now - Duration::seconds(120);
        let received = now - Duration::milliseconds(500);
        let ev = evaluate_stale_data(event_ts, received, "data_api", 300, now);
        assert!(!ev.stale, "lag {:?} age {:?}", ev.ingestion_lag_seconds, ev.age_seconds);
        assert_eq!(ev.reference_field, "received_at");
    }

    #[test]
    fn future_event_ts_uses_received_at() {
        let now = Utc::now();
        let event_ts = now + Duration::hours(1);
        let received = now;
        let ev = evaluate_stale_data(event_ts, received, "rtds", 60, now);
        assert!(!ev.stale);
        assert_eq!(ev.reference_field, "received_at");
    }

    #[test]
    fn delayed_data_api_within_lag_stays_fresh() {
        let now = Utc::now();
        let event_ts = now - Duration::seconds(240);
        let received = now - Duration::seconds(30);
        let ev = evaluate_stale_data(event_ts, received, "data_api", 300, now);
        assert!(!ev.stale);
        assert_eq!(ev.reference_field, "received_at");
        assert_eq!(ev.ingestion_lag_seconds, 210);
    }

    #[test]
    fn rtds_sell_not_relaxed_vs_data_api() {
        let now = Utc::now();
        let event_ts = now - Duration::seconds(120);
        let received = now - Duration::seconds(5);
        let ev = evaluate_stale_data(event_ts, received, "rtds", 60, now);
        assert!(ev.stale);
        assert_eq!(ev.reference_field, "event_ts");
    }

    #[test]
    fn out_of_order_delivery_uses_trade_time_when_lag_high() {
        let now = Utc::now();
        let event_ts = now - Duration::hours(3);
        let received = now;
        let ev = evaluate_stale_data(event_ts, received, "data_api", 300, now);
        assert!(ev.stale);
        assert_eq!(ev.reference_field, "event_ts");
        assert_eq!(ev.reason, "exceeded_max_stale");
    }
}
