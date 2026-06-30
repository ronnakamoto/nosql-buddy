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
//! 1. Get account sequence number from the Soroban RPC ledger
//! 2. Build a `Transaction` with an `InvokeHostFunction` operation
//! 3. Simulate the transaction via Soroban RPC `simulateTransaction`
//! 4. Attach the simulation's `SorobanTransactionData` to the transaction
//! 5. Sign the transaction with ed25519 (over the `TransactionSignaturePayload` hash)
//! 6. Submit the signed transaction via Soroban RPC `sendTransaction`

use base64::Engine;
use ed25519_dalek::{Signer, SigningKey};
use rand::RngCore;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use tokio::sync::Mutex as TokioMutex;
use stellar_xdr::curr::{
    AccountId, BytesM, ContractExecutable, ContractId, ContractIdPreimage,
    ContractIdPreimageFromAddress, CreateContractArgs, DecoratedSignature, Hash, HashIdPreimage,
    HashIdPreimageContractId, HostFunction, InvokeContractArgs,
    InvokeHostFunctionOp, LedgerEntryData, LedgerKey, LedgerKeyAccount, Limits, Memo, MuxedAccount,
    Operation, OperationBody, Preconditions,
    PublicKey, ReadXdr, ScAddress, ScBytes, ScMap, ScMapEntry, ScString, ScSymbol, ScVal, ScVec,
    SequenceNumber, Signature, SignatureHint, SorobanAuthorizationEntry, SorobanTransactionData,
    Transaction, TransactionEnvelope, TransactionExt, TransactionSignaturePayload,
    TransactionSignaturePayloadTaggedTransaction, TransactionV1Envelope, Uint256, VecM, WriteXdr,
};

use crate::audit::stellar::{
    CommitResult, OnChainAttestationVerification, OnChainOplogCommitment, OnChainRoot,
    VerifyInclusionResult,
};
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

    /// Sign arbitrary bytes with the keypair's ed25519 signing key.
    pub fn sign_message(&self, message: &[u8]) -> [u8; 64] {
        self.signing_key.sign(message).to_bytes()
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

/// Get the account sequence number from the Soroban RPC ledger.
///
/// The sequence is read from the same RPC node that the transaction is
/// submitted to (via `getLedgerEntries`), rather than from Horizon. Horizon is
/// a separate service whose ledger ingestion lags behind the RPC's live state;
/// reading the sequence from Horizon while submitting to the RPC causes
/// `txBAD_SEQ` rejections whenever the account already has a transaction the
/// RPC has applied but Horizon has not yet ingested.
async fn get_account_sequence_from_rpc(
    rpc_url: &str,
    public_key: &[u8; 32],
) -> AuditResult<i64> {
    let ledger_key = LedgerKey::Account(LedgerKeyAccount {
        account_id: AccountId(PublicKey::PublicKeyTypeEd25519(Uint256(*public_key))),
    });
    let key_xdr = ledger_key
        .to_xdr(Limits::none())
        .map_err(|e| AuditError::Validation(format!("account ledger key XDR encoding failed: {e}")))?;
    let key_b64 = base64::engine::general_purpose::STANDARD.encode(key_xdr);

    let client = reqwest::Client::new();
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getLedgerEntries",
        "params": {
            "keys": [key_b64]
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

    let result: JsonRpcResponse<GetLedgerEntriesResult> =
        serde_json::from_str(&body).map_err(|e| {
            AuditError::Validation(format!(
                "failed to parse RPC response: {e}\nResponse body: {}",
                &body[..body.len().min(2000)]
            ))
        })?;

    if let Some(err) = result.error {
        return Err(AuditError::Validation(format!(
            "getLedgerEntries error: code {} message {}",
            err.code, err.message
        )));
    }

    let ledger_result = result
        .result
        .ok_or_else(|| AuditError::Validation("getLedgerEntries returned no result".to_string()))?;
    let entry = ledger_result.entries.into_iter().next().ok_or_else(|| {
        AuditError::Validation("signing account not found on Soroban RPC ledger".to_string())
    })?;
    let entry_xdr = base64::engine::general_purpose::STANDARD
        .decode(&entry.xdr)
        .map_err(|e| AuditError::Validation(format!("base64 decode account ledger entry: {e}")))?;
    let ledger_entry = LedgerEntryData::from_xdr(&entry_xdr, Limits::none())
        .map_err(|e| AuditError::Validation(format!("decode account ledger entry: {e}")))?;

    match ledger_entry {
        LedgerEntryData::Account(account_entry) => Ok(account_entry.seq_num.0),
        _ => Err(AuditError::Validation(
            "RPC account ledger entry returned non-account data".to_string(),
        )),
    }
}

#[derive(Deserialize)]
struct GetLedgerEntriesResult {
    entries: Vec<GetLedgerEntry>,
}

#[derive(Deserialize)]
struct GetLedgerEntry {
    xdr: String,
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
    // Convert contract_id to ScAddress::Contract.
    // Accepts both C... strkey and hex formats.
    let contract_arr = decode_contract_id(contract_id)?;
    let contract_addr = ScAddress::Contract(ContractId(Hash(contract_arr)));

    // Build function name as ScSymbol.
    let symbol = ScSymbol(function_name.as_bytes().to_vec().try_into().map_err(
        |e: stellar_xdr::curr::Error| AuditError::Validation(format!("invalid function name: {e}")),
    )?);

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

    build_host_function_transaction(
        source_public_key,
        sequence,
        HostFunction::InvokeContract(invoke_args),
        fee,
    )
}

fn build_host_function_transaction(
    source_public_key: &[u8; 32],
    sequence: i64,
    host_function: HostFunction,
    fee: u32,
) -> AuditResult<Transaction> {
    // Convert public key to MuxedAccount.
    let muxed = MuxedAccount::Ed25519(Uint256(*source_public_key));

    // Build operations VecM.
    let op = Operation {
        source_account: None,
        body: OperationBody::InvokeHostFunction(InvokeHostFunctionOp {
            host_function,
            auth: VecM::default(),
        }),
    };
    let operations: VecM<Operation, 100> =
        vec![op].try_into().map_err(|e: stellar_xdr::curr::Error| {
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

pub const CONTRACT_FUNCTION_NOT_FOUND_MESSAGE: &str = "Contract function not found on-chain. The deployed contract may be outdated, redeploy the Soroban contract and update the contract ID in settings.";

pub fn is_contract_function_not_found_error(error: &AuditError) -> bool {
    let msg = error.to_string();
    msg.contains(CONTRACT_FUNCTION_NOT_FOUND_MESSAGE)
        || (msg.contains("MissingValue") && msg.contains("WasmVm"))
}

/// Simulate a transaction via the Soroban RPC `simulateTransaction` method.
async fn simulate_transaction(rpc_url: &str, tx: &Transaction) -> AuditResult<SimulationResult> {
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

    let result: JsonRpcResponse<SimulationResult> = serde_json::from_str(&body).map_err(|e| {
        AuditError::Validation(format!(
            "failed to parse RPC response: {e}\nResponse body: {}",
            &body[..body.len().min(2000)]
        ))
    })?;

    if let Some(err) = result.error {
        return Err(AuditError::Validation(format!(
            "simulateTransaction error: code {} message {}",
            err.code, err.message
        )));
    }

    let mut sim = result.result.ok_or_else(|| {
        AuditError::Validation("simulateTransaction returned no result".to_string())
    })?;

    // Check for simulation-level error (the RPC returns this inside `result`,
    // not as a top-level JSON-RPC error).
    if let Some(sim_err) = &sim.error {
        // Classify common Soroban host errors into actionable messages.
        let msg = if sim_err.contains("MissingValue") && sim_err.contains("WasmVm") {
            // The contract WASM on-chain doesn't export the invoked function.
            // Most likely the deployed contract is an older build that predates
            // this function being added.  The contract needs to be redeployed.
            CONTRACT_FUNCTION_NOT_FOUND_MESSAGE.to_string()
        } else if sim_err.contains("InvalidAction") || sim_err.contains("not authorized") {
            format!("Contract call not authorized: {sim_err}")
        } else {
            // Surface the raw Soroban host error — it is not sensitive and is
            // far more actionable than a generic message when setup or a commit
            // fails (e.g. propagation timing, missing contract instance).
            format!("Contract invocation failed: {sim_err}")
        };
        return Err(AuditError::Validation(msg));
    }

    if sim.transaction_data.is_none() {
        return Err(AuditError::Validation(
            "Contract call returned no data. The contract may not be deployed at the \
             configured contract ID, or the RPC node may be unreachable."
                .to_string(),
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
    #[serde(
        rename = "minResourceFee",
        default,
        deserialize_with = "deserialize_optional_sequence_string"
    )]
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

    /// Compute the max fee (stroops) the source account must authorize.
    /// This is the simulation's `minResourceFee` plus the network base fee.
    fn max_fee_stroops(&self) -> u32 {
        const BASE_FEE: i64 = 100;
        let required = self.min_resource_fee.unwrap_or(0) + BASE_FEE;
        // Cap at u32::MAX and leave a small buffer to avoid spurious failures.
        required
            .saturating_mul(110)
            .saturating_div(100)
            .clamp(BASE_FEE, u32::MAX as i64) as u32
    }
}

#[derive(Deserialize)]
struct SimulationResultEntry {
    auth: Vec<String>,
    xdr: String,
}

// ─── Transaction submission ───────────────────────────────────────────

/// Poll the Soroban RPC `getTransaction` method until the transaction is applied,
/// rejected, or times out.
async fn wait_for_transaction(rpc_url: &str, tx_hash: &str) -> AuditResult<GetTransactionResult> {
    let client = reqwest::Client::new();
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getTransaction",
        "params": {
            "hash": tx_hash
        }
    });

    let mut attempt = 0;
    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(if attempt == 0 { 2 } else { 3 })).await;
        attempt += 1;

        let resp = client
            .post(rpc_url)
            .json(&request)
            .send()
            .await
            .map_err(|e| AuditError::Validation(format!("RPC getTransaction failed: {e}")))?;

        if !resp.status().is_success() {
            continue;
        }
        let body = resp.text().await.unwrap_or_default();
        let result: Result<JsonRpcResponse<GetTransactionResult>, _> = serde_json::from_str(&body);
        if let Ok(res) = result {
            if let Some(tx_res) = res.result {
                match tx_res.status.as_str() {
                    "SUCCESS" | "FAILED" => return Ok(tx_res),
                    "NOT_FOUND" => {}
                    _ => {} // e.g. PENDING
                }
            }
        }
        if attempt > 15 {
            return Err(AuditError::Validation(format!(
                "transaction {tx_hash} timed out waiting for confirmation"
            )));
        }
    }
}

#[derive(Deserialize, Debug)]
struct GetTransactionResult {
    status: String,
    #[serde(rename = "resultXdr")]
    result_xdr: Option<String>,
}

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

    let result: JsonRpcResponse<SendResult> = serde_json::from_str(&body).map_err(|e| {
        AuditError::Validation(format!(
            "failed to parse RPC response: {e}\nResponse body: {}",
            &body[..body.len().min(2000)]
        ))
    })?;

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
        let xdr = send_result.error_result_xdr.unwrap_or_default();
        let code = decode_tx_result_code(&xdr);
        let name = code.map(tx_result_code_name).unwrap_or("unknown");
        let code_str = code.map(|c| c.to_string()).unwrap_or_else(|| "?".to_string());
        return Err(AuditError::Validation(format!(
            "transaction rejected: {name} ({code_str}): {xdr}"
        )));
    }

    Ok(send_result.hash)
}

