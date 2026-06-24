//! Native Rust Stellar RPC client for Soroban contract interaction.
//!
//! This module replaces the `stellar` CLI subprocess for read operations
//! (querying committed roots) with native HTTP calls to the Soroban
//! JSON-RPC API. Write operations (committing roots) still use the CLI
//! for now, as they require transaction building and signing.
//!
//! ## Architecture
//!
//! The client calls the Soroban RPC's `getContractData` method to read
//! contract storage directly. The storage keys are XDR-encoded `ScVal`
//! values, constructed manually using the known encoding for the
//! contract's `#[contracttype]` enums:
//!
//! - `InstanceKey::CurrentRoot` → `ScVal::Vec([ScVal::Symbol("CurrentRoot")])`
//! - `PersistentKey::RootEntry(u64)` → `ScVal::Vec([ScVal::Symbol("RootEntry"), ScVal::U64(seq)])`
//!
//! ## XDR encoding
//!
//! The XDR format is defined by the Stellar XDR definitions (v25.0.0,
//! matching `soroban-sdk 25.1.0`). Key discriminant values:
//! - `ScVal::U64` = 5
//! - `ScVal::Bytes` = 13
//! - `ScVal::String` = 14
//! - `ScVal::Symbol` = 15
//! - `ScVal::Vec` = 16
//! - `ScVal::Map` = 17

use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::audit::stellar::{OnChainRoot, CONTRACT_ID};
use crate::error::{AppError, AppResult};

/// The Soroban RPC endpoint for Stellar testnet.
pub const TESTNET_RPC_URL: &str = "https://soroban-testnet.stellar.org:443";

/// ScValType discriminant values (from stellar-xdr 25.0.0).
const SCV_U64: u32 = 5;
const SCV_BYTES: u32 = 13;
const SCV_STRING: u32 = 14;
const SCV_SYMBOL: u32 = 15;
const SCV_VEC: u32 = 16;
const SCV_MAP: u32 = 17;

/// A native Stellar RPC client that calls the Soroban JSON-RPC API.
pub struct StellarRpcClient {
    rpc_url: String,
    contract_id: String,
    http: reqwest::Client,
}

impl StellarRpcClient {
    /// Create a new RPC client for the Stellar testnet.
    pub fn new() -> Self {
        Self::with_url(TESTNET_RPC_URL)
    }

    /// Create a new RPC client with a custom RPC URL.
    pub fn with_url(rpc_url: &str) -> Self {
        Self {
            rpc_url: rpc_url.to_string(),
            contract_id: CONTRACT_ID.to_string(),
            http: reqwest::Client::new(),
        }
    }

    /// Get the latest committed root from the Soroban contract.
    ///
    /// This reads the `CurrentRoot` instance storage key to get the
    /// sequence number, then reads the `RootEntry(sequence)` persistent
    /// storage key to get the full root entry.
    pub async fn get_current_root(&self) -> AppResult<Option<OnChainRoot>> {
        // Step 1: Read the CurrentRoot instance key to get the sequence.
        let key_xdr = encode_instance_key_current_root();
        let response = self
            .get_contract_data(&key_xdr, "instance")
            .await?;

        let sequence = match response {
            Some(data) => decode_scval_u64(&data)?,
            None => return Ok(None),
        };

        if sequence == 0 {
            return Ok(None);
        }

        // Step 2: Read the RootEntry(sequence) persistent key.
        let key_xdr = encode_persistent_key_root_entry(sequence);
        let response = self
            .get_contract_data(&key_xdr, "persistent")
            .await?;

        match response {
            Some(data) => decode_root_entry(&data, sequence),
            None => Ok(None),
        }
    }

    /// Call the Soroban RPC's `getContractData` method.
    async fn get_contract_data(
        &self,
        key_xdr: &str,
        durability: &str,
    ) -> AppResult<Option<Vec<u8>>> {
        let request = JsonRpcRequest {
            jsonrpc: "2.0",
            id: 1,
            method: "getContractData",
            params: JsonRpcParams {
                contract_id: &self.contract_id,
                key: key_xdr,
                durability,
            },
        };

        let resp = self
            .http
            .post(&self.rpc_url)
            .json(&request)
            .send()
            .await
            .map_err(|e| AppError::Validation(format!("RPC request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AppError::Validation(format!(
                "RPC returned error: {} {}",
                status, text
            )));
        }

