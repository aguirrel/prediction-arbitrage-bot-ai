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
    size: Decimal,
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

    info!(
        token_id = token_id,
        side = ?side,
        price = %price,
        size = %size,
        "Polymarket: placing order"
    );

    let order = client
        .limit_order()
        .token_id(token)
        .side(side)
        .price(price)
        .size(size)
        .build()
        .await?;

    let signed_order = client.sign(&signer, order).await?;
    let response = client.post_order(signed_order).await?;

    info!(order_id = %response.order_id, "Polymarket: order placed");
    Ok(response.order_id)
}
