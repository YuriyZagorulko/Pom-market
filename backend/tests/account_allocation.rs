use pommarket::account_allocation::{effective_allocation_cap, AllocationMode, StrategyAllocation};
use rust_decimal::Decimal;

fn d(v: &str) -> Decimal {
    v.parse().unwrap()
}

#[test]
fn fixed_allocation_ignores_account_equity() {
    let alloc = StrategyAllocation {
        allocation_mode: AllocationMode::FixedUsd,
        strategy_budget_usd: d("2000"),
        allocation_percent: None,
    };
    assert_eq!(effective_allocation_cap(&alloc, d("5000")), d("2000"));
    assert_eq!(effective_allocation_cap(&alloc, d("6000")), d("2000"));
}

#[test]
fn percent_allocation_tracks_equity() {
    let alloc = StrategyAllocation {
        allocation_mode: AllocationMode::PercentOfAccount,
        strategy_budget_usd: d("0"),
        allocation_percent: Some(d("40")),
    };
    assert_eq!(effective_allocation_cap(&alloc, d("5000")), d("2000"));
    assert_eq!(effective_allocation_cap(&alloc, d("6000")), d("2400"));
}
