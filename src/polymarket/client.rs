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

    // Subscribe to orderbook updates for both tokens
    let stream_a = ws_client.subscribe_orderbook(vec![token_a])?;
    let stream_b = ws_client.subscribe_orderbook(vec![token_b])?;

    info!("Polymarket WS: subscribed to orderbook for both outcomes");

    // Track current best asks for each outcome
    let mut best_ask_a: Option<Decimal> = None;
    let mut best_ask_b: Option<Decimal> = None;

    let mut stream_a = Box::pin(stream_a);
    let mut stream_b = Box::pin(stream_b);

    loop {
        tokio::select! {
            Some(result) = stream_a.next() => {
                match result {
                    Ok(book) => {
                        if let Some(ask) = book.asks.first() {
                            best_ask_a = Some(ask.price);
                            debug!(ask_a = %ask.price, "Polymarket: outcome A best ask");
                        } else {
                            best_ask_a = None;
                        }

                        if let (Some(a), Some(b)) = (best_ask_a, best_ask_b) {
                            let update = PolyBestAsks { outcome_a: a, outcome_b: b };
                            if tx.send(PlatformUpdate::Polymarket(update)).await.is_err() {
                                info!("Polymarket WS: receiver dropped, shutting down");
                                return Ok(());
                            }
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "Polymarket WS: stream A error");
                    }
                }
            }
            Some(result) = stream_b.next() => {
                match result {
                    Ok(book) => {
                        if let Some(ask) = book.asks.first() {
                            best_ask_b = Some(ask.price);
                            debug!(ask_b = %ask.price, "Polymarket: outcome B best ask");
                        } else {
                            best_ask_b = None;
                        }

                        if let (Some(a), Some(b)) = (best_ask_a, best_ask_b) {
                            let update = PolyBestAsks { outcome_a: a, outcome_b: b };
                            if tx.send(PlatformUpdate::Polymarket(update)).await.is_err() {
                                info!("Polymarket WS: receiver dropped, shutting down");
                                return Ok(());
                            }
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "Polymarket WS: stream B error");
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
