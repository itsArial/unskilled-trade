//! Managed Solana wallets held on behalf of account holders.
//!
//! **Holding a user's private key makes this service a custodian.** That is a
//! legal status, not an implementation detail: in the EU it is a CASP activity
//! under MiCA, and in the US it is money transmission in most states. Nothing in
//! this file discharges that obligation. It is written so the obligation is at
//! least met technically — keys are never stored in the clear, never logged,
//! never returned by any listing route, and released exactly once.
//!
//! Custody is disabled unless `CUSTODY_MASTER_KEY` is set. A missing key is not a
//! reason to fall back to plaintext storage; it is a reason to refuse to create
//! managed wallets at all.
//!
//! # Export versus release
//!
//! The wallets the incumbents generate are exportable **at any time** — Axiom
//! hands over a seed phrase at signup, Trojan exports on request — so a user is
//! never locked out of their own funds by the platform's convenience. This
//! implementation does the same through `export`, and keeps `release` for
//! closing the account.
//!
//! The two are not the same act and must never be described as if they were:
//!
//! - `export` copies the key to its owner. The service **still holds it** and
//!   can still sign. Control becomes shared, not transferred.
//! - `release` hands the key over and destroys our ciphertext. Control is
//!   transferred, and afterwards this service cannot sign at all.
//!
//! Anyone who wants sole control has to move the funds to a wallet this service
//! never generated, and the API says so rather than implying an export is an
//! exit.
//!
//! The master key lives in the process environment, which means an operator with
//! host access can decrypt every user's funds. A production deployment needs an
//! external KMS or HSM with per-operation authorization and an audit trail. This
//! is a real gap, recorded rather than hidden.
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce, aead::Aead};
use ed25519_dalek::SigningKey;
use rand::RngCore;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

/// Length of the exported secret: 32 seed bytes followed by 32 public-key bytes,
/// which is the layout Phantom and the Solana CLI both accept.
const KEYPAIR_BYTES: usize = 64;

pub struct Custody {
    cipher: XChaCha20Poly1305,
}
impl Custody {
    /// `None` when no master key is configured. Callers must treat that as
    /// "custody unavailable", never as "store it unencrypted".
    pub fn from_env() -> Option<Self> {
        // An unset and an empty key both mean "custody is not configured", and
        // neither deserves a warning.
        let raw = std::env::var("CUSTODY_MASTER_KEY")
            .ok()
            .filter(|v| !v.trim().is_empty())?;
        let key = bs58::decode(raw.trim()).into_vec().ok()?;
        if key.len() != 32 {
            eprintln!("custody: CUSTODY_MASTER_KEY must decode to 32 bytes; custody disabled");
            return None;
        }
        XChaCha20Poly1305::new_from_slice(&key)
            .ok()
            .map(|cipher| Self { cipher })
    }
    pub fn seal(&self, secret: &[u8]) -> Option<String> {
        let mut nonce = [0u8; 24];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let mut blob = nonce.to_vec();
        blob.extend(self.cipher.encrypt(&XNonce::from(nonce), secret).ok()?);
        Some(bs58::encode(blob).into_string())
    }
    pub fn open(&self, blob: &str) -> Option<Vec<u8>> {
        let bytes = bs58::decode(blob).into_vec().ok()?;
        if bytes.len() <= 24 {
            return None;
        }
        let (nonce, body) = bytes.split_at(24);
        let nonce: [u8; 24] = nonce.try_into().ok()?;
        self.cipher.decrypt(&XNonce::from(nonce), body).ok()
    }
}

/// A fresh Solana keypair. Returns the base58 address and the 64-byte secret.
pub fn generate() -> (String, [u8; KEYPAIR_BYTES]) {
    let mut seed = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut seed);
    let signing = SigningKey::from_bytes(&seed);
    let verifying = signing.verifying_key();
    let mut secret = [0u8; KEYPAIR_BYTES];
    secret[..32].copy_from_slice(&signing.to_bytes());
    secret[32..].copy_from_slice(verifying.as_bytes());
    (bs58::encode(verifying.as_bytes()).into_string(), secret)
}

