//! Offline verification commands for `dologctl`.
//!
//! Offline audit-chain verification — verify log file signature
//! chains, external anchor JSON files, and WORM directory LSN continuity.
//!
//! # Commands
//!
//! | Command            | Description |
//! |--------------------|-------------|
//! | `verify-log`       | Verify audit log signature chain (sidecar + LSN, ADR-002 A.6) |
//! | `verify-anchor`    | Verify external anchor JSON file signatures and ordering |
//! | `recovery-report`  | Scan *.worm files, report LSN continuity and gaps |

use std::collections::HashMap;
use std::fs;

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};

use crate::output::{self, color, OutputFormat};
use crate::EXIT_VERIFY_FAILED;
use crate::{stderr, stdout};
use dologger_core::record::Record;
use dologger_core::sif::{decode_record_with, DecodeOptions, MAX_FRAME_SIZE};

// ---------------------------------------------------------------------------
// Colour helpers
// ---------------------------------------------------------------------------

fn green() -> &'static str {
    output::when_color(color::GREEN)
}
fn red() -> &'static str {
    output::when_color(color::RED)
}
fn yellow() -> &'static str {
    output::when_color(color::YELLOW)
}
fn cyan() -> &'static str {
    output::when_color(color::CYAN)
}
fn bold() -> &'static str {
    output::when_color(color::BOLD)
}
fn dim() -> &'static str {
    output::when_color(color::DIM)
}
fn bright_green() -> &'static str {
    output::when_color(color::BRIGHT_GREEN)
}
fn bright_cyan() -> &'static str {
    output::when_color(color::BRIGHT_CYAN)
}

// ---------------------------------------------------------------------------
// SIF record parsing
// ---------------------------------------------------------------------------

/// Parsed wire-record fields relevant to audit verification.
#[derive(Debug, Clone)]
struct SifRecord {
    /// Metadata retained for future diagnostics/display; tamper-evidence for
    /// these fields is carried by the A.3 content hash, not the signed digest.
    #[allow(dead_code)]
    id_hi: u64,
    #[allow(dead_code)]
    id_lo: u64,
    lsn: u64,
    #[allow(dead_code)]
    timestamp_hi: u64,
    #[allow(dead_code)]
    timestamp_lo: u64,
    #[allow(dead_code)]
    level: u8,
    #[allow(dead_code)]
    thread_id: u64,
    #[allow(dead_code)]
    process_id: u32,
    #[allow(dead_code)]
    message: String,
    #[allow(dead_code)]
    source_file: String,
    #[allow(dead_code)]
    host_name: String,
    #[allow(dead_code)]
    source_line: u32,
    #[allow(dead_code)]
    source_column: u32,
    /// A.3 canonical-serialization hash carried by the wire record.
    content_hash: [u8; 32],
    /// True when `content_hash` is nonzero — the record participates in the
    /// audit chain and must carry a sidecar signature.
    in_chain: bool,
    /// True when a fresh canonical-serialization hash (A.3) matches the stored
    /// `content_hash`. False means a hashed field was altered after writing.
    content_ok: bool,
    /// Byte offset in the source file where this record begins (length prefix).
    #[allow(dead_code)]
    file_offset: u64,
}

/// Parse a single SIF record from a byte slice.
///
/// Returns `None` if the frame is not a valid canonical wire record.
fn parse_sif(data: &[u8]) -> Option<SifRecord> {
    let rec = decode_record_with(data, DecodeOptions::untrusted()).ok()?;

    let stored_hash = rec.content_hash;
    let in_chain = stored_hash != [0u8; 32];
    let content_ok = in_chain && Record::compute_content_hash_from(&rec) == stored_hash;

    Some(SifRecord {
        id_hi: rec.id_hi(),
        id_lo: rec.id_lo(),
        lsn: rec.lsn,
        timestamp_hi: rec.timestamp / 1_000_000_000,
        timestamp_lo: rec.timestamp % 1_000_000_000,
        level: rec.level as u8,
        thread_id: rec.thread_id as u64,
        process_id: rec.process_id,
        message: rec.message.display_lossy().into_owned(),
        source_file: rec.source_file(),
        host_name: rec.host_name(),
        source_line: rec.source_line(),
        source_column: rec.source_column(),
        content_hash: stored_hash,
        in_chain,
        content_ok,
        file_offset: 0,
    })
}

/// Read SIF records from a framed binary file.
///
/// Each record: `[4B LE length][SIF payload]`
fn read_sif_file(path: &str) -> Result<Vec<SifRecord>, String> {
    let data = fs::read(path).map_err(|e| format!("Cannot read '{path}': {e}"))?;

    let mut records = Vec::new();
    let mut offset: u64 = 0;

    while (offset as usize) + 4 <= data.len() {
        let off = offset as usize;
        let frame_len =
            u32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]) as usize;

        if frame_len == 0 {
            offset += 4;
            continue;
        }

        if frame_len > MAX_FRAME_SIZE {
            return Err(format!(
                "Frame at offset {offset} exceeds the {MAX_FRAME_SIZE}B verification limit"
            ));
        }

        let payload_start = off
            .checked_add(4)
            .ok_or_else(|| format!("Frame offset overflow at {offset}"))?;
        let payload_end = payload_start
            .checked_add(frame_len)
            .ok_or_else(|| format!("Frame length overflow at {offset}"))?;

        if payload_end > data.len() {
            stderr!(
                "Warning: Truncated frame at offset {offset} (len={frame_len}, available={})",
                data.len() - payload_start
            );
            break;
        }

        if let Some(mut rec) = parse_sif(&data[payload_start..payload_end]) {
            rec.file_offset = offset;
            records.push(rec);
        }

        offset = payload_end as u64;
    }

    Ok(records)
}

