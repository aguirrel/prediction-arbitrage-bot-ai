use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::config::Config;
use crate::kalshi::executor as kalshi_exec;
use crate::polymarket::executor as poly_exec;
use crate::types::{
    ArbitrageOpportunity, KalshiBestAsks, Platform, PlatformUpdate, PolyBestAsks, TradeLeg,
};

use kalshi_rs::{Account, KalshiClient};
use polymarket_client_sdk::clob::types::Side as PolySide;

/// Estimated total fees as a decimal fraction.
/// Kalshi: ~2 cents per contract = 0.02
/// Polymarket: ~1-2% fee ≈ 0.02
/// Total buffer: ~0.04
const TOTAL_FEE_ESTIMATE: Decimal = dec!(0.04);

/// Minimum profit threshold to execute a trade
const MIN_PROFIT: Decimal = dec!(0.005);

/// Default trade quantity (contracts) for Kalshi
const TRADE_QUANTITY: u64 = 10;
/// Default trade size in USDC for Polymarket
const TRADE_SIZE: Decimal = dec!(10.0);

pub async fn run(
    config: &Config,
    mut rx: mpsc::Receiver<PlatformUpdate>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut kalshi: Option<KalshiBestAsks> = None;
    let mut poly: Option<PolyBestAsks> = None;

    let kalshi_account = Account::from_file(&config.kalshi_pem_path, &config.kalshi_api_key_id)?;
    let kalshi_client = KalshiClient::new(kalshi_account);

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

        // Strategy 1: Poly A + Kalshi Market-B Yes
        check_and_execute(
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
        )
        .await;

        // Strategy 2: Poly A + Kalshi Market-A No (A loses = B wins)
        check_and_execute(
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
        )
        .await;

        // Strategy 3: Kalshi Market-A Yes + Poly B
        check_and_execute(
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
        )
        .await;

        // Strategy 4: Kalshi Market-B No (B loses = A wins) + Poly B
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
        )
        .await;
    }

    info!("Arbitrage detector: channel closed, shutting down");
    Ok(())
}

async fn check_and_execute(
    leg_a_wins: TradeLeg,
    leg_b_wins: TradeLeg,
    config: &Config,
    kalshi_client: &KalshiClient,
) {
    let total_cost = leg_a_wins.price + leg_b_wins.price + TOTAL_FEE_ESTIMATE;

    if total_cost >= Decimal::ONE {
        return;
    }

    let profit = Decimal::ONE - total_cost;
    if profit < MIN_PROFIT {
        return;
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
        execute_leg(&leg_a_wins, config, kalshi_client),
        execute_leg(&leg_b_wins, config, kalshi_client),
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
}

async fn execute_leg(
    leg: &TradeLeg,
    config: &Config,
    kalshi_client: &KalshiClient,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    match leg.platform {
        Platform::Kalshi => {
            kalshi_exec::place_order(
                kalshi_client,
                &leg.market_id,
                &leg.side,
                leg.price,
                TRADE_QUANTITY,
            )
            .await
        }
        Platform::Polymarket => {
            poly_exec::place_order(
                &config.polymarket_private_key,
                &leg.market_id,
                PolySide::Buy,
                leg.price,
                TRADE_SIZE,
            )
            .await
        }
    }
}
