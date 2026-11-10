//! Source trader equity for portfolio-relative copy sizing.
//!
//! ## Sources (Polymarket public Data API + optional Polygon RPC)
//!
//! | Component | Endpoint / method | Notes |
//! |-----------|-------------------|-------|
//! | Positions MTM | `GET /v2/value?user={proxy}` | Open position mark value; **excludes** free collateral |
//! | Combo positions | `GET /v2/positions/combos` | Stored for disclosure; **not added** to total (avoids overlap with `/v2/value`) |
//! | Wallet USDC | Polygon `balanceOf` on USDC.e + native USDC | Wallet balance only; **excludes CLOB/CTF escrow** |
//!
//! ## Total equity (estimated)
//!
//! `estimated_total = positions_value_usd + wallet_collateral_usd`
//!
//! We never set status `complete` until CLOB escrow is observable without the trader's credentials.
//! Portfolio-relative BUY uses status `partial` when the estimate is above `SOURCE_EQUITY_MIN_USD`.

use chrono::{DateTime, Duration, Utc};
use reqwest::Client;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::config::Config;
use crate::db::begin_as;
use crate::polymarket::http::PublicClient;

const POLYGON_USDC_E: &str = "0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174";
const POLYGON_USDC_NATIVE: &str = "0x3c499c542cEF5E3811eEb2e5c0d3Ee8bF4F3388d";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EquityStatus {
    /// Not used today: requires CLOB escrow visibility.
    Complete,
    /// Estimated total (positions MTM + wallet USDC); CLOB escrow excluded.
    Partial,
    Stale,
    Unavailable,
}