// ===========================================================================
// Signature verification helpers
// ===========================================================================

/// Build the A.6 signing digest — byte-for-byte identical to
/// `SignatureEngine::build_signing_payload_static` in the core crate, so
/// `verify-log` accepts records signed by the core.
///
/// The signed digest is `SHA-256(lsn || content_hash || prev_hash)` where
/// `prev_hash` derives from the previous signed record's chain state
/// (`SHA-256(prev.content_hash || prev.lsn)`, or the all-zeros genesis
/// derivation for the first record).
fn build_signing_payload(rec: &SifRecord, prev_hash: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(rec.lsn.to_le_bytes());
    hasher.update(rec.content_hash);
    hasher.update(prev_hash);
    hasher.finalize().into()
}

/// Derive the A.6 predecessor hash for a record's chain state (ruling #15):
/// `SHA-256(content_hash || lsn)`.
fn derive_prev_hash(content_hash: &[u8; 32], lsn: u64) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(content_hash);
    hasher.update(lsn.to_le_bytes());
    hasher.finalize().into()
}

/// Genesis chain state used before the first signed record:
/// `prev_content_hash = 0`, `prev_lsn = 0`.
fn genesis_prev_hash() -> [u8; 32] {
    derive_prev_hash(&[0u8; 32], 0)
}

/// Verify a record's Ed25519 signature using the given public key and the
/// derived predecessor hash (A.6).
fn verify_signature(
    rec: &SifRecord,
    prev_hash: &[u8; 32],
    sig: &[u8; 64],
    verifying_key: &VerifyingKey,
) -> bool {
    let sig = Signature::from_bytes(sig);
    let payload = build_signing_payload(rec, prev_hash);
    verifying_key.verify(&payload, &sig).is_ok()
}

/// Verify the chain link between two consecutive records.
///
/// Per ruling #15 the chain relation is LSN monotonicity only: `prev_hash` is
/// a derivation (`SHA-256(prev.content_hash || prev.lsn)`) computed at
/// sign/verify time, never stored, so there is no stored predecessor hash to
/// compare here. The signature covers the derived hash (ADR-002 A.6); a
/// tampered predecessor fails at signature verification.
fn verify_chain_link(prev: &SifRecord, next: &SifRecord) -> Result<(), &'static str> {
    if next.lsn <= prev.lsn {
        return Err("LSN regression");
    }
    Ok(())
}

// ===========================================================================
// Signature sidecar parsing
// ===========================================================================

/// One signature-sidecar entry: the A.3 content hash and Ed25519 signature
/// the pipeline wrote for one LSN.
#[derive(Debug, Clone, Copy)]
struct SidecarEntry {
    content_hash: [u8; 32],
    signature: [u8; 64],
}

/// Parse `<lsn>:<content_hash_hex>:<signature_hex>` lines (decimal LSN,
/// lowercase hex blobs) into an LSN-keyed map.
fn read_sidecar(path: &str) -> Result<HashMap<u64, SidecarEntry>, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("Cannot read '{path}': {e}"))?;
    let mut entries = HashMap::new();
    for (idx, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split(':');
        let lsn = parts
            .next()
            .and_then(|s| s.parse::<u64>().ok())
            .ok_or_else(|| format!("{path}:{}: malformed LSN in sidecar line", idx + 1))?;
        let content_hash: [u8; 32] = parts
            .next()
            .and_then(|s| dologger_core::hex::decode(s).ok())
            .and_then(|b| <[u8; 32]>::try_from(b).ok())
            .ok_or_else(|| format!("{path}:{}: malformed content_hash hex", idx + 1))?;
        let signature: [u8; 64] = parts
            .next()
            .and_then(|s| dologger_core::hex::decode(s).ok())
            .and_then(|b| <[u8; 64]>::try_from(b).ok())
            .ok_or_else(|| format!("{path}:{}: malformed signature hex", idx + 1))?;
        if parts.next().is_some() {
            return Err(format!("{path}:{}: extra fields in sidecar line", idx + 1));
        }
        entries.insert(
            lsn,
            SidecarEntry {
                content_hash,
                signature,
            },
        );
    }
    Ok(entries)
}

// ===========================================================================
// cmd_verify_log — verify audit log file signature chain
// ===========================================================================

