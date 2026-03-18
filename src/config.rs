use std::env;

use polymarket_client_sdk::clob::types::SignatureType;

pub struct Config {
    pub kalshi_api_key_id: String,
    pub kalshi_pem_path: String,
    pub polymarket_private_key: String,
    /// Kalshi market ticker for outcome A (e.g., KXNCAAMBGAME-26FEB23LOUUNC-LOU)
    pub kalshi_market_ticker_a: String,
    /// Kalshi market ticker for outcome B (e.g., KXNCAAMBGAME-26FEB23LOUUNC-UNC)
    pub kalshi_market_ticker_b: String,
    pub polymarket_asset_id_a: String,
    pub polymarket_asset_id_b: String,
    pub polymarket_signature_type: SignatureType,
    /// If true, exit after the first arbitrage trade is sent (useful for testing).
    pub exit_after_first_trade: bool,
    /// Number of contracts to trade per leg (applies to both Kalshi and Polymarket).
    /// Kalshi: contracts (each pays $1 if wins).
    /// Polymarket: outcome tokens/shares (each pays $1 if wins).
    pub trade_quantity: u64,
}

impl Config {
    pub fn from_env() -> Result<Self, env::VarError> {
        dotenvy::dotenv().ok();

        let signature_type = match env::var("POLYMARKET_SIGNATURE_TYPE")
            .unwrap_or_else(|_| "eoa".into())
            .to_lowercase()
            .as_str()
        {
            "gnosis_safe" | "gnosissafe" => SignatureType::GnosisSafe,
            "proxy" => SignatureType::Proxy,
            _ => SignatureType::Eoa,
        };

        let exit_after_first_trade = env::var("EXIT_AFTER_FIRST_TRADE")
            .unwrap_or_default()
            .to_lowercase()
            == "true";

        Ok(Self {
            kalshi_api_key_id: env::var("KALSHI_API_KEY_ID")?,
            kalshi_pem_path: env::var("KALSHI_PEM_PATH")?,
            polymarket_private_key: env::var("POLYMARKET_PRIVATE_KEY")?,
            kalshi_market_ticker_a: env::var("KALSHI_MARKET_TICKER_A")?,
            kalshi_market_ticker_b: env::var("KALSHI_MARKET_TICKER_B")?,
            polymarket_asset_id_a: env::var("POLYMARKET_ASSET_ID_A")?,
            polymarket_asset_id_b: env::var("POLYMARKET_ASSET_ID_B")?,
            polymarket_signature_type: signature_type,
            exit_after_first_trade,
            trade_quantity: env::var("TRADE_QUANTITY")
                .unwrap_or_else(|_| "10".into())
                .parse()
                .unwrap_or(10),
        })
    }
}
