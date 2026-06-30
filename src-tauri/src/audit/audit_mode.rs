//! Audit mode selection: Dev vs Production.
//!
//! The user picks a mode every time they open the Audit tab.
//! - **Dev mode** runs the full audit stack locally via Docker Compose
//!   (publisher + attester + reader daemons, K-of-N attestation, oplog
//!   verification). The 3-node MongoDB replica set is a separate Compose
//!   file (the hackathon one).
//! - **Production mode** runs the in-app audit pipeline with the user's own
//!   Stellar keypair + contract, on a network of their choice (testnet or
//!   mainnet). This is the "double check" that a deployment elsewhere works.
//!
//! Mode + network + mainnet contract/rpc are persisted in the settings store.
//! The mainnet secret key is stored in the OS keychain (never the store).

use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tauri_plugin_store::StoreExt;

use crate::audit::dev_setup::load_keypair_from_keychain;
use crate::audit::stellar_native::StellarKeypair;
use crate::error::{AppError, AppResult};

const STORE_FILE: &str = "nosqlbuddy.settings.json";
const AUDIT_MODE_KEY: &str = "auditMode";
const AUDIT_NETWORK_KEY: &str = "auditNetwork";
const AUDIT_TESTNET_CONTRACT_KEY: &str = "auditTestnetContractId";
const AUDIT_MAINNET_CONTRACT_KEY: &str = "auditMainnetContractId";
const AUDIT_MAINNET_RPC_KEY: &str = "auditMainnetRpcUrl";

/// Keychain entry name for the production mainnet keypair.
const PROD_KEYCHAIN_ENTRY: &str = "stellar-audit-keypair-prod";

/// Which audit experience the user selected.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AuditMode {
    /// Full stack locally via Docker Compose.
    Dev,
    /// In-app pipeline with the user's own keys on testnet or mainnet.
    Production,
}

impl Default for AuditMode {
    fn default() -> Self {
        Self::Dev
    }
}

/// Which Stellar network to anchor commitments to.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AuditNetwork {
    Testnet,
    Mainnet,
}

impl Default for AuditNetwork {
    fn default() -> Self {
        Self::Testnet
    }
}

/// The full audit mode configuration, returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditModeConfig {
    pub mode: AuditMode,
    pub network: AuditNetwork,
    /// Testnet Soroban contract ID. Empty means use the bundled demo contract.
    pub testnet_contract_id: String,
    /// Mainnet Soroban contract ID (only used when network == Mainnet).
    pub mainnet_contract_id: String,
    /// Mainnet Soroban RPC URL (only used when network == Mainnet).
    pub mainnet_rpc_url: String,
    /// Whether a production keypair is saved in the keychain.
    pub has_production_keypair: bool,
}

impl Default for AuditModeConfig {
    fn default() -> Self {
        Self {
            mode: AuditMode::Dev,
            network: AuditNetwork::Testnet,
            testnet_contract_id: String::new(),
            mainnet_contract_id: String::new(),
            mainnet_rpc_url: "https://rpc.mainnet.stellar.org".to_string(),
            has_production_keypair: false,
        }
    }
}

/// Read the persisted audit mode config from the settings store + keychain.
pub fn load_mode_config(app: &AppHandle) -> AppResult<AuditModeConfig> {
    let store = app
        .store(STORE_FILE)
        .map_err(|e| AppError::Internal(format!("settings store open: {e}")))?;

    let mode = match store.get(AUDIT_MODE_KEY) {
        Some(v) => serde_json::from_value(v).unwrap_or(AuditMode::Dev),
        None => AuditMode::Dev,
    };
    let network = match store.get(AUDIT_NETWORK_KEY) {
        Some(v) => serde_json::from_value(v).unwrap_or(AuditNetwork::Testnet),
        None => AuditNetwork::Testnet,
    };
    let testnet_contract_id = store
        .get(AUDIT_TESTNET_CONTRACT_KEY)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_default();
    let mainnet_contract_id = store
        .get(AUDIT_MAINNET_CONTRACT_KEY)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_default();
    let mainnet_rpc_url = store
        .get(AUDIT_MAINNET_RPC_KEY)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "https://rpc.mainnet.stellar.org".to_string());

    let has_production_keypair = load_production_keypair()?.is_some();

    Ok(AuditModeConfig {
        mode,
        network,
        testnet_contract_id,
        mainnet_contract_id,
        mainnet_rpc_url,
        has_production_keypair,
    })
}