/// `dologctl verify-log <path>` — verify audit log file signature chain.
///
/// Parse records, verify Ed25519 signatures, check LSN monotonicity and
/// prev_hash chain. Report total, valid, tampered, gaps.
///
/// If `pubkey_hex` is provided, also verify Ed25519 signatures against it.
pub fn cmd_verify_log(
    path: &str,
    pubkey_hex: Option<&str>,
    sidecar_path: Option<&str>,
    format: OutputFormat,
) {
    if format == OutputFormat::Json {
        cmd_verify_log_json(path, pubkey_hex, sidecar_path);
        return;
    }

    let bg = bright_cyan();
    let b = bold();
    let d = dim();
    let g = green();
    let r = red();
    let y = yellow();
    let c = cyan();
    let reset = output::when_color(color::RESET);

    stdout!("{b}{bg}Log File Verification{reset}");
    stdout!("{d}─────────────────────{reset}");
    stdout!("  File: {path}");

    // Try to parse the public key
    let verifying_key: Option<VerifyingKey> = match pubkey_hex {
        Some(hex_str) => {
            let hex_str = hex_str.trim();
            match dologger_core::hex::decode(hex_str) {
                Ok(bytes) if bytes.len() == 32 => {
                    let arr: [u8; 32] = bytes.try_into().unwrap();
                    match VerifyingKey::from_bytes(&arr) {
                        Ok(vk) => {
                            stdout!("  Pubkey: {hex_str}");
                            stdout!("  Signature verification: {g}ENABLED{reset}");
                            Some(vk)
                        }
                        Err(e) => {
                            stderr!("{y}Warning:{reset} Invalid public key ({e}) — signature checks disabled");
                            None
                        }
                    }
                }
                Ok(bytes) => {
                    stderr!(
                        "{y}Warning:{reset} Public key must be 32 bytes (got {}) — signature checks disabled",
                        bytes.len()
                    );
                    None
                }
                Err(e) => {
                    stderr!("{y}Warning:{reset} Cannot decode public key hex ({e}) — signature checks disabled");
                    None
                }
            }
        }
        None => {
            stdout!("  Signature verification: {d}DISABLED{reset} (no --pubkey)");
            None
        }
    };
    stdout!("");

    // Read the signature sidecar (LSN → hash + signature) if requested.
    let sidecar: Option<HashMap<u64, SidecarEntry>> = match sidecar_path {
        Some(p) => match read_sidecar(p) {
            Ok(map) => {
                stdout!("  Sidecar: {p} ({c}{}{reset} entries)", map.len());
                Some(map)
            }
            Err(e) => {
                stderr!("{r}Error:{reset} {e}");
                std::process::exit(EXIT_VERIFY_FAILED);
            }
        },
        None => None,
    };

    // Read and parse the file
    let records = match read_sif_file(path) {
        Ok(recs) => {
            stdout!("  Records parsed: {c}{}{reset}", recs.len());
            recs
        }
        Err(e) => {
            stderr!("{r}Error:{reset} {e}");
            std::process::exit(EXIT_VERIFY_FAILED);
        }
    };

    if records.is_empty() {
        stdout!("{d}No records found — nothing to verify.{reset}");
        return;
    }

    // Sort by LSN if not already ordered
    let mut sorted: Vec<&SifRecord> = records.iter().collect();
    sorted.sort_by_key(|r| r.lsn);

    let total = sorted.len();
    let mut valid_sig: u32 = 0;
    let mut tampered_sig: u32 = 0;
    let mut valid_chain: u32 = 0;
    let mut broken_chain: u32 = 0;
    let mut lsn_gaps: Vec<(u64, u64)> = Vec::new(); // (missing_from, missing_to)

    // Check chain continuity
    for i in 1..total {
        let prev = sorted[i - 1];
        let next = sorted[i];

        match verify_chain_link(prev, next) {
            Ok(()) => valid_chain += 1,
            Err(e) => {
                broken_chain += 1;
                stderr!(
                    "  {r}CHAIN BROKEN{reset} LSN {} → {}: {e}",
                    prev.lsn,
                    next.lsn
                );
            }
        }

        // Detect LSN gaps
        let expected = prev.lsn.saturating_add(1);
        if next.lsn > expected {
            lsn_gaps.push((expected, next.lsn.saturating_sub(1)));
            stderr!(
                "  {y}LSN GAP{reset} Expected {expected}, found {} (missing {})",
                next.lsn,
                next.lsn - expected
            );
        }
    }

    // Signature verification: signed records carry a sidecar entry keyed by
    // LSN. Each signature covers SHA-256(lsn || content_hash || prev_hash)
    // with prev_hash derived from the previous *signed* record's chain state
    // (A.6) — non-audit records do not advance the chain. Content tampering
    // is caught both by re-hashing (A.3) and by the Ed25519 signature.
    let mut missing_sig: u32 = 0;
    let mut orphan_entries: u32 = 0;
    if let Some(ref map) = sidecar {
        stdout!("");
        stdout!(
            "  Verifying signatures ({c}{}{reset} sidecar entries)...",
            map.len()
        );
        let mut prev_chain: Option<(&[u8; 32], u64)> = None;
        for rec in sorted.iter() {
            if !rec.in_chain {
                continue;
            }
            let prev_hash = match prev_chain {
                Some((ch, lsn)) => derive_prev_hash(ch, lsn),
                None => genesis_prev_hash(),
            };
            // The chain advances through every signed record even if its own
            // sidecar entry was lost — later signatures bind to this record.
            prev_chain = Some((&rec.content_hash, rec.lsn));
            match map.get(&rec.lsn) {
                Some(entry) => {
                    if !rec.content_ok {
                        tampered_sig += 1;
                        stderr!(
                            "  {r}TAMPERED{reset} LSN {} — content_hash no longer matches record content",
                            rec.lsn
                        );
                    } else if entry.content_hash != rec.content_hash {
                        tampered_sig += 1;
                        stderr!(
                            "  {r}TAMPERED{reset} LSN {} — sidecar hash differs from record hash",
                            rec.lsn
                        );
                    } else if let Some(ref vk) = verifying_key {
                        if verify_signature(rec, &prev_hash, &entry.signature, vk) {
                            valid_sig += 1;
                        } else {
                            tampered_sig += 1;
                            stderr!("  {r}TAMPERED{reset} LSN {} — signature invalid", rec.lsn);
                        }
                    } else {
                        valid_sig += 1;
                    }
                }
                None => {
                    missing_sig += 1;
                    stderr!(
                        "  {r}MISSING SIGNATURE{reset} LSN {} — no sidecar entry",
                        rec.lsn
                    );
                }
            }
        }
        // Orphan sidecar entries — LSNs with a signature but no matching
        // record in the log — imply records were deleted after signing.
        for lsn in map.keys() {
            if !sorted.iter().any(|r| r.lsn == *lsn) {
                orphan_entries += 1;
                stderr!("  {r}ORPHAN SIDECAR{reset} LSN {lsn} — no matching record in log");
            }
        }
    }

    // Summary
    stdout!("");
    stdout!("{b}Verification Results{reset}");
    stdout!("{d}────────────────────{reset}");
    stdout!("  Total records:     {total}");

    let chain_total = valid_chain + broken_chain;
    if chain_total > 0 {
        let pct = valid_chain as f64 / chain_total as f64 * 100.0;
        stdout!(
            "  Chain links:       {g}{valid_chain} valid{reset}, {r}{broken_chain} broken{reset} ({pct:.1}% ok)"
        );
    }

    if lsn_gaps.is_empty() {
        stdout!("  LSN continuity:    {g}PASS{reset} — no gaps detected");
    } else {
        stdout!("  LSN gaps:          {r}{}{reset}", lsn_gaps.len());
        for (from, to) in &lsn_gaps {
            stdout!("    Missing LSN {from} – {to} ({} records)", to - from + 1);
        }
    }

    if sidecar.is_some() {
        let sig_total = valid_sig + tampered_sig;
        if sig_total > 0 {
            let pct = valid_sig as f64 / sig_total as f64 * 100.0;
            stdout!(
                "  Signatures:        {g}{valid_sig} valid{reset}, {r}{tampered_sig} tampered{reset} ({pct:.1}% ok)"
            );
        } else {
            stdout!("  Signatures:        {d}no signed records found{reset}");
        }
        if missing_sig > 0 {
            stdout!("  Missing signatures:{r} {missing_sig}{reset}");
        }
        if orphan_entries > 0 {
            stdout!("  Orphan sidecar:    {r} {orphan_entries}{reset}");
        }
    }

    // Exit code
    if broken_chain > 0
        || tampered_sig > 0
        || !lsn_gaps.is_empty()
        || missing_sig > 0
        || orphan_entries > 0
    {
        stdout!("");
        let issue_count =
            broken_chain + tampered_sig + lsn_gaps.len() as u32 + missing_sig + orphan_entries;
        stderr!("{r}{b}VERIFICATION FAILED{reset}{r} — {issue_count} issue(s) detected{reset}");
        std::process::exit(EXIT_VERIFY_FAILED);
    } else {
        stdout!("");
        stdout!(
            "{bright_green}{b}VERIFICATION PASSED{reset}{bright_green} — all checks OK{reset}",
            bright_green = bright_green(),
            b = b,
            reset = reset
        );
    }
}

