//! Signing and broadcasting a native SOL transfer from a managed wallet.
//!
//! This is the one place in the codebase that moves real money. Everything else
//! is observation or simulation. It is deliberately narrow: a single System
//! Program transfer, one signer, no token accounts, no CPI, no arbitrary
//! instruction data. A withdrawal is the only operation it can express, so a bug
//! elsewhere cannot turn it into a general-purpose signer.
//!
//! Withdrawals are disabled unless `WITHDRAWALS_ENABLED=true`.
//!
//! A send that times out is **not** a failed withdrawal. The transaction may
//! still land. Callers record the attempt before broadcasting and reconcile by
//! signature afterwards; they must never retry by rebuilding, because a second
//! transfer with a fresh blockhash is a second payment.
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{Value, json};

const SYSTEM_PROGRAM: [u8; 32] = [0u8; 32];
const TRANSFER_INSTRUCTION: u32 = 2;
/// Minimum balance a system account must retain to stay rent exempt. Draining
/// below this closes the account, so it is withheld from every withdrawal.
pub const RENT_EXEMPT_LAMPORTS: u64 = 890_880;
/// Reserved for the network fee of the withdrawal itself.
pub const FEE_RESERVE_LAMPORTS: u64 = 5_000;

pub fn enabled() -> bool {
    std::env::var("WITHDRAWALS_ENABLED").unwrap_or_default() == "true"
}

/// Lamports a user may actually take out: balance, less anything owed, less the
/// reserves the chain requires. Saturating throughout, so an account that cannot
/// afford a withdrawal reports zero rather than underflowing into a huge number.
pub fn withdrawable(balance: u64, owed: u64) -> u64 {
    balance
        .saturating_sub(owed)
        .saturating_sub(RENT_EXEMPT_LAMPORTS)
        .saturating_sub(FEE_RESERVE_LAMPORTS)
}

/// Solana's compact-u16 (ShortVec) length prefix.
fn compact_u16(mut n: u16) -> Vec<u8> {
    let mut out = vec![];
    loop {
        let mut byte = (n & 0x7f) as u8;
        n >>= 7;
        if n == 0 {
            out.push(byte);
            return out;
        }
        byte |= 0x80;
        out.push(byte);
    }
}

/// Serializes a legacy transaction message carrying exactly one transfer.
///
/// Account order is fixed by the layout: index 0 is the fee-paying signer, index
/// 1 the recipient, index 2 the read-only System Program.
pub fn build_message(
    from: &[u8; 32],
    to: &[u8; 32],
    lamports: u64,
    blockhash: &[u8; 32],
) -> Vec<u8> {
    let mut message = vec![1u8, 0, 1];
    message.extend(compact_u16(3));
    message.extend_from_slice(from);
    message.extend_from_slice(to);
    message.extend_from_slice(&SYSTEM_PROGRAM);
    message.extend_from_slice(blockhash);
    message.extend(compact_u16(1));
    message.push(2); // program id index: System Program
    message.extend(compact_u16(2));
    message.extend_from_slice(&[0, 1]); // from, to
    let mut data = TRANSFER_INSTRUCTION.to_le_bytes().to_vec();
    data.extend_from_slice(&lamports.to_le_bytes());
    message.extend(compact_u16(data.len() as u16));
    message.extend(data);
    message
}

/// Signs a message and returns the base64 wire transaction.
pub fn sign(message: &[u8], secret: &[u8]) -> Option<String> {
    use base64::Engine;
    let seed: [u8; 32] = secret.get(..32)?.try_into().ok()?;
    let key = SigningKey::from_bytes(&seed);
    let mut wire = compact_u16(1);
    wire.extend_from_slice(&key.sign(message).to_bytes());
    wire.extend_from_slice(message);
    Some(base64::engine::general_purpose::STANDARD.encode(wire))
}

async fn call(url: &str, method: &str, params: Value) -> Result<Value, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|_| "Cannot create RPC client")?;
    let response = client
        .post(url)
        .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
        .send()
        .await
        .map_err(|_| "RPC connection failed".to_string())?;
    if !response.status().is_success() {
        return Err(format!("RPC returned HTTP {}", response.status()));
    }
    let v: Value = response
        .json()
        .await
        .map_err(|_| "Unreadable RPC response")?;
    if let Some(e) = v.get("error") {
        return Err(format!(
            "RPC rejected the transaction: {}",
            e["message"].as_str().unwrap_or("unknown error")
        ));
    }
    Ok(v["result"].clone())
}

pub async fn latest_blockhash(url: &str) -> Result<[u8; 32], String> {
    let result = call(
        url,
        "getLatestBlockhash",
        json!([{"commitment":"finalized"}]),
    )
    .await?;
    let encoded = result["value"]["blockhash"]
        .as_str()
        .ok_or("No blockhash was returned")?;
    bs58::decode(encoded)
        .into_vec()
        .ok()
        .and_then(|v| <[u8; 32]>::try_from(v.as_slice()).ok())
        .ok_or("Blockhash was not a 32-byte value".into())
}

