//! Sign-in with a browser wallet: Phantom (Solana, ed25519) and MetaMask
//! (Ethereum, secp256k1 over EIP-191 `personal_sign`).
//!
//! Signing a login message proves control of an address. It authorizes nothing
//! else — no transfer, no allowance, no delegation — and the message says so in
//! plain language so a user reading their wallet prompt is not misled.
//!
//! Replay is prevented on three axes: the message carries the origin domain, a
//! single-use server-issued nonce, and an expiry. The nonce is consumed by the
//! verifying statement itself, so two concurrent submissions of one signature
//! cannot both succeed.
use k256::ecdsa::{RecoveryId, Signature as EcdsaSignature, VerifyingKey};
use sha3::{Digest, Keccak256};

pub const CHALLENGE_TTL_SECONDS: i64 = 300;

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Chain {
    Solana,
    Ethereum,
}
impl Chain {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "solana" => Some(Self::Solana),
            "ethereum" => Some(Self::Ethereum),
            _ => None,
        }
    }
    pub fn name(&self) -> &'static str {
        match self {
            Self::Solana => "solana",
            Self::Ethereum => "ethereum",
        }
    }
}

/// Canonical address form. Ethereum is compared lowercased because wallets
/// disagree on checksum casing; Solana base58 is case-significant and is not
/// folded.
pub fn normalize(chain: Chain, address: &str) -> Option<String> {
    match chain {
        Chain::Solana => crate::rpc::valid_address(address).then(|| address.to_string()),
        Chain::Ethereum => {
            let lower = address.trim().to_lowercase();
            let hex = lower.strip_prefix("0x")?;
            (hex.len() == 40 && hex.chars().all(|c| c.is_ascii_hexdigit())).then_some(lower)
        }
    }
}

/// The exact text the wallet displays. Any change here invalidates outstanding
/// challenges, which is correct: the signature covers this string verbatim.
pub fn message(domain: &str, chain: Chain, address: &str, nonce: &str, issued: i64) -> String {
    format!(
        "{domain} wants you to sign in with your {} account:\n{address}\n\nSign in to Unskilled Trade. This signature proves you control this address. It does not approve a transaction, a transfer or a spending allowance.\n\nNonce: {nonce}\nIssued At: {issued}\nExpires In: {CHALLENGE_TTL_SECONDS} seconds",
        chain.name()
    )
}