/// JSON variant of `cmd_verify_log`. Outputs a single JSON object to stdout.
fn cmd_verify_log_json(path: &str, pubkey_hex: Option<&str>, sidecar_path: Option<&str>) {
    // Parse public key (silently)
    let verifying_key: Option<VerifyingKey> = pubkey_hex
        .and_then(|h| dologger_core::hex::decode(h.trim()).ok())
        .and_then(|b| <[u8; 32]>::try_from(b).ok())
        .and_then(|arr| VerifyingKey::from_bytes(&arr).ok());

    let sidecar: Option<HashMap<u64, SidecarEntry>> = match sidecar_path {
        Some(p) => match read_sidecar(p) {
            Ok(m) => Some(m),
            Err(e) => {
                let obj = serde_json::json!({"status": "error", "error_code": EXIT_VERIFY_FAILED, "message": e});
                output::stdout_line(&obj.to_string());
                std::process::exit(EXIT_VERIFY_FAILED);
            }
        },
        None => None,
    };

    let records = match read_sif_file(path) {
        Ok(r) => r,
        Err(e) => {
            let obj = serde_json::json!({"status": "error", "error_code": EXIT_VERIFY_FAILED, "message": e});
            output::stdout_line(&obj.to_string());
            std::process::exit(EXIT_VERIFY_FAILED);
        }
    };

    let mut sorted: Vec<&SifRecord> = records.iter().collect();
    sorted.sort_by_key(|r| r.lsn);

    let total = sorted.len();
    let mut valid_sig: u32 = 0;
    let mut tampered_sig: u32 = 0;
    let mut valid_chain: u32 = 0;
    let mut broken_chain: u32 = 0;
    let mut lsn_gaps_count: u32 = 0;
    let mut missing_sig: u32 = 0;
    let mut orphan_entries: u32 = 0;

    for i in 1..total {
        match verify_chain_link(sorted[i - 1], sorted[i]) {
            Ok(()) => valid_chain += 1,
            Err(_) => broken_chain += 1,
        }
        let expected = sorted[i - 1].lsn.saturating_add(1);
        if sorted[i].lsn > expected {
            lsn_gaps_count += 1;
        }
    }

    if let Some(ref map) = sidecar {
        let mut prev_chain: Option<(&[u8; 32], u64)> = None;
        for rec in sorted.iter().filter(|r| r.in_chain) {
            let prev_hash = match prev_chain {
                Some((ch, lsn)) => derive_prev_hash(ch, lsn),
                None => genesis_prev_hash(),
            };
            prev_chain = Some((&rec.content_hash, rec.lsn));
            match map.get(&rec.lsn) {
                Some(entry) => {
                    if !rec.content_ok || entry.content_hash != rec.content_hash {
                        tampered_sig += 1;
                    } else if let Some(ref vk) = verifying_key {
                        if verify_signature(rec, &prev_hash, &entry.signature, vk) {
                            valid_sig += 1;
                        } else {
                            tampered_sig += 1;
                        }
                    } else {
                        valid_sig += 1;
                    }
                }
                None => missing_sig += 1,
            }
        }
        orphan_entries = map
            .keys()
            .filter(|lsn| !sorted.iter().any(|r| r.lsn == **lsn))
            .count() as u32;
    }

    let passed = broken_chain == 0
        && tampered_sig == 0
        && lsn_gaps_count == 0
        && missing_sig == 0
        && orphan_entries == 0;

    let obj = serde_json::json!({
        "status": if passed { "passed" } else { "failed" },
        "file": path,
        "total_records": total,
        "valid_chain_links": valid_chain,
        "broken_chain_links": broken_chain,
        "lsn_gaps": lsn_gaps_count,
        "signatures": {
            "valid": valid_sig,
            "tampered": tampered_sig,
            "missing": missing_sig,
            "orphan_sidecar_entries": orphan_entries
        }
    });
    output::stdout_line(&obj.to_string());

    if !passed {
        std::process::exit(EXIT_VERIFY_FAILED);
    }
}

