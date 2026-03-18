use std::str::FromStr;

use alloy::signers::Signer;
use alloy::signers::local::LocalSigner;
use kalshi_rs::{Account, KalshiClient};
use polymarket_client_sdk::clob::Client;
use polymarket_client_sdk::clob::types::request::BalanceAllowanceRequest;
use polymarket_client_sdk::POLYGON;
use tracing::{info, warn};

use crate::config::Config;

pub async fn check_and_log(config: &Config) {
    check_kalshi(config).await;
    check_polymarket(config).await;
}

async fn check_kalshi(config: &Config) {
    let account = match Account::from_file(&config.kalshi_pem_path, &config.kalshi_api_key_id) {
        Ok(a) => a,
        Err(e) => {
            warn!(error = %e, "Kalshi: could not load account for balance check");
            return;
        }
    };
    let client = KalshiClient::new(account);
    match client.get_balance().await {
        Ok(resp) => {
            // Kalshi returns balance in cents
            let dollars = resp.balance as f64 / 100.0;
            info!(balance_usd = dollars, "Kalshi balance");
        }
        Err(e) => warn!(error = %e, "Kalshi: balance check failed"),
    }
}

async fn check_polymarket(config: &Config) {
    let signer = match LocalSigner::from_str(&config.polymarket_private_key) {
        Ok(s) => s.with_chain_id(Some(POLYGON)),
        Err(e) => {
            warn!(error = %e, "Polymarket: could not build signer for balance check");
            return;
        }
    };
    let client = match Client::new("https://clob.polymarket.com", Default::default()) {
        Ok(c) => c,
        Err(e) => {
            warn!(error = %e, "Polymarket: could not create client for balance check");
            return;
        }
    };
    let client = match client
        .authentication_builder(&signer)
        .signature_type(config.polymarket_signature_type)
        .authenticate()
        .await
    {
        Ok(c) => c,
        Err(e) => {
            warn!(error = %e, "Polymarket: authentication failed for balance check");
            return;
        }
    };
    match client.balance_allowance(BalanceAllowanceRequest::default()).await {
        Ok(resp) => {
            info!(
                balance_usdc = %resp.balance,
                allowances = resp.allowances.len(),
                "Polymarket balance"
            );
        }
        Err(e) => warn!(error = %e, "Polymarket: balance check failed"),
    }
}
