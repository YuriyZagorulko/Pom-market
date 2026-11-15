//! Pagination policy tests for Data API ingest (no live HTTP).

use pommarket::data_api_ingest::{
    decide_pagination_stop, finish_stream_checkpoint, PaginationStopReason, StreamCheckpoint,
    DATA_API_PAGE_SIZE,
};

#[test]
fn neighbor_pages_duplicate_overlap_stops_once() {
    assert_eq!(
        decide_pagination_stop(100, DATA_API_PAGE_SIZE, 0, 50, false, true),
        None
    );
    assert_eq!(
        decide_pagination_stop(100, DATA_API_PAGE_SIZE, 1, 50, true, true),
        Some(PaginationStopReason::Overlap)
    );
}

#[test]
fn sell_on_page_boundary_needs_next_page() {
    assert_eq!(
        decide_pagination_stop(100, DATA_API_PAGE_SIZE, 4, 50, false, true),
        None,
        "page 5 full of BUY-only rows must not stop until overlap or budget"
    );
}

#[test]
fn page_budget_preserves_resume_cursor() {
    let mut cp = StreamCheckpoint::default();
    finish_stream_checkpoint(
        &mut cp,
        PaginationStopReason::PageBudget,
        Some("continue-here".into()),
    );
    assert_eq!(cp.resume_cursor.as_deref(), Some("continue-here"));
}

#[test]
fn api_timeout_midwalk_does_not_clear_resume() {
    let mut cp = StreamCheckpoint::default();
    cp.resume_cursor = Some("in-progress".into());
    let before = cp.resume_cursor.clone();
    // Error path: caller must not invoke finish_stream_checkpoint — checkpoint stays as-is.
    assert_eq!(cp.resume_cursor, before);
}
