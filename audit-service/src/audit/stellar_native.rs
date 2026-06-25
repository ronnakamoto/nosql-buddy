//! Native Stellar Soroban transaction signing and submission.
//!
//! Replaces the `stellar` CLI subprocess with native Rust XDR building,
//! ed25519 signing, and Soroban RPC submission. This module implements
//! the full transaction flow: build → simulate → sign → submit.
//!
//! ## Architecture
//!
//! The module uses `stellar-xdr` for XDR type definitions and encoding,
//! `ed25519-dalek` for key generation and signing, and `reqwest` for
//! HTTP calls to the Soroban RPC and Horizon APIs.
//!
//! ## Transaction flow
//!
//! 1. Get account sequence number from Horizon API
//! 2. Build a `Transaction` with an `InvokeHostFunction` operation
//! 3. Simulate the transaction via Soroban RPC `simulateTransaction`
//! 4. Attach the simulation's `SorobanTransactionData` to the transaction
//! 5. Sign the transaction with ed25519 (over the `TransactionSignaturePayload` hash)
//! 6. Submit the signed transaction via Soroban RPC `sendTransaction`

use base64::Engine;
use ed25519_dalek::{Signer, SigningKey};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use stellar_xdr::curr::{
    AccountId, ContractId, DecoratedSignature, Hash, HostFunction, InvokeContractArgs,
    InvokeHostFunctionOp, Limits, Memo, MuxedAccount, Operation, OperationBody,
    Preconditions, PublicKey, ReadXdr, ScAddress, ScBytes, ScString, ScSymbol, ScVal,
    SequenceNumber, Signature, SignatureHint, SorobanAuthorizationEntry,
    SorobanTransactionData, Transaction, TransactionEnvelope, TransactionExt,
    TransactionSignaturePayload, TransactionSignaturePayloadTaggedTransaction,
    TransactionV1Envelope, Uint256, VecM, WriteXdr,
};

use crate::audit::stellar::{CommitResult, OnChainOplogCommitment};
use crate::error::{AuditError, AuditResult};

/// The Stellar testnet network passphrase.
pub const TESTNET_PASSPHRASE: &str = "Test SDF Network ; September 2015";

/// The Stellar mainnet network passphrase.
pub const MAINNET_PASSPHRASE: &str = "Public Global Stellar Network ; September 2015";

/// The Soroban RPC endpoint for Stellar testnet.
pub const TESTNET_RPC_URL: &str = "https://soroban-testnet.stellar.org:443";

/// The Horizon API endpoint for Stellar testnet (for account lookups).
pub const TESTNET_HORIZON_URL: &str = "https://horizon-testnet.stellar.org";

/// The friendbot URL for funding testnet accounts.
pub const FRIENDBOT_URL: &str = "https://friendbot.stellar.org";

// ─── Keypair ──────────────────────────────────────────────────────────

/// A Stellar ed25519 keypair for signing transactions.
pub struct StellarKeypair {
    pub signing_key: SigningKey,
    pub verifying_key: ed25519_dalek::VerifyingKey,
}

impl StellarKeypair {
    /// Generate a new random keypair.
    pub fn generate() -> Self {
        let mut rng = rand::rngs::OsRng;
        let signing_key = SigningKey::generate(&mut rng);
        let verifying_key = signing_key.verifying_key();
        Self {
            signing_key,
            verifying_key,
        }
    }

    /// Restore a keypair from a 32-byte secret key.
    pub fn from_secret_bytes(secret: &[u8; 32]) -> Self {
        let signing_key = SigningKey::from_bytes(secret);
        let verifying_key = signing_key.verifying_key();
        Self {
            signing_key,
            verifying_key,
        }
    }

    /// Get the secret key as 32 bytes.
    pub fn secret_bytes(&self) -> [u8; 32] {
        self.signing_key.to_bytes()
    }

    /// Get the public key as 32 bytes.
    pub fn public_bytes(&self) -> [u8; 32] {
        self.verifying_key.to_bytes()
    }

    /// Get the account ID as a Stellar strkey (G... format, 56 chars).
    pub fn account_id(&self) -> String {
        encode_account_id(&self.public_bytes())
    }

    /// Get the secret key as a Stellar strkey (S... format, 56 chars).
    pub fn secret_key_str(&self) -> String {
        encode_secret_key(&self.secret_bytes())
    }
}

/// Generate a new random Stellar keypair.
pub fn generate_keypair() -> StellarKeypair {
    StellarKeypair::generate()
}

// ─── Account funding ──────────────────────────────────────────────────

/// Fund a testnet account via friendbot.
///
/// Sends a GET request to `https://friendbot.stellar.org?addr=<public_key>`.
/// The account receives 10,000 XLM on testnet.
pub async fn fund_account(public_key_str: &str) -> AuditResult<()> {
    let url = format!("{}?addr={}", FRIENDBOT_URL, public_key_str);
    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| AuditError::Validation(format!("friendbot request failed: {e}")))?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(AuditError::Validation(format!(
            "friendbot funding failed: {text}"
        )));
    }

    Ok(())
}

// ─── Account sequence lookup ──────────────────────────────────────────

/// Get the account sequence number from the Horizon API.
async fn get_account_sequence(horizon_url: &str, account_id: &str) -> AuditResult<i64> {
    let url = format!("{}/accounts/{}", horizon_url, account_id);
    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| AuditError::Validation(format!("Horizon request failed: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(AuditError::Validation(format!(
            "Horizon returned error: {status} {text}"
        )));
    }

    let account: HorizonAccount = resp
        .json()
        .await
        .map_err(|e| AuditError::Validation(format!("failed to parse Horizon response: {e}")))?;

    Ok(account.sequence)
}

#[derive(Deserialize)]
struct HorizonAccount {
    /// Horizon returns sequence as a JSON string (e.g. "14035291698364416"),
    /// not a number. Use a custom deserializer to parse it.
    #[serde(deserialize_with = "deserialize_sequence_string")]
    sequence: i64,
}

fn deserialize_sequence_string<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de;
    let s: std::borrow::Cow<'de, str> = serde::Deserialize::deserialize(deserializer)?;
    s.parse::<i64>().map_err(de::Error::custom)
}

/// Deserialize an optional integer that may be encoded as a JSON string.
fn deserialize_optional_sequence_string<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de;
    let opt: Option<std::borrow::Cow<'de, str>> = serde::Deserialize::deserialize(deserializer)?;
    match opt {
        Some(s) => s.parse::<i64>().map(Some).map_err(de::Error::custom),
        None => Ok(None),
    }
}

// ─── Transaction building ─────────────────────────────────────────────

