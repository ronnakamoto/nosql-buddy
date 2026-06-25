//! Dev mode onboarding: auto-provisioning for the ZK audit trial.
//!
//! This module implements the "Start Audit Trial" flow:
//! 1. Check MongoDB connection and replica set status
//! 2. Verify Pinata IPFS storage is configured (or prompt for API key)
//! 3. Generate and fund a Stellar testnet account
//! 4. Start the change stream listener
//!
//! The result is a fully provisioned audit environment that the user
//! activated with a single button (plus entering a Pinata API key).

use serde::{Deserialize, Serialize};

use crate::audit::pinata::PinataConfig;
use crate::audit::stellar_native::{
    self, StellarKeypair, TESTNET_HORIZON_URL, TESTNET_PASSPHRASE, TESTNET_RPC_URL,
};
use crate::audit::stellar::CONTRACT_ID;
use crate::error::{AuditError, AuditResult};

/// The network passphrase and RPC URLs for the active chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainConfig {
    pub network: String,
    pub rpc_url: String,
    pub horizon_url: String,
    pub passphrase: String,
    pub contract_id: String,
}

impl Default for ChainConfig {
    fn default() -> Self {
        Self::testnet()
    }
}

impl ChainConfig {
    pub fn testnet() -> Self {
        Self {
            network: "testnet".to_string(),
            rpc_url: TESTNET_RPC_URL.to_string(),
            horizon_url: TESTNET_HORIZON_URL.to_string(),
            passphrase: TESTNET_PASSPHRASE.to_string(),
            contract_id: CONTRACT_ID.to_string(),
        }
    }

    pub fn mainnet(rpc_url: String, contract_id: String) -> Self {
        Self {
            network: "mainnet".to_string(),
            rpc_url,
            horizon_url: "https://horizon.stellar.org".to_string(),
            passphrase: stellar_native::MAINNET_PASSPHRASE.to_string(),
            contract_id,
        }
    }
}

/// The result of a successful onboarding setup.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditSetupResult {
    pub account_id: String,
    pub chain: ChainConfig,
    pub pinata_configured: bool,
    pub change_stream_started: bool,
}

/// Check if a MongoDB client is connected to a replica set.
///
/// Change streams require a replica set. This queries the `hello` command
/// and checks for the `setName` field.
pub async fn check_mongodb_rs(client: &mongodb::Client) -> AuditResult<bool> {
    let db = client.database("admin");
    let result = db.run_command(bson::doc! { "hello": 1 }).await;
    match result {
        Ok(doc) => {
            let set_name = doc.get_str("setName").ok();
            Ok(set_name.is_some())
        }
        Err(_) => Ok(false),
    }
}

/// Generate a new Stellar keypair and fund it via friendbot (testnet only).
pub async fn generate_and_fund_account() -> AuditResult<StellarKeypair> {
    let kp = stellar_native::generate_keypair();
    let account_id = kp.account_id();
    stellar_native::fund_account(&account_id).await?;
    Ok(kp)
}

/// Save a Stellar keypair to the OS keychain.
pub fn save_keypair_to_keychain(keypair: &StellarKeypair) -> AuditResult<()> {
    use keyring::Entry;

    let entry = Entry::new("nosqlbuddy", "stellar-audit-keypair")
        .map_err(|e| AuditError::Credential(format!("keyring entry: {e}")))?;
    let secret = keypair.secret_key_str();
    entry
        .set_password(&secret)
        .map_err(|e| AuditError::Credential(format!("keyring set: {e}")))
}

/// Load a Stellar keypair from the OS keychain, if one was saved.
pub fn load_keypair_from_keychain() -> AuditResult<Option<StellarKeypair>> {
    use keyring::Entry;

    let entry = Entry::new("nosqlbuddy", "stellar-audit-keypair")
        .map_err(|e| AuditError::Credential(format!("keyring entry: {e}")))?;
    match entry.get_password() {
        Ok(secret_str) => {
            // Decode the S... strkey back to bytes.
            let secret_bytes = decode_secret_key(&secret_str)?;
            Ok(Some(StellarKeypair::from_secret_bytes(&secret_bytes)))
        }
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(AuditError::Credential(format!("keyring get: {e}"))),
    }
}