// ===========================================================================
// cmd_verify_anchor — verify external anchor JSON file
// ===========================================================================

/// Parsed anchor record from JSON.
#[derive(Debug, serde::Deserialize)]
struct AnchorJson {
    anchor_id: u64,
    timestamp_ms: u64,
    last_lsn: u64,
    chain_root_hash: String,
    signature: String,
}

/// `dologctl verify-anchor <path>` — verify external anchor JSON file.
///
/// Check each anchor's Ed25519 signature, verify sequential IDs and
/// monotonic timestamps. Requires `--pubkey` for signature verification.
pub fn cmd_verify_anchor(path: &str, pubkey_hex: Option<&str>, format: OutputFormat) {
    if format == OutputFormat::Json {
        cmd_verify_anchor_json(path, pubkey_hex);
        return;
    }

    let bg = bright_cyan();
    let b = bold();
    let d = dim();
    let g = green();
    let r = red();
    let y = yellow();
    let c = cyan();
    let reset = output::when_color(color::RESET);

    stdout!("{b}{bg}Anchor File Verification{reset}");
    stdout!("{d}────────────────────────{reset}");
    stdout!("  File: {path}");

    // Parse public key
    let verifying_key: Option<VerifyingKey> = match pubkey_hex {
        Some(hex_str) => {
            let hex_str = hex_str.trim();
            match dologger_core::hex::decode(hex_str) {
                Ok(bytes) if bytes.len() == 32 => {
                    let arr: [u8; 32] = bytes.try_into().unwrap();
                    match VerifyingKey::from_bytes(&arr) {
                        Ok(vk) => {
                            stdout!("  Pubkey: {hex_str}");
                            Some(vk)
                        }
                        Err(e) => {
                            stderr!("{r}Error:{reset} Invalid public key: {e}");
                            std::process::exit(EXIT_VERIFY_FAILED);
                        }
                    }
                }
                Ok(bytes) => {
                    stderr!(
                        "{r}Error:{reset} Public key must be 32 bytes (got {})",
                        bytes.len()
                    );
                    std::process::exit(EXIT_VERIFY_FAILED);
                }
                Err(e) => {
                    stderr!("{r}Error:{reset} Cannot decode public key hex: {e}");
                    std::process::exit(EXIT_VERIFY_FAILED);
                }
            }
        }
        None => {
            stderr!("{r}Error:{reset} --pubkey is required for anchor verification");
            std::process::exit(EXIT_VERIFY_FAILED);
        }
    };
    stdout!("");

    // Read and parse the anchor JSON file
    let json_str = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            stderr!("{r}Error:{reset} Cannot read '{path}': {e}");
            std::process::exit(EXIT_VERIFY_FAILED);
        }
    };

    let anchors: Vec<AnchorJson> = match serde_json::from_str(&json_str) {
        Ok(a) => a,
        Err(e) => {
            stderr!("{r}Error:{reset} Invalid anchor JSON: {e}");
            std::process::exit(EXIT_VERIFY_FAILED);
        }
    };

    if anchors.is_empty() {
        stdout!("{d}No anchors found — nothing to verify.{reset}");
        return;
    }

    stdout!("  Anchors loaded: {c}{}{reset}", anchors.len());
    stdout!("");

    let total = anchors.len();
    let mut valid_sig: u32 = 0;
    let mut invalid_sig: u32 = 0;
    let mut id_issues: u32 = 0;
    let mut ts_issues: u32 = 0;

    let vk = verifying_key.unwrap();

    for (i, anchor) in anchors.iter().enumerate() {
        let idx = i + 1;
        stdout!("  Anchor #{idx} (id={})", anchor.anchor_id);

        // Check sequential IDs
        if anchor.anchor_id != idx as u64 {
            stderr!(
                "    {y}ID ISSUE{reset} Expected anchor_id={idx}, got {}",
                anchor.anchor_id
            );
            id_issues += 1;
        }

        // Check monotonic timestamps
        if i > 0 {
            let prev = &anchors[i - 1];
            if anchor.timestamp_ms < prev.timestamp_ms {
                stderr!(
                    "    {y}TIMESTAMP ISSUE{reset} {} < {} (regression)",
                    anchor.timestamp_ms,
                    prev.timestamp_ms
                );
                ts_issues += 1;
            }
        }

        // Verify Ed25519 signature
        let chain_root_hash = match dologger_core::hex::decode(&anchor.chain_root_hash) {
            Ok(bytes) if bytes.len() == 32 => {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&bytes);
                arr
            }
            _ => {
                stderr!("    {r}ERROR{reset} Invalid chain_root_hash hex");
                invalid_sig += 1;
                continue;
            }
        };

        let sig_bytes = match dologger_core::hex::decode(&anchor.signature) {
            Ok(bytes) if bytes.len() == 64 => {
                let mut arr = [0u8; 64];
                arr.copy_from_slice(&bytes);
                arr
            }
            _ => {
                stderr!("    {r}ERROR{reset} Invalid signature hex");
                invalid_sig += 1;
                continue;
            }
        };

        // Build anchor payload: anchor_id(8) || timestamp_ms(8) || last_lsn(8) || chain_root_hash(32)
        let mut payload = Vec::with_capacity(56);
        payload.extend_from_slice(&anchor.anchor_id.to_le_bytes());
        payload.extend_from_slice(&anchor.timestamp_ms.to_le_bytes());
        payload.extend_from_slice(&anchor.last_lsn.to_le_bytes());
        payload.extend_from_slice(&chain_root_hash);

        let sig = Signature::from_bytes(&sig_bytes);

        if vk.verify(&payload, &sig).is_ok() {
            stdout!(
                "    Signature: {g}VALID{reset}  last_lsn={}  ts={}",
                anchor.last_lsn,
                anchor.timestamp_ms
            );
            valid_sig += 1;
        } else {
            stderr!(
                "    Signature: {r}INVALID{reset}  last_lsn={}  ts={}",
                anchor.last_lsn,
                anchor.timestamp_ms
            );
            invalid_sig += 1;
        }

        stdout!("");
    }

    // Summary
    stdout!("{b}Anchor Verification Results{reset}");
    stdout!("{d}──────────────────────────{reset}");
    stdout!("  Total anchors:     {total}");
    stdout!("  Signatures:        {g}{valid_sig} valid{reset}, {r}{invalid_sig} invalid{reset}");
    if id_issues > 0 {
        stdout!("  ID sequence:       {y}{id_issues} issue(s){reset}");
    } else {
        stdout!("  ID sequence:       {g}PASS{reset}");
    }
    if ts_issues > 0 {
        stdout!("  Timestamps:        {y}{ts_issues} issue(s){reset}");
    } else {
        stdout!("  Timestamps:        {g}PASS{reset}");
    }

    if invalid_sig > 0 || id_issues > 0 || ts_issues > 0 {
        stdout!("");
        let issue_count = invalid_sig + id_issues + ts_issues;
        stderr!(
            "{r}{b}ANCHOR VERIFICATION FAILED{reset}{r} — {issue_count} issue(s) detected{reset}"
        );
        std::process::exit(EXIT_VERIFY_FAILED);
    } else {
        stdout!("");
        stdout!(
            "{bright_green}{b}ANCHOR VERIFICATION PASSED{reset}",
            bright_green = bright_green(),
            b = b,
            reset = reset
        );
    }
}