/// Creates the managed wallet for a new account. Idempotent: an account that
/// already has one keeps it, because reissuing would strand any deposited funds.
pub fn provision(
    db: &Connection,
    custody: &Custody,
    user_id: &str,
    at: i64,
) -> Result<String, rusqlite::Error> {
    if let Some(address) = db
        .query_row(
            "SELECT address FROM managed_wallets WHERE user_id=?1",
            [user_id],
            |r| r.get::<_, String>(0),
        )
        .optional()?
    {
        return Ok(address);
    }
    let (address, secret) = generate();
    let Some(sealed) = custody.seal(&secret) else {
        return Err(rusqlite::Error::InvalidQuery);
    };
    db.execute(
        "INSERT INTO managed_wallets(user_id,address,secret,created) VALUES(?1,?2,?3,?4)",
        params![user_id, address, sealed, at],
    )?;
    Ok(address)
}

pub fn address(db: &Connection, user_id: &str) -> Result<Option<String>, rusqlite::Error> {
    db.query_row(
        "SELECT address FROM managed_wallets WHERE user_id=?1 AND released=0",
        [user_id],
        |r| r.get(0),
    )
    .optional()
}

/// Hands the secret to its owner and permanently marks the wallet released.
///
/// One-shot by construction: the update is conditional on `released=0` and the
/// stored ciphertext is destroyed in the same statement, so a replayed request
/// cannot return the key twice and the service cannot sign for it afterwards.
pub fn release(
    db: &Connection,
    custody: &Custody,
    user_id: &str,
    at: i64,
) -> Result<Option<(String, String)>, rusqlite::Error> {
    let Some((address, sealed)) = db
        .query_row(
            "SELECT address,secret FROM managed_wallets WHERE user_id=?1 AND released=0",
            [user_id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()?
    else {
        return Ok(None);
    };
    let Some(secret) = custody.open(&sealed) else {
        return Ok(None);
    };
    if db.execute(
        "UPDATE managed_wallets SET released=?1,secret='' WHERE user_id=?2 AND released=0",
        params![at, user_id],
    )? == 0
    {
        return Ok(None);
    }
    Ok(Some((address, bs58::encode(secret).into_string())))
}

/// Hands the owner a copy of their key without ending custody.
///
/// Deliberately does not touch `released`: this is a copy, not a handover, and
/// pretending otherwise would let someone believe the service can no longer
/// sign for a wallet it can still sign for.
pub fn export(
    db: &Connection,
    custody: &Custody,
    user_id: &str,
) -> Result<Option<(String, String)>, rusqlite::Error> {
    let Some((address, sealed)) = db
        .query_row(
            "SELECT address,secret FROM managed_wallets WHERE user_id=?1 AND released=0",
            [user_id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()?
    else {
        return Ok(None);
    };
    let Some(secret) = custody.open(&sealed) else {
        return Ok(None);
    };
    db.execute(
        "UPDATE managed_wallets SET exports=exports+1,last_export=?1 WHERE user_id=?2",
        params![crate::now(), user_id],
    )?;
    Ok(Some((address, bs58::encode(secret).into_string())))
}

/// On-chain lamport balance. A provider failure is an error, never a zero
/// balance: showing 0 for "we could not ask" would be a lie about someone's money.
pub async fn balance(url: &str, address: &str) -> Result<u64, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|_| "Cannot create RPC client")?;
    let response = client
        .post(url)
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"getBalance","params":[address,{"commitment":"confirmed"}]}))
        .send()
        .await
        .map_err(|_| "Balance lookup failed".to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "Balance lookup returned HTTP {}",
            response.status()
        ));
    }
    let v: Value = response
        .json()
        .await
        .map_err(|_| "Unreadable balance response")?;
    v["result"]["value"]
        .as_u64()
        .ok_or("Balance was not reported".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn custody() -> Custody {
        Custody {
            cipher: XChaCha20Poly1305::new_from_slice(&[7u8; 32]).unwrap(),
        }
    }
    fn db() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(crate::CUSTODY_SCHEMA).unwrap();
        db
    }
    #[test]
    fn generated_wallets_are_valid_distinct_addresses() {
        let (a, sa) = generate();
        let (b, _) = generate();
        assert!(crate::rpc::valid_address(&a));
        assert_ne!(a, b);
        assert_eq!(sa.len(), 64);
        // The exported form carries the public key in its second half.
        assert_eq!(bs58::encode(&sa[32..]).into_string(), a);
    }
    #[test]
    fn sealed_secrets_are_not_recoverable_without_the_master_key() {
        let c = custody();
        let (_, secret) = generate();
        let blob = c.seal(&secret).unwrap();
        assert!(!blob.contains(&bs58::encode(secret).into_string()));
        assert_eq!(c.open(&blob).unwrap(), secret.to_vec());
        // A different master key must not open it, and neither must noise.
        let other = Custody {
            cipher: XChaCha20Poly1305::new_from_slice(&[8u8; 32]).unwrap(),
        };
        assert!(other.open(&blob).is_none());
        assert!(c.open("not-base58-!!").is_none());
        assert!(c.open(&bs58::encode([1u8; 8]).into_string()).is_none());
    }
    #[test]
    fn sealing_is_randomised_so_two_accounts_never_share_ciphertext() {
        let c = custody();
        let (_, secret) = generate();
        assert_ne!(c.seal(&secret).unwrap(), c.seal(&secret).unwrap());
    }
    #[test]
    fn provisioning_is_idempotent() {
        let db = db();
        let c = custody();
        let first = provision(&db, &c, "user", 100).unwrap();
        assert_eq!(provision(&db, &c, "user", 200).unwrap(), first);
        assert_eq!(address(&db, "user").unwrap(), Some(first));
    }
    #[test]
    fn export_gives_a_copy_and_does_not_end_custody() {
        let db = db();
        let c = custody();
        let created = provision(&db, &c, "user", 100).unwrap();
        let (address, secret) = export(&db, &c, "user").unwrap().unwrap();
        assert_eq!(address, created);
        assert_eq!(bs58::decode(&secret).into_vec().unwrap().len(), 64);
        // Unlike release: the wallet still resolves and we can still sign.
        assert_eq!(super::address(&db, "user").unwrap(), Some(created.clone()));
        let stored: String = db
            .query_row("SELECT secret FROM managed_wallets", [], |r| r.get(0))
            .unwrap();
        assert!(!stored.is_empty(), "our copy survives an export");
        // Exporting repeatedly is allowed and counted.
        let (_, again) = export(&db, &c, "user").unwrap().unwrap();
        assert_eq!(again, secret, "the same wallet yields the same key");
        let exports: i64 = db
            .query_row("SELECT exports FROM managed_wallets", [], |r| r.get(0))
            .unwrap();
        assert_eq!(exports, 2);
    }
    #[test]
    fn a_released_wallet_can_no_longer_be_exported() {
        let db = db();
        let c = custody();
        provision(&db, &c, "user", 100).unwrap();
        release(&db, &c, "user", 200).unwrap().unwrap();
        assert!(export(&db, &c, "user").unwrap().is_none());
    }
    #[test]
    fn release_hands_over_the_key_exactly_once_and_destroys_it() {
        let db = db();
        let c = custody();
        let created = provision(&db, &c, "user", 100).unwrap();
        let (address_out, secret) = release(&db, &c, "user", 200).unwrap().unwrap();
        assert_eq!(address_out, created);
        assert_eq!(bs58::decode(&secret).into_vec().unwrap().len(), 64);
        // Replay returns nothing, the wallet no longer resolves, and the stored
        // ciphertext is gone so the service can never sign for it again.
        assert!(release(&db, &c, "user", 300).unwrap().is_none());
        assert_eq!(address(&db, "user").unwrap(), None);
        let stored: String = db
            .query_row("SELECT secret FROM managed_wallets", [], |r| r.get(0))
            .unwrap();
        assert!(stored.is_empty());
    }
    #[test]
    fn releasing_an_unknown_account_is_not_an_error() {
        assert!(release(&db(), &custody(), "nobody", 1).unwrap().is_none());
    }
}