/// Decode the `TransactionResult` XDR returned by `sendTransaction` on rejection
/// and extract the result code. The XDR layout is `feeCharged` (i64, 8 bytes)
/// followed by the `TransactionResultResult` union discriminant (i32, 4 bytes),
/// which is the `TransactionResultCode`.
fn decode_tx_result_code(error_result_xdr_b64: &str) -> Option<i32> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(error_result_xdr_b64)
        .ok()?;
    if bytes.len() < 12 {
        return None;
    }
    Some(i32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]))
}

/// Map a Stellar `TransactionResultCode` to its name for human-readable errors.
fn tx_result_code_name(code: i32) -> &'static str {
    match code {
        0 => "txSUCCESS",
        -1 => "txFAILED",
        -2 => "txTOO_EARLY",
        -3 => "txTOO_LATE",
        -4 => "txMISSING_OPERATION",
        -5 => "txBAD_SEQ",
        -6 => "txBAD_AUTH",
        -7 => "txINSUFFICIENT_BALANCE",
        -8 => "txNO_ACCOUNT",
        -9 => "txINSUFFICIENT_FEE",
        -10 => "txBAD_AUTH_EXTRA",
        -11 => "txINTERNAL_ERROR",
        -12 => "txNOT_SUPPORTED",
        -13 => "txFEE_BUMP_INNER_FAILED",
        -14 => "txBAD_SPONSORSHIP",
        -15 => "txBAD_MIN_SEQ_AGE_OR_GAP",
        -16 => "txMALFORMED",
        -17 => "txSOROBAN_INVALID",
        _ => "unknown",
    }
}

/// Decode a failed `TransactionResult` XDR into a human-readable summary,
/// including the inner `InvokeHostFunction` result code when present.
///
/// The on-chain `TransactionResult` only carries the operation result code
/// (e.g. `TRAPPED`); the specific contract error lives in diagnostic events,
/// which the Soroban RPC does not return through `getTransaction`. So we name
/// the codes and add an actionable hint for the common trap cause.
fn describe_failed_result_xdr(b64: &str) -> String {
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) else {
        return format!("undecodable result XDR ({b64})");
    };
    let tx_code = decode_tx_result_code(b64);
    let tx_name = tx_code.map(tx_result_code_name).unwrap_or("unknown");

    // For txFAILED, surface the first operation's InvokeHostFunction code.
    // Layout after feeCharged(8) + txCode(4) + results-len(4):
    //   opResultCode(4) opType(4) invokeHostFunctionResultCode(4)
    if tx_code == Some(-1) && bytes.len() >= 28 {
        let op_type = i32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        if op_type == 24 {
            let ihf = i32::from_be_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]);
            let (name, hint): (&str, &str) = match ihf {
                0 => ("INVOKE_HOST_FUNCTION_SUCCESS", ""),
                -1 => ("INVOKE_HOST_FUNCTION_MALFORMED", ""),
                -2 => (
                    "INVOKE_HOST_FUNCTION_TRAPPED",
                    " — the contract rejected the call; the most common cause is a failed admin.require_auth() (the signing key is not the contract admin), or the contract returned an error",
                ),
                -3 => ("INVOKE_HOST_FUNCTION_RESOURCE_LIMIT_EXCEEDED", ""),
                -4 => ("INVOKE_HOST_FUNCTION_ENTRY_ARCHIVED", ""),
                -5 => ("INVOKE_HOST_FUNCTION_INSUFFICIENT_REFUNDABLE_FEE", ""),
                _ => ("unknown InvokeHostFunction result", ""),
            };
            return format!("{tx_name} / {name}{hint}");
        }
    }

    format!("{tx_name} (raw {b64})")
}

/// True if the error is a `txBAD_SEQ` rejection (stale account sequence).
fn is_bad_seq_error(err: &AuditError) -> bool {
    err.to_string().contains("txBAD_SEQ")
}

/// Maximum number of `txBAD_SEQ` retries before giving up.
const MAX_BAD_SEQ_RETRIES: u32 = 5;

/// Return a process-wide async lock keyed by signing account. Serializing
/// submissions from the same account prevents two in-flight transactions from
/// racing on the account sequence number — the RPC ledger only reflects a new
/// sequence once the prior transaction has been applied, so concurrent or
/// rapid back-to-back submissions otherwise collide with `txBAD_SEQ`.
fn submit_lock_for(public_key: &[u8; 32]) -> Arc<TokioMutex<()>> {
    static LOCKS: OnceLock<StdMutex<HashMap<[u8; 32], Arc<TokioMutex<()>>>>> = OnceLock::new();
    let map = LOCKS.get_or_init(|| StdMutex::new(HashMap::new()));
    let mut guard = map.lock().unwrap_or_else(|e| e.into_inner());
    guard
        .entry(*public_key)
        .or_insert_with(|| Arc::new(TokioMutex::new(())))
        .clone()
}