impl EquityStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Partial => "partial",
            Self::Stale => "stale",
            Self::Unavailable => "unavailable",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "complete" => Self::Complete,
            "partial" => Self::Partial,
            "stale" => Self::Stale,
            "ok" => Self::Partial, // legacy rows
            _ => Self::Unavailable,
        }
    }

    pub fn allows_portfolio_buy(self) -> bool {
        matches!(self, Self::Partial | Self::Complete)
    }

    pub fn is_estimated(self) -> bool {
        matches!(self, Self::Partial)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct EquitySnapshot {
    pub wallet_queried: String,
    pub positions_value_usd: Option<Decimal>,
    pub combo_positions_value_usd: Option<Decimal>,
    pub wallet_collateral_usd: Option<Decimal>,
    pub total_equity_usd: Option<Decimal>,
    pub status: EquityStatus,
    pub quality_note: Option<String>,
    pub observed_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub age_seconds: i64,
}

pub async fn fetch_live_equity(
    data: &PublicClient,
    cfg: &Config,
    proxy_wallet: &str,
) -> EquitySnapshot {
    let now = Utc::now();
    let max_age = Duration::seconds(cfg.source_equity_max_age_seconds);
    let mut positions_value = None;
    let mut combo_value = None;
    let mut wallet_collateral = None;
    let mut notes = vec![
        "Query address is the Polymarket proxy/profile wallet (source_traders.wallet_address).".into(),
        "Estimated total = positions MTM (/v2/value) + wallet USDC; combo value is informational only.".into(),
        "CLOB/CTF escrow collateral is not included — equity is partial, not complete.".into(),
    ];

    match data.portfolio_value_usd(proxy_wallet).await {
        Ok(v) => positions_value = Some(v),
        Err(e) => notes.push(format!("/v2/value failed: {e}")),
    }
    match data.combo_positions_value_usd(proxy_wallet).await {
        Ok(v) => combo_value = Some(v),
        Err(e) => notes.push(format!("/v2/positions/combos failed: {e}")),
    }
    if let Some(rpc) = &cfg.polygon_rpc_url {
        match wallet_usdc_balance(rpc, proxy_wallet).await {
            Ok(v) => wallet_collateral = Some(v),
            Err(e) => notes.push(format!("wallet USDC RPC failed: {e}")),
        }
    } else {
        notes.push("POLYGON_RPC_URL unset; wallet USDC unknown".into());
    }

    let status;
    let total;
    if positions_value.is_none() || wallet_collateral.is_none() {
        status = EquityStatus::Unavailable;
        total = None;
    } else {
        let pos = positions_value.unwrap_or(Decimal::ZERO);
        let cash = wallet_collateral.unwrap_or(Decimal::ZERO);
        total = Some((pos + cash).round_dp(8));
        if total.unwrap_or(Decimal::ZERO) < cfg.source_equity_min_usd {
            status = EquityStatus::Unavailable;
            notes.push(format!(
                "estimated equity below SOURCE_EQUITY_MIN_USD ({})",
                cfg.source_equity_min_usd
            ));
        } else {
            status = EquityStatus::Partial;
        }
    }

    EquitySnapshot {
        wallet_queried: proxy_wallet.to_lowercase(),
        positions_value_usd: positions_value,
        combo_positions_value_usd: combo_value,
        wallet_collateral_usd: wallet_collateral,
        total_equity_usd: total,
        quality_note: Some(notes.join(" ")),
        status,
        observed_at: now,
        expires_at: now + max_age,
        age_seconds: 0,
    }
}

pub async fn upsert_snapshot(
    pool: &PgPool,
    source_trader_id: Uuid,
    snap: &EquitySnapshot,
) -> anyhow::Result<()> {
    let mut tx = begin_as(pool, None, "worker").await?;
    let raw = json!({
        "positions_value_usd": snap.positions_value_usd,
        "combo_positions_value_usd": snap.combo_positions_value_usd,
        "wallet_collateral_usd": snap.wallet_collateral_usd,
        "total_equity_usd": snap.total_equity_usd,
        "quality_note": snap.quality_note,
    });
    sqlx::query(
        "INSERT INTO source_trader_equity_snapshots (
            source_trader_id, wallet_queried, positions_value_usd, combo_positions_value_usd,
            wallet_collateral_usd, total_equity_usd, status, quality_note, observed_at, expires_at, raw
         ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
         ON CONFLICT (source_trader_id) DO UPDATE SET
            wallet_queried = EXCLUDED.wallet_queried,
            positions_value_usd = EXCLUDED.positions_value_usd,
            combo_positions_value_usd = EXCLUDED.combo_positions_value_usd,
            wallet_collateral_usd = EXCLUDED.wallet_collateral_usd,
            total_equity_usd = EXCLUDED.total_equity_usd,
            status = EXCLUDED.status,
            quality_note = EXCLUDED.quality_note,
            observed_at = EXCLUDED.observed_at,
            expires_at = EXCLUDED.expires_at,
            raw = EXCLUDED.raw",
    )
    .bind(source_trader_id)
    .bind(&snap.wallet_queried)
    .bind(snap.positions_value_usd)
    .bind(snap.combo_positions_value_usd)
    .bind(snap.wallet_collateral_usd)
    .bind(snap.total_equity_usd)
    .bind(snap.status.as_str())
    .bind(&snap.quality_note)
    .bind(snap.observed_at)
    .bind(snap.expires_at)
    .bind(raw)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

type SnapshotRow = (
    String,
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
    String,
    Option<String>,
    DateTime<Utc>,
    DateTime<Utc>,
);

pub async fn load_snapshot(
    pool: &PgPool,
    source_trader_id: Uuid,
    cfg: &Config,
) -> anyhow::Result<Option<EquitySnapshot>> {
    let mut tx = begin_as(pool, None, "worker").await?;
    let row: Option<SnapshotRow> =
        sqlx::query_as(
            "SELECT wallet_queried, positions_value_usd, combo_positions_value_usd, wallet_collateral_usd,
                    total_equity_usd, status, quality_note, observed_at, expires_at
             FROM source_trader_equity_snapshots WHERE source_trader_id = $1",
        )
        .bind(source_trader_id)
        .fetch_optional(&mut *tx)
        .await?;
    tx.commit().await?;
    let Some((wallet, p, c, w, t, status, note, observed, expires)) = row else {
        return Ok(None);
    };
    let now = Utc::now();
    let mut status = EquityStatus::parse(&status);
    if status.allows_portfolio_buy() && expires < now {
        status = EquityStatus::Stale;
    }
    if status == EquityStatus::Stale
        && expires + Duration::seconds(cfg.source_equity_max_age_seconds) < now
    {
        status = EquityStatus::Unavailable;
    }
    let age = now.signed_duration_since(observed).num_seconds().max(0);
    Ok(Some(EquitySnapshot {
        wallet_queried: wallet,
        positions_value_usd: p,
        combo_positions_value_usd: c,
        wallet_collateral_usd: w,
        total_equity_usd: t,
        status,
        quality_note: note,
        observed_at: observed,
        expires_at: expires,
        age_seconds: age,
    }))
}

pub async fn ensure_fresh_snapshot(
    pool: &PgPool,
    data: &PublicClient,
    cfg: &Config,
    source_trader_id: Uuid,
    proxy_wallet: &str,
) -> anyhow::Result<EquitySnapshot> {
    if let Some(snap) = load_snapshot(pool, source_trader_id, cfg).await? {
        if snap.status.allows_portfolio_buy() {
            return Ok(snap);
        }
    }
    let live = fetch_live_equity(data, cfg, proxy_wallet).await;
    upsert_snapshot(pool, source_trader_id, &live).await?;
    Ok(live)
}

async fn wallet_usdc_balance(rpc: &str, wallet: &str) -> Result<Decimal, String> {
    let client = Client::new();
    let wallet = wallet.trim_start_matches("0x").to_lowercase();
    let addr = format!("0x{wallet:0>40}");
    let mut total = Decimal::ZERO;
    for token in [POLYGON_USDC_E, POLYGON_USDC_NATIVE] {
        let data = format!(
            "0x70a08231000000000000000000000000{}",
            addr.trim_start_matches("0x")
        );
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_call",
            "params": [{"to": token, "data": data}, "latest"]
        });
        let resp: Value = client
            .post(rpc)
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        let hex = resp
            .get("result")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "rpc missing result".to_string())?;
        total += hex_u256_decimal(hex)?;
    }
    Ok(total.round_dp(8))
}

fn hex_u256_decimal(hex: &str) -> Result<Decimal, String> {
    let clean = hex.trim_start_matches("0x");
    if clean.is_empty() || clean == "0" {
        return Ok(Decimal::ZERO);
    }
    let n = u128::from_str_radix(clean.chars().take(32).collect::<String>().as_str(), 16)
        .map_err(|e| e.to_string())?;
    Ok(Decimal::from(n) / Decimal::from(1_000_000u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_allows_buy_complete_would_too() {
        assert!(EquityStatus::Partial.allows_portfolio_buy());
        assert!(EquityStatus::Complete.allows_portfolio_buy());
        assert!(!EquityStatus::Unavailable.allows_portfolio_buy());
        assert!(!EquityStatus::Stale.allows_portfolio_buy());
        assert!(EquityStatus::Partial.is_estimated());
        assert!(!EquityStatus::Complete.is_estimated());
    }
}