/// Broadcasts a signed transaction and returns its signature.
///
/// Preflight is left on: a withdrawal that would fail should fail before it
/// costs the user a fee.
pub async fn send(url: &str, wire_base64: &str) -> Result<String, String> {
    let result = call(
        url,
        "sendTransaction",
        json!([wire_base64, {"encoding":"base64","preflightCommitment":"confirmed","maxRetries":3}]),
    )
    .await?;
    result
        .as_str()
        .map(str::to_owned)
        .ok_or("No transaction signature was returned".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    #[test]
    fn withdrawable_never_underflows_and_always_withholds_reserves() {
        // 1 SOL balance, nothing owed.
        let expected = 1_000_000_000 - RENT_EXEMPT_LAMPORTS - FEE_RESERVE_LAMPORTS;
        assert_eq!(withdrawable(1_000_000_000, 0), expected);
        // Outstanding commission comes out first.
        assert_eq!(
            withdrawable(1_000_000_000, 500_000_000),
            expected - 500_000_000
        );
        // Owing more than the balance yields zero, not a wrapped value.
        assert_eq!(withdrawable(1_000, 999_999_999), 0);
        assert_eq!(withdrawable(0, 0), 0);
        // A balance that only covers the reserves is not withdrawable.
        assert_eq!(
            withdrawable(RENT_EXEMPT_LAMPORTS + FEE_RESERVE_LAMPORTS, 0),
            0
        );
    }
    #[test]
    fn compact_u16_matches_the_shortvec_encoding() {
        assert_eq!(compact_u16(0), vec![0]);
        assert_eq!(compact_u16(1), vec![1]);
        assert_eq!(compact_u16(127), vec![127]);
        assert_eq!(compact_u16(128), vec![0x80, 0x01]);
        assert_eq!(compact_u16(16383), vec![0xff, 0x7f]);
    }
    #[test]
    fn the_message_carries_exactly_one_system_transfer() {
        let from = [1u8; 32];
        let to = [2u8; 32];
        let blockhash = [3u8; 32];
        let m = build_message(&from, &to, 1_234_567, &blockhash);
        // header + 3 keys + blockhash + instruction
        assert_eq!(&m[..3], &[1, 0, 1], "one signer, one readonly unsigned");
        assert_eq!(m[3], 3, "exactly three accounts");
        assert_eq!(&m[4..36], &from);
        assert_eq!(&m[36..68], &to);
        assert_eq!(&m[68..100], &SYSTEM_PROGRAM, "program must be System");
        assert_eq!(&m[100..132], &blockhash);
        assert_eq!(m[132], 1, "exactly one instruction");
        assert_eq!(m[133], 2, "instruction targets the System Program");
        assert_eq!(&m[134..137], &[2, 0, 1], "two accounts: from, to");
        assert_eq!(m[137], 12, "12 bytes of instruction data");
        assert_eq!(&m[138..142], &TRANSFER_INSTRUCTION.to_le_bytes());
        assert_eq!(
            u64::from_le_bytes(m[142..150].try_into().unwrap()),
            1_234_567
        );
        assert_eq!(m.len(), 150, "no trailing bytes");
    }
    #[test]
    fn a_different_amount_or_recipient_produces_a_different_message() {
        let base = build_message(&[1u8; 32], &[2u8; 32], 1, &[3u8; 32]);
        assert_ne!(base, build_message(&[1u8; 32], &[2u8; 32], 2, &[3u8; 32]));
        assert_ne!(base, build_message(&[1u8; 32], &[9u8; 32], 1, &[3u8; 32]));
        // A fresh blockhash is a different transaction: never retry by rebuilding.
        assert_ne!(base, build_message(&[1u8; 32], &[2u8; 32], 1, &[4u8; 32]));
    }
    #[test]
    fn the_wire_transaction_is_signed_by_the_wallets_own_key() {
        let (_, secret) = crate::custody::generate();
        let from: [u8; 32] = secret[32..].try_into().unwrap();
        let message = build_message(&from, &[2u8; 32], 5_000, &[3u8; 32]);
        let wire = sign(&message, &secret).unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&wire)
            .unwrap();
        assert_eq!(bytes[0], 1, "one signature");
        assert_eq!(&bytes[65..], &message[..], "message follows the signature");
        let signature: [u8; 64] = bytes[1..65].try_into().unwrap();
        let key = ed25519_dalek::VerifyingKey::from_bytes(&from).unwrap();
        assert!(key.verify_strict(&message, &signature.into()).is_ok());
        // A truncated secret cannot be used to sign.
        assert!(sign(&message, &secret[..8]).is_none());
    }
}