/// Build → simulate → sign → submit → confirm a contract invocation, serialized
/// per signing account and retried on `txBAD_SEQ` with a freshly fetched
/// sequence. Returns the tx hash and the simulation result so callers can parse
/// the contract return value from `sim_result.results[0].xdr`.
async fn submit_invoke_with_retry(
    rpc_url: &str,
    keypair: &StellarKeypair,
    network_passphrase: &str,
    contract_id: &str,
    function_name: &str,
    args: Vec<ScVal>,
    base_fee: u32,
) -> AuditResult<(String, SimulationResult)> {
    let public_key = keypair.public_bytes();
    let lock = submit_lock_for(&public_key);
    let _guard = lock.lock().await;

    let mut attempt: u32 = 0;
    loop {
        // 1. Fetch the current account sequence from the same RPC node we submit
        //    to (re-fetched on each retry so a stale sequence self-heals).
        let sequence = get_account_sequence_from_rpc(rpc_url, &public_key).await?;

        // 2. Build, simulate, attach Soroban data + auth, and sign.
        let tx = build_invoke_transaction(
            &public_key,
            sequence,
            contract_id,
            function_name,
            args.clone(),
            base_fee,
        )?;
        let sim_result = simulate_transaction(rpc_url, &tx).await?;
        let signed_tx = attach_soroban_data_and_auth(&tx, &sim_result)?;
        let decorated_sig =
            sign_transaction(&signed_tx, &keypair.signing_key, network_passphrase)?;
        let signatures: VecM<DecoratedSignature, 20> = vec![decorated_sig]
            .try_into()
            .map_err(|e: stellar_xdr::curr::Error| {
                AuditError::Validation(format!("too many signatures: {e}"))
            })?;
        let envelope = TransactionEnvelope::Tx(TransactionV1Envelope {
            tx: signed_tx,
            signatures,
        });

        // 3. Submit. On txBAD_SEQ, re-fetch the sequence and retry with backoff.
        match send_transaction(rpc_url, &envelope).await {
            Ok(tx_hash) => {
                // 4. Wait for the ledger to apply the tx so the account sequence
                //    advances before the lock is released and the next submission
                //    runs. A FAILED on-chain result is surfaced to the caller.
                let confirmed = wait_for_transaction(rpc_url, &tx_hash).await?;
                if confirmed.status == "FAILED" {
                    let detail = confirmed
                        .result_xdr
                        .as_deref()
                        .map(describe_failed_result_xdr)
                        .unwrap_or_else(|| "no result XDR returned".to_string());
                    return Err(AuditError::Validation(format!(
                        "transaction {tx_hash} failed on-chain: {detail}"
                    )));
                }
                return Ok((tx_hash, sim_result));
            }
            Err(e) if is_bad_seq_error(&e) && attempt < MAX_BAD_SEQ_RETRIES => {
                attempt += 1;
                tracing::warn!(
                    attempt,
                    function_name,
                    "on-chain submission rejected with txBAD_SEQ; re-fetching account sequence and retrying"
                );
                tokio::time::sleep(tokio::time::Duration::from_millis(
                    750 * attempt as u64,
                ))
                .await;
            }
            Err(e) => return Err(e),
        }
    }
}