        let result: JsonRpcResponse = resp
            .json()
            .await
            .map_err(|e| AppError::Validation(format!("failed to parse RPC response: {e}")))?;

        if let Some(err) = result.error {
            // If the key doesn't exist, the RPC returns an error.
            // We treat this as "no data" rather than an error.
            if err.code == -32600 || err.message.contains("not found") {
                return Ok(None);
            }
            return Err(AppError::Validation(format!(
                "RPC error: code {} message {}",
                err.code, err.message
            )));
        }

        match result.result {
            Some(res) if !res.xdr.is_empty() => {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(&res.xdr)
                    .map_err(|e| AppError::Validation(format!("base64 decode: {e}")))?;
                Ok(Some(bytes))
            }
            _ => Ok(None),
        }
    }
}

impl Default for StellarRpcClient {
    fn default() -> Self {
        Self::new()
    }
}

// ─── XDR encoding ─────────────────────────────────────────────────────

/// Encode `InstanceKey::CurrentRoot` as XDR.
///
/// Encoding: `ScVal::Vec([ScVal::Symbol("CurrentRoot")])`
fn encode_instance_key_current_root() -> String {
    let mut buf = Vec::new();
    // ScVal::Vec discriminant
    write_u32(&mut buf, SCV_VEC);
    // Vec length = 1
    write_u32(&mut buf, 1);
    // ScVal::Symbol("CurrentRoot")
    encode_symbol(&mut buf, "CurrentRoot");
    base64_encode(&buf)
}

/// Encode `PersistentKey::RootEntry(sequence)` as XDR.
///
/// Encoding: `ScVal::Vec([ScVal::Symbol("RootEntry"), ScVal::U64(sequence)])`
fn encode_persistent_key_root_entry(sequence: u64) -> String {
    let mut buf = Vec::new();
    // ScVal::Vec discriminant
    write_u32(&mut buf, SCV_VEC);
    // Vec length = 2
    write_u32(&mut buf, 2);
    // ScVal::Symbol("RootEntry")
    encode_symbol(&mut buf, "RootEntry");
    // ScVal::U64(sequence)
    write_u32(&mut buf, SCV_U64);
    write_u64(&mut buf, sequence);
    base64_encode(&buf)
}

/// Encode a `ScVal::Symbol(name)` into the buffer.
fn encode_symbol(buf: &mut Vec<u8>, name: &str) {
    write_u32(buf, SCV_SYMBOL);
    let bytes = name.as_bytes();
    write_u32(buf, bytes.len() as u32);
    buf.extend_from_slice(bytes);
    // Pad to 4-byte boundary
    let padding = (4 - (bytes.len() % 4)) % 4;
    buf.extend(std::iter::repeat(0u8).take(padding));
}

// ─── XDR decoding ─────────────────────────────────────────────────────

/// Decode a `ScVal::U64` from the XDR response.
///
/// The response from `getContractData` is a `LedgerEntryData` XDR.
/// For instance storage, the value is directly the `ScVal`.
/// We search for the `ScVal::U64` pattern in the XDR bytes.
fn decode_scval_u64(data: &[u8]) -> AppResult<u64> {
    // The LedgerEntryData XDR is a union with discriminant.
    // For ContractData (discriminant = 6), the structure is:
    //   - discriminant (4 bytes)
    //   - ContractDataEntry:
    //     - ScAddress (32 bytes for contract)
    //     - ScVal key (variable)
    //     - ScVal val (variable) ← this is what we want
    //     - ContractDataDurability (4 bytes)
    //     - ExtensionPoint (4 bytes)
    //
    // Rather than fully parsing the XDR, we search for the ScVal::U64
    // pattern (discriminant 5 followed by 8 bytes) in the data.
    // The val field is the second ScVal in the structure, so we look
    // for the second occurrence of an ScVal discriminant.

    // Strategy: find all ScVal patterns and return the U64 value.
    // The val field comes after the key field, so we need to skip
    // the key first. But since we know the key encoding, we can
    // search for the key pattern and then read the val after it.

    let key_pattern = encode_instance_key_current_root();
    let key_bytes = base64::engine::general_purpose::STANDARD
        .decode(&key_pattern)
        .map_err(|e| AppError::Validation(format!("base64 decode key: {e}")))?;

    // Find the key in the data, then read the val after it.
    if let Some(pos) = find_subsequence(data, &key_bytes) {
        let val_start = pos + key_bytes.len();
        if val_start + 12 <= data.len() {
            let disc = read_u32(&data[val_start..]);
            if disc == SCV_U64 {
                return Ok(read_u64(&data[val_start + 4..]));
            }
        }
    }

    // Fallback: search for any ScVal::U64 pattern.
    for i in 0..=data.len().saturating_sub(12) {
        if read_u32(&data[i..]) == SCV_U64 {
            return Ok(read_u64(&data[i + 4..]));
        }
    }

    Err(AppError::Validation(
        "could not decode ScVal::U64 from XDR response".to_string(),
    ))
}

