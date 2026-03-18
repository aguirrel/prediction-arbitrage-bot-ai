use std::str::FromStr;

use alloy::signers::Signer;
use alloy::signers::local::LocalSigner;
use polymarket_client_sdk::clob::types::{Side, SignatureType};
use polymarket_client_sdk::clob::Client;
use polymarket_client_sdk::types::{Decimal, U256};
use polymarket_client_sdk::POLYGON;
use tracing::info;

/// Place a limit order on Polymarket.
///
/// - `token_id`: The asset/token ID for the outcome
/// - `side`: Buy or Sell
/// - `price`: Decimal price 0.01-0.99
/// - `size`: Size in USDC
/// - `signature_type`: Eoa, Proxy, or GnosisSafe
pub async fn place_order(
    private_key: &str,
    token_id: &str,
    side: Side,
    price: Decimal,
    quantity: Decimal,
    signature_type: SignatureType,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let signer = LocalSigner::from_str(private_key)?
        .with_chain_id(Some(POLYGON));

    let client = Client::new("https://clob.polymarket.com", Default::default())?
        .authentication_builder(&signer)
        .signature_type(signature_type)
        .authenticate()
        .await?;

    let token = U256::from_str(token_id)
        .map_err(|e| format!("Invalid token_id: {e}"))?;

    // Use 0.99 as the limit price so the order always executes immediately
    // against any available ask. The arbitrage price is only used for opportunity
    // detection; at execution time we are willing to pay up to 99¢.
    let fill_price = Decimal::from_str("0.99").unwrap();

    info!(
        token_id = token_id,
        side = ?side,
        detected_price = %price,
        fill_price = %fill_price,
        quantity = %quantity,
        "Polymarket: placing order"
    );

    let order = client
        .limit_order()
        .token_id(token)
        .side(side)
        .price(fill_price)
        .size(quantity)
        .build()
        .await?;

    let signed_order = client.sign(&signer, order).await?;
    let response = client.post_order(signed_order).await?;

    info!(order_id = %response.order_id, "Polymarket: order placed");
    Ok(response.order_id)
}
