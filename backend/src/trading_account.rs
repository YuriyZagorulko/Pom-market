//! Trading account kind: wallet-connected vs paper-only (no Polymarket wallet).

pub const ACCOUNT_TYPE_PAPER: &str = "paper";
pub const ACCOUNT_TYPE_WALLET: &str = "wallet";

/// API / activation error when live trading is requested on a paper-only account.
pub const PAPER_ACCOUNT_NO_LIVE_CODE: &str = "paper_account_no_live_trading";

pub fn parse_account_type(raw: Option<&str>) -> Result<&'static str, &'static str> {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        None | Some("wallet") => Ok(ACCOUNT_TYPE_WALLET),
        Some("paper") => Ok(ACCOUNT_TYPE_PAPER),
        Some(_) => Err("account_type must be paper or wallet"),
    }
}

pub fn is_paper_account(account_type: &str) -> bool {
    account_type == ACCOUNT_TYPE_PAPER
}