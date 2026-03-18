use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::config::Config;
use crate::kalshi::executor as kalshi_exec;
use crate::polymarket::executor as poly_exec;
use crate::polymarket::executor::PolyClient;
use crate::types::{
    ArbitrageOpportunity, KalshiBestAsks, Platform, PlatformUpdate, PolyBestAsks, TradeLeg,
};

use alloy::signers::local::PrivateKeySigner;
use kalshi_rs::{Account, KalshiClient};
use polymarket_client_sdk::clob::types::Side as PolySide;

/// Estimated total fees as a decimal fraction.
/// Kalshi: ~2 cents per contract = 0.02
/// Polymarket: ~1-2% fee ≈ 0.02
/// Total buffer: ~0.04
const TOTAL_FEE_ESTIMATE: Decimal = dec!(0.04);

/// Minimum profit threshold to execute a trade
const MIN_PROFIT: Decimal = dec!(0.005);


pub async fn run(
    config: &Config,
    mut rx: mpsc::Receiver<PlatformUpdate>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut kalshi: Option<KalshiBestAsks> = None;
    let mut poly: Option<PolyBestAsks> = None;

    let kalshi_account = Account::from_file(&config.kalshi_pem_path, &config.kalshi_api_key_id)?;
    let kalshi_client = KalshiClient::new(kalshi_account);

    let (poly_client, poly_signer) = poly_exec::create_client(
        &config.polymarket_private_key,
        &config.polymarket_asset_id_a,
        &config.polymarket_asset_id_b,
        config.polymarket_signature_type,
        config.trade_quantity,
    )
    .await?;

    info!("Arbitrage detector: running");

    while let Some(update) = rx.recv().await {
        match update {
            PlatformUpdate::Kalshi(k) => {
                debug!(
                    a_yes = %k.market_a_yes, a_no = %k.market_a_no,
                    b_yes = %k.market_b_yes, b_no = %k.market_b_no,
                    "Arbitrage: Kalshi update"
                );
                kalshi = Some(k);
            }
            PlatformUpdate::Polymarket(p) => {
                debug!(
                    ask_a = %p.outcome_a, ask_b = %p.outcome_b,
                    "Arbitrage: Polymarket update"
                );
                poly = Some(p);
            }
        }

        let (k, p) = match (kalshi, poly) {
            (Some(k), Some(p)) => (k, p),
            _ => continue,
        };

        // === Cross-platform arbitrage strategies ===
        //
        // To guarantee profit we need to cover BOTH outcomes (A wins + B wins)
        // with one leg on each platform.
        //
        // Ways to bet "A wins":
        //   - Poly: buy outcome_a token         (price: p.outcome_a)
        //   - Kalshi: buy Market-A Yes           (price: k.market_a_yes)
        //   - Kalshi: buy Market-B No (B loses)  (price: k.market_b_no)
        //
        // Ways to bet "B wins":
        //   - Poly: buy outcome_b token          (price: p.outcome_b)
        //   - Kalshi: buy Market-B Yes           (price: k.market_b_yes)
        //   - Kalshi: buy Market-A No (A loses)  (price: k.market_a_no)

        // Evaluate strategies in order; stop at the first one that fires.
        // All four share legs so executing more than one would double-spend.

        // Strategy 1: Poly A + Kalshi Market-B Yes
        let fired = check_and_execute(
            TradeLeg {
                platform: Platform::Polymarket,
                description: "Poly A".into(),
                market_id: config.polymarket_asset_id_a.clone(),
                side: "buy".into(),
                price: p.outcome_a,
            },
            TradeLeg {
                platform: Platform::Kalshi,
                description: "Mkt-B Yes".into(),
                market_id: config.kalshi_market_ticker_b.clone(),
                side: "yes".into(),
                price: k.market_b_yes,
            },
            config,
            &kalshi_client,
            &poly_client,
            &poly_signer,
        )
        .await;

        // Strategy 2: Poly A + Kalshi Market-A No (A loses = B wins)
        let fired = fired || check_and_execute(
            TradeLeg {
                platform: Platform::Polymarket,
                description: "Poly A".into(),
                market_id: config.polymarket_asset_id_a.clone(),
                side: "buy".into(),
                price: p.outcome_a,
            },
            TradeLeg {
                platform: Platform::Kalshi,
                description: "Mkt-A No".into(),
                market_id: config.kalshi_market_ticker_a.clone(),
                side: "no".into(),
                price: k.market_a_no,
            },
            config,
            &kalshi_client,
            &poly_client,
            &poly_signer,
        )
        .await;

        // Strategy 3: Kalshi Market-A Yes + Poly B
        let fired = fired || check_and_execute(
            TradeLeg {
                platform: Platform::Kalshi,
                description: "Mkt-A Yes".into(),
                market_id: config.kalshi_market_ticker_a.clone(),
                side: "yes".into(),
                price: k.market_a_yes,
            },
            TradeLeg {
                platform: Platform::Polymarket,
                description: "Poly B".into(),
                market_id: config.polymarket_asset_id_b.clone(),
                side: "buy".into(),
                price: p.outcome_b,
            },
            config,
            &kalshi_client,
            &poly_client,
            &poly_signer,
        )
        .await;

        // Strategy 4: Kalshi Market-B No (B loses = A wins) + Poly B
        let fired = fired || {
            if !fired {
                check_and_execute(
                    TradeLeg {
                        platform: Platform::Kalshi,
                        description: "Mkt-B No".into(),
                        market_id: config.kalshi_market_ticker_b.clone(),
                        side: "no".into(),
                        price: k.market_b_no,
                    },
                    TradeLeg {
                        platform: Platform::Polymarket,
                        description: "Poly B".into(),
                        market_id: config.polymarket_asset_id_b.clone(),
                        side: "buy".into(),
                        price: p.outcome_b,
                    },
                    config,
                    &kalshi_client,
                    &poly_client,
                    &poly_signer,
                )
                .await
            } else {
                false
            }
        };

        if fired && config.exit_after_first_trade {
            info!("EXIT_AFTER_FIRST_TRADE is set — shutting down after first trade");
            return Ok(());
        }
    }

    info!("Arbitrage detector: channel closed, shutting down");
    Ok(())
}