/// Build a Soroban invoke contract transaction.
fn build_invoke_transaction(
    source_public_key: &[u8; 32],
    sequence: i64,
    contract_id: &str,
    function_name: &str,
    args: Vec<ScVal>,
    fee: u32,
) -> AuditResult<Transaction> {
    // Convert public key to MuxedAccount.
    let muxed = MuxedAccount::Ed25519(Uint256(*source_public_key));

    // Convert contract_id to ScAddress::Contract.
    // Accepts both C... strkey and hex formats.
    let contract_arr = decode_contract_id(contract_id)?;
    let contract_addr = ScAddress::Contract(ContractId(Hash(contract_arr)));

    // Build function name as ScSymbol.
    let symbol = ScSymbol(
        function_name
            .as_bytes()
            .to_vec()
            .try_into()
            .map_err(|e: stellar_xdr::curr::Error| {
                AuditError::Validation(format!("invalid function name: {e}"))
            })?,
    );

    // Build args VecM.
    let args_vecm: VecM<ScVal> = args.try_into().map_err(|e: stellar_xdr::curr::Error| {
        AuditError::Validation(format!("too many args: {e}"))
    })?;

    // Build InvokeContractArgs.
    let invoke_args = InvokeContractArgs {
        contract_address: contract_addr,
        function_name: symbol,
        args: args_vecm,
    };

    // Build Operation.
    let op = Operation {
        source_account: None,
        body: OperationBody::InvokeHostFunction(InvokeHostFunctionOp {
            host_function: HostFunction::InvokeContract(invoke_args),
            auth: VecM::default(),
        }),
    };

    // Build operations VecM.
    let operations: VecM<Operation, 100> = vec![op]
        .try_into()
        .map_err(|e: stellar_xdr::curr::Error| {
            AuditError::Validation(format!("too many operations: {e}"))
        })?;

    // Build Transaction.
    Ok(Transaction {
        source_account: muxed,
        fee,
        seq_num: SequenceNumber(sequence + 1),
        cond: Preconditions::None,
        memo: Memo::None,
        operations,
        ext: TransactionExt::V0,
    })
}

// ─── Transaction simulation ───────────────────────────────────────────

/// Simulate a transaction via the Soroban RPC `simulateTransaction` method.
async fn simulate_transaction(
    rpc_url: &str,
    tx: &Transaction,
) -> AuditResult<SimulationResult> {
    // Encode the transaction envelope to XDR, then base64.
    let envelope = TransactionEnvelope::Tx(TransactionV1Envelope {
        tx: tx.clone(),
        signatures: VecM::default(),
    });
    let xdr_bytes = envelope
        .to_xdr(Limits::none())
        .map_err(|e| AuditError::Validation(format!("XDR encoding failed: {e}")))?;
    let xdr_b64 = base64::engine::general_purpose::STANDARD.encode(&xdr_bytes);

    let client = reqwest::Client::new();
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "simulateTransaction",
        "params": {
            "transaction": xdr_b64
        }
    });

    let resp = client
        .post(rpc_url)
        .json(&request)
        .send()
        .await
        .map_err(|e| AuditError::Validation(format!("RPC request failed: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(AuditError::Validation(format!(
            "RPC returned error: {status} {text}"
        )));
    }

    let body = resp
        .text()
        .await
        .map_err(|e| AuditError::Validation(format!("failed to read RPC response body: {e}")))?;

    let result: JsonRpcResponse<SimulationResult> = serde_json::from_str(&body)
        .map_err(|e| AuditError::Validation(format!(
            "failed to parse RPC response: {e}\nResponse body: {}",
            &body[..body.len().min(2000)]
        )))?;

    if let Some(err) = result.error {
        return Err(AuditError::Validation(format!(
            "simulateTransaction error: code {} message {}",
            err.code, err.message
        )));
    }

    let mut sim = result
        .result
        .ok_or_else(|| AuditError::Validation("simulateTransaction returned no result".to_string()))?;

    // Check for simulation-level error (the RPC returns this inside `result`,
    // not as a top-level JSON-RPC error).
    if let Some(sim_err) = &sim.error {
        return Err(AuditError::Validation(format!(
            "simulateTransaction returned error: {sim_err}"
        )));
    }

    if sim.transaction_data.is_none() {
        return Err(AuditError::Validation(
            "simulateTransaction returned no transactionData — the contract function may not exist or the contract may not be initialized".to_string(),
        ));
    }

    // Unwrap transaction_data so callers don't need to handle Option.
    sim.transaction_data_owned = sim.transaction_data.take();

    Ok(sim)
}

#[derive(Deserialize)]
struct SimulationResult {
    #[serde(rename = "transactionData")]
    transaction_data: Option<String>,
    /// Filled in by simulate_transaction after validation (callers use this).
    #[serde(skip)]
    transaction_data_owned: Option<String>,
    #[serde(rename = "minResourceFee", default, deserialize_with = "deserialize_optional_sequence_string")]
    #[allow(dead_code)]
    min_resource_fee: Option<i64>,
    #[serde(default)]
    results: Vec<SimulationResultEntry>,
    #[serde(rename = "latestLedger")]
    #[allow(dead_code)]
    latest_ledger: u32,
    /// Present when the simulation fails (e.g. contract function not found).
    #[serde(default)]
    error: Option<String>,
}

impl SimulationResult {
    /// Get the transaction data (guaranteed to be Some after simulate_transaction returns Ok).
    fn transaction_data(&self) -> &str {
        self.transaction_data_owned
            .as_deref()
            .or(self.transaction_data.as_deref())
            .expect("transaction_data validated by simulate_transaction")
    }
}

#[derive(Deserialize)]
struct SimulationResultEntry {
    auth: Vec<String>,
    xdr: String,
}

// ─── Transaction submission ───────────────────────────────────────────

/// Submit a signed transaction via the Soroban RPC `sendTransaction` method.
async fn send_transaction(rpc_url: &str, envelope: &TransactionEnvelope) -> AuditResult<String> {
    let xdr_bytes = envelope
        .to_xdr(Limits::none())
        .map_err(|e| AuditError::Validation(format!("XDR encoding failed: {e}")))?;
    let xdr_b64 = base64::engine::general_purpose::STANDARD.encode(&xdr_bytes);

    let client = reqwest::Client::new();
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "sendTransaction",
        "params": {
            "transaction": xdr_b64
        }
    });

    let resp = client
        .post(rpc_url)
        .json(&request)
        .send()
        .await
        .map_err(|e| AuditError::Validation(format!("RPC request failed: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(AuditError::Validation(format!(
            "RPC returned error: {status} {text}"
        )));
    }

    let body = resp
        .text()
        .await
        .map_err(|e| AuditError::Validation(format!("failed to read RPC response body: {e}")))?;

    let result: JsonRpcResponse<SendResult> = serde_json::from_str(&body)
        .map_err(|e| AuditError::Validation(format!(
            "failed to parse RPC response: {e}\nResponse body: {}",
            &body[..body.len().min(2000)]
        )))?;

    if let Some(err) = result.error {
        return Err(AuditError::Validation(format!(
            "sendTransaction error: code {} message {}",
            err.code, err.message
        )));
    }

    let send_result = result
        .result
        .ok_or_else(|| AuditError::Validation("sendTransaction returned no result".to_string()))?;

    if send_result.status == "ERROR" {
        return Err(AuditError::Validation(format!(
            "transaction rejected: {}",
            send_result.error_result_xdr.unwrap_or_default()
        )));
    }

    Ok(send_result.hash)
}