/// Verifies an ed25519 signature over the exact message bytes.
pub fn verify_solana(address: &str, message: &str, signature_b58: &str) -> bool {
    let Ok(key) = bs58::decode(address).into_vec() else {
        return false;
    };
    let Ok(sig) = bs58::decode(signature_b58).into_vec() else {
        return false;
    };
    let (Ok(key), Ok(sig)) = (
        <[u8; 32]>::try_from(key.as_slice()),
        <[u8; 64]>::try_from(sig.as_slice()),
    ) else {
        return false;
    };
    ed25519_dalek::VerifyingKey::from_bytes(&key)
        .is_ok_and(|k| k.verify_strict(message.as_bytes(), &sig.into()).is_ok())
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    (s.len().is_multiple_of(2))
        .then(|| {
            (0..s.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
                .collect::<Option<Vec<u8>>>()
        })
        .flatten()
}

/// EIP-191 prefixed digest, which is what `personal_sign` actually signs.
fn eip191(message: &str) -> [u8; 32] {
    let mut hasher = Keccak256::new();
    hasher.update(format!("\x19Ethereum Signed Message:\n{}", message.len()).as_bytes());
    hasher.update(message.as_bytes());
    hasher.finalize().into()
}

/// Recovers the signer and compares it to the claimed address.
///
/// Recovery, not verification: `personal_sign` returns `r||s||v` without a public
/// key, so the address is derived from the signature and only then compared.
pub fn verify_ethereum(address: &str, message: &str, signature_hex: &str) -> bool {
    let Some(bytes) = unhex(signature_hex) else {
        return false;
    };
    if bytes.len() != 65 {
        return false;
    }
    // Wallets emit v as 27/28, occasionally 0/1. Anything else is malformed.
    let v = match bytes[64] {
        0 | 27 => 0u8,
        1 | 28 => 1u8,
        _ => return false,
    };
    let (Ok(sig), Some(recid)) = (
        EcdsaSignature::from_slice(&bytes[..64]),
        RecoveryId::from_byte(v),
    ) else {
        return false;
    };
    let Ok(key) = VerifyingKey::recover_from_prehash(&eip191(message), &sig, recid) else {
        return false;
    };
    let encoded = key.to_sec1_point(false);
    let hash = Keccak256::digest(&encoded.as_bytes()[1..]);
    let recovered = format!("0x{}", hex_lower(&hash[12..]));
    normalize(Chain::Ethereum, address).is_some_and(|a| a == recovered)
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn verify(chain: Chain, address: &str, message: &str, signature: &str) -> bool {
    match chain {
        Chain::Solana => verify_solana(address, message, signature),
        Chain::Ethereum => verify_ethereum(address, message, signature),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use k256::ecdsa::{SigningKey as EcdsaSigningKey, signature::hazmat::PrehashSigner};

    fn solana_signer() -> (String, SigningKey) {
        let key = SigningKey::from_bytes(&[42u8; 32]);
        (
            bs58::encode(key.verifying_key().as_bytes()).into_string(),
            key,
        )
    }
    fn ethereum_signer() -> (String, EcdsaSigningKey) {
        let key = EcdsaSigningKey::from_slice(&[9u8; 32]).unwrap();
        let encoded = key.verifying_key().to_sec1_point(false);
        let hash = Keccak256::digest(&encoded.as_bytes()[1..]);
        (format!("0x{}", hex_lower(&hash[12..])), key)
    }
    fn sign_eth(key: &EcdsaSigningKey, message: &str) -> String {
        let (sig, recid): (EcdsaSignature, RecoveryId) =
            key.sign_prehash(&eip191(message)).unwrap();
        let mut out = sig.to_bytes().to_vec();
        out.push(recid.to_byte() + 27);
        format!("0x{}", hex_lower(&out))
    }

    #[test]
    fn the_signed_message_states_that_nothing_is_authorized() {
        let m = message("app.example", Chain::Solana, "addr", "nonce123", 1700000000);
        assert!(m.contains("app.example wants you to sign in"));
        assert!(m.contains("does not approve a transaction"));
        assert!(m.contains("Nonce: nonce123"));
        assert!(m.contains("solana"));
    }
    #[test]
    fn solana_signatures_round_trip_and_are_bound_to_the_message() {
        let (address, key) = solana_signer();
        let m = message("app", Chain::Solana, &address, "n1", 1);
        let sig = bs58::encode(key.sign(m.as_bytes()).to_bytes()).into_string();
        assert!(verify_solana(&address, &m, &sig));
        // A different message, nonce or address must all fail.
        assert!(!verify_solana(&address, "other message", &sig));
        assert!(!verify_solana(
            &address,
            &message("app", Chain::Solana, &address, "n2", 1),
            &sig
        ));
        let (other, _) = (
            bs58::encode(
                SigningKey::from_bytes(&[43u8; 32])
                    .verifying_key()
                    .as_bytes(),
            )
            .into_string(),
            (),
        );
        assert!(!verify_solana(&other, &m, &sig));
    }
    #[test]
    fn ethereum_signatures_round_trip_and_recover_the_signer() {
        let (address, key) = ethereum_signer();
        let m = message("app", Chain::Ethereum, &address, "n1", 1);
        let sig = sign_eth(&key, &m);
        assert!(verify_ethereum(&address, &m, &sig));
        // Checksum casing must not change the outcome.
        assert!(verify_ethereum(
            &address.to_uppercase().replace("0X", "0x"),
            &m,
            &sig
        ));
        assert!(!verify_ethereum(&address, "tampered", &sig));
        assert!(!verify_ethereum(
            "0x0000000000000000000000000000000000000001",
            &m,
            &sig
        ));
    }
    #[test]
    fn malformed_signatures_are_rejected_rather_than_panicking() {
        let (address, _) = ethereum_signer();
        for bad in ["", "0x", "nothex", "0x1234"] {
            assert!(!verify_ethereum(&address, "m", bad));
        }
        // Right length, impossible recovery byte.
        let mut bytes = vec![1u8; 65];
        bytes[64] = 99;
        assert!(!verify_ethereum(
            &address,
            "m",
            &format!("0x{}", hex_lower(&bytes))
        ));
        let (solana, _) = solana_signer();
        for bad in ["", "!!!", "abc"] {
            assert!(!verify_solana(&solana, "m", bad));
        }
        assert!(!verify_solana("not-an-address", "m", "abc"));
    }
    #[test]
    fn address_normalization_rejects_the_wrong_shape() {
        assert_eq!(
            normalize(
                Chain::Ethereum,
                "0xAbC0000000000000000000000000000000000001"
            ),
            Some("0xabc0000000000000000000000000000000000001".into())
        );
        assert!(normalize(Chain::Ethereum, "0xtoo-short").is_none());
        assert!(normalize(Chain::Ethereum, "abc0000000000000000000000000000000000001").is_none());
        let (solana, _) = solana_signer();
        assert_eq!(normalize(Chain::Solana, &solana), Some(solana.clone()));
        assert!(normalize(Chain::Solana, "nope").is_none());
        // A Solana address must not be accepted as an Ethereum one, or vice versa.
        assert!(normalize(Chain::Ethereum, &solana).is_none());
        assert!(Chain::parse("bitcoin").is_none());
    }
}
