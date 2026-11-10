use std::str::FromStr;

use polymarket_client_sdk_v2::auth::state::Authenticated;
use polymarket_client_sdk_v2::auth::{LocalSigner, Normal, Signer};
use polymarket_client_sdk_v2::clob::types::request::BalanceAllowanceRequest;
use polymarket_client_sdk_v2::clob::types::{Amount, OrderType, Side, SignatureType};
use polymarket_client_sdk_v2::clob::{Client, Config};
use polymarket_client_sdk_v2::types::{Address, U256};
use polymarket_client_sdk_v2::POLYGON;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;

use crate::security::redact;

#[derive(Clone, Debug, Serialize)]
pub struct ConnectionReport {
    pub clob_ok: bool,
    pub signer_address: String,
    pub funder_address: String,
    pub balance_usd: Option<Decimal>,
    pub api_credentials_created: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct LiveOrderRequest {
    pub private_key: SecretString,
    pub signature_type: String,
    pub funder: Option<String>,
    pub token_id: String,
    pub side_buy: bool,
    pub limit_price: Decimal,
    pub shares: Decimal,
    pub notional_usd: Decimal,
}

#[derive(Clone, Debug, Serialize)]
pub struct LiveOrderResult {
    pub outcome: String,
    pub exchange_order_id: Option<String>,
    pub status: Option<String>,
    pub error: Option<String>,
    pub response: serde_json::Value,
}

pub async fn probe_account(
    clob_host: &str,
    private_key: &str,
    signature_type: &str,
    funder: Option<&str>,
) -> ConnectionReport {
    let signer = match signer_from_key(private_key) {
        Ok(signer) => signer,
        Err(error) => {
            return ConnectionReport {
                clob_ok: false,
                signer_address: String::new(),
                funder_address: funder.unwrap_or_default().to_string(),
                balance_usd: None,
                api_credentials_created: false,
                error: Some(error),
            };
        }
    };
    let signer_address = format!("{:?}", signer.address()).to_lowercase();
    let funder_address = funder.unwrap_or(&signer_address).to_string();
    let client =
        match authenticated(clob_host, &signer, signature_type, Some(&funder_address)).await {
            Ok(client) => client,
            Err(error) => {
                return ConnectionReport {
                    clob_ok: false,
                    signer_address,
                    funder_address,
                    balance_usd: None,
                    api_credentials_created: false,
                    error: Some(error),
                };
            }
        };
    let clob_ok = client.ok().await.is_ok();
    let balance_usd = read_balance(&client).await;
    ConnectionReport {
        clob_ok,
        signer_address,
        funder_address,
        balance_usd,
        api_credentials_created: true,
        error: None,
    }
}

pub async fn place_fak(clob_host: &str, request: LiveOrderRequest) -> LiveOrderResult {
    let signer = match signer_from_key(request.private_key.expose_secret()) {
        Ok(signer) => signer,
        Err(error) => return failed(error),
    };
    let client = match authenticated(
        clob_host,
        &signer,
        &request.signature_type,
        request.funder.as_deref(),
    )
    .await
    {
        Ok(client) => client,
        Err(error) => return unknown(error),
    };
    let side = if request.side_buy {
        Side::Buy
    } else {
        Side::Sell
    };
    let token = match U256::from_str(&request.token_id) {
        Ok(token) => token,
        Err(error) => return failed(redact(&error.to_string())),
    };
    let built = if request.side_buy {
        client
            .market_order()
            .token_id(token)
            .amount(match Amount::usdc(request.notional_usd) {
                Ok(amount) => amount,
                Err(error) => return failed(redact(&error.to_string())),
            })
            .side(side)
            .order_type(OrderType::FAK)
            .build()
            .await
    } else {
        client
            .limit_order()
            .token_id(token)
            .price(request.limit_price)
            .size(request.shares)
            .side(side)
            .order_type(OrderType::FAK)
            .build()
            .await
    };
    let order = match built {
        Ok(order) => order,
        Err(error) => {
            let message = redact(&error.to_string());
            if message.to_ascii_lowercase().contains("timeout") {
                return unknown(message);
            }
            return failed(message);
        }
    };
    let signed = match client.sign(&signer, order).await {
        Ok(signed) => signed,
        Err(error) => return failed(redact(&error.to_string())),
    };
    match client.post_order(signed).await {
        Ok(response) => LiveOrderResult {
            outcome: if response.success {
                "accepted".into()
            } else {
                "rejected".into()
            },
            exchange_order_id: Some(response.order_id),
            status: Some(response.status.to_string()),
            error: response.error_msg,
            response: serde_json::json!({
                "success": response.success,
                "making_amount": response.making_amount.to_string(),
                "taking_amount": response.taking_amount.to_string(),
                "status": response.status.to_string(),
                "trade_ids": response.trade_ids,
                "transaction_hashes": response.transaction_hashes.iter().map(|hash| format!("{hash:?}")).collect::<Vec<_>>()
            }),
        },
        Err(error) => {
            let message = redact(&error.to_string());
            let lower = message.to_ascii_lowercase();
            if lower.contains("timeout") || lower.contains("connection") || lower.contains("504") {
                unknown(message)
            } else {
                failed(message)
            }
        }
    }
}

pub fn classify_remote_order(
    status: &str,
    original: Decimal,
    matched: Decimal,
) -> crate::engine::RemoteObservation {
    let status = status.to_ascii_lowercase();
    if status.contains("cancel") {
        return crate::engine::RemoteObservation::Cancelled;
    }
    if original > Decimal::ZERO && matched > Decimal::ZERO && matched < original {
        return crate::engine::RemoteObservation::Partial;
    }
    if status == "matched"
        || status == "filled"
        || (original > Decimal::ZERO && matched >= original && matched > Decimal::ZERO)
    {
        return crate::engine::RemoteObservation::Filled;
    }
    if matches!(
        status.as_str(),
        "live" | "delayed" | "unmatched" | "open" | "submitted"
    ) {
        return crate::engine::RemoteObservation::Open;
    }
    crate::engine::RemoteObservation::Unrecognized
}

pub async fn observe_remote_order(
    clob_host: &str,
    private_key: &str,
    signature_type: &str,
    funder: Option<&str>,
    order_id: &str,
) -> Result<crate::engine::RemoteObservation, String> {
    let signer = signer_from_key(private_key)?;
    let client = authenticated(clob_host, &signer, signature_type, funder).await?;
    let response = client
        .order(order_id)
        .await
        .map_err(|e| redact(&e.to_string()))?;
    Ok(classify_remote_order(
        &response.status.to_string(),
        response.original_size,
        response.size_matched,
    ))
}

pub async fn lookup_order(
    clob_host: &str,
    private_key: &str,
    signature_type: &str,
    funder: Option<&str>,
    order_id: &str,
) -> Result<serde_json::Value, String> {
    let signer = signer_from_key(private_key)?;
    let client = authenticated(clob_host, &signer, signature_type, funder).await?;
    let response = client
        .order(order_id)
        .await
        .map_err(|e| redact(&e.to_string()))?;
    Ok(serde_json::json!({"status": format!("{response:?}")}))
}

pub fn address_of(private_key: &str) -> Result<String, String> {
    let signer = signer_from_key(private_key)?;
    Ok(format!("{:#x}", signer.address()))
}

fn signer_from_key(private_key: &str) -> Result<impl Signer + Clone, String> {
    let signer = LocalSigner::from_str(private_key).map_err(|e| redact(&e.to_string()))?;
    Ok(signer.with_chain_id(Some(POLYGON)))
}

async fn authenticated<S>(
    host: &str,
    signer: &S,
    signature_type: &str,
    funder: Option<&str>,
) -> Result<Client<Authenticated<Normal>>, String>
where
    S: Signer + Sync,
{
    let mut builder = Client::new(host, Config::default())
        .map_err(|e| redact(&e.to_string()))?
        .authentication_builder(signer)
        .signature_type(map_signature(signature_type));
    if signature_type != "eoa" {
        if let Some(funder) = funder {
            let address = Address::from_str(funder).map_err(|err| redact(&err.to_string()))?;
            builder = builder.funder(address);
        }
    }
    builder
        .authenticate()
        .await
        .map_err(|e| redact(&e.to_string()))
}

fn map_signature(value: &str) -> SignatureType {
    match value {
        "poly_proxy" => SignatureType::Proxy,
        "poly_gnosis_safe" => SignatureType::GnosisSafe,
        "poly_1271" => SignatureType::Poly1271,
        _ => SignatureType::Eoa,
    }
}

async fn read_balance(client: &Client<Authenticated<Normal>>) -> Option<Decimal> {
    let response = client
        .balance_allowance(BalanceAllowanceRequest::default())
        .await
        .ok()?;
    Some(response.balance)
}

fn failed(error: String) -> LiveOrderResult {
    LiveOrderResult {
        outcome: "rejected".into(),
        exchange_order_id: None,
        status: Some("rejected".into()),
        error: Some(error),
        response: serde_json::json!({}),
    }
}

fn unknown(error: String) -> LiveOrderResult {
    LiveOrderResult {
        outcome: "unknown".into(),
        exchange_order_id: None,
        status: Some("unknown".into()),
        error: Some(error),
        response: serde_json::json!({}),
    }
}