#[derive(Deserialize)]
struct SendResult {
    status: String,
    hash: String,
    #[serde(rename = "errorResultXdr")]
    error_result_xdr: Option<String>,
}

#[derive(Deserialize)]
struct JsonRpcResponse<T> {
    result: Option<T>,
    error: Option<JsonRpcError>,
}

#[derive(Deserialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

// ─── Transaction signing ──────────────────────────────────────────────

/// Sign a transaction with an ed25519 key.
///
/// The signature is over the SHA256 hash of a `TransactionSignaturePayload`
/// containing the network ID and the transaction. The network ID is
/// `SHA256(network_passphrase)`.
fn sign_transaction(
    tx: &Transaction,
    signing_key: &SigningKey,
    network_passphrase: &str,
) -> AuditResult<DecoratedSignature> {
    // Compute the network ID (SHA256 of the passphrase).
    let network_id = Sha256::digest(network_passphrase.as_bytes());

    // Build the TransactionSignaturePayload.
    let payload = TransactionSignaturePayload {
        network_id: Hash(network_id.into()),
        tagged_transaction: TransactionSignaturePayloadTaggedTransaction::Tx(tx.clone()),
    };

    // Encode and hash.
    let payload_xdr = payload
        .to_xdr(Limits::none())
        .map_err(|e| AuditError::Validation(format!("payload XDR encoding failed: {e}")))?;
    let hash = Sha256::digest(&payload_xdr);

    // Sign the hash.
    let signature = signing_key.sign(&hash);
    let sig_bytes = signature.to_bytes();

    // The signature hint is the last 4 bytes of the public key.
    let pub_bytes = signing_key.verifying_key().to_bytes();
    let hint = SignatureHint(pub_bytes[28..32].try_into().unwrap());

    Ok(DecoratedSignature {
        hint,
        signature: Signature(
            sig_bytes
                .to_vec()
                .try_into()
                .map_err(|e: stellar_xdr::curr::Error| {
                    AuditError::Validation(format!("invalid signature length: {e}"))
                })?,
        ),
    })
}

// ─── High-level: commit_root ──────────────────────────────────────────

/// Commit a Merkle root to the Soroban contract using native signing.
///
/// This is the native replacement for `stellar::commit_root()`. It builds
/// the transaction, simulates it, signs it, and submits it — all without
/// the `stellar` CLI.
///
/// # Arguments
/// * `root_hex` - 64-character hex string (32 bytes)
/// * `metadata` - arbitrary string stored on-chain with the commitment
/// * `keypair` - the Stellar keypair to sign with
/// * `rpc_url` - Soroban RPC endpoint (e.g. `TESTNET_RPC_URL`)
/// * `horizon_url` - Horizon API endpoint for account lookups
/// * `contract_id` - hex contract ID (32 bytes = 64 hex chars)
/// * `network_passphrase` - network passphrase (e.g. `TESTNET_PASSPHRASE`)
pub async fn commit_root_native(
    root_hex: &str,
    metadata: &str,
    keypair: &StellarKeypair,
    rpc_url: &str,
    horizon_url: &str,
    contract_id: &str,
    network_passphrase: &str,
) -> AuditResult<CommitResult> {
    // 1. Get account sequence number from Horizon.
    let account_id = keypair.account_id();
    let sequence = get_account_sequence(horizon_url, &account_id).await?;

    // 2. Build ScVal args: root (Bytes), metadata (String).
    let root_bytes = hex::decode(root_hex)
        .map_err(|e| AuditError::Validation(format!("invalid root hex: {e}")))?;

    let root_scval = ScVal::Bytes(ScBytes(
        root_bytes
            .clone()
            .try_into()
            .map_err(|e: stellar_xdr::curr::Error| {
                AuditError::Validation(format!("invalid root bytes: {e}"))
            })?,
    ));

    let metadata_scval = ScVal::String(ScString(
        metadata
            .as_bytes()
            .to_vec()
            .try_into()
            .map_err(|e: stellar_xdr::curr::Error| {
                AuditError::Validation(format!("invalid metadata: {e}"))
            })?,
    ));

    let args = vec![root_scval, metadata_scval];

    // 3. Build the transaction.
    let tx = build_invoke_transaction(
        &keypair.public_bytes(),
        sequence,
        contract_id,
        "commit_root",
        args,
        100, // base fee per operation (stroops)
    )?;

    // 4. Simulate the transaction.
    let sim_result = simulate_transaction(rpc_url, &tx).await?;

    // 5. Attach the simulation's SorobanTransactionData to the transaction.
    let soroban_data_xdr = base64::engine::general_purpose::STANDARD
        .decode(&sim_result.transaction_data())
        .map_err(|e| AuditError::Validation(format!("base64 decode transactionData: {e}")))?;
    let soroban_data = SorobanTransactionData::from_xdr(&soroban_data_xdr, Limits::none())
        .map_err(|e| AuditError::Validation(format!("decode SorobanTransactionData: {e}")))?;

    let mut signed_tx = tx.clone();
    signed_tx.ext = TransactionExt::V1(soroban_data);

    // 6. Attach auth entries if the simulation returned any.
    if !sim_result.results.is_empty() && !sim_result.results[0].auth.is_empty() {
        let auth_entries: Result<Vec<SorobanAuthorizationEntry>, AuditError> = sim_result.results[0]
            .auth
            .iter()
            .map(|auth_b64| {
                let auth_xdr = base64::engine::general_purpose::STANDARD
                    .decode(auth_b64)
                    .map_err(|e| AuditError::Validation(format!("base64 decode auth: {e}")))?;
                SorobanAuthorizationEntry::from_xdr(&auth_xdr, Limits::none())
                    .map_err(|e| AuditError::Validation(format!("decode auth: {e}")))
            })
            .collect();

        let auth_entries = auth_entries?;
        let auth_vecm: VecM<SorobanAuthorizationEntry> =
            auth_entries.try_into().map_err(|e: stellar_xdr::curr::Error| {
                AuditError::Validation(format!("too many auth entries: {e}"))
            })?;

        // Rebuild the operations with auth attached. VecM doesn't impl DerefMut,
        // so we reconstruct the operation list.
        let mut ops = signed_tx.operations.to_vec();
        if let OperationBody::InvokeHostFunction(ref mut invoke_op) = ops[0].body {
            invoke_op.auth = auth_vecm;
        }
        signed_tx.operations = ops
            .try_into()
            .map_err(|e: stellar_xdr::curr::Error| {
                AuditError::Validation(format!("too many operations: {e}"))
            })?;
    }

    // 7. Sign the transaction.
    let decorated_sig =
        sign_transaction(&signed_tx, &keypair.signing_key, network_passphrase)?;

    // 8. Build the signed envelope.
    let signatures: VecM<DecoratedSignature, 20> =
        vec![decorated_sig]
            .try_into()
            .map_err(|e: stellar_xdr::curr::Error| {
                AuditError::Validation(format!("too many signatures: {e}"))
            })?;

    let signed_envelope = TransactionEnvelope::Tx(TransactionV1Envelope {
        tx: signed_tx,
        signatures,
    });

    // 9. Submit the transaction.
    let tx_hash = send_transaction(rpc_url, &signed_envelope).await?;

    // 10. Parse the return value from the simulation to get the on-chain sequence.
    let on_chain_sequence = if !sim_result.results.is_empty() {
        let return_val_xdr = base64::engine::general_purpose::STANDARD
            .decode(&sim_result.results[0].xdr)
            .map_err(|e| AuditError::Validation(format!("base64 decode return value: {e}")))?;
        let return_val = ScVal::from_xdr(&return_val_xdr, Limits::none())
            .map_err(|e| AuditError::Validation(format!("decode return value: {e}")))?;
        match return_val {
            ScVal::U64(seq) => seq,
            _ => 0,
        }
    } else {
        0
    };

    Ok(CommitResult {
        sequence: on_chain_sequence,
        tx_hash,
        root_hex: root_hex.to_string(),
    })
}