/// JSON variant of `cmd_verify_anchor`.
fn cmd_verify_anchor_json(path: &str, pubkey_hex: Option<&str>) {
    let vk = match pubkey_hex {
        Some(h) => {
            match dologger_core::hex::decode(h.trim())
                .ok()
                .and_then(|b| <[u8; 32]>::try_from(b).ok())
                .and_then(|arr| VerifyingKey::from_bytes(&arr).ok())
            {
                Some(k) => k,
                None => {
                    let obj = serde_json::json!({"status": "error", "error_code": EXIT_VERIFY_FAILED, "message": "Invalid or missing --pubkey"});
                    output::stdout_line(&obj.to_string());
                    std::process::exit(EXIT_VERIFY_FAILED);
                }
            }
        }
        None => {
            let obj = serde_json::json!({"status": "error", "error_code": EXIT_VERIFY_FAILED, "message": "--pubkey is required for anchor verification"});
            output::stdout_line(&obj.to_string());
            std::process::exit(EXIT_VERIFY_FAILED);
        }
    };

    let json_str = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            let obj = serde_json::json!({"status": "error", "error_code": EXIT_VERIFY_FAILED, "message": format!("Cannot read file: {e}")});
            output::stdout_line(&obj.to_string());
            std::process::exit(EXIT_VERIFY_FAILED);
        }
    };

    let anchors: Vec<AnchorJson> = match serde_json::from_str(&json_str) {
        Ok(a) => a,
        Err(e) => {
            let obj = serde_json::json!({"status": "error", "error_code": EXIT_VERIFY_FAILED, "message": format!("Invalid JSON: {e}")});
            output::stdout_line(&obj.to_string());
            std::process::exit(EXIT_VERIFY_FAILED);
        }
    };

    let total = anchors.len();
    let mut valid_sig: u32 = 0;
    let mut invalid_sig: u32 = 0;
    let mut id_issues: u32 = 0;
    let mut ts_issues: u32 = 0;

    for (i, anchor) in anchors.iter().enumerate() {
        if anchor.anchor_id != (i + 1) as u64 {
            id_issues += 1;
        }
        if i > 0 && anchor.timestamp_ms < anchors[i - 1].timestamp_ms {
            ts_issues += 1;
        }

        let hash = match dologger_core::hex::decode(&anchor.chain_root_hash) {
            Ok(b) if b.len() == 32 => {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&b);
                arr
            }
            _ => {
                invalid_sig += 1;
                continue;
            }
        };
        let sig = match dologger_core::hex::decode(&anchor.signature) {
            Ok(b) if b.len() == 64 => {
                let mut arr = [0u8; 64];
                arr.copy_from_slice(&b);
                arr
            }
            _ => {
                invalid_sig += 1;
                continue;
            }
        };

        let mut payload = Vec::with_capacity(56);
        payload.extend_from_slice(&anchor.anchor_id.to_le_bytes());
        payload.extend_from_slice(&anchor.timestamp_ms.to_le_bytes());
        payload.extend_from_slice(&anchor.last_lsn.to_le_bytes());
        payload.extend_from_slice(&hash);

        if vk.verify(&payload, &Signature::from_bytes(&sig)).is_ok() {
            valid_sig += 1;
        } else {
            invalid_sig += 1;
        }
    }

    let passed = invalid_sig == 0 && id_issues == 0 && ts_issues == 0;

    let obj = serde_json::json!({
        "status": if passed { "passed" } else { "failed" },
        "file": path,
        "total_anchors": total,
        "signatures_valid": valid_sig,
        "signatures_invalid": invalid_sig,
        "id_sequence_issues": id_issues,
        "timestamp_issues": ts_issues
    });
    output::stdout_line(&obj.to_string());

    if !passed {
        std::process::exit(EXIT_VERIFY_FAILED);
    }
}

