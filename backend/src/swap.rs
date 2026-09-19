//! Swap execution through Jupiter, non-custodially.
//!
//! This is the trading product. The service quotes a route, builds an unsigned
//! transaction, and hands it back; the user's own wallet signs and broadcasts
//! it. No key of theirs is ever held for this path.
//!
//! # Why the quote is never accepted from the client
//!
//! A quote fully determines the route, the amounts and the platform fee. If a
//! caller could hand us one, they could hand us a route we did not choose, a
//! slippage we did not agree, or a fee of zero. So the client sends *parameters*
//! and the server fetches the quote itself, immediately before building. A
//! quote is a server artefact from end to end.
//!
//! # Fees
//!
//! Jupiter takes no cut; the integrator sets `platformFeeBps` and names a token
//! account to receive it. The fee is charged inside the swap, so it either
//! settles with the trade or the trade does not happen. That is what removes
//! any need to hold customer funds to guarantee collection.
//!
//! Fees are only charged when a fee account is configured, and the account's
//! mint must be one of the two being swapped. No account, no fee — never a
//! silent fallback to charging nothing while claiming otherwise, and never a
//! charge we cannot actually receive.
use serde::Serialize;
use serde_json::{Value, json};

pub const DEFAULT_BASE_URL: &str = "https://api.jup.ag/swap/v2";
/// Verified against the live API: v2 names the signer `taker`, not
/// `userPublicKey` as v1 did.
const TAKER_FIELD: &str = "taker";

pub struct Config {
    pub base_url: String,
    pub platform_fee_bps: u16,
    /// Token account that receives the fee. Its mint must be part of the swap.
    pub fee_account: Option<String>,
    pub max_slippage_bps: u16,
    pub api_key: Option<String>,
}
impl Config {
    pub fn from_env() -> Self {
        let number = |key: &str, fallback: u64, max: u64| {
            std::env::var(key)
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(fallback)
                .min(max)
        };
        Self {
            base_url: std::env::var("JUPITER_BASE_URL").unwrap_or(DEFAULT_BASE_URL.into()),
            // Capped at 3%: a higher integrator fee than that is predatory, and
            // a typo should not be able to take a tenth of someone's order.
            platform_fee_bps: number("PLATFORM_FEE_BPS", 50, 300) as u16,
            fee_account: std::env::var("PLATFORM_FEE_ACCOUNT")
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty()),
            max_slippage_bps: number("MAX_SLIPPAGE_BPS", 1000, 5000) as u16,
            api_key: std::env::var("JUPITER_API_KEY")
                .ok()
                .filter(|k| !k.is_empty()),
        }
    }
    /// A fee is only requested when there is somewhere to put it.
    pub fn effective_fee_bps(&self) -> u16 {
        if self.fee_account.is_some() {
            self.platform_fee_bps
        } else {
            0
        }
    }
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Request {
    pub input_mint: String,
    pub output_mint: String,
    pub amount: u64,
    pub slippage_bps: u16,
}