// ─── High-level: commit_root_with_oplog ───────────────────────────────

/// Commit a Merkle root with an oplog completeness commitment using native signing.
///
/// This is the native replacement for `stellar::commit_root_with_oplog()`. It
/// calls `commit_root_with_oplog` on the contract with 6 args: root, oplog_root,
/// oplog_start_ts, oplog_end_ts, oplog_entry_count, metadata.
///
/// Timestamps are packed as `(time << 32) | increment`.
pub async fn commit_root_with_oplog_native(
    root_hex: &str,
    oplog_root_hex: &str,
    oplog_start_ts: u64,
    oplog_end_ts: u64,
    oplog_entry_count: u64,
    metadata: &str,
    keypair: &StellarKeypair,
    rpc_url: &str,
    horizon_url: &str,
    contract_id: &str,
    network_passphrase: &str,
) -> AuditResult<CommitResult> {
    let account_id = keypair.account_id();
    let sequence = get_account_sequence(horizon_url, &account_id).await?;

    let root_bytes = hex::decode(root_hex)
        .map_err(|e| AuditError::Validation(format!("invalid root hex: {e}")))?;
    let root_scval = ScVal::Bytes(ScBytes(
        root_bytes
            .clone()
            .try_into()
            .map_err(|e: stellar_xdr::curr::Error| {
                AuditError::Validation(format!("invalid root bytes: {e}"))
            })?,
    ));

    let oplog_root_bytes = hex::decode(oplog_root_hex)
        .map_err(|e| AuditError::Validation(format!("invalid oplog root hex: {e}")))?;
    let oplog_root_scval = ScVal::Bytes(ScBytes(
        oplog_root_bytes
            .try_into()
            .map_err(|e: stellar_xdr::curr::Error| {
                AuditError::Validation(format!("invalid oplog root bytes: {e}"))
            })?,
    ));

    let metadata_scval = ScVal::String(ScString(
        metadata
            .as_bytes()
            .to_vec()
            .try_into()
            .map_err(|e: stellar_xdr::curr::Error| {
                AuditError::Validation(format!("invalid metadata: {e}"))
            })?,
    ));

    let args = vec![
        root_scval,
        oplog_root_scval,
        ScVal::U64(oplog_start_ts),
        ScVal::U64(oplog_end_ts),
        ScVal::U64(oplog_entry_count),
        metadata_scval,
    ];

    let tx = build_invoke_transaction(
        &keypair.public_bytes(),
        sequence,
        contract_id,
        "commit_root_with_oplog",
        args,
        100,
    )?;

    let sim_result = simulate_transaction(rpc_url, &tx).await?;

    let soroban_data_xdr = base64::engine::general_purpose::STANDARD
        .decode(&sim_result.transaction_data())
        .map_err(|e| AuditError::Validation(format!("base64 decode transactionData: {e}")))?;
    let soroban_data = SorobanTransactionData::from_xdr(&soroban_data_xdr, Limits::none())
        .map_err(|e| AuditError::Validation(format!("decode SorobanTransactionData: {e}")))?;

    let mut signed_tx = tx.clone();
    signed_tx.ext = TransactionExt::V1(soroban_data);

    if !sim_result.results.is_empty() && !sim_result.results[0].auth.is_empty() {
        let auth_entries: Result<Vec<SorobanAuthorizationEntry>, AuditError> = sim_result.results[0]
            .auth
            .iter()
            .map(|auth_b64| {
                let auth_xdr = base64::engine::general_purpose::STANDARD
                    .decode(auth_b64)
                    .map_err(|e| AuditError::Validation(format!("base64 decode auth: {e}")))?;
                SorobanAuthorizationEntry::from_xdr(&auth_xdr, Limits::none())
                    .map_err(|e| AuditError::Validation(format!("decode auth: {e}")))
            })
            .collect();

        let auth_entries = auth_entries?;
        let auth_vecm: VecM<SorobanAuthorizationEntry> =
            auth_entries.try_into().map_err(|e: stellar_xdr::curr::Error| {
                AuditError::Validation(format!("too many auth entries: {e}"))
            })?;

        let mut ops = signed_tx.operations.to_vec();
        if let OperationBody::InvokeHostFunction(ref mut invoke_op) = ops[0].body {
            invoke_op.auth = auth_vecm;
        }
        signed_tx.operations = ops
            .try_into()
            .map_err(|e: stellar_xdr::curr::Error| {
                AuditError::Validation(format!("too many operations: {e}"))
            })?;
    }

    let decorated_sig =
        sign_transaction(&signed_tx, &keypair.signing_key, network_passphrase)?;

    let signatures: VecM<DecoratedSignature, 20> =
        vec![decorated_sig]
            .try_into()
            .map_err(|e: stellar_xdr::curr::Error| {
                AuditError::Validation(format!("too many signatures: {e}"))
            })?;

    let signed_envelope = TransactionEnvelope::Tx(TransactionV1Envelope {
        tx: signed_tx,
        signatures,
    });

    let tx_hash = send_transaction(rpc_url, &signed_envelope).await?;

    let on_chain_sequence = if !sim_result.results.is_empty() {
        let return_val_xdr = base64::engine::general_purpose::STANDARD
            .decode(&sim_result.results[0].xdr)
            .map_err(|e| AuditError::Validation(format!("base64 decode return value: {e}")))?;
        let return_val = ScVal::from_xdr(&return_val_xdr, Limits::none())
            .map_err(|e| AuditError::Validation(format!("decode return value: {e}")))?;
        match return_val {
            ScVal::U64(seq) => seq,
            _ => 0,
        }
    } else {
        0
    };

    Ok(CommitResult {
        sequence: on_chain_sequence,
        tx_hash,
        root_hex: root_hex.to_string(),
    })
}