/// Decode a `RootEntry` from the XDR response.
///
/// The RootEntry is a `#[contracttype]` struct encoded as `ScVal::Map`:
/// ```text
/// ScVal::Map([
///   (Symbol("sequence"), U64(seq)),
///   (Symbol("root"), Bytes(root_bytes)),
///   (Symbol("timestamp"), U64(timestamp)),
///   (Symbol("metadata"), String(metadata_str)),
/// ])
/// ```
fn decode_root_entry(data: &[u8], expected_sequence: u64) -> AppResult<Option<OnChainRoot>> {
    // Find the ScVal::Map pattern in the data.
    // The map discriminant is SCV_MAP = 17.
    let map_pos = find_scval_map(data);
    let map_start = match map_pos {
        Some(pos) => pos,
        None => return Ok(None),
    };

    // Read the map entries.
    // After the SCV_MAP discriminant (4 bytes), there's the map length (4 bytes),
    // then entries as (ScVal key, ScVal value) pairs.
    let mut offset = map_start + 4; // skip discriminant
    if offset + 4 > data.len() {
        return Ok(None);
    }
    let map_len = read_u32(&data[offset..]) as usize;
    offset += 4;

    let mut root_hex = String::new();
    let mut timestamp: u64 = 0;
    let mut metadata = String::new();

    for _ in 0..map_len {
        // Skip the key (ScVal::Symbol)
        let (key_name, new_offset) = read_scval_symbol(data, offset)?;
        offset = new_offset;

        // Read the value based on the key name
        match key_name.as_str() {
            "sequence" => {
                let (val, new_offset) = read_scval_u64(data, offset)?;
                offset = new_offset;
                let _ = val; // We already know the sequence
            }
            "root" => {
                let (bytes, new_offset) = read_scval_bytes(data, offset)?;
                offset = new_offset;
                root_hex = hex::encode(&bytes);
            }
            "timestamp" => {
                let (val, new_offset) = read_scval_u64(data, offset)?;
                offset = new_offset;
                timestamp = val;
            }
            "metadata" => {
                let (s, new_offset) = read_scval_string(data, offset)?;
                offset = new_offset;
                metadata = s;
            }
            _ => {
                // Unknown field — skip the value.
                offset = skip_scval(data, offset);
            }
        }
    }

    if root_hex.is_empty() {
        return Ok(None);
    }

    Ok(Some(OnChainRoot {
        sequence: expected_sequence,
        root_hex,
        timestamp,
        metadata,
    }))
}

/// Find the position of an `ScVal::Map` discriminant in the data.
fn find_scval_map(data: &[u8]) -> Option<usize> {
    (0..data.len().saturating_sub(4)).find(|&i| read_u32(&data[i..]) == SCV_MAP)
}

/// Read an `ScVal::Symbol` from the data at the given offset.
/// Returns (symbol_string, new_offset).
fn read_scval_symbol(data: &[u8], offset: usize) -> AppResult<(String, usize)> {
    if offset + 4 > data.len() {
        return Err(AppError::Validation("XTR: symbol discriminant out of bounds".to_string()));
    }
    let disc = read_u32(&data[offset..]);
    if disc != SCV_SYMBOL {
        return Err(AppError::Validation(format!(
            "expected ScVal::Symbol (disc {}), got {}",
            SCV_SYMBOL, disc
        )));
    }
    let mut pos = offset + 4;
    if pos + 4 > data.len() {
        return Err(AppError::Validation("XDR: symbol length out of bounds".to_string()));
    }
    let len = read_u32(&data[pos..]) as usize;
    pos += 4;
    if pos + len > data.len() {
        return Err(AppError::Validation("XDR: symbol bytes out of bounds".to_string()));
    }
    let s = String::from_utf8_lossy(&data[pos..pos + len]).to_string();
    pos += len;
    // Skip padding
    let padding = (4 - (len % 4)) % 4;
    pos += padding;
    Ok((s, pos))
}

