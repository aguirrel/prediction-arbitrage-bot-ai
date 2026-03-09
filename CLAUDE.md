# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
# Build
cargo build

# Build release
cargo build --release

# Run
cargo run

# Check (faster than build, no linking)
cargo check

# Lint
cargo clippy

# Format
cargo fmt

# Run tests
cargo test

# Run a single test
cargo test <test_name>
```

Log verbosity is controlled via `RUST_LOG` (e.g., `RUST_LOG=debug cargo run`).

## Configuration

Copy `.env.example` to `.env` and fill in credentials. Config is loaded from environment via `src/config.rs`:

| Variable | Description |
|---|---|
| `KALSHI_API_KEY_ID` | Kalshi API key ID |
| `KALSHI_PEM_PATH` | Path to Kalshi private key PEM file |
| `POLYMARKET_PRIVATE_KEY` | Ethereum private key (hex, no 0x prefix) |
| `KALSHI_MARKET_TICKER_A` | Kalshi market ticker for outcome A |
| `KALSHI_MARKET_TICKER_B` | Kalshi market ticker for outcome B |
| `POLYMARKET_ASSET_ID_A` | Polymarket token ID for outcome A |
| `POLYMARKET_ASSET_ID_B` | Polymarket token ID for outcome B |
| `POLYMARKET_SIGNATURE_TYPE` | `eoa` (default), `proxy`, or `gnosis_safe` |

**Critical**: Outcomes A and B must be aligned across platforms — `KALSHI_MARKET_TICKER_A` and `POLYMARKET_ASSET_ID_A` must represent the same real-world outcome.

## Architecture

The bot watches live orderbooks on Kalshi and Polymarket for the same binary event and executes cross-platform arbitrage when the combined cost of covering both outcomes is less than $1.

### Data flow

```
Kalshi WS ──────────────────────────────────────┐
  (subscribe orderbook_delta for ticker_a/b)     │
  (maintains LocalOrderbook per market)          ├──> mpsc::channel<PlatformUpdate>(256)
                                                 │         │
Polymarket WS ──────────────────────────────────┘         │
  (subscribe_orderbook for token_a/b)                     ▼
                                                  arbitrage::run()
                                                    ├── detects opportunity
                                                    └── tokio::join!(execute leg_a, execute leg_b)
                                                           ├── kalshi::executor::place_order()
                                                           └── polymarket::executor::place_order()
```

### Module layout

- **`src/main.rs`** — Spawns three tasks (Kalshi WS, Polymarket WS, arbitrage detector) and waits for Ctrl+C or any task to exit.
- **`src/config.rs`** — Loads `Config` from env vars (via `dotenvy`).
- **`src/types.rs`** — Shared types: `KalshiBestAsks`, `PolyBestAsks`, `TradeLeg`, `ArbitrageOpportunity`, `PlatformUpdate`.
- **`src/kalshi/client.rs`** — Maintains a local orderbook (BTreeMap price→qty) from WebSocket snapshot+delta messages. Reconnects automatically on failure.
- **`src/polymarket/client.rs`** — Subscribes to two Polymarket orderbook streams and tracks best asks per outcome.
- **`src/arbitrage.rs`** — Receives `PlatformUpdate`s, evaluates 4 cross-platform strategies, and executes when `1 - (leg_a_price + leg_b_price + TOTAL_FEE_ESTIMATE) >= MIN_PROFIT`.
- **`src/kalshi/executor.rs`** — Places a Kalshi limit order via `kalshi_rs`.
- **`src/polymarket/executor.rs`** — Signs and posts a Polymarket limit order via `polymarket-client-sdk` with an `alloy` local signer.

### Arbitrage logic

Kalshi models a binary event as **two separate markets** (Market A and Market B), each with yes/no sides. This gives four ways to express "A wins" or "B wins". The bot checks all 4 pairings of one leg from each platform:

1. Poly outcome_a + Kalshi Market-B Yes
2. Poly outcome_a + Kalshi Market-A No
3. Kalshi Market-A Yes + Poly outcome_b
4. Kalshi Market-B No + Poly outcome_b

Constants in `src/arbitrage.rs`:
- `TOTAL_FEE_ESTIMATE = 0.04` (4 cents buffer for fees)
- `MIN_PROFIT = 0.005` (minimum profit per $1 payout to execute)
- `TRADE_QUANTITY = 10` contracts (Kalshi)
- `TRADE_SIZE = 10.0` USDC (Polymarket)

Both legs of an opportunity are executed concurrently with `tokio::join!`. If either leg fails, the error is logged but the other leg's result is preserved (no automatic unwind/hedge).
