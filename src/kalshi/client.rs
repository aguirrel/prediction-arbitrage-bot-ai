use kalshi_rs::websocket::models::{
    KalshiSocketMessage, OrderbookDelta, OrderbookSnapshot, OrderbookSnapshotMessage,
};
use kalshi_rs::{Account, KalshiWebsocketClient};
use rust_decimal::Decimal;
use std::collections::BTreeMap;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::types::{KalshiBestAsks, PlatformUpdate};

/// Local orderbook maintained from WS snapshots and deltas.
/// Maps price (in cents, 1-99) -> quantity for each side.
struct LocalOrderbook {
    yes: BTreeMap<u8, i64>,
    no: BTreeMap<u8, i64>,
}

impl LocalOrderbook {
    fn new() -> Self {
        Self {
            yes: BTreeMap::new(),
            no: BTreeMap::new(),
        }
    }

    fn apply_snapshot(&mut self, msg: &OrderbookSnapshotMessage) {
        self.yes.clear();
        self.no.clear();

        if let Some(ref yes_levels) = msg.yes {
            for &(price, qty) in yes_levels {
                if qty > 0 {
                    self.yes.insert(price, qty);
                }
            }
        }
        if let Some(ref no_levels) = msg.no {
            for &(price, qty) in no_levels {
                if qty > 0 {
                    self.no.insert(price, qty);
                }
            }
        }
    }

    fn apply_delta(&mut self, msg: &kalshi_rs::websocket::models::OrderbookDeltaMessage) {
        let book = match msg.side.as_str() {
            "yes" => &mut self.yes,
            "no" => &mut self.no,
            _ => return,
        };

        let entry = book.entry(msg.price).or_insert(0);
        *entry += msg.delta;
        if *entry <= 0 {
            book.remove(&msg.price);
        }
    }

    /// Returns the best (lowest) ask price for Yes side in cents.
    fn best_yes_ask(&self) -> Option<u8> {
        self.yes.keys().next().copied()
    }

    /// Returns the best (lowest) ask price for No side in cents.
    fn best_no_ask(&self) -> Option<u8> {
        self.no.keys().next().copied()
    }

    /// Convert cents to Decimal (0.01-0.99)
    fn cents_to_decimal(cents: u8) -> Decimal {
        Decimal::from(cents) / Decimal::from(100)
    }
}

pub async fn run(
    account: Account,
    ticker_a: String,
    ticker_b: String,
    tx: mpsc::Sender<PlatformUpdate>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut book_a = LocalOrderbook::new();
    let mut book_b = LocalOrderbook::new();

    loop {
        info!(
            ticker_a = %ticker_a,
            ticker_b = %ticker_b,
            "Kalshi WS: connecting..."
        );

        let ws_client = KalshiWebsocketClient::new(account.clone());

        if let Err(e) = ws_client.connect().await {
            error!(error = %e, "Kalshi WS: connection failed");
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            continue;
        }

        info!("Kalshi WS: connected, subscribing to orderbook_delta for both markets");

        // Subscribe to both markets in one call
        if let Err(e) = ws_client
            .subscribe(vec!["orderbook_delta"], vec![&ticker_a, &ticker_b])
            .await
        {
            error!(error = %e, "Kalshi WS: subscribe failed");
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            continue;
        }

        loop {
            match ws_client.next_message().await {
                Ok(msg) => match msg {
                    KalshiSocketMessage::OrderbookSnapshot(OrderbookSnapshot { msg, .. }) => {
                        let ticker = &msg.market_ticker;
                        debug!(ticker = %ticker, "Kalshi: orderbook snapshot");

                        if *ticker == ticker_a {
                            book_a.apply_snapshot(&msg);
                        } else if *ticker == ticker_b {
                            book_b.apply_snapshot(&msg);
                        }

                        if let Some(update) = build_update(&book_a, &book_b) {
                            debug!(
                                a_yes = %update.market_a_yes,
                                a_no = %update.market_a_no,
                                b_yes = %update.market_b_yes,
                                b_no = %update.market_b_no,
                                "Kalshi best asks"
                            );
                            if tx.send(PlatformUpdate::Kalshi(update)).await.is_err() {
                                info!("Kalshi WS: receiver dropped, shutting down");
                                return Ok(());
                            }
                        }
                    }
                    KalshiSocketMessage::OrderbookDelta(OrderbookDelta { msg, .. }) => {
                        if msg.market_ticker == ticker_a {
                            book_a.apply_delta(&msg);
                        } else if msg.market_ticker == ticker_b {
                            book_b.apply_delta(&msg);
                        }

                        if let Some(update) = build_update(&book_a, &book_b) {
                            if tx.send(PlatformUpdate::Kalshi(update)).await.is_err() {
                                info!("Kalshi WS: receiver dropped, shutting down");
                                return Ok(());
                            }
                        }
                    }
                    KalshiSocketMessage::SubscribedResponse(resp) => {
                        info!(
                            channel = %resp.msg.channel,
                            sid = resp.msg.sid,
                            "Kalshi WS: subscribed"
                        );
                    }
                    KalshiSocketMessage::ErrorResponse(err) => {
                        error!(
                            code = err.msg.code,
                            msg = %err.msg.msg,
                            "Kalshi WS: error response"
                        );
                    }
                    KalshiSocketMessage::Ping | KalshiSocketMessage::Pong => {}
                    KalshiSocketMessage::Close(_) => {
                        warn!("Kalshi WS: connection closed, reconnecting...");
                        break;
                    }
                    other => {
                        debug!("Kalshi WS: unhandled message: {:?}", other);
                    }
                },
                Err(e) => {
                    error!(error = %e, "Kalshi WS: error reading message, reconnecting...");
                    break;
                }
            }
        }

        // Reset orderbooks on reconnect
        book_a = LocalOrderbook::new();
        book_b = LocalOrderbook::new();
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

/// Build a KalshiBestAsks only if we have all 4 prices.
fn build_update(book_a: &LocalOrderbook, book_b: &LocalOrderbook) -> Option<KalshiBestAsks> {
    Some(KalshiBestAsks {
        market_a_yes: LocalOrderbook::cents_to_decimal(book_a.best_yes_ask()?),
        market_a_no: LocalOrderbook::cents_to_decimal(book_a.best_no_ask()?),
        market_b_yes: LocalOrderbook::cents_to_decimal(book_b.best_yes_ask()?),
        market_b_no: LocalOrderbook::cents_to_decimal(book_b.best_no_ask()?),
    })
}