// ─── High-level: attest_oplog ─────────────────────────────────────────

/// Submit an oplog attestation to the contract using native signing.
///
/// This is the native replacement for `stellar::attest_oplog()`. It calls
/// `attest_oplog` on the contract with 3 args: attester (Address), sequence
/// (U64), signature (Bytes).
///
/// The `attester_keypair` is the Stellar account keypair (not the ed25519
/// attester signing key — that's a separate key used to produce `signature_hex`).
pub async fn attest_oplog_native(
    attester_keypair: &StellarKeypair,
    sequence: u64,
    signature_hex: &str,
    rpc_url: &str,
    horizon_url: &str,
    contract_id: &str,
    network_passphrase: &str,
) -> AuditResult<()> {
    let account_id = attester_keypair.account_id();
    let acct_sequence = get_account_sequence(horizon_url, &account_id).await?;

    // Build the attester Address ScVal from the keypair's public key.
    let attester_address_scval = ScVal::Address(ScAddress::Account(AccountId(
        PublicKey::PublicKeyTypeEd25519(Uint256(attester_keypair.public_bytes())),
    )));

    let signature_bytes = hex::decode(signature_hex)
        .map_err(|e| AuditError::Validation(format!("invalid signature hex: {e}")))?;
    let signature_scval = ScVal::Bytes(ScBytes(
        signature_bytes
            .try_into()
            .map_err(|e: stellar_xdr::curr::Error| {
                AuditError::Validation(format!("invalid signature bytes: {e}"))
            })?,
    ));

    let args = vec![
        attester_address_scval,
        ScVal::U64(sequence),
        signature_scval,
    ];

    let tx = build_invoke_transaction(
        &attester_keypair.public_bytes(),
        acct_sequence,
        contract_id,
        "attest_oplog",
        args,
        100,
    )?;

    let sim_result = simulate_transaction(rpc_url, &tx).await?;

    let soroban_data_xdr = base64::engine::general_purpose::STANDARD
        .decode(&sim_result.transaction_data())
        .map_err(|e| AuditError::Validation(format!("base64 decode transactionData: {e}")))?;
    let soroban_data = SorobanTransactionData::from_xdr(&soroban_data_xdr, Limits::none())
        .map_err(|e| AuditError::Validation(format!("decode SorobanTransactionData: {e}")))?;

    let mut signed_tx = tx.clone();
    signed_tx.ext = TransactionExt::V1(soroban_data);

    if !sim_result.results.is_empty() && !sim_result.results[0].auth.is_empty() {
        let auth_entries: Result<Vec<SorobanAuthorizationEntry>, AuditError> = sim_result.results[0]
            .auth
            .iter()
            .map(|auth_b64| {
                let auth_xdr = base64::engine::general_purpose::STANDARD
                    .decode(auth_b64)
                    .map_err(|e| AuditError::Validation(format!("base64 decode auth: {e}")))?;
                SorobanAuthorizationEntry::from_xdr(&auth_xdr, Limits::none())
                    .map_err(|e| AuditError::Validation(format!("decode auth: {e}")))
            })
            .collect();

        let auth_entries = auth_entries?;
        let auth_vecm: VecM<SorobanAuthorizationEntry> =
            auth_entries.try_into().map_err(|e: stellar_xdr::curr::Error| {
                AuditError::Validation(format!("too many auth entries: {e}"))
            })?;

        let mut ops = signed_tx.operations.to_vec();
        if let OperationBody::InvokeHostFunction(ref mut invoke_op) = ops[0].body {
            invoke_op.auth = auth_vecm;
        }
        signed_tx.operations = ops
            .try_into()
            .map_err(|e: stellar_xdr::curr::Error| {
                AuditError::Validation(format!("too many operations: {e}"))
            })?;
    }

    let decorated_sig =
        sign_transaction(&signed_tx, &attester_keypair.signing_key, network_passphrase)?;

    let signatures: VecM<DecoratedSignature, 20> =
        vec![decorated_sig]
            .try_into()
            .map_err(|e: stellar_xdr::curr::Error| {
                AuditError::Validation(format!("too many signatures: {e}"))
            })?;

    let signed_envelope = TransactionEnvelope::Tx(TransactionV1Envelope {
        tx: signed_tx,
        signatures,
    });

    send_transaction(rpc_url, &signed_envelope).await?;
    Ok(())
}

// ─── High-level: get_oplog_commitment (read-only simulation) ──────────