/// Returns `true` if an opportunity was found (and execution attempted), `false` otherwise.
async fn check_and_execute(
    leg_a_wins: TradeLeg,
    leg_b_wins: TradeLeg,
    config: &Config,
    kalshi_client: &KalshiClient,
    poly_client: &PolyClient,
    poly_signer: &PrivateKeySigner,
) -> bool {
    let total_cost = leg_a_wins.price + leg_b_wins.price + TOTAL_FEE_ESTIMATE;

    if total_cost >= Decimal::ONE {
        return false;
    }

    let profit = Decimal::ONE - total_cost;
    if profit < MIN_PROFIT {
        return false;
    }

    let opportunity = ArbitrageOpportunity {
        leg_a_wins: leg_a_wins.clone(),
        leg_b_wins: leg_b_wins.clone(),
        total_cost,
        profit,
    };

    info!(%opportunity, "ARBITRAGE OPPORTUNITY DETECTED");

    // Execute both legs in parallel
    let (result_a, result_b) = tokio::join!(
        execute_leg(&leg_a_wins, config, kalshi_client, poly_client, poly_signer),
        execute_leg(&leg_b_wins, config, kalshi_client, poly_client, poly_signer),
    );

    match (&result_a, &result_b) {
        (Ok(id_a), Ok(id_b)) => {
            info!(
                leg_a_order = %id_a,
                leg_b_order = %id_b,
                profit = %profit,
                "Both legs executed successfully"
            );
        }
        _ => {
            if let Err(e) = &result_a {
                warn!(leg = %leg_a_wins, error = %e, "Leg A failed");
            }
            if let Err(e) = &result_b {
                warn!(leg = %leg_b_wins, error = %e, "Leg B failed");
            }
        }
    }

    true
}

async fn execute_leg(
    leg: &TradeLeg,
    config: &Config,
    kalshi_client: &KalshiClient,
    poly_client: &PolyClient,
    poly_signer: &PrivateKeySigner,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    match leg.platform {
        Platform::Kalshi => {
            kalshi_exec::place_order(
                kalshi_client,
                &leg.market_id,
                &leg.side,
                leg.price,
                config.trade_quantity,
            )
            .await
        }
        Platform::Polymarket => {
            poly_exec::place_order(
                poly_client,
                poly_signer,
                &leg.market_id,
                PolySide::Buy,
                leg.price,
                Decimal::from(config.trade_quantity),
            )
            .await
        }
    }
}