/// Persist the audit mode (dev/production) to the settings store.
pub fn save_mode(app: &AppHandle, mode: AuditMode) -> AppResult<()> {
    let store = app
        .store(STORE_FILE)
        .map_err(|e| AppError::Internal(format!("settings store open: {e}")))?;
    store.set(AUDIT_MODE_KEY, serde_json::json!(mode));
    store
        .save()
        .map_err(|e| AppError::Internal(format!("settings store save: {e}")))?;
    Ok(())
}

/// Persist the production network choice + contract/rpc.
pub fn save_production_network(
    app: &AppHandle,
    network: AuditNetwork,
    contract_id: String,
    rpc_url: String,
) -> AppResult<()> {
    let store = app
        .store(STORE_FILE)
        .map_err(|e| AppError::Internal(format!("settings store open: {e}")))?;
    store.set(AUDIT_NETWORK_KEY, serde_json::json!(network));
    if !contract_id.is_empty() {
        match network {
            AuditNetwork::Testnet => {
                store.set(AUDIT_TESTNET_CONTRACT_KEY, serde_json::json!(contract_id));
            }
            AuditNetwork::Mainnet => {
                store.set(AUDIT_MAINNET_CONTRACT_KEY, serde_json::json!(contract_id));
            }
        }
    }
    if !rpc_url.is_empty() {
        store.set(AUDIT_MAINNET_RPC_KEY, serde_json::json!(rpc_url));
    }
    store
        .save()
        .map_err(|e| AppError::Internal(format!("settings store save: {e}")))?;
    Ok(())
}

// ─── Production keypair (keychain) ─────────────────────────────────────

/// Load the production mainnet keypair from the keychain, if saved.
pub fn load_production_keypair() -> AppResult<Option<StellarKeypair>> {
    use keyring::Entry;

    let entry = Entry::new("nosqlbuddy", PROD_KEYCHAIN_ENTRY)
        .map_err(|e| AppError::Credential(format!("keyring entry: {e}")))?;
    match entry.get_password() {
        Ok(secret_str) => {
            let secret_bytes = decode_secret_key(&secret_str)?;
            Ok(Some(StellarKeypair::from_secret_bytes(&secret_bytes)))
        }
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(AppError::Credential(format!("keyring get: {e}"))),
    }
}

/// Save a production keypair (from an imported S... secret key) to the keychain.
pub fn save_production_keypair(secret_key_str: &str) -> AppResult<String> {
    use keyring::Entry;

    let secret_bytes = decode_secret_key(secret_key_str)?;
    let kp = StellarKeypair::from_secret_bytes(&secret_bytes);
    let account_id = kp.account_id();

    let entry = Entry::new("nosqlbuddy", PROD_KEYCHAIN_ENTRY)
        .map_err(|e| AppError::Credential(format!("keyring entry: {e}")))?;
    entry
        .set_password(secret_key_str)
        .map_err(|e| AppError::Credential(format!("keyring set: {e}")))?;

    Ok(account_id)
}

/// Clear the production keypair from the keychain.
pub fn clear_production_keypair() -> AppResult<()> {
    use keyring::Entry;

    let entry = Entry::new("nosqlbuddy", PROD_KEYCHAIN_ENTRY)
        .map_err(|e| AppError::Credential(format!("keyring entry: {e}")))?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(AppError::Credential(format!("keyring delete: {e}"))),
    }
}

