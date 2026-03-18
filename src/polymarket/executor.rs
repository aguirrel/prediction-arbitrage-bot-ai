use std::str::FromStr;

use alloy::signers::Signer;
use alloy::signers::local::{LocalSigner, PrivateKeySigner};
use polymarket_client_sdk::POLYGON;
use polymarket_client_sdk::auth::Normal;
use polymarket_client_sdk::auth::state::Authenticated;
use polymarket_client_sdk::clob::Client;
use polymarket_client_sdk::clob::types::{Side, SignatureType};
use polymarket_client_sdk::types::{Decimal, U256};
use tracing::info;

/// Alias for the authenticated Polymarket CLOB client.
pub type PolyClient = Client<Authenticated<Normal>>;

/// Create an authenticated Polymarket client and warm up the signing cache
/// by building + signing (but NOT posting) a dummy order for each token.
pub async fn create_client(
    private_key: &str,
    token_id_a: &str,
    token_id_b: &str,
    signature_type: SignatureType,
    trade_quantity: u64,
) -> Result<(PolyClient, PrivateKeySigner), Box<dyn std::error::Error + Send + Sync>> {
    let signer = LocalSigner::from_str(private_key)?.with_chain_id(Some(POLYGON));

    let client = Client::new("https://clob.polymarket.com", Default::default())?
        .authentication_builder(&signer)
        .signature_type(signature_type)
        .authenticate()
        .await?;

    let warmup_price = Decimal::from_str("0.99").unwrap();
    let warmup_size = Decimal::from(trade_quantity);

    // Warm up cache for token A
    let token_a = U256::from_str(token_id_a).map_err(|e| format!("Invalid token_id_a: {e}"))?;
    let order_a = client
        .limit_order()
        .token_id(token_a)
        .side(Side::Buy)
        .price(warmup_price)
        .size(warmup_size)
        .build()
        .await?;
    let _signed_a = client.sign(&signer, order_a).await?;
    info!(
        token_id = token_id_a,
        "Polymarket: cache warmed for token A"
    );

    // Warm up cache for token B
    let token_b = U256::from_str(token_id_b).map_err(|e| format!("Invalid token_id_b: {e}"))?;
    let order_b = client
        .limit_order()
        .token_id(token_b)
        .side(Side::Buy)
        .price(warmup_price)
        .size(warmup_size)
        .build()
        .await?;
    let _signed_b = client.sign(&signer, order_b).await?;
    info!(
        token_id = token_id_b,
        "Polymarket: cache warmed for token B"
    );

    Ok((client, signer))
}

/// Place a limit order on Polymarket using a pre-created client.
pub async fn place_order(
    client: &PolyClient,
    signer: &PrivateKeySigner,
    token_id: &str,
    side: Side,
    price: Decimal,
    quantity: Decimal,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let token = U256::from_str(token_id).map_err(|e| format!("Invalid token_id: {e}"))?;

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

    let signed_order = client.sign(signer, order).await?;
    let response = client.post_order(signed_order).await?;

    info!(order_id = %response.order_id, "Polymarket: order placed");
    Ok(response.order_id)
}