// ===========================================================================
// cmd_recovery_report — scan *.worm files for LSN continuity
// ===========================================================================

/// `dologctl recovery-report <worm_dir>` — scan *.worm files, report
/// LSN continuity, last valid LSN, and gaps.
pub fn cmd_recovery_report(worm_dir: &str, format: OutputFormat) {
    if format == OutputFormat::Json {
        cmd_recovery_report_json(worm_dir);
        return;
    }

    let bg = bright_cyan();
    let b = bold();
    let d = dim();
    let g = green();
    let r = red();
    let y = yellow();
    let c = cyan();
    let reset = output::when_color(color::RESET);

    stdout!("{b}{bg}WORM Recovery Report{reset}");
    stdout!("{d}────────────────────{reset}");
    stdout!("  Directory: {worm_dir}");
    stdout!("");

    let dir = match fs::read_dir(worm_dir) {
        Ok(d) => d,
        Err(e) => {
            stderr!("{r}Error:{reset} Cannot read directory '{worm_dir}': {e}");
            std::process::exit(EXIT_VERIFY_FAILED);
        }
    };

    // Collect all .worm files sorted by filename
    let mut worm_files: Vec<std::path::PathBuf> = Vec::new();
    for entry in dir.flatten() {
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) == Some("worm") {
            worm_files.push(p);
        }
    }
    worm_files.sort();

    if worm_files.is_empty() {
        stdout!("{d}No .worm files found in '{worm_dir}'.{reset}");
        return;
    }

    stdout!("  Worm files found: {c}{}{reset}", worm_files.len());
    stdout!("");

    // Collect all records from all worm files
    let mut all_records: Vec<SifRecord> = Vec::new();

    for worm_path in &worm_files {
        let fname = worm_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?");
        stdout!("  Scanning: {d}{fname}{reset}");

        match read_sif_file(&worm_path.to_string_lossy()) {
            Ok(recs) => {
                let n = recs.len();
                stdout!("    Records: {n}");
                all_records.extend(recs);
            }
            Err(e) => {
                stderr!("    {y}Warning:{reset} {e}");
            }
        }
    }

    if all_records.is_empty() {
        stdout!("");
        stdout!("{d}No records found across all worm files.{reset}");
        return;
    }

    // Sort by LSN
    all_records.sort_by_key(|r| r.lsn);

    let total = all_records.len();
    let first_lsn = all_records.first().map(|r| r.lsn).unwrap_or(0);
    let last_lsn = all_records.last().map(|r| r.lsn).unwrap_or(0);

    stdout!("");
    stdout!("{b}LSN Continuity Analysis{reset}");
    stdout!("{d}───────────────────────{reset}");
    stdout!("  Total records:     {total}");
    stdout!("  First LSN:         {first_lsn}");
    stdout!("  Last valid LSN:    {c}{last_lsn}{reset}");

    // Check continuity
    let mut gaps: Vec<(u64, u64)> = Vec::new();
    let mut chain_broken: u32 = 0;

    for i in 1..total {
        let prev = &all_records[i - 1];
        let next = &all_records[i];

        let expected = prev.lsn.saturating_add(1);
        if next.lsn > expected {
            gaps.push((expected, next.lsn.saturating_sub(1)));
        }

        match verify_chain_link(prev, next) {
            Ok(()) => {}
            Err(_) => chain_broken += 1,
        }
    }

    if gaps.is_empty() {
        let expected_total = last_lsn.saturating_sub(first_lsn).saturating_add(1);
        let completeness = if expected_total > 0 {
            total as f64 / expected_total as f64 * 100.0
        } else {
            100.0
        };
        stdout!("  Coverage:          {g}{completeness:.1}%{reset} ({total}/{expected_total})");
        stdout!("  LSN gaps:          {g}none{reset}");
    } else {
        let missing_total: u64 = gaps.iter().map(|(f, t)| t - f + 1).sum();
        stdout!(
            "  LSN gaps:          {r}{}{reset} ({} records missing)",
            gaps.len(),
            missing_total
        );

        for (from, to) in &gaps {
            let missing = to - from + 1;
            stdout!(
                "    Gap: LSN {} → {} (missing {} record{})",
                from,
                to,
                missing,
                if missing > 1 { "s" } else { "" }
            );
        }
    }

    if chain_broken > 0 {
        stdout!("  Chain integrity:   {r}{chain_broken} broken link(s){reset}");
    } else if total > 1 {
        stdout!("  Chain integrity:   {g}PASS{reset}");
    }

    // Recommend last valid LSN for recovery
    stdout!("");
    stdout!("{b}Recovery Recommendation{reset}");
    stdout!("{d}───────────────────────{reset}");

    if gaps.is_empty() && chain_broken == 0 {
        stdout!("  Status:  {g}Healthy{reset} — all records intact and in sequence");
        stdout!("  Last valid LSN for resume: {c}{last_lsn}{reset}");
    } else {
        // Find the last valid continuous segment
        let mut continuous_last = all_records[0].lsn;
        for i in 1..total {
            let prev = &all_records[i - 1];
            let next = &all_records[i];
            if next.lsn == prev.lsn.saturating_add(1) && verify_chain_link(prev, next).is_ok() {
                continuous_last = next.lsn;
            } else {
                break;
            }
        }
        stdout!(
            "  Status:  {y}Degraded{reset} — {} gap(s), {} broken link(s)",
            gaps.len(),
            chain_broken
        );
        stdout!("  Last continuous valid LSN:  {c}{continuous_last}{reset}");
        stdout!(
            "  Resume LSN recommendation:  {y}{}{reset}",
            continuous_last.saturating_add(1)
        );
    }
}