/// Read an `ScVal::U64` from the data at the given offset.
/// Returns (value, new_offset).
fn read_scval_u64(data: &[u8], offset: usize) -> AppResult<(u64, usize)> {
    if offset + 12 > data.len() {
        return Err(AppError::Validation("XDR: U64 out of bounds".to_string()));
    }
    let disc = read_u32(&data[offset..]);
    if disc != SCV_U64 {
        return Err(AppError::Validation(format!(
            "expected ScVal::U64 (disc {}), got {}",
            SCV_U64, disc
        )));
    }
    let val = read_u64(&data[offset + 4..]);
    Ok((val, offset + 12))
}

/// Read an `ScVal::Bytes` from the data at the given offset.
/// Returns (bytes, new_offset).
fn read_scval_bytes(data: &[u8], offset: usize) -> AppResult<(Vec<u8>, usize)> {
    if offset + 4 > data.len() {
        return Err(AppError::Validation("XDR: Bytes discriminant out of bounds".to_string()));
    }
    let disc = read_u32(&data[offset..]);
    if disc != SCV_BYTES {
        return Err(AppError::Validation(format!(
            "expected ScVal::Bytes (disc {}), got {}",
            SCV_BYTES, disc
        )));
    }
    let mut pos = offset + 4;
    if pos + 4 > data.len() {
        return Err(AppError::Validation("XDR: Bytes length out of bounds".to_string()));
    }
    let len = read_u32(&data[pos..]) as usize;
    pos += 4;
    if pos + len > data.len() {
        return Err(AppError::Validation("XDR: Bytes data out of bounds".to_string()));
    }
    let bytes = data[pos..pos + len].to_vec();
    pos += len;
    let padding = (4 - (len % 4)) % 4;
    pos += padding;
    Ok((bytes, pos))
}

/// Read an `ScVal::String` from the data at the given offset.
/// Returns (string, new_offset).
fn read_scval_string(data: &[u8], offset: usize) -> AppResult<(String, usize)> {
    if offset + 4 > data.len() {
        return Err(AppError::Validation("XDR: String discriminant out of bounds".to_string()));
    }
    let disc = read_u32(&data[offset..]);
    if disc != SCV_STRING {
        return Err(AppError::Validation(format!(
            "expected ScVal::String (disc {}), got {}",
            SCV_STRING, disc
        )));
    }
    let mut pos = offset + 4;
    if pos + 4 > data.len() {
        return Err(AppError::Validation("XDR: String length out of bounds".to_string()));
    }
    let len = read_u32(&data[pos..]) as usize;
    pos += 4;
    if pos + len > data.len() {
        return Err(AppError::Validation("XDR: String data out of bounds".to_string()));
    }
    let s = String::from_utf8_lossy(&data[pos..pos + len]).to_string();
    pos += len;
    let padding = (4 - (len % 4)) % 4;
    pos += padding;
    Ok((s, pos))
}

/// Skip an `ScVal` value at the given offset. Returns the new offset.
/// This is a simplified version that handles common ScVal types.
fn skip_scval(data: &[u8], offset: usize) -> usize {
    if offset + 4 > data.len() {
        return data.len();
    }
    let disc = read_u32(&data[offset..]);
    let mut pos = offset + 4;

    match disc {
        SCV_U64 => pos + 8,
        x if x <= 12 => pos, // void, bool, u32, i32, etc. (fixed-size, already consumed)
        SCV_BYTES | SCV_STRING | SCV_SYMBOL => {
            if pos + 4 > data.len() {
                return data.len();
            }
            let len = read_u32(&data[pos..]) as usize;
            pos += 4 + len;
            let padding = (4 - (len % 4)) % 4;
            pos += padding;
            pos
        }
        SCV_VEC => {
            if pos + 4 > data.len() {
                return data.len();
            }
            let len = read_u32(&data[pos..]) as usize;
            pos += 4;
            for _ in 0..len {
                pos = skip_scval(data, pos);
            }
            pos
        }
        SCV_MAP => {
            if pos + 4 > data.len() {
                return data.len();
            }
            let len = read_u32(&data[pos..]) as usize;
            pos += 4;
            for _ in 0..len {
                pos = skip_scval(data, pos); // key
                pos = skip_scval(data, pos); // value
            }
            pos
        }
        _ => pos,
    }
}