async fn submit_host_function_with_retry(
    rpc_url: &str,
    keypair: &StellarKeypair,
    network_passphrase: &str,
    host_function: HostFunction,
    operation_label: &str,
    base_fee: u32,
) -> AuditResult<(String, SimulationResult)> {
    let public_key = keypair.public_bytes();
    let lock = submit_lock_for(&public_key);
    let _guard = lock.lock().await;

    let mut attempt: u32 = 0;
    loop {
        let sequence = get_account_sequence_from_rpc(rpc_url, &public_key).await?;
        let tx = build_host_function_transaction(
            &public_key,
            sequence,
            host_function.clone(),
            base_fee,
        )?;
        let sim_result = simulate_transaction(rpc_url, &tx).await?;
        let signed_tx = attach_soroban_data_and_auth(&tx, &sim_result)?;
        let decorated_sig =
            sign_transaction(&signed_tx, &keypair.signing_key, network_passphrase)?;
        let signatures: VecM<DecoratedSignature, 20> = vec![decorated_sig]
            .try_into()
            .map_err(|e: stellar_xdr::curr::Error| {
                AuditError::Validation(format!("too many signatures: {e}"))
            })?;
        let envelope = TransactionEnvelope::Tx(TransactionV1Envelope {
            tx: signed_tx,
            signatures,
        });

        match send_transaction(rpc_url, &envelope).await {
            Ok(tx_hash) => {
                let confirmed = wait_for_transaction(rpc_url, &tx_hash).await?;
                if confirmed.status == "FAILED" {
                    let detail = confirmed
                        .result_xdr
                        .as_deref()
                        .map(describe_failed_result_xdr)
                        .unwrap_or_else(|| "no result XDR returned".to_string());
                    return Err(AuditError::Validation(format!(
                        "transaction {tx_hash} failed on-chain: {detail}"
                    )));
                }
                return Ok((tx_hash, sim_result));
            }
            Err(e) if is_bad_seq_error(&e) && attempt < MAX_BAD_SEQ_RETRIES => {
                attempt += 1;
                tracing::warn!(
                    attempt,
                    operation_label,
                    "on-chain submission rejected with txBAD_SEQ; re-fetching account sequence and retrying"
                );
                tokio::time::sleep(tokio::time::Duration::from_millis(
                    750 * attempt as u64,
                ))
                .await;
            }
            Err(e) => return Err(e),
        }
    }
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
        signature: Signature(sig_bytes.to_vec().try_into().map_err(
            |e: stellar_xdr::curr::Error| {
                AuditError::Validation(format!("invalid signature length: {e}"))
            },
        )?),
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
/// * `contract_id` - hex contract ID (32 bytes = 64 hex chars)
/// * `network_passphrase` - network passphrase (e.g. `TESTNET_PASSPHRASE`)
pub async fn commit_root_native(
    root_hex: &str,
    metadata: &str,
    keypair: &StellarKeypair,
    rpc_url: &str,
    contract_id: &str,
    network_passphrase: &str,
) -> AuditResult<CommitResult> {
    // Build ScVal args: root (Bytes), metadata (String).
    let root_bytes = hex::decode(root_hex)
        .map_err(|e| AuditError::Validation(format!("invalid root hex: {e}")))?;

    let root_scval = ScVal::Bytes(ScBytes(root_bytes.clone().try_into().map_err(
        |e: stellar_xdr::curr::Error| AuditError::Validation(format!("invalid root bytes: {e}")),
    )?));

    let metadata_scval = ScVal::String(ScString(metadata.as_bytes().to_vec().try_into().map_err(
        |e: stellar_xdr::curr::Error| AuditError::Validation(format!("invalid metadata: {e}")),
    )?));

    let args = vec![root_scval, metadata_scval];

    // Submit (serialized per account, confirmed, and retried on txBAD_SEQ).
    let (tx_hash, sim_result) = submit_invoke_with_retry(
        rpc_url,
        keypair,
        network_passphrase,
        contract_id,
        "commit_root",
        args,
        100, // base fee per operation (stroops)
    )
    .await?;

    // Parse the return value from the simulation to get the on-chain sequence.
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
    contract_id: &str,
    network_passphrase: &str,
) -> AuditResult<CommitResult> {
    let root_bytes = hex::decode(root_hex)
        .map_err(|e| AuditError::Validation(format!("invalid root hex: {e}")))?;
    let root_scval = ScVal::Bytes(ScBytes(root_bytes.clone().try_into().map_err(
        |e: stellar_xdr::curr::Error| AuditError::Validation(format!("invalid root bytes: {e}")),
    )?));

    let oplog_root_bytes = hex::decode(oplog_root_hex)
        .map_err(|e| AuditError::Validation(format!("invalid oplog root hex: {e}")))?;
    let oplog_root_scval = ScVal::Bytes(ScBytes(oplog_root_bytes.try_into().map_err(
        |e: stellar_xdr::curr::Error| {
            AuditError::Validation(format!("invalid oplog root bytes: {e}"))
        },
    )?));

    let metadata_scval = ScVal::String(ScString(metadata.as_bytes().to_vec().try_into().map_err(
        |e: stellar_xdr::curr::Error| AuditError::Validation(format!("invalid metadata: {e}")),
    )?));

    let args = vec![
        root_scval,
        oplog_root_scval,
        ScVal::U64(oplog_start_ts),
        ScVal::U64(oplog_end_ts),
        ScVal::U64(oplog_entry_count),
        metadata_scval,
    ];

    let (tx_hash, sim_result) = submit_invoke_with_retry(
        rpc_url,
        keypair,
        network_passphrase,
        contract_id,
        "commit_root_with_oplog",
        args,
        100,
    )
    .await?;

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
    contract_id: &str,
    network_passphrase: &str,
) -> AuditResult<()> {
    // Build the attester Address ScVal from the keypair's public key.
    let attester_address_scval = ScVal::Address(ScAddress::Account(AccountId(
        PublicKey::PublicKeyTypeEd25519(Uint256(attester_keypair.public_bytes())),
    )));

    let signature_bytes = hex::decode(signature_hex)
        .map_err(|e| AuditError::Validation(format!("invalid signature hex: {e}")))?;
    let signature_scval = ScVal::Bytes(ScBytes(signature_bytes.try_into().map_err(
        |e: stellar_xdr::curr::Error| {
            AuditError::Validation(format!("invalid signature bytes: {e}"))
        },
    )?));

    let args = vec![
        attester_address_scval,
        ScVal::U64(sequence),
        signature_scval,
    ];

    submit_invoke_with_retry(
        rpc_url,
        attester_keypair,
        network_passphrase,
        contract_id,
        "attest_oplog",
        args,
        100,
    )
    .await?;
    Ok(())
}

// ─── High-level: verify_inclusion ─────────────────────────────────────

/// Verify a Groth16 inclusion proof on-chain using the Soroban contract.
///
/// Calls `verify_inclusion(root, proof, vk)` on the contract and returns the
/// transaction hash plus the boolean verification result.
pub async fn verify_inclusion_native(
    root_hex: &str,
    proof_a_hex: &str,
    proof_b_hex: &str,
    proof_c_hex: &str,
    vk_alpha_hex: &str,
    vk_beta_hex: &str,
    vk_gamma_hex: &str,
    vk_delta_hex: &str,
    vk_ic_hex: &[String],
    keypair: &StellarKeypair,
    rpc_url: &str,
    contract_id: &str,
    network_passphrase: &str,
) -> AuditResult<VerifyInclusionResult> {
    // Build ScVal args: root (Bytes), proof (Map), vk (Map).
    let root_bytes = hex::decode(root_hex)
        .map_err(|e| AuditError::Validation(format!("invalid root hex: {e}")))?;
    let root_scval = ScVal::Bytes(ScBytes(root_bytes.clone().try_into().map_err(
        |e: stellar_xdr::curr::Error| AuditError::Validation(format!("invalid root bytes: {e}")),
    )?));

    let proof_scval = build_proof_scval(proof_a_hex, proof_b_hex, proof_c_hex)?;
    let vk_scval = build_verifying_key_scval(
        vk_alpha_hex,
        vk_beta_hex,
        vk_gamma_hex,
        vk_delta_hex,
        vk_ic_hex,
    )?;

    let args = vec![root_scval, proof_scval, vk_scval];

    // Submit (serialized per account, confirmed, and retried on txBAD_SEQ).
    let (tx_hash, sim_result) = submit_invoke_with_retry(
        rpc_url,
        keypair,
        network_passphrase,
        contract_id,
        "verify_inclusion",
        args,
        100,
    )
    .await?;

    // Parse the return value to get the verification result.
    //    Soroban encodes Result<bool, CommitmentError> as:
    //      Ok(v)  -> ScVal::Vec(Some(ScVec([ScVal::Bool(v)])))
    //      Err(e) -> ScVal::Vec(Some(ScVec([ScVal::Error(...)])))
    let mut verified = false;
    if !sim_result.results.is_empty() {
        let return_val_xdr = base64::engine::general_purpose::STANDARD
            .decode(&sim_result.results[0].xdr)
            .map_err(|e| AuditError::Validation(format!("base64 decode return value: {e}")))?;
        let return_val = ScVal::from_xdr(&return_val_xdr, Limits::none())
            .map_err(|e| AuditError::Validation(format!("decode return value: {e}")))?;
        if let ScVal::Vec(Some(vec)) = return_val {
            if let Some(ScVal::Bool(b)) = vec.first() {
                verified = *b;
            }
        }
    }

    Ok(VerifyInclusionResult { tx_hash, verified })
}

/// Build a Soroban symbol ScVal from a string.
fn sc_symbol(s: &str) -> AuditResult<ScVal> {
    Ok(ScVal::Symbol(ScSymbol(
        s.as_bytes()
            .to_vec()
            .try_into()
            .map_err(|e: stellar_xdr::curr::Error| {
                AuditError::Validation(format!("invalid symbol '{s}': {e}"))
            })?,
    )))
}

/// Build a sorted `ScMap` from key-value pairs. Soroban requires map entries
/// to be sorted lexicographically by key.
fn build_sorted_scmap(pairs: Vec<(ScVal, ScVal)>) -> AuditResult<ScVal> {
    let mut entries: Vec<ScMapEntry> = pairs
        .into_iter()
        .map(|(key, val)| ScMapEntry { key, val })
        .collect();

    // Sort by key. ScVal::Symbol comparison is lexicographic on the inner bytes.
    entries.sort_by(|a, b| {
        let a_key = symbol_bytes(&a.key);
        let b_key = symbol_bytes(&b.key);
        a_key.cmp(b_key)
    });

    let map = ScMap(entries.try_into().map_err(|e: stellar_xdr::curr::Error| {
        AuditError::Validation(format!("too many map entries: {e}"))
    })?);

    Ok(ScVal::Map(Some(map)))
}

/// Extract the byte slice from an ScVal::Symbol for sorting.
fn symbol_bytes(val: &ScVal) -> &[u8] {
    match val {
        ScVal::Symbol(s) => s.as_ref(),
        _ => &[],
    }
}

/// Decode hex to ScBytes.
fn hex_to_scbytes(label: &str, hex_str: &str) -> AuditResult<ScBytes> {
    let bytes = hex::decode(hex_str)
        .map_err(|e| AuditError::Validation(format!("invalid {label} hex: {e}")))?;
    bytes
        .try_into()
        .map_err(|e: stellar_xdr::curr::Error| {
            AuditError::Validation(format!("invalid {label} bytes: {e}"))
        })
        .map(ScBytes)
}

/// Build a Soroban `Proof` struct as `ScVal::Map` from hex-encoded point bytes.
fn build_proof_scval(a_hex: &str, b_hex: &str, c_hex: &str) -> AuditResult<ScVal> {
    build_sorted_scmap(vec![
        (
            sc_symbol("a")?,
            ScVal::Bytes(hex_to_scbytes("proof.a", a_hex)?),
        ),
        (
            sc_symbol("b")?,
            ScVal::Bytes(hex_to_scbytes("proof.b", b_hex)?),
        ),
        (
            sc_symbol("c")?,
            ScVal::Bytes(hex_to_scbytes("proof.c", c_hex)?),
        ),
    ])
}

/// Build a Soroban `VerifyingKey` struct as `ScVal::Map` from hex-encoded point bytes.
fn build_verifying_key_scval(
    alpha_hex: &str,
    beta_hex: &str,
    gamma_hex: &str,
    delta_hex: &str,
    ic_hex: &[String],
) -> AuditResult<ScVal> {
    let ic_vals: Vec<ScVal> = ic_hex
        .iter()
        .map(|h| Ok(ScVal::Bytes(hex_to_scbytes("vk.ic", h)?)))
        .collect::<AuditResult<Vec<ScVal>>>()?;

    let ic_vec = ScVec(ic_vals.try_into().map_err(|e: stellar_xdr::curr::Error| {
        AuditError::Validation(format!("too many ic entries: {e}"))
    })?);

    build_sorted_scmap(vec![
        (
            sc_symbol("alpha")?,
            ScVal::Bytes(hex_to_scbytes("vk.alpha", alpha_hex)?),
        ),
        (
            sc_symbol("beta")?,
            ScVal::Bytes(hex_to_scbytes("vk.beta", beta_hex)?),
        ),
        (
            sc_symbol("gamma")?,
            ScVal::Bytes(hex_to_scbytes("vk.gamma", gamma_hex)?),
        ),
        (
            sc_symbol("delta")?,
            ScVal::Bytes(hex_to_scbytes("vk.delta", delta_hex)?),
        ),
        (sc_symbol("ic")?, ScVal::Vec(Some(ic_vec))),
    ])
}

// ─── High-level: get_admin (read-only simulation) ─────────────────────

/// Read the contract admin address via a read-only simulation.
///
/// Returns the admin as a Stellar account strkey (`G...`), or `None` if the
/// contract is uninitialized or the admin can't be decoded. Used as a
/// preflight before owner-only writes (`commit_root*`): those call
/// `admin.require_auth()`, which passes simulation (recording auth does not
/// verify signatures) but traps on-chain (`INVOKE_HOST_FUNCTION_TRAPPED`) when
/// the signing key is not the admin. Checking up front lets callers fail with
/// an actionable message instead of an opaque on-chain trap and a wasted fee.
pub async fn get_admin_native(
    source_keypair: &StellarKeypair,
    rpc_url: &str,
    contract_id: &str,
) -> AuditResult<Option<String>> {
    let tx = build_invoke_transaction(
        &source_keypair.public_bytes(),
        0,
        contract_id,
        "get_admin",
        vec![],
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

    // `get_admin` returns `Result<Address, CommitmentError>`. Soroban encodes
    // `Ok(addr)` as `Vec([Address])`; an uninitialized contract yields an
    // error variant, which we treat as "no admin".
    let inner = match &return_val {
        ScVal::Vec(Some(vec)) if !vec.0.is_empty() => &vec.0[0],
        ScVal::Address(_) => &return_val,
        _ => return Ok(None),
    };

    if let ScVal::Address(ScAddress::Account(AccountId(PublicKey::PublicKeyTypeEd25519(
        Uint256(bytes),
    )))) = inner
    {
        Ok(Some(encode_account_id(bytes)))
    } else {
        Ok(None)
    }
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

    // The contract returns Option<OplogCommitment>.
    //
    // Soroban encodes Option<T> as:
    //   None        → ScVal::Void
    //   Some(T)     → ScVal::Vec(Some(VecM([T])))
    //
    // So Some(OplogCommitment{...}) arrives as Vec([Map(...)]).
    let inner_val = match &return_val {
        ScVal::Void => return Ok(None),
        ScVal::Vec(None) => return Ok(None),
        ScVal::Vec(Some(vec)) if !vec.0.is_empty() => &vec.0[0],
        ScVal::Map(_) => &return_val, // fallback: already a bare Map
        _ => return Ok(None),
    };

    match inner_val {
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

/// Read the current root from the contract via a read-only simulation.
///
/// Calls the contract's `get_current_root` function, which returns
/// `Option<RootEntry>`. This replaces the fragile `getContractData` raw
/// storage read in `StellarRpcClient` — simulation goes through the Soroban
/// runtime, which correctly handles `#[contracttype]` enum key encoding.
///
/// The `source_keypair` only needs a valid ed25519 public key; the account
/// does not need to exist or be funded for a read-only simulation.
pub async fn get_current_root_native(
    source_keypair: &StellarKeypair,
    rpc_url: &str,
    contract_id: &str,
) -> AuditResult<Option<OnChainRoot>> {
    let tx = build_invoke_transaction(
        &source_keypair.public_bytes(),
        0,
        contract_id,
        "get_current_root",
        vec![],
        100,
    )?;

    let sim_result = simulate_transaction(rpc_url, &tx).await?;

    log::debug!(
        "get_current_root_native: sim results count={}, rpc_url={}, contract={}",
        sim_result.results.len(),
        rpc_url,
        contract_id
    );

    if sim_result.results.is_empty() {
        return Ok(None);
    }

    let return_val_xdr = base64::engine::general_purpose::STANDARD
        .decode(&sim_result.results[0].xdr)
        .map_err(|e| AuditError::Validation(format!("base64 decode return value: {e}")))?;
    let return_val = ScVal::from_xdr(&return_val_xdr, Limits::none())
        .map_err(|e| AuditError::Validation(format!("decode return value: {e}")))?;

    // The contract returns Option<RootEntry>.
    //
    // Soroban encodes Option<T> as:
    //   None        → ScVal::Void
    //   Some(T)     → ScVal::Vec(Some(VecM([T])))
    //
    // So Some(RootEntry{...}) arrives as Vec([Map(...)]).
    // We unwrap the Vec, then parse the inner Map.
    let inner_val = match &return_val {
        ScVal::Void => return Ok(None),
        ScVal::Vec(None) => return Ok(None),
        ScVal::Vec(Some(vec)) if !vec.0.is_empty() => &vec.0[0],
        ScVal::Map(_) => &return_val, // fallback: already a bare Map
        _ => return Ok(None),
    };

    // RootEntry is a #[contracttype] struct → ScVal::Map.
    match inner_val {
        ScVal::Map(None) => Ok(None),
        ScVal::Map(Some(map)) => {
            let mut sequence: u64 = 0;
            let mut root_hex = String::new();
            let mut timestamp: u64 = 0;
            let mut metadata = String::new();

            for entry in map.0.iter() {
                if let ScVal::Symbol(s) = &entry.key {
                    let name = String::from_utf8_lossy(s.0.as_slice());
                    match name.as_ref() {
                        "sequence" => {
                            if let ScVal::U64(v) = &entry.val {
                                sequence = *v;
                            }
                        }
                        "root" => {
                            if let ScVal::Bytes(b) = &entry.val {
                                root_hex = hex::encode(b.0.as_slice());
                            }
                        }
                        "timestamp" => {
                            if let ScVal::U64(v) = &entry.val {
                                timestamp = *v;
                            }
                        }
                        "metadata" => {
                            if let ScVal::String(s) = &entry.val {
                                metadata = String::from_utf8_lossy(s.0.as_slice()).to_string();
                            }
                        }
                        _ => {}
                    }
                }
            }

            if root_hex.is_empty() {
                return Ok(None);
            }

            Ok(Some(OnChainRoot {
                sequence,
                root_hex,
                timestamp,
                metadata,
            }))
        }
        _ => Ok(None),
    }
}

/// Query the on-chain root history (most-recent-first) and return up to `limit`
/// entries. Returns an empty Vec if no roots are committed yet.
///
/// Calls `get_root_history(limit)` on the contract via simulation.
pub async fn get_root_history_native(
    source_keypair: &StellarKeypair,
    rpc_url: &str,
    contract_id: &str,
    limit: u32,
) -> AuditResult<Vec<OnChainRoot>> {
    let limit_val = ScVal::U32(limit);
    let tx = build_invoke_transaction(
        &source_keypair.public_bytes(),
        0,
        contract_id,
        "get_root_history",
        vec![limit_val],
        100,
    )?;

    let sim_result = simulate_transaction(rpc_url, &tx).await?;

    if sim_result.results.is_empty() {
        return Ok(vec![]);
    }

    let return_val_xdr = base64::engine::general_purpose::STANDARD
        .decode(&sim_result.results[0].xdr)
        .map_err(|e| AuditError::Validation(format!("base64 decode root_history return: {e}")))?;
    let return_val = ScVal::from_xdr(&return_val_xdr, Limits::none())
        .map_err(|e| AuditError::Validation(format!("decode root_history ScVal: {e}")))?;

    // The simulation unwraps the Ok() and returns the inner Vec directly.
    // Top-level ScVal is Vec([Map, Map, ...]), where each Map is a RootEntry.
    // An error result (e.g. InvalidPageSize) arrives as ScVal::Error → empty list.
    let entries = match &return_val {
        ScVal::Vec(Some(vec)) => &vec.0,
        _ => return Ok(vec![]),
    };

    let mut result = Vec::with_capacity(entries.len());
    for entry in entries.iter() {
        let map = match entry {
            ScVal::Map(Some(m)) => m,
            _ => continue,
        };
        let mut sequence: u64 = 0;
        let mut root_hex = String::new();
        let mut timestamp: u64 = 0;
        let mut metadata = String::new();
        for kv in map.0.iter() {
            if let ScVal::Symbol(s) = &kv.key {
                match String::from_utf8_lossy(s.0.as_slice()).as_ref() {
                    "sequence" => {
                        if let ScVal::U64(v) = &kv.val {
                            sequence = *v;
                        }
                    }
                    "root" => {
                        if let ScVal::Bytes(b) = &kv.val {
                            root_hex = hex::encode(b.0.as_slice());
                        }
                    }
                    "timestamp" => {
                        if let ScVal::U64(v) = &kv.val {
                            timestamp = *v;
                        }
                    }
                    "metadata" => {
                        if let ScVal::String(s) = &kv.val {
                            metadata = String::from_utf8_lossy(s.0.as_slice()).to_string();
                        }
                    }
                    _ => {}
                }
            }
        }
        if !root_hex.is_empty() {
            result.push(OnChainRoot {
                sequence,
                root_hex,
                timestamp,
                metadata,
            });
        }
    }
    Ok(result)
}

/// Query `get_oplog_attestations(sequence)` and return the Stellar addresses
/// of all attesters that have submitted a valid on-chain attestation.
pub async fn get_oplog_attestations_native(
    source_keypair: &StellarKeypair,
    rpc_url: &str,
    contract_id: &str,
    sequence: u64,
) -> AuditResult<Vec<String>> {
    let tx = build_invoke_transaction(
        &source_keypair.public_bytes(),
        0,
        contract_id,
        "get_oplog_attestations",
        vec![ScVal::U64(sequence)],
        100,
    )?;

    let sim_result = simulate_transaction(rpc_url, &tx).await?;
    if sim_result.results.is_empty() {
        return Ok(vec![]);
    }

    let return_val_xdr = base64::engine::general_purpose::STANDARD
        .decode(&sim_result.results[0].xdr)
        .map_err(|e| AuditError::Validation(format!("base64 decode attestations: {e}")))?;
    let return_val = ScVal::from_xdr(&return_val_xdr, Limits::none())
        .map_err(|e| AuditError::Validation(format!("decode attestations ScVal: {e}")))?;

    // get_oplog_attestations returns Vec<OplogAttestation> directly (not wrapped in Result).
    // Each OplogAttestation is a Map with keys: attester (Address), signature (Bytes), timestamp (U64).
    let entries = match &return_val {
        ScVal::Vec(Some(v)) => &v.0,
        _ => return Ok(vec![]),
    };

    let mut attesters = Vec::with_capacity(entries.len());
    for entry in entries.iter() {
        let map = match entry {
            ScVal::Map(Some(m)) => m,
            _ => continue,
        };
        for kv in map.0.iter() {
            if let ScVal::Symbol(s) = &kv.key {
                if String::from_utf8_lossy(s.0.as_slice()) == "attester" {
                    // Address is encoded as ScVal::Address containing an AccountId.
                    // The easiest way is to re-encode it as Strkey.
                    if let ScVal::Address(ScAddress::Account(account_id)) = &kv.val {
                        // AccountId wraps PublicKey::PublicKeyTypeEd25519(Uint256).
                        let stellar_xdr::curr::PublicKey::PublicKeyTypeEd25519(key) =
                            &account_id.0;
                        // Stellar G-address strkey version byte is 0x30 (48).
                        attesters.push(encode_strkey(0x30, &key.0));
                    }
                }
            }
        }
    }
    Ok(attesters)
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
    signed_tx.fee = sim_result.max_fee_stroops();

    // Update the fee to include the simulation's minResourceFee.
    // Soroban transactions require: fee >= base_fee + min_resource_fee.
    // The simulation returns minResourceFee as a string; we already parsed it.
    if let Some(min_fee) = sim_result.min_resource_fee {
        let total_fee = (tx.fee as i64) + min_fee + 1; // +1 for safety margin
        signed_tx.fee = total_fee as u32;
    }

    // Attach auth entries if the simulation returned any.
    if !sim_result.results.is_empty() && !sim_result.results[0].auth.is_empty() {
        let auth_entries: Result<Vec<SorobanAuthorizationEntry>, AuditError> = sim_result.results
            [0]
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
            auth_entries
                .try_into()
                .map_err(|e: stellar_xdr::curr::Error| {
                    AuditError::Validation(format!("too many auth entries: {e}"))
                })?;

        let mut ops = signed_tx.operations.to_vec();
        if let OperationBody::InvokeHostFunction(ref mut invoke_op) = ops[0].body {
            invoke_op.auth = auth_vecm;
        }
        signed_tx.operations = ops.try_into().map_err(|e: stellar_xdr::curr::Error| {
            AuditError::Validation(format!("too many operations: {e}"))
        })?;
    }

    Ok(signed_tx)
}

#[derive(Debug, Clone)]
pub struct DeployedContract {
    pub contract_id: String,
    pub wasm_hash_hex: String,
    pub upload_tx_hash: String,
    pub create_tx_hash: String,
}

/// Deploy the commitment contract using native Soroban host functions.
///
/// The same function works on testnet and mainnet; callers only swap the RPC
/// URL and network passphrase. Testnet account funding is intentionally handled
/// outside this function.
pub async fn deploy_contract_native(
    wasm_bytes: &[u8],
    deployer_keypair: &StellarKeypair,
    rpc_url: &str,
    network_passphrase: &str,
) -> AuditResult<DeployedContract> {
    if wasm_bytes.is_empty() {
        return Err(AuditError::Validation(
            "contract WASM resource is empty".to_string(),
        ));
    }

    let wasm_hash: [u8; 32] = Sha256::digest(wasm_bytes).into();
    let wasm = BytesM::try_from(wasm_bytes.to_vec()).map_err(|e: stellar_xdr::curr::Error| {
        AuditError::Validation(format!("invalid contract WASM bytes: {e}"))
    })?;

    let (upload_tx_hash, _) = submit_host_function_with_retry(
        rpc_url,
        deployer_keypair,
        network_passphrase,
        HostFunction::UploadContractWasm(wasm),
        "upload_contract_wasm",
        100,
    )
    .await?;

    let mut salt = [0u8; 32];
    let mut rng = rand::rngs::OsRng;
    rng.fill_bytes(&mut salt);

    let contract_id_preimage = ContractIdPreimage::Address(ContractIdPreimageFromAddress {
        address: ScAddress::Account(AccountId(PublicKey::PublicKeyTypeEd25519(Uint256(
            deployer_keypair.public_bytes(),
        )))),
        salt: Uint256(salt),
    });
    let contract_id_bytes = derive_contract_id(network_passphrase, &contract_id_preimage)?;

    let create_args = CreateContractArgs {
        contract_id_preimage,
        executable: ContractExecutable::Wasm(Hash(wasm_hash)),
    };

    let (create_tx_hash, _) = submit_host_function_with_retry(
        rpc_url,
        deployer_keypair,
        network_passphrase,
        HostFunction::CreateContract(create_args),
        "create_contract",
        100,
    )
    .await?;

    Ok(DeployedContract {
        contract_id: encode_contract_id(&contract_id_bytes),
        wasm_hash_hex: hex::encode(wasm_hash),
        upload_tx_hash,
        create_tx_hash,
    })
}

fn derive_contract_id(
    network_passphrase: &str,
    contract_id_preimage: &ContractIdPreimage,
) -> AuditResult<[u8; 32]> {
    let network_id = Sha256::digest(network_passphrase.as_bytes());
    let preimage = HashIdPreimage::ContractId(HashIdPreimageContractId {
        network_id: Hash(network_id.into()),
        contract_id_preimage: contract_id_preimage.clone(),
    });
    let xdr = preimage
        .to_xdr(Limits::none())
        .map_err(|e| AuditError::Validation(format!("contract ID preimage XDR failed: {e}")))?;
    Ok(Sha256::digest(&xdr).into())
}

fn encode_contract_id(contract_id: &[u8; 32]) -> String {
    encode_strkey(2 << 3, contract_id)
}

/// Call `initialize(admin)` on the contract via native signing.
///
/// Used by the setup wizard to initialize a newly deployed contract.
/// The `admin_keypair` becomes the contract admin (authorized to commit roots).
pub async fn initialize_contract_native(
    contract_id: &str,
    admin_keypair: &StellarKeypair,
    rpc_url: &str,
    network_passphrase: &str,
) -> AuditResult<()> {
    let admin_scval = ScVal::Address(ScAddress::Account(AccountId(
        PublicKey::PublicKeyTypeEd25519(Uint256(admin_keypair.public_bytes())),
    )));

    let args = vec![admin_scval];

    submit_invoke_with_retry(
        rpc_url,
        admin_keypair,
        network_passphrase,
        contract_id,
        "initialize",
        args,
        100,
    )
    .await?;
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
    network_passphrase: &str,
) -> AuditResult<()> {
    // Decode the attester's G... address to public key bytes.
    let attester_pubkey_bytes = decode_account_id_strkey(attester_address).ok_or_else(|| {
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
    let pubkey_scval = ScVal::Bytes(ScBytes(pubkey_bytes.try_into().map_err(
        |e: stellar_xdr::curr::Error| AuditError::Validation(format!("invalid pubkey bytes: {e}")),
    )?));

    let args = vec![attester_addr_scval, pubkey_scval];

    submit_invoke_with_retry(
        rpc_url,
        admin_keypair,
        network_passphrase,
        contract_id,
        "authorize_attester",
        args,
        100,
    )
    .await?;
    Ok(())
}

/// Set the on-chain K-of-N attestation threshold (admin only).
///
/// The threshold is the minimum number of distinct currently-authorized
/// attesters required before `verify_attestation` returns the `verified`
/// verdict. `admin_keypair` must be the contract admin. Threshold must be >= 1.
pub async fn set_threshold_native(
    contract_id: &str,
    admin_keypair: &StellarKeypair,
    threshold: u32,
    rpc_url: &str,
    network_passphrase: &str,
) -> AuditResult<()> {
    if threshold < 1 {
        return Err(AuditError::Validation(
            "attestation threshold must be at least 1".to_string(),
        ));
    }
    let args = vec![ScVal::U32(threshold)];
    submit_invoke_with_retry(
        rpc_url,
        admin_keypair,
        network_passphrase,
        contract_id,
        "set_threshold",
        args,
        100,
    )
    .await?;
    Ok(())
}

/// Read the on-chain K-of-N attestation threshold via simulation.
///
/// Defaults to 1 for contracts initialized before threshold support was added
/// (the contract's `get_threshold` returns 1 in that case).
pub async fn get_threshold_native(
    source_keypair: &StellarKeypair,
    rpc_url: &str,
    contract_id: &str,
) -> AuditResult<u32> {
    let tx = build_invoke_transaction(
        &source_keypair.public_bytes(),
        0,
        contract_id,
        "get_threshold",
        vec![],
        100,
    )?;

    let sim_result = simulate_transaction(rpc_url, &tx).await?;
    if sim_result.results.is_empty() {
        return Ok(1);
    }

    let return_val_xdr = base64::engine::general_purpose::STANDARD
        .decode(&sim_result.results[0].xdr)
        .map_err(|e| AuditError::Validation(format!("base64 decode return value: {e}")))?;
    let return_val = ScVal::from_xdr(&return_val_xdr, Limits::none())
        .map_err(|e| AuditError::Validation(format!("decode return value: {e}")))?;

    match return_val {
        ScVal::U32(v) => Ok(v),
        _ => Ok(1),
    }
}

/// Revoke a previously-authorized attester (admin only).
///
/// After revocation the attester's on-record attestations no longer count
/// toward the threshold, so `verify_attestation` will no longer treat them as
/// authorized signatures.
pub async fn revoke_attester_native(
    contract_id: &str,
    admin_keypair: &StellarKeypair,
    attester_address: &str,
    rpc_url: &str,
    network_passphrase: &str,
) -> AuditResult<()> {
    let attester_pubkey_bytes = decode_account_id_strkey(attester_address).ok_or_else(|| {
        AuditError::Validation(format!(
            "invalid attester Stellar address (expected G... strkey): {attester_address}"
        ))
    })?;
    let attester_addr_scval = ScVal::Address(ScAddress::Account(AccountId(
        PublicKey::PublicKeyTypeEd25519(Uint256(attester_pubkey_bytes)),
    )));
    submit_invoke_with_retry(
        rpc_url,
        admin_keypair,
        network_passphrase,
        contract_id,
        "revoke_attester",
        vec![attester_addr_scval],
        100,
    )
    .await?;
    Ok(())
}

/// Query the contract's independent attestation verdict for a sequence.
///
/// Calls `verify_attestation(sequence)` via simulation and decodes the full
/// `AttestationVerification` struct. This is the trust anchor the UI should
/// show: `verified` means K distinct authorized attesters each signed the exact
/// committed oplog root.
pub async fn verify_attestation_native(
    source_keypair: &StellarKeypair,
    rpc_url: &str,
    contract_id: &str,
    sequence: u64,
) -> AuditResult<OnChainAttestationVerification> {
    let tx = build_invoke_transaction(
        &source_keypair.public_bytes(),
        0,
        contract_id,
        "verify_attestation",
        vec![ScVal::U64(sequence)],
        100,
    )?;

    let sim_result = simulate_transaction(rpc_url, &tx).await?;
    if sim_result.results.is_empty() {
        return Err(AuditError::Validation(
            "verify_attestation returned no result".to_string(),
        ));
    }

    let return_val_xdr = base64::engine::general_purpose::STANDARD
        .decode(&sim_result.results[0].xdr)
        .map_err(|e| AuditError::Validation(format!("base64 decode return value: {e}")))?;
    let return_val = ScVal::from_xdr(&return_val_xdr, Limits::none())
        .map_err(|e| AuditError::Validation(format!("decode return value: {e}")))?;

    let map = match &return_val {
        ScVal::Map(Some(map)) => map,
        _ => {
            return Err(AuditError::Validation(
                "verify_attestation did not return a struct".to_string(),
            ))
        }
    };

    let mut out = OnChainAttestationVerification {
        sequence,
        oplog_root_hex: String::new(),
        attestation_count: 0,
        authorized_count: 0,
        threshold: 1,
        all_match: false,
        verdict: String::new(),
    };

    for entry in map.0.iter() {
        if let ScVal::Symbol(s) = &entry.key {
            let name = String::from_utf8_lossy(s.0.as_slice());
            match name.as_ref() {
                "sequence" => {
                    if let ScVal::U64(v) = &entry.val {
                        out.sequence = *v;
                    }
                }
                "oplog_root" => {
                    if let ScVal::Bytes(b) = &entry.val {
                        out.oplog_root_hex = hex::encode(b.0.as_slice());
                    }
                }
                "attestation_count" => {
                    if let ScVal::U32(v) = &entry.val {
                        out.attestation_count = *v;
                    }
                }
                "authorized_count" => {
                    if let ScVal::U32(v) = &entry.val {
                        out.authorized_count = *v;
                    }
                }
                "threshold" => {
                    if let ScVal::U32(v) = &entry.val {
                        out.threshold = *v;
                    }
                }
                "all_match" => {
                    if let ScVal::Bool(v) = &entry.val {
                        out.all_match = *v;
                    }
                }
                "verdict" => {
                    if let ScVal::String(s) = &entry.val {
                        out.verdict = String::from_utf8_lossy(s.0.as_slice()).to_string();
                    }
                }
                _ => {}
            }
        }
    }

    Ok(out)
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
        let expected_le = [
            (expected_checksum & 0xff) as u8,
            (expected_checksum >> 8) as u8,
        ];
        if checksum != expected_le {
            return Err(AuditError::Validation(
                "contract ID checksum mismatch".to_string(),
            ));
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
        let index = ALPHABET
            .iter()
            .position(|&c| c == byte.to_ascii_uppercase())?;
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
    fn test_decode_tx_result_code_bad_seq() {
        // The exact TransactionResult XDR a Soroban RPC returns on a stale
        // sequence: feeCharged=0x93D1, result code -5 (txBAD_SEQ).
        let code = decode_tx_result_code("AAAAAAAAk9H////7AAAAAA==");
        assert_eq!(code, Some(-5));
        assert_eq!(tx_result_code_name(code.unwrap()), "txBAD_SEQ");
        assert!(is_bad_seq_error(&AuditError::Validation(
            "transaction rejected: txBAD_SEQ (-5): AAAAAAAAk9H////7AAAAAA==".to_string()
        )));
        assert!(!is_bad_seq_error(&AuditError::Validation(
            "transaction rejected: txINSUFFICIENT_FEE (-9): x".to_string()
        )));
    }

    #[test]
    fn test_decode_tx_result_code_rejects_malformed() {
        assert_eq!(decode_tx_result_code("not-base64!!"), None);
        assert_eq!(decode_tx_result_code("AAAA"), None); // too short
    }

    #[test]
    fn describe_failed_result_xdr_names_invoke_host_function_trap() {
        // The exact TransactionResult XDR from a real commit_root failure:
        // txFAILED with one InvokeHostFunction op that TRAPPED (-2). This is
        // the signature of a failed admin.require_auth() — passes simulation,
        // traps on apply.
        let detail = describe_failed_result_xdr("AAAAAAAAT1b/////AAAAAQAAAAAAAAAY/////gAAAAA=");
        assert!(detail.contains("txFAILED"), "got: {detail}");
        assert!(
            detail.contains("INVOKE_HOST_FUNCTION_TRAPPED"),
            "got: {detail}"
        );
        assert!(detail.contains("require_auth"), "got: {detail}");
    }

    #[test]
    fn describe_failed_result_xdr_falls_back_to_code_name() {
        // txBAD_SEQ has no inner InvokeHostFunction result to expand.
        let detail = describe_failed_result_xdr("AAAAAAAAk9H////7AAAAAA==");
        assert!(detail.contains("txBAD_SEQ"), "got: {detail}");
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
    fn test_contract_function_not_found_error_detection() {
        let classified = AuditError::Validation(CONTRACT_FUNCTION_NOT_FOUND_MESSAGE.to_string());
        assert!(is_contract_function_not_found_error(&classified));

        let raw = AuditError::Validation(
            "HostError: Error(WasmVm, InvalidAction) trapped with MissingValue".to_string(),
        );
        assert!(is_contract_function_not_found_error(&raw));

        let unrelated = AuditError::Validation("RPC endpoint unavailable".to_string());
        assert!(!is_contract_function_not_found_error(&unrelated));
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

    #[test]
    fn test_soroban_option_root_entry_decoding() {
        use stellar_xdr::curr::{ScMap, ScMapEntry, ScVec};

        // Soroban encodes Option<RootEntry> as:
        //   Some(RootEntry{...}) → ScVal::Vec(Some(ScVec([ScVal::Map(ScMap(...))])))
        //
        // Build the inner RootEntry map.
        let root_bytes = vec![0xABu8; 32];

        let map_entries = vec![
            ScMapEntry {
                key: ScVal::Symbol(ScSymbol("sequence".as_bytes().to_vec().try_into().unwrap())),
                val: ScVal::U64(1),
            },
            ScMapEntry {
                key: ScVal::Symbol(ScSymbol("root".as_bytes().to_vec().try_into().unwrap())),
                val: ScVal::Bytes(ScBytes(root_bytes.try_into().unwrap())),
            },
            ScMapEntry {
                key: ScVal::Symbol(ScSymbol(
                    "timestamp".as_bytes().to_vec().try_into().unwrap(),
                )),
                val: ScVal::U64(1700000000),
            },
            ScMapEntry {
                key: ScVal::Symbol(ScSymbol("metadata".as_bytes().to_vec().try_into().unwrap())),
                val: ScVal::String(ScString("test".as_bytes().to_vec().try_into().unwrap())),
            },
        ];

        let root_map_scval = ScVal::Map(Some(ScMap(map_entries.try_into().unwrap())));

        // Wrap in Vec([map]) — this is how Soroban encodes Some(T).
        let option_some = ScVal::Vec(Some(ScVec(vec![root_map_scval].try_into().unwrap())));

        // Verify our decoding logic handles the Vec wrapper.
        let inner_val = match &option_some {
            ScVal::Vec(Some(vec)) if !vec.0.is_empty() => &vec.0[0],
            _ => panic!("expected Vec wrapper"),
        };

        match inner_val {
            ScVal::Map(Some(map)) => {
                let mut found_root = false;
                for entry in map.0.iter() {
                    if let ScVal::Symbol(s) = &entry.key {
                        if String::from_utf8_lossy(s.0.as_slice()) == "root" {
                            if let ScVal::Bytes(b) = &entry.val {
                                assert_eq!(
                                    hex::encode(b.0.as_slice()),
                                    "abababababababababababababababababababababababababababababababab"
                                );
                                found_root = true;
                            }
                        }
                    }
                }
                assert!(found_root, "root field not found in decoded map");
            }
            _ => panic!("expected Map inside Vec"),
        }
    }

    /// Live integration test: calls the real Stellar testnet to verify
    /// get_current_root_native returns a committed root.
    /// Run with: cargo test test_get_current_root_native_live -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn test_get_current_root_native_live() {
        let kp = generate_keypair();
        let result = get_current_root_native(
            &kp,
            "https://soroban-testnet.stellar.org:443",
            "CB6M5T7XYUKCM6YNSSMOGAFIKO53CQSOTDCTJCHOLV6DTQLSYXVQVTXB",
        )
        .await;
        println!("Result: {:?}", result);
        match result {
            Ok(Some(root)) => {
                println!("root_hex={} seq={}", root.root_hex, root.sequence);
                assert!(!root.root_hex.is_empty());
            }
            Ok(None) => panic!("got Ok(None) — simulation returned no root"),
            Err(e) => panic!("got Err: {e}"),
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_root_history_native_live() {
        let kp = generate_keypair();
        let result = get_root_history_native(
            &kp,
            "https://soroban-testnet.stellar.org:443",
            "CB6M5T7XYUKCM6YNSSMOGAFIKO53CQSOTDCTJCHOLV6DTQLSYXVQVTXB",
            5,
        )
        .await;
        println!("Result: {:?}", result);
        match result {
            Ok(entries) => {
                println!("Got {} entries", entries.len());
                assert!(!entries.is_empty(), "expected entries but got empty vec");
                for e in &entries {
                    println!("  seq={} root={}...", e.sequence, &e.root_hex[..16]);
                }
            }
            Err(e) => panic!("got Err: {e}"),
        }
    }

    /// Live end-to-end smoke test for the in-app per-user contract flow.
    ///
    /// Mirrors `audit_provision_testnet_contract` + a Production-mode commit:
    /// fund a key, deploy the bundled WASM, initialize (admin = key), confirm
    /// the on-chain admin equals the deployer, then commit a root signed by the
    /// SAME key. The commit must succeed — a key that is not the admin would
    /// trap on apply (`INVOKE_HOST_FUNCTION_TRAPPED`), which is the exact bug
    /// the mode-aware provisioning fix prevents.
    ///
    /// Run with:
    ///   cargo test -p nosqlbuddy-audit-service deploy_commit_same_key_live \
    ///     -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn deploy_commit_same_key_live() {
        let rpc = "https://soroban-testnet.stellar.org:443";
        let wasm_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../src-tauri/resources/contract/zk_audit_commitment.wasm"
        );
        let wasm = std::fs::read(wasm_path).expect("read bundled contract WASM");
        assert!(!wasm.is_empty(), "bundled WASM is empty");

        let kp = generate_keypair();
        println!("deployer account: {}", kp.account_id());
        fund_account(&kp.account_id())
            .await
            .expect("friendbot fund deployer");

        let deployed = deploy_contract_native(&wasm, &kp, rpc, TESTNET_PASSPHRASE)
            .await
            .expect("deploy contract");
        println!(
            "deployed contract={} wasm_hash={} upload_tx={} create_tx={}",
            deployed.contract_id,
            deployed.wasm_hash_hex,
            deployed.upload_tx_hash,
            deployed.create_tx_hash
        );

        initialize_contract_native(&deployed.contract_id, &kp, rpc, TESTNET_PASSPHRASE)
            .await
            .expect("initialize contract");

        let admin = get_admin_native(&kp, rpc, &deployed.contract_id)
            .await
            .expect("read admin")
            .expect("admin should be set after initialize");
        println!("on-chain admin: {admin}");
        assert_eq!(
            admin,
            kp.account_id(),
            "deployer key must be the contract admin so its commits are authorized"
        );

        // 32-byte root, signed by the same key. Before the fix the commit key
        // differed from the admin and this trapped on apply.
        let mut root = [0u8; 32];
        let mut rng = rand::rngs::OsRng;
        rng.fill_bytes(&mut root);
        let root_hex = hex::encode(root);

        let result = commit_root_native(
            &root_hex,
            "smoke-test deploy_commit_same_key_live",
            &kp,
            rpc,
            &deployed.contract_id,
            TESTNET_PASSPHRASE,
        )
        .await
        .expect("commit must succeed when signed by the admin key (no trap)");
        println!(
            "commit ok: seq={} tx={} root={}",
            result.sequence, result.tx_hash, result.root_hex
        );
        assert_eq!(result.root_hex, root_hex);
    }

    /// Live testnet smoke test for the on-chain K-of-N threshold (F3).
    ///
    /// Deploys the freshly built contract, initializes it, confirms the default
    /// threshold is 1, raises it to 2 via `set_threshold` (admin-signed), reads
    /// it back, and verifies that a sub-1 threshold is rejected on-chain.
    ///
    /// Run with:
    ///   cargo test -p nosqlbuddy-audit-service deploy_set_threshold_live \
    ///     -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn deploy_set_threshold_live() {
        let rpc = "https://soroban-testnet.stellar.org:443";
        let wasm_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../src-tauri/resources/contract/zk_audit_commitment.wasm"
        );
        let wasm = std::fs::read(wasm_path).expect("read bundled contract WASM");
        assert!(!wasm.is_empty(), "bundled WASM is empty");

        let kp = generate_keypair();
        println!("deployer account: {}", kp.account_id());
        fund_account(&kp.account_id())
            .await
            .expect("friendbot fund deployer");

        let deployed = deploy_contract_native(&wasm, &kp, rpc, TESTNET_PASSPHRASE)
            .await
            .expect("deploy contract");
        println!(
            "deployed contract={} wasm_hash={}",
            deployed.contract_id, deployed.wasm_hash_hex
        );

        initialize_contract_native(&deployed.contract_id, &kp, rpc, TESTNET_PASSPHRASE)
            .await
            .expect("initialize contract");

        // Default threshold is 1.
        let default_threshold = get_threshold_native(&kp, rpc, &deployed.contract_id)
            .await
            .expect("read default threshold");
        println!("default on-chain threshold: {default_threshold}");
        assert_eq!(default_threshold, 1, "default threshold must be 1");

        // Raise to 2 (admin-signed).
        set_threshold_native(&deployed.contract_id, &kp, 2, rpc, TESTNET_PASSPHRASE)
            .await
            .expect("set threshold to 2");

        let raised = get_threshold_native(&kp, rpc, &deployed.contract_id)
            .await
            .expect("read raised threshold");
        println!("raised on-chain threshold: {raised}");
        assert_eq!(raised, 2, "threshold must be 2 after set_threshold(2)");

        // A sub-1 threshold is rejected client-side before submission.
        let err = set_threshold_native(&deployed.contract_id, &kp, 0, rpc, TESTNET_PASSPHRASE)
            .await;
        assert!(err.is_err(), "threshold of 0 must be rejected");
    }
}