/// Read an oplog commitment from the contract via a read-only simulation.
///
/// This is the native replacement for `stellar::get_oplog_commitment()`. It
/// builds a transaction invoking `get_oplog_commitment(sequence)`, simulates
/// it (without signing or submitting), and parses the return value.
///
/// The `source_keypair` is needed to build a valid transaction structure, but
/// since this is a read-only simulation, the account doesn't need to exist or
/// be funded — any valid ed25519 public key works.
pub async fn get_oplog_commitment_native(
    sequence: u64,
    source_keypair: &StellarKeypair,
    rpc_url: &str,
    contract_id: &str,
) -> AuditResult<Option<OnChainOplogCommitment>> {
    let args = vec![ScVal::U64(sequence)];

    // Use sequence=0 for the transaction — simulation doesn't require a valid
    // account sequence for read-only functions.
    let tx = build_invoke_transaction(
        &source_keypair.public_bytes(),
        0,
        contract_id,
        "get_oplog_commitment",
        args,
        100,
    )?;

    let sim_result = simulate_transaction(rpc_url, &tx).await?;

    if sim_result.results.is_empty() {
        return Ok(None);
    }

    let return_val_xdr = base64::engine::general_purpose::STANDARD
        .decode(&sim_result.results[0].xdr)
        .map_err(|e| AuditError::Validation(format!("base64 decode return value: {e}")))?;
    let return_val = ScVal::from_xdr(&return_val_xdr, Limits::none())
        .map_err(|e| AuditError::Validation(format!("decode return value: {e}")))?;

    // The contract returns Option<OplogCommitment>. When None, the ScVal is Void.
    match return_val {
        ScVal::Void => Ok(None),
        ScVal::Map(None) => Ok(None),
        ScVal::Map(Some(map)) => {
            let mut oplog_root_hex = String::new();
            let mut oplog_start_ts = 0u64;
            let mut oplog_end_ts = 0u64;
            let mut oplog_entry_count = 0u64;

            for entry in map.0.iter() {
                let key = &entry.key;
                let val = &entry.val;
                match key {
                    ScVal::Symbol(s) => {
                        let name = String::from_utf8_lossy(s.0.as_slice());
                        match name.as_ref() {
                            "oplog_root" => {
                                if let ScVal::Bytes(b) = val {
                                    oplog_root_hex = hex::encode(b.0.as_slice());
                                }
                            }
                            "oplog_start_ts" => {
                                if let ScVal::U64(v) = val {
                                    oplog_start_ts = *v;
                                }
                            }
                            "oplog_end_ts" => {
                                if let ScVal::U64(v) = val {
                                    oplog_end_ts = *v;
                                }
                            }
                            "oplog_entry_count" => {
                                if let ScVal::U64(v) = val {
                                    oplog_entry_count = *v;
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }

            if oplog_root_hex.is_empty() {
                return Ok(None);
            }

            Ok(Some(OnChainOplogCommitment {
                sequence,
                oplog_root_hex,
                oplog_start_ts,
                oplog_end_ts,
                oplog_entry_count,
            }))
        }
        _ => Ok(None),
    }
}

// ─── High-level: initialize + authorize_attester (setup wizard) ───────

/// Attach Soroban data and auth entries from a simulation result to a transaction.
///
/// This is the common pattern used by all native signing functions: simulate,
/// attach the `SorobanTransactionData` to `tx.ext`, attach any auth entries
/// from the simulation to the `InvokeHostFunctionOp.auth` field, and update
/// the fee to include the simulation's `minResourceFee`.
fn attach_soroban_data_and_auth(
    tx: &Transaction,
    sim_result: &SimulationResult,
) -> AuditResult<Transaction> {
    let soroban_data_xdr = base64::engine::general_purpose::STANDARD
        .decode(sim_result.transaction_data())
        .map_err(|e| AuditError::Validation(format!("base64 decode transactionData: {e}")))?;
    let soroban_data = SorobanTransactionData::from_xdr(&soroban_data_xdr, Limits::none())
        .map_err(|e| AuditError::Validation(format!("decode SorobanTransactionData: {e}")))?;

    let mut signed_tx = tx.clone();
    signed_tx.ext = TransactionExt::V1(soroban_data);

    // Update the fee to include the simulation's minResourceFee.
    // Soroban transactions require: fee >= base_fee + min_resource_fee.
    // The simulation returns minResourceFee as a string; we already parsed it.
    if let Some(min_fee) = sim_result.min_resource_fee {
        let total_fee = (tx.fee as i64) + min_fee + 1; // +1 for safety margin
        signed_tx.fee = total_fee as u32;
    }

    // Attach auth entries if the simulation returned any.
    if !sim_result.results.is_empty() && !sim_result.results[0].auth.is_empty() {
        let auth_entries: Result<Vec<SorobanAuthorizationEntry>, AuditError> = sim_result.results[0]
            .auth
            .iter()
            .map(|auth_b64| {
                let auth_xdr = base64::engine::general_purpose::STANDARD
                    .decode(auth_b64)
                    .map_err(|e| AuditError::Validation(format!("base64 decode auth: {e}")))?;
                SorobanAuthorizationEntry::from_xdr(&auth_xdr, Limits::none())
                    .map_err(|e| AuditError::Validation(format!("decode auth: {e}")))
            })
            .collect();

        let auth_entries = auth_entries?;
        let auth_vecm: VecM<SorobanAuthorizationEntry> =
            auth_entries.try_into().map_err(|e: stellar_xdr::curr::Error| {
                AuditError::Validation(format!("too many auth entries: {e}"))
            })?;

        let mut ops = signed_tx.operations.to_vec();
        if let OperationBody::InvokeHostFunction(ref mut invoke_op) = ops[0].body {
            invoke_op.auth = auth_vecm;
        }
        signed_tx.operations = ops
            .try_into()
            .map_err(|e: stellar_xdr::curr::Error| {
                AuditError::Validation(format!("too many operations: {e}"))
            })?;
    }

    Ok(signed_tx)
}

/// Sign a transaction and submit it via the Soroban RPC.
async fn sign_and_send(
    tx: &Transaction,
    signing_key: &SigningKey,
    network_passphrase: &str,
    rpc_url: &str,
) -> AuditResult<String> {
    let decorated_sig = sign_transaction(tx, signing_key, network_passphrase)?;

    let signatures: VecM<DecoratedSignature, 20> =
        vec![decorated_sig]
            .try_into()
            .map_err(|e: stellar_xdr::curr::Error| {
                AuditError::Validation(format!("too many signatures: {e}"))
            })?;

    let envelope = TransactionEnvelope::Tx(TransactionV1Envelope {
        tx: tx.clone(),
        signatures,
    });
    send_transaction(rpc_url, &envelope).await
}

/// Call `initialize(admin)` on the contract via native signing.
///
/// Used by the setup wizard to initialize a newly deployed contract.
/// The `admin_keypair` becomes the contract admin (authorized to commit roots).
pub async fn initialize_contract_native(
    contract_id: &str,
    admin_keypair: &StellarKeypair,
    rpc_url: &str,
    horizon_url: &str,
    network_passphrase: &str,
) -> AuditResult<()> {
    let account_id = admin_keypair.account_id();
    let sequence = get_account_sequence(horizon_url, &account_id).await?;

    let admin_scval = ScVal::Address(ScAddress::Account(AccountId(
        PublicKey::PublicKeyTypeEd25519(Uint256(admin_keypair.public_bytes())),
    )));

    let args = vec![admin_scval];

    let tx = build_invoke_transaction(
        &admin_keypair.public_bytes(),
        sequence,
        contract_id,
        "initialize",
        args,
        100,
    )?;

    let sim_result = simulate_transaction(rpc_url, &tx).await?;
    let signed_tx = attach_soroban_data_and_auth(&tx, &sim_result)?;
    sign_and_send(&signed_tx, &admin_keypair.signing_key, network_passphrase, rpc_url).await?;
    Ok(())
}

/// Call `authorize_attester(address, pubkey)` on the contract via native signing.
///
/// Used by the setup wizard to authorize the attester. The `admin_keypair`
/// must be the contract admin (set during `initialize`).
///
/// - `attester_address` is the attester's Stellar account address (G... strkey).
/// - `attester_ed25519_pubkey_hex` is the hex-encoded ed25519 public key that
///   signs oplog attestations (32 bytes = 64 hex chars).
pub async fn authorize_attester_native(
    contract_id: &str,
    admin_keypair: &StellarKeypair,
    attester_address: &str,
    attester_ed25519_pubkey_hex: &str,
    rpc_url: &str,
    horizon_url: &str,
    network_passphrase: &str,
) -> AuditResult<()> {
    let account_id = admin_keypair.account_id();
    let sequence = get_account_sequence(horizon_url, &account_id).await?;

    // Decode the attester's G... address to public key bytes.
    let attester_pubkey_bytes = decode_account_id_strkey(attester_address)
        .ok_or_else(|| {
            AuditError::Validation(format!(
                "invalid attester Stellar address (expected G... strkey): {attester_address}"
            ))
        })?;

    let attester_addr_scval = ScVal::Address(ScAddress::Account(AccountId(
        PublicKey::PublicKeyTypeEd25519(Uint256(attester_pubkey_bytes)),
    )));

    let pubkey_bytes = hex::decode(attester_ed25519_pubkey_hex)
        .map_err(|e| AuditError::Validation(format!("invalid ed25519 pubkey hex: {e}")))?;
    // Soroban SDK's BytesN<32> maps to ScVal::Bytes in the XDR layer.
    let pubkey_scval = ScVal::Bytes(ScBytes(
        pubkey_bytes
            .try_into()
            .map_err(|e: stellar_xdr::curr::Error| {
                AuditError::Validation(format!("invalid pubkey bytes: {e}"))
            })?,
    ));

    let args = vec![attester_addr_scval, pubkey_scval];

    let tx = build_invoke_transaction(
        &admin_keypair.public_bytes(),
        sequence,
        contract_id,
        "authorize_attester",
        args,
        100,
    )?;

    let sim_result = simulate_transaction(rpc_url, &tx).await?;
    let signed_tx = attach_soroban_data_and_auth(&tx, &sim_result)?;
    sign_and_send(&signed_tx, &admin_keypair.signing_key, network_passphrase, rpc_url).await?;
    Ok(())
}

/// Decode a Stellar account ID strkey (G...) to 32 raw bytes.
fn decode_account_id_strkey(s: &str) -> Option<[u8; 32]> {
    if !s.starts_with('G') {
        return None;
    }
    let decoded = base32_decode(s)?;
    if decoded.len() != 35 {
        return None;
    }
    // Version byte: 6 << 3 = 0x30 (ED25519 public key)
    if decoded[0] != 6 << 3 {
        return None;
    }
    let payload = &decoded[..33];
    let checksum = &decoded[33..];
    let expected = crc16_xmodem(payload);
    let expected_le = [(expected & 0xff) as u8, (expected >> 8) as u8];
    if checksum != expected_le {
        return None;
    }
    let mut result = [0u8; 32];
    result.copy_from_slice(&decoded[1..33]);
    Some(result)
}

// ─── Stellar strkey encoding ──────────────────────────────────────────

/// Encode a 32-byte ed25519 public key as a Stellar account ID (G...).
fn encode_account_id(public_key: &[u8; 32]) -> String {
    // Version byte: 6 << 3 | 0 (ED25519) = 0x30
    encode_strkey(6 << 3, public_key)
}

/// Encode a 32-byte ed25519 secret key as a Stellar secret key (S...).
fn encode_secret_key(secret: &[u8; 32]) -> String {
    // Version byte: 18 << 3 | 0 (ED25519) = 0x90
    encode_strkey(18 << 3, secret)
}

/// Decode a Stellar contract ID strkey (C...) to 32 raw bytes.
///
/// Contract IDs can be passed as either:
/// - A C... strkey (e.g. "CCUCFDRF6IMY3STBIFRBBGFFPETBSAPPTDACNOBYWKNPG5QUCAMGGUQ5")
/// - A 64-char hex string (e.g. "abc123...def")
pub fn decode_contract_id(contract_id: &str) -> AuditResult<[u8; 32]> {
    // If it starts with 'C', it's a strkey.
    if contract_id.starts_with('C') {
        let decoded = base32_decode(contract_id)
            .ok_or_else(|| AuditError::Validation("invalid contract ID strkey".to_string()))?;
        // Strkey format: version (1) + payload (32) + checksum (2) = 35 bytes
        if decoded.len() != 35 {
            return Err(AuditError::Validation(format!(
                "invalid contract ID strkey length: expected 35 bytes, got {}",
                decoded.len()
            )));
        }
        // Verify version byte: 2 << 3 = 0x10
        if decoded[0] != 2 << 3 {
            return Err(AuditError::Validation(format!(
                "invalid contract ID version byte: expected 0x{:02x}, got 0x{:02x}",
                2 << 3,
                decoded[0]
            )));
        }
        // Verify checksum (little-endian byte order)
        let payload = &decoded[..33];
        let checksum = &decoded[33..];
        let expected_checksum = crc16_xmodem(payload);
        let expected_le = [(expected_checksum & 0xff) as u8, (expected_checksum >> 8) as u8];
        if checksum != expected_le {
            return Err(AuditError::Validation("contract ID checksum mismatch".to_string()));
        }
        let mut result = [0u8; 32];
        result.copy_from_slice(&decoded[1..33]);
        Ok(result)
    } else {
        // Treat as hex.
        let bytes = hex::decode(contract_id)
            .map_err(|e| AuditError::Validation(format!("invalid contract ID hex: {e}")))?;
        bytes
            .as_slice()
            .try_into()
            .map_err(|_| AuditError::Validation("contract ID must be 32 bytes".to_string()))
    }
}

/// Encode a payload with a version byte using the Stellar strkey format:
/// version_byte || payload || CRC16-XModem checksum, base32-encoded (no padding).
fn encode_strkey(version: u8, payload: &[u8; 32]) -> String {
    let mut data = Vec::with_capacity(35);
    data.push(version);
    data.extend_from_slice(payload);
    let checksum = crc16_xmodem(&data);
    // Stellar strkey uses little-endian checksum byte order.
    data.push((checksum & 0xff) as u8);
    data.push((checksum >> 8) as u8);
    base32_encode(&data)
}

/// CRC16-XModem checksum (polynomial 0x1021, initial value 0x0000).
pub fn crc16_xmodem(data: &[u8]) -> u16 {
    let mut crc: u16 = 0x0000;
    for &byte in data {
        crc ^= u16::from(byte) << 8;
        for _ in 0..8 {
            if crc & 0x8000 != 0 {
                crc = (crc << 1) ^ 0x1021;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

/// Base32 encoding (RFC4648, no padding).
fn base32_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut result = String::new();
    let mut buffer: u32 = 0;
    let mut bits_left: u32 = 0;

    for &byte in data {
        buffer = (buffer << 8) | u32::from(byte);
        bits_left += 8;
        while bits_left >= 5 {
            bits_left -= 5;
            let index = ((buffer >> bits_left) & 0x1F) as usize;
            result.push(ALPHABET[index] as char);
        }
    }

    if bits_left > 0 {
        let index = ((buffer << (5 - bits_left)) & 0x1F) as usize;
        result.push(ALPHABET[index] as char);
    }

    result
}

/// Base32 decoding (RFC4648, no padding).
pub fn base32_decode(s: &str) -> Option<Vec<u8>> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut result = Vec::new();
    let mut buffer: u32 = 0;
    let mut bits_left: u32 = 0;

    for ch in s.chars() {
        let byte = ch as u8;
        let index = ALPHABET.iter().position(|&c| c == byte.to_ascii_uppercase())?;
        buffer = (buffer << 5) | index as u32;
        bits_left += 5;
        if bits_left >= 8 {
            bits_left -= 8;
            let byte_val = ((buffer >> bits_left) & 0xFF) as u8;
            result.push(byte_val);
        }
    }

    Some(result)
}

// ─── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crc16_xmodem_known_vector() {
        // CRC16-XModem of "123456789" is 0x31C3.
        assert_eq!(crc16_xmodem(b"123456789"), 0x31C3);
    }

    #[test]
    fn test_base32_rfc4648_vectors() {
        assert_eq!(base32_encode(b""), "");
        assert_eq!(base32_encode(b"f"), "MY");
        assert_eq!(base32_encode(b"fo"), "MZXQ");
        assert_eq!(base32_encode(b"foo"), "MZXW6");
        assert_eq!(base32_encode(b"foob"), "MZXW6YQ");
        assert_eq!(base32_encode(b"fooba"), "MZXW6YTB");
        assert_eq!(base32_encode(b"foobar"), "MZXW6YTBOI");
    }

    #[test]
    fn test_keypair_generates_valid_strkeys() {
        let kp = generate_keypair();
        let account_id = kp.account_id();
        assert!(account_id.starts_with('G'));
        assert_eq!(account_id.len(), 56);
        let secret = kp.secret_key_str();
        assert!(secret.starts_with('S'));
        assert_eq!(secret.len(), 56);
    }

    #[test]
    fn test_keypair_roundtrip_from_secret() {
        let kp = generate_keypair();
        let secret = kp.secret_bytes();
        let kp2 = StellarKeypair::from_secret_bytes(&secret);
        assert_eq!(kp.public_bytes(), kp2.public_bytes());
        assert_eq!(kp.account_id(), kp2.account_id());
    }

    #[test]
    fn test_strkey_zero_public_key() {
        // All-zero public key should produce a valid 56-char G... strkey.
        let zero_pk = [0u8; 32];
        let account_id = encode_account_id(&zero_pk);
        assert!(account_id.starts_with('G'));
        assert_eq!(account_id.len(), 56);
    }

    #[test]
    fn test_decode_contract_id_strkey() {
        // The contract ID from stellar.rs, as a C... strkey.
        let contract_strkey = "CCUCFDRF6IMY3STBIFRBBGFFPETBSAPPTDACNOBYWKNPG5QUCAMGGUQ5";
        let bytes = decode_contract_id(contract_strkey);
        assert!(bytes.is_ok());
        let bytes = bytes.unwrap();
        assert_eq!(bytes.len(), 32);
        // Verify roundtrip: re-encode as strkey should give the same C... string.
        // (Contract strkey version byte: 2 << 3 = 0x10)
        let reencoded = encode_strkey(2 << 3, &bytes);
        assert_eq!(reencoded, contract_strkey);
    }

    #[test]
    fn test_decode_contract_id_hex() {
        // 32 bytes as hex (64 chars).
        let hex_id = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
        let bytes = decode_contract_id(hex_id);
        assert!(bytes.is_ok());
        let bytes = bytes.unwrap();
        assert_eq!(bytes.len(), 32);
        assert_eq!(bytes[0], 0xab);
    }

    #[test]
    fn test_build_invoke_transaction_succeeds() {
        let kp = generate_keypair();
        let tx = build_invoke_transaction(
            &kp.public_bytes(),
            0,
            "CCUCFDRF6IMY3STBIFRBBGFFPETBSAPPTDACNOBYWKNPG5QUCAMGGUQ5",
            "commit_root",
            vec![ScVal::U32(42)],
            100,
        );
        assert!(tx.is_ok());
        let tx = tx.unwrap();
        assert_eq!(tx.fee, 100);
        assert!(matches!(tx.ext, TransactionExt::V0));
        assert_eq!(tx.operations.len(), 1);
    }

    #[test]
    fn test_build_attest_oplog_transaction() {
        // Verify that building a transaction for attest_oplog with an Address
        // arg + U64 + Bytes compiles and produces a valid structure.
        let kp = generate_keypair();
        let attester_addr = ScVal::Address(ScAddress::Account(AccountId(
            PublicKey::PublicKeyTypeEd25519(Uint256(kp.public_bytes())),
        )));
        let sig = vec![0u8; 64];
        let args = vec![
            attester_addr,
            ScVal::U64(1),
            ScVal::Bytes(ScBytes(sig.try_into().unwrap())),
        ];
        let tx = build_invoke_transaction(
            &kp.public_bytes(),
            0,
            "CCUCFDRF6IMY3STBIFRBBGFFPETBSAPPTDACNOBYWKNPG5QUCAMGGUQ5",
            "attest_oplog",
            args,
            100,
        );
        assert!(tx.is_ok());
        let tx = tx.unwrap();
        assert_eq!(tx.operations.len(), 1);
    }

    #[test]
    fn test_build_commit_root_with_oplog_transaction() {
        // Verify that building a transaction for commit_root_with_oplog with
        // 6 args (Bytes, Bytes, U64, U64, U64, String) compiles and produces
        // a valid structure.
        let kp = generate_keypair();
        let root = vec![0u8; 32];
        let oplog_root = vec![1u8; 32];
        let args = vec![
            ScVal::Bytes(ScBytes(root.try_into().unwrap())),
            ScVal::Bytes(ScBytes(oplog_root.try_into().unwrap())),
            ScVal::U64(100),
            ScVal::U64(200),
            ScVal::U64(42),
            ScVal::String(ScString("test".as_bytes().to_vec().try_into().unwrap())),
        ];
        let tx = build_invoke_transaction(
            &kp.public_bytes(),
            0,
            "CCUCFDRF6IMY3STBIFRBBGFFPETBSAPPTDACNOBYWKNPG5QUCAMGGUQ5",
            "commit_root_with_oplog",
            args,
            100,
        );
        assert!(tx.is_ok());
    }

    #[test]
    fn test_sign_transaction_produces_valid_signature() {
        let kp = generate_keypair();
        let tx = build_invoke_transaction(
            &kp.public_bytes(),
            0,
            "CCUCFDRF6IMY3STBIFRBBGFFPETBSAPPTDACNOBYWKNPG5QUCAMGGUQ5",
            "commit_root",
            vec![ScVal::U32(42)],
            100,
        )
        .unwrap();

        let sig = sign_transaction(&tx, &kp.signing_key, TESTNET_PASSPHRASE);
        assert!(sig.is_ok());
        let sig = sig.unwrap();
        // Signature hint is last 4 bytes of public key.
        let pub_bytes = kp.public_bytes();
        assert_eq!(sig.hint.0, pub_bytes[28..32]);
    }
}