/// JSON variant of `cmd_recovery_report`.
fn cmd_recovery_report_json(worm_dir: &str) {
    let dir = match fs::read_dir(worm_dir) {
        Ok(d) => d,
        Err(e) => {
            let obj = serde_json::json!({"status": "error", "error_code": EXIT_VERIFY_FAILED, "message": format!("Cannot read directory: {e}")});
            output::stdout_line(&obj.to_string());
            std::process::exit(EXIT_VERIFY_FAILED);
        }
    };

    let mut worm_files: Vec<std::path::PathBuf> = Vec::new();
    for entry in dir.flatten() {
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) == Some("worm") {
            worm_files.push(p);
        }
    }
    worm_files.sort();

    let worm_count = worm_files.len();
    let mut all_records: Vec<SifRecord> = Vec::new();

    for worm_path in &worm_files {
        if let Ok(recs) = read_sif_file(&worm_path.to_string_lossy()) {
            all_records.extend(recs);
        }
    }

    all_records.sort_by_key(|r| r.lsn);

    let total = all_records.len();
    let first_lsn = all_records.first().map(|r| r.lsn).unwrap_or(0);
    let last_lsn = all_records.last().map(|r| r.lsn).unwrap_or(0);

    let mut gaps_count: u32 = 0;
    let mut chain_broken: u32 = 0;
    let mut missing_total: u64 = 0;

    for i in 1..total {
        let prev = &all_records[i - 1];
        let next = &all_records[i];

        let expected = prev.lsn.saturating_add(1);
        if next.lsn > expected {
            gaps_count += 1;
            missing_total += next.lsn - expected;
        }
        if verify_chain_link(prev, next).is_err() {
            chain_broken += 1;
        }
    }

    let healthy = gaps_count == 0 && chain_broken == 0;
    let mut continuous_last = all_records.first().map(|r| r.lsn).unwrap_or(0);
    for i in 1..total {
        let prev = &all_records[i - 1];
        let next = &all_records[i];
        if next.lsn == prev.lsn.saturating_add(1) && verify_chain_link(prev, next).is_ok() {
            continuous_last = next.lsn;
        } else {
            break;
        }
    }

    let obj = serde_json::json!({
        "status": if healthy { "healthy" } else { "degraded" },
        "directory": worm_dir,
        "worm_files": worm_count,
        "total_records": total,
        "first_lsn": first_lsn,
        "last_lsn": last_lsn,
        "lsn_gaps": gaps_count,
        "missing_records": missing_total,
        "broken_chain_links": chain_broken,
        "last_continuous_valid_lsn": continuous_last,
        "resume_lsn_recommendation": continuous_last.saturating_add(1)
    });
    output::stdout_line(&obj.to_string());
}
