use kalshi_rs::portfolio::models::CreateOrderRequest;
use kalshi_rs::KalshiClient;
use polymarket_client_sdk::auth::Uuid;
use rust_decimal::Decimal;
use tracing::{error, info};

/// Place a limit order on Kalshi.
///
/// - `side`: "yes" or "no"
/// - `price`: Decimal price in 0.01-0.99 range (will be converted to cents)
/// - `quantity`: Number of contracts
pub async fn place_order(
    client: &KalshiClient,
    ticker: &str,
    side: &str,
    price: Decimal,
    quantity: u64,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // Use 99 cents as the limit price so the order always executes immediately
    // against any available ask. The arbitrage price is only used for opportunity
    // detection; at execution time we are willing to pay up to 99¢.
    let _ = price;
    let price_cents: u64 = 99;

    let (yes_price, no_price) = match side {
        "yes" => (Some(price_cents), None),
        "no" => (None, Some(price_cents)),
        _ => return Err(format!("Invalid side: {side}").into()),
    };

    let request = CreateOrderRequest {
        ticker: ticker.to_string(),
        side: side.to_string(),
        action: "buy".to_string(),
        count: quantity,
        client_order_id: Some(Uuid::new_v4().to_string()),
        type_: Some("limit".to_string()),
        yes_price,
        no_price,
        yes_price_dollars: None,
        no_price_dollars: None,
        expiration_ts: None,
        time_in_force: Some("good_till_canceled".to_string()),
        buy_max_cost: None,
        post_only: None,
        reduce_only: None,
        self_trade_prevention_type: None,
        order_group_id: None,
        cancel_order_on_pause: None,
    };

    info!(
        ticker = ticker,
        side = side,
        price_cents = price_cents,
        quantity = quantity,
        "Kalshi: placing order"
    );

    match client.create_order(&request).await {
        Ok(resp) => {
            info!(order_id = %resp.order, "Kalshi: order placed");
            Ok(resp.order.order_id)
        }
        Err(e) => {
            error!(error = %e, "Kalshi: order placement failed");
            Err(Box::new(e))
        }
    }
}