/// Decode a Stellar secret key strkey (S...) to 32 raw bytes.
///
/// Mirrors the logic in `dev_setup::decode_secret_key` but kept local so
/// this module is self-contained.
fn decode_secret_key(s: &str) -> AppResult<[u8; 32]> {
    use crate::audit::stellar_native;

    if !s.starts_with('S') || s.len() != 56 {
        return Err(AppError::Validation(
            "invalid secret key format: expected 56-char S... strkey".to_string(),
        ));
    }

    let decoded = stellar_native::base32_decode(s)
        .ok_or_else(|| AppError::Validation("invalid base32 in secret key".to_string()))?;

    if decoded.len() != 35 {
        return Err(AppError::Validation(format!(
            "invalid secret key length: expected 35 bytes, got {}",
            decoded.len()
        )));
    }

    if decoded[0] != 18 << 3 {
        return Err(AppError::Validation(format!(
            "invalid secret key version byte: expected 0x{:02x}, got 0x{:02x}",
            18 << 3,
            decoded[0]
        )));
    }

    let payload = &decoded[..33];
    let checksum = &decoded[33..];
    let expected = stellar_native::crc16_xmodem(payload);
    let expected_le = [(expected & 0xff) as u8, (expected >> 8) as u8];
    if checksum != expected_le {
        return Err(AppError::Validation(
            "secret key checksum mismatch".to_string(),
        ));
    }

    let mut result = [0u8; 32];
    result.copy_from_slice(&decoded[1..33]);
    Ok(result)
}

// ─── Tauri commands ────────────────────────────────────────────────────

/// Get the current audit mode configuration.
#[tauri::command]
pub async fn audit_get_mode_config(app: AppHandle) -> AppResult<AuditModeConfig> {
    load_mode_config(&app)
}

/// Set the audit mode (dev or production).
#[tauri::command]
pub async fn audit_set_audit_mode(mode: AuditMode, app: AppHandle) -> AppResult<()> {
    save_mode(&app, mode)
}

/// Set the production network (testnet or mainnet) + mainnet contract/rpc.
#[tauri::command]
pub async fn audit_set_production_network(
    network: AuditNetwork,
    contract_id: String,
    rpc_url: String,
    app: AppHandle,
) -> AppResult<()> {
    save_production_network(&app, network, contract_id, rpc_url)
}

/// Import a production mainnet keypair from an S... secret key string.
/// Returns the derived account ID (G...).
#[tauri::command]
pub async fn audit_import_production_keypair(secret_key: String) -> AppResult<String> {
    save_production_keypair(&secret_key)
}

/// Clear the saved production keypair.
#[tauri::command]
pub async fn audit_clear_production_keypair() -> AppResult<()> {
    clear_production_keypair()
}

/// Re-export the dev keypair helpers so the frontend can use one command
/// surface for both modes. This checks which keypair is active based on mode.
#[tauri::command]
pub async fn audit_get_active_account(app: AppHandle) -> AppResult<Option<String>> {
    let config = load_mode_config(&app)?;
    match config.mode {
        AuditMode::Production => {
            // Production uses the imported keypair (testnet or mainnet).
            Ok(load_production_keypair()?.map(|kp| kp.account_id()))
        }
        AuditMode::Dev => {
            // Dev uses the auto-funded testnet keypair from onboarding.
            Ok(load_keypair_from_keychain()?.map(|kp| kp.account_id()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audit_mode_default_is_dev() {
        assert_eq!(AuditMode::default(), AuditMode::Dev);
    }

    #[test]
    fn test_audit_network_default_is_testnet() {
        assert_eq!(AuditNetwork::default(), AuditNetwork::Testnet);
    }

    #[test]
    fn test_mode_config_default() {
        let cfg = AuditModeConfig::default();
        assert_eq!(cfg.mode, AuditMode::Dev);
        assert_eq!(cfg.network, AuditNetwork::Testnet);
        assert!(!cfg.has_production_keypair);
    }

    #[test]
    fn test_decode_secret_key_rejects_garbage() {
        assert!(decode_secret_key("not_a_key").is_err());
        assert!(decode_secret_key("S").is_err());
    }
}
