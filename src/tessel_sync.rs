//! End-to-end encrypted Tessel Account Sync (`settingsSync.js`, `syncPhrase.js`):
//! - NFKC-normalized passphrase validation (`>= 12` codepoints, `phrase_is_weak` on creation,
//!   unambiguous 29-char `suggest_phrase()` over `"abcdefghjkmnpqrstuvwxyz23456789"`).
//! - RFC 7914 `scrypt` (`log_n = 17` / `N = 131_072`, `r = 8`, `p = 1`, 16-byte salt -> 32-byte key)
//!   with local Windows DPAPI key cache (`sync.dat`).
//! - Gzip compression (`MAX_PLAIN_BYTES = 64 MiB`) + `[u32_be(len)] || compressed || 0x00*`
//!   bucket padding (`[65_536, 262_144, 1_048_576, 1_400_000]`) + `AES-256-GCM`
//!   (`0x01 || nonce(12) || tag(16) || ciphertext` in base64url) with AAD `"tessel-sync\n1\n{user_id}"`.
//! - Explicit `"rust_schema": 1` / `"rust_bundle"` envelope separation so Rust snapshots never
//!   corrupt Electron snapshots, sensitive-field stripping, shrink guard, and `sync-floor.json`
//!   monotonic `seq` anti-rollback.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use ring::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, Tag, UnboundKey};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, Zeroizing};

pub const MIN_PHRASE_CODEPOINTS: usize = 12;
pub const UNAMBIGUOUS_ALPHABET: &[u8; 31] = b"abcdefghjkmnpqrstuvwxyz23456789";
pub const SCRYPT_LOG_N: u8 = 17; // N = 131072
pub const SCRYPT_R: usize = 8;
pub const SCRYPT_P: usize = 1;
pub const MAX_PLAIN_BYTES: usize = 64 * 1024 * 1024;
pub const SIZE_BUCKETS: [usize; 4] = [65_536, 262_144, 1_048_576, 1_400_000];
pub const KEPT_VERSIONS: usize = 5;
pub const WIRE_VERSION: u8 = 0x01;

const COMMON_IN_PHRASES: [&str; 14] = [
    "password",
    "passphrase",
    "motdepasse",
    "tessel",
    "qwerty",
    "azerty",
    "123456",
    "abcdef",
    "admin",
    "welcome",
    "letmein",
    "monkey",
    "dragon",
    "iloveyou",
];

// ---------------------------------------------------------------------------
// 1. NFKC Passphrase Normalization, Strength Check & Generator (`syncPhrase.js`)
// ---------------------------------------------------------------------------

#[must_use]
pub fn normalize_sync_phrase(raw: &str) -> String {
    let nfkc = nfkc_normalize(raw);
    nfkc.trim().to_owned()
}

#[cfg(windows)]
fn nfkc_normalize(input: &str) -> String {
    use windows::Win32::Globalization::{NormalizationKC, NormalizeString};
    if input.is_empty() || input.is_ascii() {
        return input.to_owned();
    }
    let wide: Vec<u16> = input.encode_utf16().collect();
    unsafe {
        let est = NormalizeString(NormalizationKC, &wide, None);
        if est <= 0 {
            return input.to_owned();
        }
        let mut out = vec![0u16; est as usize + 8];
        let written = NormalizeString(NormalizationKC, &wide, Some(&mut out));
        if written <= 0 {
            return input.to_owned();
        }
        String::from_utf16_lossy(&out[..written as usize])
    }
}

#[cfg(not(windows))]
fn nfkc_normalize(input: &str) -> String {
    input.to_owned()
}

/// Generates a 29-char `xxxxx-xxxxx-xxxxx-xxxxx-xxxxx` passphrase over the 31-char
/// unambiguous alphabet using rejection sampling (`< 248`) for zero modulo bias.
pub fn suggest_phrase() -> Result<String, String> {
    let mut chars = Vec::with_capacity(25);
    let mut buf = [0u8; 64];
    while chars.len() < 25 {
        getrandom::fill(&mut buf).map_err(|_| "Random generator unavailable".to_owned())?;
        for &b in &buf {
            if b < 248 {
                let idx = (b as usize) % UNAMBIGUOUS_ALPHABET.len();
                chars.push(UNAMBIGUOUS_ALPHABET[idx] as char);
                if chars.len() == 25 {
                    break;
                }
            }
        }
    }
    let mut out = String::with_capacity(29);
    for (group_idx, group) in chars.chunks(5).enumerate() {
        if group_idx > 0 {
            out.push('-');
        }
        for &ch in group {
            out.push(ch);
        }
    }
    Ok(out)
}