/// Save Pinata config to the OS keychain.
pub fn save_pinata_to_keychain(config: &PinataConfig) -> AuditResult<()> {
    use keyring::Entry;

    let entry = Entry::new("nosqlbuddy", "pinata-api-key")
        .map_err(|e| AuditError::Credential(format!("keyring entry: {e}")))?;
    let json = serde_json::to_string(config)
        .map_err(|e| AuditError::Internal(format!("serialize pinata config: {e}")))?;
    entry
        .set_password(&json)
        .map_err(|e| AuditError::Credential(format!("keyring set: {e}")))
}

/// Load Pinata config from the OS keychain, if saved.
pub fn load_pinata_from_keychain() -> AuditResult<Option<PinataConfig>> {
    use keyring::Entry;

    let entry = Entry::new("nosqlbuddy", "pinata-api-key")
        .map_err(|e| AuditError::Credential(format!("keyring entry: {e}")))?;
    match entry.get_password() {
        Ok(json) => {
            let config: PinataConfig = serde_json::from_str(&json)
                .map_err(|e| AuditError::Internal(format!("deserialize pinata config: {e}")))?;
            Ok(Some(config))
        }
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(AuditError::Credential(format!("keyring get: {e}"))),
    }
}

/// Check if onboarding has been completed (keypair + Pinata in keychain).
pub fn check_onboarding_status() -> AuditResult<OnboardingStatus> {
    let has_keypair = load_keypair_from_keychain()?.is_some();
    let has_pinata = load_pinata_from_keychain()?.is_some();
    Ok(OnboardingStatus {
        has_keypair,
        has_pinata,
        is_complete: has_keypair && has_pinata,
    })
}

/// The status of onboarding — which components are already provisioned.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnboardingStatus {
    pub has_keypair: bool,
    pub has_pinata: bool,
    pub is_complete: bool,
}

/// Decode a Stellar secret key strkey (S...) to 32 raw bytes.
fn decode_secret_key(s: &str) -> AuditResult<[u8; 32]> {
    if !s.starts_with('S') || s.len() != 56 {
        return Err(AuditError::Validation(
            "invalid secret key format: expected 56-char S... strkey".to_string(),
        ));
    }

    let decoded = stellar_native::base32_decode(s)
        .ok_or_else(|| AuditError::Validation("invalid base32 in secret key".to_string()))?;

    if decoded.len() != 35 {
        return Err(AuditError::Validation(format!(
            "invalid secret key length: expected 35 bytes, got {}",
            decoded.len()
        )));
    }

    // Verify version byte: 18 << 3 = 0x90
    if decoded[0] != 18 << 3 {
        return Err(AuditError::Validation(format!(
            "invalid secret key version byte: expected 0x{:02x}, got 0x{:02x}",
            18 << 3,
            decoded[0]
        )));
    }

    // Verify checksum
    let payload = &decoded[..33];
    let checksum = &decoded[33..];
    let expected = stellar_native::crc16_xmodem(payload);
    let expected_le = [(expected & 0xff) as u8, (expected >> 8) as u8];
    if checksum != expected_le {
        return Err(AuditError::Validation("secret key checksum mismatch".to_string()));
    }

    let mut result = [0u8; 32];
    result.copy_from_slice(&decoded[1..33]);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chain_config_testnet_defaults() {
        let config = ChainConfig::testnet();
        assert_eq!(config.network, "testnet");
        assert_eq!(config.passphrase, TESTNET_PASSPHRASE);
        assert_eq!(config.contract_id, CONTRACT_ID);
    }

    #[test]
    fn test_chain_config_mainnet() {
        let config = ChainConfig::mainnet(
            "https://rpc.mainnet.stellar.org".to_string(),
            "ABC123".to_string(),
        );
        assert_eq!(config.network, "mainnet");
        assert_eq!(config.contract_id, "ABC123");
    }

    #[test]
    fn test_onboarding_status_serializes_camel_case() {
        let status = OnboardingStatus {
            has_keypair: true,
            has_pinata: false,
            is_complete: false,
        };
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("\"hasKeypair\""));
        assert!(json.contains("\"hasPinata\""));
        assert!(json.contains("\"isComplete\""));
    }

    #[test]
    fn test_decode_secret_key_roundtrip() {
        // Generate a keypair, encode the secret, decode it back.
        let kp = stellar_native::generate_keypair();
        let secret_str = kp.secret_key_str();
        let decoded = decode_secret_key(&secret_str);
        assert!(decoded.is_ok());
        let decoded = decoded.unwrap();
        assert_eq!(decoded, kp.secret_bytes());
    }

    #[test]
    fn test_decode_secret_key_invalid_format() {
        assert!(decode_secret_key("not-a-key").is_err());
        assert!(decode_secret_key("GXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX").is_err());
    }
}
