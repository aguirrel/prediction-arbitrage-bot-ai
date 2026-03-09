mod arbitrage;
mod config;
mod kalshi;
mod polymarket;
mod types;

use kalshi_rs::Account;
use tokio::sync::mpsc;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::types::PlatformUpdate;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Install rustls crypto provider before any TLS connections
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    // Load configuration
    let config = Config::from_env().map_err(|e| {
        error!(error = %e, "Failed to load config from environment");
        e
    })?;

    info!(
        kalshi_ticker_a = %config.kalshi_market_ticker_a,
        kalshi_ticker_b = %config.kalshi_market_ticker_b,
        "Starting arbitrage bot"
    );

    // Create channel for platform updates
    let (tx, rx) = mpsc::channel::<PlatformUpdate>(256);

    // Prepare Kalshi account
    let kalshi_account =
        Account::from_file(&config.kalshi_pem_path, &config.kalshi_api_key_id)?;

    let kalshi_ticker_a = config.kalshi_market_ticker_a.clone();
    let kalshi_ticker_b = config.kalshi_market_ticker_b.clone();
    let poly_asset_a = config.polymarket_asset_id_a.clone();
    let poly_asset_b = config.polymarket_asset_id_b.clone();

    let tx_kalshi = tx.clone();
    let tx_poly = tx;

    // Spawn Kalshi WebSocket task (subscribes to BOTH market tickers)
    let kalshi_handle = tokio::spawn(async move {
        if let Err(e) =
            kalshi::client::run(kalshi_account, kalshi_ticker_a, kalshi_ticker_b, tx_kalshi).await
        {
            error!(error = %e, "Kalshi WS task failed");
        }
    });

    // Spawn Polymarket WebSocket task
    let poly_handle = tokio::spawn(async move {
        if let Err(e) = polymarket::client::run(poly_asset_a, poly_asset_b, tx_poly).await {
            error!(error = %e, "Polymarket WS task failed");
        }
    });

    // Spawn arbitrage detector task
    let arb_handle = tokio::spawn(async move {
        if let Err(e) = arbitrage::run(&config, rx).await {
            error!(error = %e, "Arbitrage detector failed");
        }
    });

    // Wait for ctrl+c or any task to finish
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("Received Ctrl+C, shutting down...");
        }
        result = kalshi_handle => {
            if let Err(e) = result {
                error!(error = %e, "Kalshi task panicked");
            }
        }
        result = poly_handle => {
            if let Err(e) = result {
                error!(error = %e, "Polymarket task panicked");
            }
        }
        result = arb_handle => {
            if let Err(e) = result {
                error!(error = %e, "Arbitrage task panicked");
            }
        }
    }

    info!("Bot shut down");
    Ok(())
}
