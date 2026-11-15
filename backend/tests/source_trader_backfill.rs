//! Backfill status resolution (no live Polymarket).

use pommarket::source_trader_analytics::{
    backfill_stop_reason, resolve_sync_finish, IngestStopReason, SyncFinishInput,
};

#[test]
fn worker_requeues_ready_but_incomplete() {
    let partial = resolve_sync_finish(SyncFinishInput {
        backfill_complete: false,
        has_cursor: true,
        had_error: false,
    });
    assert_eq!(partial.status, "stale");
    assert_eq!(partial.phase, "backfill_partial");
    let done = resolve_sync_finish(SyncFinishInput {
        backfill_complete: true,
        has_cursor: false,
        had_error: false,
    });
    assert_eq!(done.phase, "complete");
}

#[test]
fn api_end_and_page_limit_distinct() {
    assert_eq!(backfill_stop_reason(IngestStopReason::ApiEnd), Some("api_end"));
    assert_eq!(backfill_stop_reason(IngestStopReason::PageLimit), Some("page_limit"));
    assert_eq!(backfill_stop_reason(IngestStopReason::EmptyBatch), Some("api_end"));
}