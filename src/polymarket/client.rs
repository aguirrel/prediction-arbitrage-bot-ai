use std::str::FromStr;

use futures::StreamExt;
use polymarket_client_sdk::clob::ws;
use polymarket_client_sdk::types::U256;
use rust_decimal::Decimal;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::types::{PlatformUpdate, PolyBestAsks};

pub async fn run(
    asset_id_a: String,
    asset_id_b: String,
    tx: mpsc::Sender<PlatformUpdate>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let token_a =
        U256::from_str(&asset_id_a).map_err(|e| format!("Invalid asset_id_a: {e}"))?;
    let token_b =
        U256::from_str(&asset_id_b).map_err(|e| format!("Invalid asset_id_b: {e}"))?;

    info!(
        asset_a = %asset_id_a,
        asset_b = %asset_id_b,
        "Polymarket WS: connecting..."
    );

    let ws_client = ws::Client::default();
    let both = vec![token_a, token_b];

    // Book events: full snapshot on subscribe + after trades/cancellations
    let book_stream = ws_client.subscribe_orderbook(both.clone())?;
    // Price-change events: fires whenever the best bid/ask changes
    let price_stream = ws_client.subscribe_prices(both)?;

    info!("Polymarket WS: subscribed to orderbook + prices for both outcomes");

    let mut best_ask_a: Option<Decimal> = None;
    let mut best_ask_b: Option<Decimal> = None;

    let mut book_stream = Box::pin(book_stream);
    let mut price_stream = Box::pin(price_stream);

    loop {
        tokio::select! {
            Some(result) = book_stream.next() => {
                match result {
                    Ok(book) => {
                        let best_ask = book.asks.iter().map(|l| l.price).min();
                        info!(
                            asset_id = %book.asset_id,
                            best_ask = ?best_ask,
                            asks_len = book.asks.len(),
                            bids_len = book.bids.len(),
                            "Polymarket: book snapshot"
                        );
                        if book.asset_id == token_a {
                            best_ask_a = best_ask;
                        } else if book.asset_id == token_b {
                            best_ask_b = best_ask;
                        }
                        maybe_send(&mut best_ask_a, &mut best_ask_b, &tx).await?;
                    }
                    Err(e) => {
                        warn!(error = %e, "Polymarket WS: book stream error");
                    }
                }
            }
            Some(result) = price_stream.next() => {
                match result {
                    Ok(price_change) => {
                        for entry in &price_change.price_changes {
                            // best_ask is explicitly present when the ask side changes
                            if let Some(ask) = entry.best_ask {
                                debug!(
                                    asset_id = %entry.asset_id,
                                    best_ask = %ask,
                                    "Polymarket: price_change best ask"
                                );
                                if entry.asset_id == token_a {
                                    best_ask_a = Some(ask);
                                } else if entry.asset_id == token_b {
                                    best_ask_b = Some(ask);
                                }
                            }
                        }
                        maybe_send(&mut best_ask_a, &mut best_ask_b, &tx).await?;
                    }
                    Err(e) => {
                        warn!(error = %e, "Polymarket WS: price stream error");
                    }
                }
            }
            else => {
                error!("Polymarket WS: both streams ended");
                break;
            }
        }
    }

    Ok(())
}

async fn maybe_send(
    best_ask_a: &mut Option<Decimal>,
    best_ask_b: &mut Option<Decimal>,
    tx: &mpsc::Sender<PlatformUpdate>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if let (Some(a), Some(b)) = (*best_ask_a, *best_ask_b) {
        let update = PolyBestAsks { outcome_a: a, outcome_b: b };
        if tx.send(PlatformUpdate::Polymarket(update)).await.is_err() {
            info!("Polymarket WS: receiver dropped, shutting down");
        }
    }
    Ok(())
}
