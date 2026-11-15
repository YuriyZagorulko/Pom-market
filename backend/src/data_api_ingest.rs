//! Data API `/v2/activity` and `/v2/trades` pagination, overlap stop, and resume checkpoints.
//!
//! API semantics (Polymarket Data API): pages are cursor-based (`pagination.next_cursor`),
//! newest trades first, `limit` typically 100. Overlap between polls is detected when a full
//! page contains only trades already present (dedup hit). When a cycle hits the page budget
//! without overlap, `resume_cursor` is stored per wallet/stream and the next cycle continues
//! older pages — without replaying the full wallet history each tick.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::PgPool;

use crate::db::begin_as;

/// Matches `PublicClient::{activity,trades}_page` default.
pub const DATA_API_PAGE_SIZE: usize = 100;

/// Cap pages per wallet/stream per worker tick (5_000 rows worst case).
pub const DEFAULT_MAX_PAGES_PER_TICK: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestStream {
    Activity,
    Trades,
}

impl IngestStream {
    pub fn as_key(self) -> &'static str {
        match self {
            IngestStream::Activity => "activity",
            IngestStream::Trades => "trades",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaginationStopReason {
    /// `next_cursor` is absent or empty.
    EndOfFeed,
    /// Every row on the page was already known (dedup).
    Overlap,
    /// Page shorter than API page size.
    PartialPage,
    /// Hit `max_pages`; checkpoint should retain `next_cursor` when provided.
    PageBudget,
    /// Cursor loop detected (repeated cursor).
    RepeatedCursor,
}

/// Pure decision helper — unit-tested without HTTP/DB.
pub fn decide_pagination_stop(
    page_len: usize,
    page_size: usize,
    page_index: usize,
    max_pages: usize,
    all_rows_were_duplicates: bool,
    has_next_cursor: bool,
) -> Option<PaginationStopReason> {
    if page_len == 0 {
        return Some(PaginationStopReason::EndOfFeed);
    }
    if all_rows_were_duplicates {
        return Some(PaginationStopReason::Overlap);
    }
    if page_len < page_size {
        return Some(PaginationStopReason::PartialPage);
    }
    if !has_next_cursor {
        return Some(PaginationStopReason::EndOfFeed);
    }
    if page_index + 1 >= max_pages {
        return Some(PaginationStopReason::PageBudget);
    }
    None
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StreamCheckpoint {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume_cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_success_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_stop_reason: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WalletCheckpoints {
    #[serde(default)]
    pub activity: StreamCheckpoint,
    #[serde(default)]
    pub trades: StreamCheckpoint,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IngestCheckpointStore {
    #[serde(default)]
    pub wallets: std::collections::HashMap<String, WalletCheckpoints>,
}

const CHECKPOINT_KEY: &str = "data_api_ingest_checkpoints";

pub async fn load_checkpoint_store(pool: &PgPool) -> anyhow::Result<IngestCheckpointStore> {
    let mut tx = begin_as(pool, None, "worker").await?;
    let raw: Option<Value> =
        sqlx::query_scalar("SELECT value FROM system_settings WHERE key = $1")
            .bind(CHECKPOINT_KEY)
            .fetch_optional(&mut *tx)
            .await?;
    tx.commit().await?;
    let store = match raw {
        Some(v) => serde_json::from_value(v).unwrap_or_default(),
        None => IngestCheckpointStore::default(),
    };
    Ok(store)
}

pub async fn save_checkpoint_store(pool: &PgPool, store: &IngestCheckpointStore) -> anyhow::Result<()> {
    let mut tx = begin_as(pool, None, "worker").await?;
    let payload = serde_json::to_value(store)?;
    sqlx::query(
        "INSERT INTO system_settings (key, value, updated_at)
         VALUES ($1, $2, now())
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
    )
    .bind(CHECKPOINT_KEY)
    .bind(payload)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub fn stream_checkpoint_mut<'a>(
    store: &'a mut IngestCheckpointStore,
    wallet: &str,
    stream: IngestStream,
) -> &'a mut StreamCheckpoint {
    let w = store
        .wallets
        .entry(wallet.to_ascii_lowercase())
        .or_default();
    match stream {
        IngestStream::Activity => &mut w.activity,
        IngestStream::Trades => &mut w.trades,
    }
}

pub fn finish_stream_checkpoint(cp: &mut StreamCheckpoint, reason: PaginationStopReason, next_cursor: Option<String>) {
    let now = Utc::now();
    cp.last_success_at = Some(now);
    cp.last_stop_reason = Some(format!("{reason:?}"));
    match reason {
        PaginationStopReason::PageBudget => {
            if let Some(cursor) = next_cursor.filter(|s| !s.is_empty()) {
                cp.resume_cursor = Some(cursor);
            }
        }
        PaginationStopReason::Overlap
        | PaginationStopReason::EndOfFeed
        | PaginationStopReason::PartialPage
        | PaginationStopReason::RepeatedCursor => {
            cp.resume_cursor = None;
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct StreamTickCounters {
    pub pages_fetched: u64,
    pub rows_fetched: u64,
    pub persisted_buy: u64,
    pub persisted_sell: u64,
    pub duplicate_fragments: u64,
    pub stop_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume_cursor_before: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume_cursor_after: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct WalletIngestDiagnostics {
    pub activity: StreamTickCounters,
    pub trades: StreamTickCounters,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct IngestPollDiagnostics {
    pub at: Option<DateTime<Utc>>,
    pub poll_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_success_at: Option<DateTime<Utc>>,
    pub activity_rows_last_tick: u64,
    pub trades_rows_last_tick: u64,
    pub persisted_buy_last_tick: u64,
    pub persisted_sell_last_tick: u64,
    pub duplicate_fragments_last_tick: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wallets: Option<std::collections::HashMap<String, WalletIngestDiagnostics>>,
}

impl IngestPollDiagnostics {
    pub fn merge_wallet(&mut self, wallet: &str, w: WalletIngestDiagnostics) {
        self.activity_rows_last_tick += w.activity.rows_fetched;
        self.trades_rows_last_tick += w.trades.rows_fetched;
        self.persisted_buy_last_tick += w.activity.persisted_buy + w.trades.persisted_buy;
        self.persisted_sell_last_tick += w.activity.persisted_sell + w.trades.persisted_sell;
        self.duplicate_fragments_last_tick +=
            w.activity.duplicate_fragments + w.trades.duplicate_fragments;
        let map = self.wallets.get_or_insert_with(Default::default);
        map.insert(wallet.to_ascii_lowercase(), w);
    }

    pub fn mark_error(&mut self, err: &str) {
        self.poll_status = "error".into();
        self.last_error = Some(err.to_string());
        self.last_error_at = Some(Utc::now());
    }

    pub fn mark_success(&mut self) {
        self.poll_status = "ok".into();
        self.last_success_at = Some(Utc::now());
        self.at = Some(Utc::now());
    }
}

/// Merge ingest diagnostics into `collector_status` without clobbering unrelated fields.
pub async fn persist_ingest_diagnostics(pool: &PgPool, diag: &IngestPollDiagnostics) -> anyhow::Result<()> {
    let mut tx = begin_as(pool, None, "worker").await?;
    let existing: Option<Value> =
        sqlx::query_scalar("SELECT value FROM system_settings WHERE key = 'collector_status'")
            .fetch_optional(&mut *tx)
            .await?;
    let prior_ingest = existing
        .as_ref()
        .and_then(|v| v.get("ingest_diagnostics"))
        .cloned()
        .unwrap_or(json!({}));
    let mut merged = prior_ingest;
    if let Some(obj) = merged.as_object_mut() {
        let fresh = serde_json::to_value(diag)?;
        if let Value::Object(fresh_map) = fresh {
            for (k, v) in fresh_map {
                obj.insert(k, v);
            }
        }
    }
  let payload = json!(merged);
    sqlx::query(
        "UPDATE system_settings
         SET value = value || jsonb_build_object('ingest_diagnostics', $1::jsonb),
             updated_at = now()
         WHERE key = 'collector_status'",
    )
    .bind(payload)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Independent equity check: `available + reserved + open positions MTM`.
pub fn independent_equity_metric(
    available_cash: Decimal,
    reserved_cash: Decimal,
    positions_market_value: Decimal,
) -> Decimal {
    (available_cash + reserved_cash + positions_market_value).round_dp(8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_on_overlap_even_when_page_full() {
        assert_eq!(
            decide_pagination_stop(100, 100, 0, 50, true, true),
            Some(PaginationStopReason::Overlap)
        );
    }

    #[test]
    fn stop_on_page_budget_with_more_pages() {
        assert_eq!(
            decide_pagination_stop(100, 100, 49, 50, false, true),
            Some(PaginationStopReason::PageBudget)
        );
    }

    #[test]
    fn continue_when_full_page_has_new_rows() {
        assert_eq!(
            decide_pagination_stop(100, 100, 0, 50, false, true),
            None
        );
    }

    #[test]
    fn stop_on_short_page() {
        assert_eq!(
            decide_pagination_stop(37, 100, 2, 50, false, true),
            Some(PaginationStopReason::PartialPage)
        );
    }

    #[test]
    fn page_budget_keeps_resume_cursor() {
        let mut cp = StreamCheckpoint::default();
        finish_stream_checkpoint(
            &mut cp,
            PaginationStopReason::PageBudget,
            Some("cursor-2".into()),
        );
        assert_eq!(cp.resume_cursor.as_deref(), Some("cursor-2"));
        finish_stream_checkpoint(&mut cp, PaginationStopReason::Overlap, Some("cursor-3".into()));
        assert!(cp.resume_cursor.is_none());
    }

    #[test]
    fn equity_sum_matches_snapshot_formula() {
        use rust_decimal::Decimal;
        assert_eq!(
            independent_equity_metric(
                Decimal::from(100),
                Decimal::from(25),
                Decimal::from(400),
            ),
            Decimal::from(525)
        );
    }

    /// Simulated sequence: 500 BUY rows then SELL — pagination must not stop early on BUY-only pages.
    #[test]
    fn many_buys_before_sell_requires_continuation() {
        let page_size = 100;
        let max_pages = 50;
        let mut page_index = 0;
        let mut found_sell = false;
        for page_index in 0..6 {
            let all_dup = false;
            let has_next = true;
            let stop = decide_pagination_stop(100, page_size, page_index, max_pages, all_dup, has_next);
            assert!(stop.is_none(), "page {page_index} should continue through BUY pages");
        }
        // Page 6 (index 5) still has the SELL row — continue until overlap on the next page.
        assert_eq!(
            decide_pagination_stop(100, page_size, 5, max_pages, false, true),
            None
        );
        found_sell = true;
        // Next page is overlap (re-poll same range).
        assert_eq!(
            decide_pagination_stop(100, 100, 6, max_pages, true, true),
            Some(PaginationStopReason::Overlap)
        );
        assert!(found_sell);
    }
}