// ─── XDR primitives ───────────────────────────────────────────────────

fn write_u32(buf: &mut Vec<u8>, val: u32) {
    buf.extend_from_slice(&val.to_be_bytes());
}

fn write_u64(buf: &mut Vec<u8>, val: u64) {
    buf.extend_from_slice(&val.to_be_bytes());
}

fn read_u32(data: &[u8]) -> u32 {
    if data.len() < 4 {
        return 0;
    }
    u32::from_be_bytes([data[0], data[1], data[2], data[3]])
}

fn read_u64(data: &[u8]) -> u64 {
    if data.len() < 8 {
        return 0;
    }
    u64::from_be_bytes([
        data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
    ])
}

fn base64_encode(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

// ─── JSON-RPC types ───────────────────────────────────────────────────

#[derive(Serialize)]
struct JsonRpcRequest<'a> {
    jsonrpc: &'a str,
    id: u32,
    method: &'a str,
    params: JsonRpcParams<'a>,
}

#[derive(Serialize)]
struct JsonRpcParams<'a> {
    contract_id: &'a str,
    key: &'a str,
    durability: &'a str,
}

#[derive(Deserialize)]
struct JsonRpcResponse {
    result: Option<JsonRpcResult>,
    error: Option<JsonRpcError>,
}

#[derive(Deserialize)]
struct JsonRpcResult {
    xdr: String,
}