/// Checks passphrase weakness **only** when creating a new sync passphrase.
#[must_use]
pub fn phrase_is_weak(raw_phrase: &str) -> Option<&'static str> {
    let norm = normalize_sync_phrase(raw_phrase);
    if norm.chars().count() < MIN_PHRASE_CODEPOINTS {
        return Some("Passphrase must be at least 12 characters.");
    }
    let unique_chars: HashSet<char> = norm
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect();
    if unique_chars.len() < 5 {
        return Some("Passphrase repeats too few distinct characters.");
    }
    let unleet: String = norm
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-' && *c != '_')
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            '0' => 'o',
            '1' | '!' => 'i',
            '3' => 'e',
            '4' | '@' => 'a',
            '5' | '$' => 's',
            '7' => 't',
            other => other,
        })
        .collect();
    for bad in COMMON_IN_PHRASES {
        if unleet.contains(bad) && unleet.chars().count() < 20 {
            return Some("Passphrase contains a common word or keyboard pattern.");
        }
    }
    if has_long_sequential_run(&norm, 6) {
        return Some("Passphrase contains a predictable character sequence.");
    }
    None
}

fn has_long_sequential_run(s: &str, run_len: usize) -> bool {
    let bytes: Vec<i16> = s
        .bytes()
        .filter(|b| b.is_ascii_alphanumeric())
        .map(i16::from)
        .collect();
    if bytes.len() < run_len {
        return false;
    }
    let mut asc = 1usize;
    let mut desc = 1usize;
    let mut same = 1usize;
    for i in 1..bytes.len() {
        let diff = bytes[i] - bytes[i - 1];
        asc = if diff == 1 { asc + 1 } else { 1 };
        desc = if diff == -1 { desc + 1 } else { 1 };
        same = if diff == 0 { same + 1 } else { 1 };
        if asc >= run_len || desc >= run_len || same >= run_len {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// 2. RFC 7914 `scrypt` + PBKDF2-HMAC-SHA256 Key Derivation
// ---------------------------------------------------------------------------

fn hmac_sha256_local(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut k_block = [0u8; 64];
    if key.len() > 64 {
        let hashed = Sha256::digest(key);
        k_block[..32].copy_from_slice(&hashed);
    } else {
        k_block[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36_u8; 64];
    let mut opad = [0x5c_u8; 64];
    for i in 0..64 {
        ipad[i] ^= k_block[i];
        opad[i] ^= k_block[i];
    }
    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(message);
    let inner_digest = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner_digest);
    let out = outer.finalize();
    k_block.zeroize();
    let mut res = [0u8; 32];
    res.copy_from_slice(&out);
    res
}

pub fn pbkdf2_hmac_sha256(password: &[u8], salt: &[u8], iterations: u32, out: &mut [u8]) {
    let mut block_idx = 1_u32;
    for chunk in out.chunks_mut(32) {
        let mut salt_block = Vec::with_capacity(salt.len() + 4);
        salt_block.extend_from_slice(salt);
        salt_block.extend_from_slice(&block_idx.to_be_bytes());

        let mut u = hmac_sha256_local(password, &salt_block);
        let mut acc = u;
        for _ in 1..iterations {
            u = hmac_sha256_local(password, &u);
            for (a, b) in acc.iter_mut().zip(u.iter()) {
                *a ^= *b;
            }
        }
        let copy_len = chunk.len().min(32);
        chunk[..copy_len].copy_from_slice(&acc[..copy_len]);
        u.zeroize();
        acc.zeroize();
        block_idx = block_idx.saturating_add(1);
    }
}

fn salsa20_8(block: &mut [u32; 16]) {
    let mut x = *block;
    for _ in 0..4 {
        x[4] ^= x[0].wrapping_add(x[12]).rotate_left(7);
        x[8] ^= x[4].wrapping_add(x[0]).rotate_left(9);
        x[12] ^= x[8].wrapping_add(x[4]).rotate_left(13);
        x[0] ^= x[12].wrapping_add(x[8]).rotate_left(18);

        x[9] ^= x[5].wrapping_add(x[1]).rotate_left(7);
        x[13] ^= x[9].wrapping_add(x[5]).rotate_left(9);
        x[1] ^= x[13].wrapping_add(x[9]).rotate_left(13);
        x[5] ^= x[1].wrapping_add(x[13]).rotate_left(18);

        x[14] ^= x[10].wrapping_add(x[6]).rotate_left(7);
        x[2] ^= x[14].wrapping_add(x[10]).rotate_left(9);
        x[6] ^= x[2].wrapping_add(x[14]).rotate_left(13);
        x[10] ^= x[6].wrapping_add(x[2]).rotate_left(18);

        x[3] ^= x[15].wrapping_add(x[11]).rotate_left(7);
        x[7] ^= x[3].wrapping_add(x[15]).rotate_left(9);
        x[11] ^= x[7].wrapping_add(x[3]).rotate_left(13);
        x[15] ^= x[11].wrapping_add(x[7]).rotate_left(18);

        x[1] ^= x[0].wrapping_add(x[3]).rotate_left(7);
        x[2] ^= x[1].wrapping_add(x[0]).rotate_left(9);
        x[3] ^= x[2].wrapping_add(x[1]).rotate_left(13);
        x[0] ^= x[3].wrapping_add(x[2]).rotate_left(18);

        x[6] ^= x[5].wrapping_add(x[4]).rotate_left(7);
        x[7] ^= x[6].wrapping_add(x[5]).rotate_left(9);
        x[4] ^= x[7].wrapping_add(x[6]).rotate_left(13);
        x[5] ^= x[4].wrapping_add(x[7]).rotate_left(18);

        x[11] ^= x[10].wrapping_add(x[9]).rotate_left(7);
        x[8] ^= x[11].wrapping_add(x[10]).rotate_left(9);
        x[9] ^= x[8].wrapping_add(x[11]).rotate_left(13);
        x[10] ^= x[9].wrapping_add(x[8]).rotate_left(18);

        x[12] ^= x[15].wrapping_add(x[14]).rotate_left(7);
        x[13] ^= x[12].wrapping_add(x[15]).rotate_left(9);
        x[14] ^= x[13].wrapping_add(x[12]).rotate_left(13);
        x[15] ^= x[14].wrapping_add(x[13]).rotate_left(18);
    }
    for i in 0..16 {
        block[i] = block[i].wrapping_add(x[i]);
    }
}

fn scrypt_block_mix(b: &[u32], y: &mut [u32], r: usize) {
    let mut x = [0u32; 16];
    let last_offset = (2 * r - 1) * 16;
    x.copy_from_slice(&b[last_offset..last_offset + 16]);
    for i in 0..(2 * r) {
        let bi = &b[i * 16..(i + 1) * 16];
        for j in 0..16 {
            x[j] ^= bi[j];
        }
        salsa20_8(&mut x);
        let dest_idx = if i % 2 == 0 { i / 2 } else { r + i / 2 };
        y[dest_idx * 16..(dest_idx + 1) * 16].copy_from_slice(&x);
    }
}

fn scrypt_romix(block_bytes: &mut [u8], n: usize, r: usize) {
    let words_len = 32 * r;
    let mut x = vec![0u32; words_len];
    let mut y = vec![0u32; words_len];
    let mut v = vec![0u32; n * words_len];

    for (i, chunk) in block_bytes.as_chunks::<4>().0.iter().enumerate() {
        x[i] = u32::from_le_bytes(*chunk);
    }
    for i in 0..n {
        v[i * words_len..(i + 1) * words_len].copy_from_slice(&x);
        scrypt_block_mix(&x, &mut y, r);
        std::mem::swap(&mut x, &mut y);
    }
    let mask = n - 1;
    let tail_offset = (2 * r - 1) * 16;
    for _ in 0..n {
        let j = (x[tail_offset] as usize) & mask;
        let vj = &v[j * words_len..(j + 1) * words_len];
        for k in 0..words_len {
            x[k] ^= vj[k];
        }
        scrypt_block_mix(&x, &mut y, r);
        std::mem::swap(&mut x, &mut y);
    }
    for (i, word) in x.iter().enumerate() {
        block_bytes[i * 4..(i + 1) * 4].copy_from_slice(&word.to_le_bytes());
    }
    x.zeroize();
    y.zeroize();
    v.zeroize();
}

pub fn scrypt_derive_key(
    passphrase: &str,
    salt: &[u8],
    log_n: u8,
    r: usize,
    p: usize,
) -> Result<Zeroizing<[u8; 32]>, String> {
    if !(1..=20).contains(&log_n) || r == 0 || p == 0 {
        return Err("Invalid scrypt parameters".into());
    }
    let norm = Zeroizing::new(normalize_sync_phrase(passphrase));
    if norm.chars().count() < MIN_PHRASE_CODEPOINTS {
        return Err("Passphrase must be at least 12 characters.".into());
    }
    let n = 1_usize << log_n;
    let block_len = 128 * r;
    let mut b = Zeroizing::new(vec![0u8; block_len * p]);
    pbkdf2_hmac_sha256(norm.as_bytes(), salt, 1, &mut b);
    for i in 0..p {
        scrypt_romix(&mut b[i * block_len..(i + 1) * block_len], n, r);
    }
    let mut dk = Zeroizing::new([0u8; 32]);
    pbkdf2_hmac_sha256(norm.as_bytes(), &b, 1, &mut *dk);
    Ok(dk)
}

// ---------------------------------------------------------------------------
// 3. Bucket Padding + AES-256-GCM Envelope (`settingsSync.js`)
// ---------------------------------------------------------------------------

#[must_use]
pub fn build_sync_aad(user_id: &str) -> String {
    format!("tessel-sync\n1\n{user_id}")
}

pub fn pad_compressed_to_bucket(compressed: &[u8]) -> Result<Vec<u8>, String> {
    let needed = 4_usize.saturating_add(compressed.len());
    let bucket = SIZE_BUCKETS
        .iter()
        .copied()
        .find(|&b| b >= needed)
        .ok_or_else(|| "Compressed sync snapshot exceeds 1.4 MB bucket limit.".to_owned())?;
    let mut out = vec![0u8; bucket];
    out[..4].copy_from_slice(&(compressed.len() as u32).to_be_bytes());
    out[4..4 + compressed.len()].copy_from_slice(compressed);
    Ok(out)
}

pub fn unpad_bucket_payload(padded: &[u8]) -> Result<&[u8], String> {
    if padded.len() < 4 || !SIZE_BUCKETS.contains(&padded.len()) {
        return Err("Invalid padded sync bucket length.".into());
    }
    let len = u32::from_be_bytes([padded[0], padded[1], padded[2], padded[3]]) as usize;
    if 4_usize.saturating_add(len) > padded.len() {
        return Err("Corrupted sync bucket length header.".into());
    }
    if padded[4 + len..].iter().any(|&b| b != 0) {
        return Err("Non-zero padding byte detected in sync bucket.".into());
    }
    Ok(&padded[4..4 + len])
}

pub fn encrypt_sync_envelope(
    key: &[u8; 32],
    user_id: &str,
    envelope_json: &Value,
) -> Result<String, String> {
    let plain = serde_json::to_vec(envelope_json).map_err(|_| "Cannot serialize sync envelope")?;
    if plain.len() > MAX_PLAIN_BYTES {
        return Err("Sync payload exceeds 64 MiB plaintext limit.".into());
    }
    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    gz.write_all(&plain)
        .map_err(|_| "Cannot compress sync snapshot")?;
    let compressed = gz.finish().map_err(|_| "Cannot finalize gzip stream")?;
    let padded = pad_compressed_to_bucket(&compressed)?;

    let mut nonce_bytes = [0u8; 12];
    getrandom::fill(&mut nonce_bytes).map_err(|_| "Random generator unavailable")?;
    let unbound = UnboundKey::new(&AES_256_GCM, key).map_err(|_| "Invalid AES-256-GCM key")?;
    let aead_key = LessSafeKey::new(unbound);
    let aad_str = build_sync_aad(user_id);

    let mut in_out = padded;
    let tag = aead_key
        .seal_in_place_separate_tag(
            Nonce::assume_unique_for_key(nonce_bytes),
            Aad::from(aad_str.as_bytes()),
            &mut in_out,
        )
        .map_err(|_| "AES-256-GCM encryption failed")?;

    // Wire format: `0x01 || nonce(12) || tag(16) || ciphertext`
    let mut wire = Vec::with_capacity(1 + 12 + 16 + in_out.len());
    wire.push(WIRE_VERSION);
    wire.extend_from_slice(&nonce_bytes);
    wire.extend_from_slice(tag.as_ref());
    wire.extend_from_slice(&in_out);
    Ok(URL_SAFE_NO_PAD.encode(wire))
}

pub fn decrypt_sync_envelope(
    key: &[u8; 32],
    user_id: &str,
    wire_b64: &str,
) -> Result<Value, String> {
    let wire = URL_SAFE_NO_PAD
        .decode(wire_b64.trim())
        .map_err(|_| "Malformed base64url sync payload")?;
    if wire.len() < 1 + 12 + 16 + 4 || wire[0] != WIRE_VERSION {
        return Err("Unsupported or truncated sync wire version".into());
    }
    let mut nonce_bytes = [0u8; 12];
    nonce_bytes.copy_from_slice(&wire[1..13]);
    let mut tag_bytes = [0u8; 16];
    tag_bytes.copy_from_slice(&wire[13..29]);

    let mut buf = Vec::with_capacity(wire.len() - 29 + 16);
    buf.extend_from_slice(&wire[29..]);
    buf.extend_from_slice(&tag_bytes);

    let unbound = UnboundKey::new(&AES_256_GCM, key).map_err(|_| "Invalid AES-256-GCM key")?;
    let aead_key = LessSafeKey::new(unbound);
    let aad_str = build_sync_aad(user_id);
    let _ = Tag::from(tag_bytes);
    let decrypted = aead_key
        .open_in_place(
            Nonce::assume_unique_for_key(nonce_bytes),
            Aad::from(aad_str.as_bytes()),
            &mut buf,
        )
        .map_err(|_| "Wrong passphrase or tampered sync snapshot")?;

    let compressed = unpad_bucket_payload(decrypted)?;
    let decoder = GzDecoder::new(compressed);
    let mut plain = Vec::new();
    decoder
        .take((MAX_PLAIN_BYTES + 1) as u64)
        .read_to_end(&mut plain)
        .map_err(|_| "Corrupted gzip stream in sync snapshot")?;
    if plain.len() > MAX_PLAIN_BYTES {
        return Err("Decompressed sync snapshot exceeds 64 MiB limit".into());
    }
    serde_json::from_slice(&plain).map_err(|_| "Invalid JSON in decrypted sync snapshot".into())
}

// ---------------------------------------------------------------------------
// 4. Sensitive Field Stripping, Shrink Guard & Monotonic Anti-Rollback Floor
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShrinkWarning {
    pub shrunk_from: usize,
    pub shrunk_to: usize,
}

#[must_use]
pub fn check_shrink_guard(prev_projects: usize, next_projects: usize) -> Option<ShrinkWarning> {
    if prev_projects >= 4 && next_projects * 2 < prev_projects {
        Some(ShrinkWarning {
            shrunk_from: prev_projects,
            shrunk_to: next_projects,
        })
    } else {
        None
    }
}

/// Strips session tokens, conversation IDs, native thread IDs, and local account rosters
/// before building a cloud sync bundle (`rust_schema = 1`).
pub fn sanitize_workspace_json_for_sync(mut raw: Value) -> Value {
    if let Some(obj) = raw.as_object_mut() {
        obj.remove("native_threads");
        obj.remove("chat_threads");
        obj.remove("claude_threads");
        obj.remove("provider_accounts_prefs");
        if let Some(sessions) = obj.get_mut("sessions").and_then(Value::as_array_mut) {
            for sess in sessions {
                if let Some(sobj) = sess.as_object_mut() {
                    sobj.remove("native_host");
                    sobj.remove("codex_seed");
                    sobj.remove("conversation_id");
                }
            }
        }
    }
    raw
}

#[must_use]
pub fn build_rust_sync_envelope(
    seq: u64,
    device_name: &str,
    at_ms: u64,
    sanitized_workspace: Value,
) -> Value {
    json!({
        "seq": seq,
        "device": device_name,
        "at": at_ms,
        "rust_schema": 1,
        "rust_bundle": sanitized_workspace,
    })
}

pub fn extract_rust_bundle_from_envelope(
    envelope: &Value,
    expected_min_seq: u64,
    allow_older_version_restore: bool,
) -> Result<(u64, Value), String> {
    let obj = envelope
        .as_object()
        .ok_or("Sync envelope is not an object")?;
    if obj.get("rust_schema").and_then(Value::as_u64) != Some(1) {
        return Err(
            "Cloud snapshot belongs to another Tessel format (missing rust_schema=1); refusing unsafe overwrite."
                .into(),
        );
    }
    let seq = obj
        .get("seq")
        .and_then(Value::as_u64)
        .ok_or("Sync envelope is missing monotonic seq")?;
    if !allow_older_version_restore && seq < expected_min_seq {
        return Err(format!(
            "Anti-rollback triggered: snapshot seq {seq} is older than local floor {expected_min_seq}."
        ));
    }
    let bundle = obj
        .get("rust_bundle")
        .cloned()
        .ok_or("Sync envelope is missing rust_bundle")?;
    Ok((seq, bundle))
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncFloorFile {
    pub floors: BTreeMap<String, u64>,
}

impl SyncFloorFile {
    #[must_use]
    pub fn load(data_dir: &Path) -> Self {
        let Ok(raw) = fs::read_to_string(data_dir.join("sync-floor.json")) else {
            return Self::default();
        };
        serde_json::from_str(&raw).unwrap_or_default()
    }

    pub fn record_seq(&mut self, data_dir: &Path, user_id: &str, seq: u64) -> Result<(), String> {
        let current = self.floors.get(user_id).copied().unwrap_or(0);
        if seq > current {
            self.floors.insert(user_id.to_owned(), seq);
        }
        fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        fs::write(data_dir.join("sync-floor.json"), text).map_err(|e| e.to_string())
    }
}

// ---------------------------------------------------------------------------
// 5. Windows DPAPI Local Sync Key Cache & `/api/sync` Client
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct CachedSyncSecret {
    user_id: String,
    salt_b64: String,
    key_b64: String,
}

impl Drop for CachedSyncSecret {
    fn drop(&mut self) {
        self.key_b64.zeroize();
    }
}

#[cfg(windows)]
pub fn save_dpapi_sync_key(
    data_dir: &Path,
    user_id: &str,
    salt: &[u8; 16],
    key: &[u8; 32],
) -> Result<(), String> {
    let rec = CachedSyncSecret {
        user_id: user_id.to_owned(),
        salt_b64: URL_SAFE_NO_PAD.encode(salt),
        key_b64: URL_SAFE_NO_PAD.encode(key),
    };
    let plain = Zeroizing::new(serde_json::to_vec(&rec).map_err(|_| "Cannot encode sync key")?);
    let sealed = crate::vault::protect(&plain, true)?;
    fs::create_dir_all(data_dir).map_err(|_| "Cannot create sync cache folder")?;
    fs::write(data_dir.join("sync-key.dat"), &*sealed).map_err(|_| "Cannot write sync key".into())
}

#[cfg(windows)]
pub type CachedSyncKeyPair = ([u8; 16], Zeroizing<[u8; 32]>);

#[cfg(windows)]
pub fn load_dpapi_sync_key(
    data_dir: &Path,
    expected_user_id: &str,
) -> Result<Option<CachedSyncKeyPair>, String> {
    let path = data_dir.join("sync-key.dat");
    if !path.exists() {
        return Ok(None);
    }
    let enc = fs::read(path).map_err(|_| "Cannot read cached sync key")?;
    let plain = crate::vault::protect(&enc, false)?;
    let rec: CachedSyncSecret =
        serde_json::from_slice(&plain).map_err(|_| "Invalid cached sync key")?;
    if rec.user_id != expected_user_id {
        return Ok(None);
    }
    let salt_vec = URL_SAFE_NO_PAD
        .decode(rec.salt_b64.trim())
        .map_err(|_| "Invalid cached salt")?;
    let key_vec = Zeroizing::new(
        URL_SAFE_NO_PAD
            .decode(rec.key_b64.trim())
            .map_err(|_| "Invalid cached key")?,
    );
    if salt_vec.len() != 16 || key_vec.len() != 32 {
        return Err("Corrupted cached sync key lengths".into());
    }
    let mut salt = [0u8; 16];
    salt.copy_from_slice(&salt_vec);
    let mut key = Zeroizing::new([0u8; 32]);
    key.copy_from_slice(&key_vec);
    Ok(Some((salt, key)))
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SyncRemoteMeta {
    pub seq: u64,
    pub updated_at: Option<u64>,
    pub salt: Option<String>,
    pub size: usize,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SyncVersionEntry {
    pub seq: u64,
    pub updated_at: u64,
    pub size: usize,
}

#[cfg(windows)]
pub struct SyncHttpClient {
    client: reqwest::blocking::Client,
    base: String,
}

#[cfg(windows)]
impl SyncHttpClient {
    pub fn production() -> Result<Self, String> {
        Self::with_base("https://api.tesselcode.com")
    }

    pub fn with_base(base: &str) -> Result<Self, String> {
        let client = reqwest::blocking::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(25))
            .build()
            .map_err(|_| "Cannot initialize sync HTTP client".to_owned())?;
        Ok(Self {
            client,
            base: base.trim_end_matches('/').to_owned(),
        })
    }

    pub fn fetch_meta(&self, bearer: &str) -> Result<Option<SyncRemoteMeta>, String> {
        let resp = self
            .client
            .get(format!("{}/api/sync?meta=1", self.base))
            .bearer_auth(bearer)
            .send()
            .map_err(|_| "Sync connection failed".to_owned())?;
        if resp.status().as_u16() == 404 {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(format!(
                "Sync meta failed (HTTP {})",
                resp.status().as_u16()
            ));
        }
        resp.json::<SyncRemoteMeta>()
            .map(Some)
            .map_err(|_| "Invalid sync metadata response".into())
    }

    pub fn fetch_snapshot(&self, bearer: &str) -> Result<Option<(SyncRemoteMeta, String)>, String> {
        #[derive(Deserialize)]
        struct Resp {
            seq: u64,
            updated_at: Option<u64>,
            salt: Option<String>,
            ciphertext: String,
        }
        let resp = self
            .client
            .get(format!("{}/api/sync", self.base))
            .bearer_auth(bearer)
            .send()
            .map_err(|_| "Sync download failed".to_owned())?;
        if resp.status().as_u16() == 404 {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(format!(
                "Sync download failed (HTTP {})",
                resp.status().as_u16()
            ));
        }
        let parsed: Resp = resp
            .json()
            .map_err(|_| "Invalid sync snapshot response".to_owned())?;
        let size = parsed.ciphertext.len();
        Ok(Some((
            SyncRemoteMeta {
                seq: parsed.seq,
                updated_at: parsed.updated_at,
                salt: parsed.salt,
                size,
            },
            parsed.ciphertext,
        )))
    }

    pub fn list_versions(&self, bearer: &str) -> Result<Vec<SyncVersionEntry>, String> {
        #[derive(Deserialize)]
        struct VersionsResp {
            #[serde(default)]
            versions: Vec<SyncVersionEntry>,
        }
        let resp = self
            .client
            .get(format!("{}/api/sync/versions", self.base))
            .bearer_auth(bearer)
            .send()
            .map_err(|_| "Sync version list failed".to_owned())?;
        if !resp.status().is_success() {
            return Err(format!(
                "Sync versions failed (HTTP {})",
                resp.status().as_u16()
            ));
        }
        let mut parsed: VersionsResp = resp
            .json()
            .map_err(|_| "Invalid sync versions response".to_owned())?;
        parsed.versions.truncate(KEPT_VERSIONS);
        Ok(parsed.versions)
    }

    pub fn upload_snapshot(
        &self,
        bearer: &str,
        expected_seq: u64,
        salt_b64: &str,
        ciphertext_b64: &str,
    ) -> Result<u64, String> {
        #[derive(Deserialize)]
        struct PutResp {
            seq: u64,
        }
        let resp = self
            .client
            .put(format!("{}/api/sync", self.base))
            .bearer_auth(bearer)
            .json(&json!({
                "expected_seq": expected_seq,
                "salt": salt_b64,
                "kdf": { "alg": "scrypt", "log_n": SCRYPT_LOG_N, "r": SCRYPT_R, "p": SCRYPT_P },
                "ciphertext": ciphertext_b64,
            }))
            .send()
            .map_err(|_| "Sync upload failed".to_owned())?;
        if !resp.status().is_success() {
            return Err(format!(
                "Sync upload failed (HTTP {})",
                resp.status().as_u16()
            ));
        }
        let out: PutResp = resp
            .json()
            .map_err(|_| "Invalid sync upload response".to_owned())?;
        Ok(out.seq)
    }

    pub fn delete_cloud_sync(&self, bearer: &str) -> Result<(), String> {
        let resp = self
            .client
            .delete(format!("{}/api/sync", self.base))
            .bearer_auth(bearer)
            .send()
            .map_err(|_| "Cloud sync deletion failed".to_owned())?;
        if !resp.status().is_success() {
            return Err(format!(
                "Cloud sync deletion failed (HTTP {})",
                resp.status().as_u16()
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggest_phrase_and_weakness_validator_follow_sync_phrase_rules() {
        let generated = suggest_phrase().unwrap();
        assert_eq!(generated.len(), 29);
        assert_eq!(generated.matches('-').count(), 4);
        assert!(phrase_is_weak(&generated).is_none());

        assert!(phrase_is_weak("short").is_some());
        assert!(phrase_is_weak("p@ssw0rd12345").is_some());
        assert!(phrase_is_weak("aaaaaaaaaaaa").is_some());
        assert!(phrase_is_weak("abcdefghijklm").is_some());
    }

    #[test]
    fn rfc7914_scrypt_vector_and_aes256gcm_bucket_roundtrip() {
        // RFC 7914 Section 12 test vector 2: P="password", S="NaCl", N=1024 (log_n=10), r=8, p=16
        let mut b = vec![0u8; 128 * 8 * 16];
        pbkdf2_hmac_sha256(b"password", b"NaCl", 1, &mut b);
        for i in 0..16 {
            scrypt_romix(&mut b[i * 1024..(i + 1) * 1024], 1024, 8);
        }
        let mut dk = [0u8; 64];
        pbkdf2_hmac_sha256(b"password", &b, 1, &mut dk);
        assert_eq!(&dk[..8], &[0xfd, 0xba, 0xbe, 0x1c, 0x9d, 0x34, 0x72, 0x00]);

        // Bucket + AES-256-GCM + AAD binding + anti-rollback roundtrip:
        let phrase = suggest_phrase().unwrap();
        let salt = [7u8; 16];
        // Use log_n = 12 in fast unit test, verifying production log_n = 17 constant separately:
        assert_eq!(SCRYPT_LOG_N, 17);
        let key = scrypt_derive_key(&phrase, &salt, 12, SCRYPT_R, SCRYPT_P).unwrap();

        let sanitized = sanitize_workspace_json_for_sync(json!({
            "projects": [{"id": 1, "name": "Demo"}],
            "native_threads": {"1": "secret"},
            "sessions": [{"id": 1, "conversation_id": "strip-me", "title": "Shell"}]
        }));
        assert!(sanitized.get("native_threads").is_none());
        assert!(sanitized["sessions"][0].get("conversation_id").is_none());

        let env = build_rust_sync_envelope(5, "Desktop-PC", 1_700_000_000_000, sanitized);
        let wire = encrypt_sync_envelope(&key, "user-42", &env).unwrap();

        // Wrong user_id AAD must fail decryption:
        assert!(decrypt_sync_envelope(&key, "user-99", &wire).is_err());

        let dec = decrypt_sync_envelope(&key, "user-42", &wire).unwrap();
        let (seq, bundle) = extract_rust_bundle_from_envelope(&dec, 5, false).unwrap();
        assert_eq!(seq, 5);
        assert_eq!(bundle["projects"][0]["name"], "Demo");

        // Anti-rollback floor > 5 rejects seq 5 unless allow_older_version_restore:
        assert!(extract_rust_bundle_from_envelope(&dec, 6, false).is_err());
        assert!(extract_rust_bundle_from_envelope(&dec, 6, true).is_ok());
    }
}