/// Validates a swap request before any network call.
///
/// Returns the reason it is refused, so the caller can say why rather than
/// failing opaquely.
pub fn validate(
    input_mint: &str,
    output_mint: &str,
    amount: u64,
    slippage_bps: u16,
    cfg: &Config,
) -> Result<Request, String> {
    if !crate::rpc::valid_address(input_mint) || !crate::rpc::valid_address(output_mint) {
        return Err("Both mints must be valid Solana addresses".into());
    }
    if input_mint == output_mint {
        return Err("Input and output mint are the same".into());
    }
    if amount == 0 {
        return Err("Amount must be greater than zero".into());
    }
    // u64 lamports of SOL cannot exceed total supply by any sane margin; this
    // catches a units mistake before it reaches a router.
    if amount > 1_000_000_000_000_000 {
        return Err("Amount is implausibly large; check the units".into());
    }
    if slippage_bps == 0 {
        return Err("Slippage tolerance must be greater than zero".into());
    }
    if slippage_bps > cfg.max_slippage_bps {
        return Err(format!(
            "Slippage of {:.2}% exceeds the {:.2}% maximum this service will submit",
            slippage_bps as f64 / 100.0,
            cfg.max_slippage_bps as f64 / 100.0
        ));
    }
    Ok(Request {
        input_mint: input_mint.to_string(),
        output_mint: output_mint.to_string(),
        amount,
        slippage_bps,
    })
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Summary {
    pub in_amount: u64,
    pub out_amount: u64,
    /// Worst acceptable output at the requested slippage.
    pub minimum_out: u64,
    pub price_impact_pct: Option<f64>,
    pub platform_fee: u64,
    pub platform_fee_bps: u16,
    pub route: Vec<String>,
    pub warnings: Vec<String>,
}

/// Reads the parts of a quote a trader needs to make a decision.
///
/// Amounts arrive as decimal strings because they are u64; parsing them as
/// floats would silently lose precision on large token amounts.
pub fn summarize(quote: &Value, cfg: &Config) -> Option<Summary> {
    let amount = |key: &str| quote[key].as_str()?.parse::<u64>().ok();
    let in_amount = amount("inAmount")?;
    let out_amount = amount("outAmount")?;
    let minimum_out = amount("otherAmountThreshold").unwrap_or(0);
    let price_impact_pct = quote["priceImpactPct"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|v| v.is_finite());
    let route: Vec<String> = quote["routePlan"]
        .as_array()
        .map(|legs| {
            legs.iter()
                .filter_map(|l| l["swapInfo"]["label"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let mut warnings = vec![];
    if price_impact_pct.is_some_and(|p| p >= 0.05) {
        warnings.push(format!(
            "Price impact of {:.1}% — this order is large for the available liquidity",
            price_impact_pct.unwrap_or(0.0) * 100.0
        ));
    }
    if price_impact_pct.is_none() {
        warnings.push("Price impact was not reported for this route".into());
    }
    if route.is_empty() {
        warnings.push("The route was not reported".into());
    } else if route.len() > 2 {
        warnings.push(format!(
            "Routed through {} pools, which adds failure risk and cost",
            route.len()
        ));
    }
    Some(Summary {
        in_amount,
        out_amount,
        minimum_out,
        price_impact_pct,
        platform_fee: quote["platformFee"]["amount"]
            .as_str()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
        platform_fee_bps: cfg.effective_fee_bps(),
        route,
        warnings,
    })
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|_| "Cannot create swap client".into())
}

pub async fn quote(cfg: &Config, req: &Request) -> Result<Value, String> {
    let fee_bps = cfg.effective_fee_bps();
    let mut url = format!(
        "{}/quote?inputMint={}&outputMint={}&amount={}&slippageBps={}",
        cfg.base_url, req.input_mint, req.output_mint, req.amount, req.slippage_bps
    );
    if fee_bps > 0 {
        url.push_str(&format!("&platformFeeBps={fee_bps}"));
    }
    let mut request = client()?.get(url);
    if let Some(key) = &cfg.api_key {
        request = request.header("x-api-key", key);
    }
    let response = request
        .send()
        .await
        .map_err(|_| "Quote request failed".to_string())?;
    if response.status() == 429 {
        return Err("Router rate limit reached; retry shortly".into());
    }
    if !response.status().is_success() {
        return Err(format!(
            "Router returned HTTP {}",
            response.status().as_u16()
        ));
    }
    let v: Value = response.json().await.map_err(|_| "Unreadable quote")?;
    if v["outAmount"].as_str().is_none() {
        return Err("No route was found for this pair and size".into());
    }
    Ok(v)
}

#[derive(Debug, Serialize)]
pub struct Unsigned {
    /// Base64 versioned transaction, unsigned, for the user's wallet to sign.
    pub transaction: String,
    pub last_valid_block_height: Option<u64>,
    pub prioritization_fee_lamports: Option<u64>,
}

pub async fn build(cfg: &Config, quote: &Value, taker: &str) -> Result<Unsigned, String> {
    if !crate::rpc::valid_address(taker) {
        return Err("Signer address is not a valid Solana address".into());
    }
    let mut body = json!({
        "quoteResponse": quote,
        TAKER_FIELD: taker,
        "wrapAndUnwrapSol": true,
        "dynamicComputeUnitLimit": true,
    });
    if let Some(account) = &cfg.fee_account {
        body["feeAccount"] = json!(account);
    }
    let mut request = client()?.post(format!("{}/swap", cfg.base_url)).json(&body);
    if let Some(key) = &cfg.api_key {
        request = request.header("x-api-key", key);
    }
    let response = request
        .send()
        .await
        .map_err(|_| "Swap build failed".to_string())?;
    let status = response.status();
    let v: Value = response
        .json()
        .await
        .map_err(|_| "Unreadable swap response")?;
    if !status.is_success() {
        let detail = v["error"]["issues"][0]["message"]
            .as_str()
            .or_else(|| v["error"].as_str())
            .unwrap_or("router rejected the request");
        return Err(format!("Swap build failed: {detail}"));
    }
    let transaction = v["swapTransaction"]
        .as_str()
        .ok_or("Router returned no transaction")?;
    Ok(Unsigned {
        transaction: transaction.to_string(),
        last_valid_block_height: v["lastValidBlockHeight"].as_u64(),
        prioritization_fee_lamports: v["prioritizationFeeLamports"].as_u64(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    const SOL: &str = "So11111111111111111111111111111111111111112";
    const USDC: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
    fn cfg(fee_account: Option<&str>) -> Config {
        Config {
            base_url: DEFAULT_BASE_URL.into(),
            platform_fee_bps: 50,
            fee_account: fee_account.map(str::to_owned),
            max_slippage_bps: 1000,
            api_key: None,
        }
    }

    #[test]
    fn a_fee_is_only_requested_when_it_can_be_received() {
        assert_eq!(cfg(None).effective_fee_bps(), 0);
        assert_eq!(cfg(Some("someTokenAccount")).effective_fee_bps(), 50);
    }
    #[test]
    fn the_platform_fee_is_capped_so_a_typo_cannot_take_a_tenth() {
        unsafe { std::env::set_var("PLATFORM_FEE_BPS", "5000") };
        assert_eq!(Config::from_env().platform_fee_bps, 300);
        unsafe { std::env::remove_var("PLATFORM_FEE_BPS") };
    }
    #[test]
    fn requests_are_validated_before_any_network_call() {
        let c = cfg(None);
        assert!(validate(SOL, USDC, 1_000_000, 50, &c).is_ok());
        assert!(
            validate("nope", USDC, 1, 50, &c)
                .unwrap_err()
                .contains("valid Solana")
        );
        assert!(
            validate(SOL, SOL, 1, 50, &c)
                .unwrap_err()
                .contains("the same")
        );
        assert!(
            validate(SOL, USDC, 0, 50, &c)
                .unwrap_err()
                .contains("greater than zero")
        );
        assert!(
            validate(SOL, USDC, u64::MAX, 50, &c)
                .unwrap_err()
                .contains("implausibly large")
        );
        assert!(
            validate(SOL, USDC, 1, 0, &c)
                .unwrap_err()
                .contains("Slippage")
        );
        // Above the configured ceiling, with both numbers named.
        let refused = validate(SOL, USDC, 1, 2000, &c).unwrap_err();
        assert!(refused.contains("20.00%") && refused.contains("10.00%"));
    }
    /// Shape captured from the live v2 API.
    fn live_quote() -> Value {
        json!({
            "inputMint": SOL, "inAmount": "10000000", "outputMint": USDC,
            "outAmount": "1115074", "otherAmountThreshold": "1109499",
            "swapMode": "ExactIn", "slippageBps": 50,
            "platformFee": {"amount": "5603", "feeBps": 50},
            "priceImpactPct": "0",
            "routePlan": [{"swapInfo": {"label": "Whirlpool"}}]
        })
    }
    #[test]
    fn a_quote_is_summarized_without_losing_precision() {
        let s = summarize(&live_quote(), &cfg(Some("acct"))).unwrap();
        assert_eq!(s.in_amount, 10_000_000);
        assert_eq!(s.out_amount, 1_115_074);
        assert_eq!(s.minimum_out, 1_109_499);
        assert_eq!(s.platform_fee, 5603);
        assert_eq!(s.platform_fee_bps, 50);
        assert_eq!(s.route, vec!["Whirlpool".to_string()]);
        assert_eq!(s.price_impact_pct, Some(0.0));
        assert!(s.warnings.is_empty());
    }
    #[test]
    fn large_token_amounts_survive_the_round_trip() {
        // Beyond f64's exact integer range: parsing as a float would corrupt it.
        let mut q = live_quote();
        q["outAmount"] = json!("9007199254740993");
        assert_eq!(
            summarize(&q, &cfg(None)).unwrap().out_amount,
            9_007_199_254_740_993
        );
    }
    #[test]
    fn a_costly_or_convoluted_route_is_called_out() {
        let mut q = live_quote();
        q["priceImpactPct"] = json!("0.084");
        q["routePlan"] = json!([
            {"swapInfo":{"label":"A"}},{"swapInfo":{"label":"B"}},{"swapInfo":{"label":"C"}}
        ]);
        let s = summarize(&q, &cfg(None)).unwrap();
        assert!(
            s.warnings
                .iter()
                .any(|w| w.contains("Price impact of 8.4%"))
        );
        assert!(
            s.warnings
                .iter()
                .any(|w| w.contains("Routed through 3 pools"))
        );
    }
    #[test]
    fn an_unreported_impact_is_a_warning_not_a_zero() {
        let mut q = live_quote();
        q["priceImpactPct"] = Value::Null;
        let s = summarize(&q, &cfg(None)).unwrap();
        assert_eq!(s.price_impact_pct, None);
        assert!(s.warnings.iter().any(|w| w.contains("not reported")));
    }
    #[test]
    fn a_quote_missing_its_amounts_is_not_a_quote() {
        assert!(summarize(&json!({}), &cfg(None)).is_none());
        let mut q = live_quote();
        q["outAmount"] = json!(1115074); // a number, not the documented string
        assert!(summarize(&q, &cfg(None)).is_none());
    }
    #[test]
    fn no_fee_account_means_no_fee_reported() {
        let mut q = live_quote();
        q["platformFee"] = Value::Null;
        let s = summarize(&q, &cfg(None)).unwrap();
        assert_eq!(s.platform_fee, 0);
        assert_eq!(s.platform_fee_bps, 0);
    }
}