#[derive(Deserialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_instance_key_current_root_produces_valid_xdr() {
        let xdr_b64 = encode_instance_key_current_root();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&xdr_b64)
            .unwrap();

        // Expected: SCV_VEC(16) + len(1) + SCV_SYMBOL(15) + len(11) + "CurrentRoot" + pad
        // 4 + 4 + 4 + 4 + 11 + 1 = 28 bytes
        assert_eq!(bytes.len(), 28);
        assert_eq!(read_u32(&bytes[0..]), SCV_VEC);
        assert_eq!(read_u32(&bytes[4..]), 1); // vec length
        assert_eq!(read_u32(&bytes[8..]), SCV_SYMBOL);
        assert_eq!(read_u32(&bytes[12..]), 11); // symbol length
        assert_eq!(&bytes[16..27], b"CurrentRoot");
        assert_eq!(bytes[27], 0); // padding
    }

    #[test]
    fn encode_persistent_key_root_entry_produces_valid_xdr() {
        let xdr_b64 = encode_persistent_key_root_entry(42);
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&xdr_b64)
            .unwrap();

        // Expected: SCV_VEC(16) + len(2) + SCV_SYMBOL(15) + len(9) + "RootEntry" + pad(3)
        //          + SCV_U64(5) + u64(42)
        // 4 + 4 + 4 + 4 + 9 + 3 + 4 + 8 = 40 bytes
        assert_eq!(bytes.len(), 40);
        assert_eq!(read_u32(&bytes[0..]), SCV_VEC);
        assert_eq!(read_u32(&bytes[4..]), 2); // vec length
        assert_eq!(read_u32(&bytes[8..]), SCV_SYMBOL);
        assert_eq!(read_u32(&bytes[12..]), 9); // symbol length
        assert_eq!(&bytes[16..25], b"RootEntry");
        // padding at 25, 26, 27
        assert_eq!(read_u32(&bytes[28..]), SCV_U64);
        assert_eq!(read_u64(&bytes[32..]), 42);
    }

    #[test]
    fn read_scval_u64_round_trip() {
        let mut buf = Vec::new();
        write_u32(&mut buf, SCV_U64);
        write_u64(&mut buf, 12345);
        let (val, offset) = read_scval_u64(&buf, 0).unwrap();
        assert_eq!(val, 12345);
        assert_eq!(offset, 12);
    }

    #[test]
    fn read_scval_symbol_round_trip() {
        let mut buf = Vec::new();
        encode_symbol(&mut buf, "test");
        let (s, offset) = read_scval_symbol(&buf, 0).unwrap();
        assert_eq!(s, "test");
        // 4 (disc) + 4 (len) + 4 (padded "test") = 12
        assert_eq!(offset, 12);
    }

    #[test]
    fn read_scval_bytes_round_trip() {
        let mut buf = Vec::new();
        write_u32(&mut buf, SCV_BYTES);
        write_u32(&mut buf, 5);
        buf.extend_from_slice(b"hello");
        buf.push(0); // padding to 8 bytes
        buf.push(0);
        buf.push(0);
        let (bytes, offset) = read_scval_bytes(&buf, 0).unwrap();
        assert_eq!(bytes, b"hello");
        assert_eq!(offset, 16); // 4 + 4 + 5 + 3 = 16
    }

    #[test]
    fn read_scval_string_round_trip() {
        let mut buf = Vec::new();
        write_u32(&mut buf, SCV_STRING);
        write_u32(&mut buf, 5);
        buf.extend_from_slice(b"hello");
        buf.push(0); // padding
        buf.push(0);
        buf.push(0);
        let (s, offset) = read_scval_string(&buf, 0).unwrap();
        assert_eq!(s, "hello");
        assert_eq!(offset, 16);
    }

    #[test]
    fn decode_scval_u64_finds_value_in_data() {
        // Construct a minimal XDR blob containing a U64.
        let mut buf = Vec::new();
        write_u32(&mut buf, 0); // some prefix
        write_u32(&mut buf, SCV_U64);
        write_u64(&mut buf, 999);
        let val = decode_scval_u64(&buf).unwrap();
        assert_eq!(val, 999);
    }

    #[test]
    fn decode_root_entry_extracts_fields() {
        // Construct a minimal RootEntry map.
        let mut buf = Vec::new();
        write_u32(&mut buf, SCV_MAP);
        write_u32(&mut buf, 4); // 4 entries

        // sequence → U64(1)
        encode_symbol(&mut buf, "sequence");
        write_u32(&mut buf, SCV_U64);
        write_u64(&mut buf, 1);

        // root → Bytes([0xAB, 0xCD])
        encode_symbol(&mut buf, "root");
        write_u32(&mut buf, SCV_BYTES);
        write_u32(&mut buf, 2);
        buf.extend_from_slice(&[0xAB, 0xCD]);
        buf.push(0); // padding
        buf.push(0);

        // timestamp → U64(1000)
        encode_symbol(&mut buf, "timestamp");
        write_u32(&mut buf, SCV_U64);
        write_u64(&mut buf, 1000);

        // metadata → String("test")
        encode_symbol(&mut buf, "metadata");
        write_u32(&mut buf, SCV_STRING);
        write_u32(&mut buf, 4);
        buf.extend_from_slice(b"test");

        let result = decode_root_entry(&buf, 1).unwrap().unwrap();
        assert_eq!(result.sequence, 1);
        assert_eq!(result.root_hex, "abcd");
        assert_eq!(result.timestamp, 1000);
        assert_eq!(result.metadata, "test");
    }

    #[test]
    fn stellar_rpc_client_default_uses_testnet() {
        let client = StellarRpcClient::new();
        assert_eq!(client.rpc_url, TESTNET_RPC_URL);
        assert_eq!(client.contract_id, CONTRACT_ID);
    }

    #[test]
    fn find_subsequence_finds_needle() {
        let haystack = [0, 1, 2, 3, 4, 5, 6];
        let needle = [3, 4, 5];
        assert_eq!(find_subsequence(&haystack, &needle), Some(3));
        assert_eq!(find_subsequence(&haystack, &[9, 9]), None);
    }

    #[test]
    fn skip_scval_handles_u64() {
        let mut buf = Vec::new();
        write_u32(&mut buf, SCV_U64);
        write_u64(&mut buf, 42);
        assert_eq!(skip_scval(&buf, 0), 12);
    }

    #[test]
    fn skip_scval_handles_symbol() {
        let mut buf = Vec::new();
        encode_symbol(&mut buf, "ab");
        // 4 (disc) + 4 (len) + 2 + 2 (pad) = 12
        assert_eq!(skip_scval(&buf, 0), 12);
    }
}
