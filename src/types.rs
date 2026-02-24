use rust_decimal::Decimal;
use std::fmt;

/// Best asks from Kalshi's TWO markets for a binary event.
///
/// Kalshi models a binary event (A vs B) as two separate markets:
/// - Market A: Yes = A wins, No = A loses (= B wins)
/// - Market B: Yes = B wins, No = B loses (= A wins)
///
/// Each market has its own orderbook with yes/no ask prices.
#[derive(Debug, Clone, Copy)]
pub struct KalshiBestAsks {
    /// Best ask to buy "A wins" via Market A Yes side
    pub market_a_yes: Decimal,
    /// Best ask to buy "A loses" (= B wins) via Market A No side
    pub market_a_no: Decimal,
    /// Best ask to buy "B wins" via Market B Yes side
    pub market_b_yes: Decimal,
    /// Best ask to buy "B loses" (= A wins) via Market B No side
    pub market_b_no: Decimal,
}

/// Best asks from Polymarket for both outcome tokens.
#[derive(Debug, Clone, Copy)]
pub struct PolyBestAsks {
    /// Best ask for outcome A token
    pub outcome_a: Decimal,
    /// Best ask for outcome B token
    pub outcome_b: Decimal,
}

#[derive(Debug, Clone, Copy)]
pub enum Platform {
    Kalshi,
    Polymarket,
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Platform::Kalshi => write!(f, "Kalshi"),
            Platform::Polymarket => write!(f, "Polymarket"),
        }
    }
}

/// Describes a specific trade: which platform, which market/token, and which side.
#[derive(Debug, Clone)]
pub struct TradeLeg {
    pub platform: Platform,
    /// Human-readable description (e.g., "MKT-A Yes", "Poly A")
    pub description: String,
    /// For Kalshi: the market ticker. For Poly: the token ID.
    pub market_id: String,
    /// For Kalshi: "yes" or "no". For Poly: always "buy".
    pub side: String,
    pub price: Decimal,
}

impl fmt::Display for TradeLeg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {} @ {}", self.platform, self.description, self.price)
    }
}

#[derive(Debug, Clone)]
pub struct ArbitrageOpportunity {
    /// Leg that covers "A wins"
    pub leg_a_wins: TradeLeg,
    /// Leg that covers "B wins"
    pub leg_b_wins: TradeLeg,
    pub total_cost: Decimal,
    pub profit: Decimal,
}

impl fmt::Display for ArbitrageOpportunity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ARB: [A wins] {} + [B wins] {} = cost {} | profit {}",
            self.leg_a_wins, self.leg_b_wins, self.total_cost, self.profit,
        )
    }
}

#[derive(Debug)]
pub enum PlatformUpdate {
    Kalshi(KalshiBestAsks),
    Polymarket(PolyBestAsks),
}
