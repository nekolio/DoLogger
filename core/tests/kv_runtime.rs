//! Integration coverage for the canonical KV runtime path.
//!
//! These tests intentionally sit outside `record` so they exercise the public
//! module boundary used by SHM, CLI replay, and future adapters.

use dologger_core::codec::log_output::{
    LineEnding, NulPolicy, TextOutputEncoder, TextOutputPolicy,
};
use dologger_core::codec::policy::{resolve_from_detection, EncodingPolicy, EncodingPreference};
use dologger_core::codec::EncodingDetection;
use dologger_core::record::wire::{
    decode_any, encode_length_prefixed, encode_record, DecodeOptions, FrameKind, FrameScanner,
    ReusableEncoder, KV_HEADER_LEN,
};
use dologger_core::record::{FieldRing, LogLevel, Record};
use dologger_core::security::{ReplayDecision, ReplayGuard, ReplayPolicy};

fn record(lsn: u64, message: &str) -> Record {
    let mut value = Record::new(11);
    value.set_id(7, lsn);
    value.timestamp = 1_700_000_000_000_000_000 + lsn;
    value.level = LogLevel::Info;
    value.lsn = lsn;
    value.thread_id = 10;
    value.process_id = 20;
    value.message.set(message);
    value.set_trace_id("trace.integration");
    value.set_source_file("integration.rs");
    value
}

#[test]
fn public_kv_frame_round_trip_survives_vendor_fields() {
    let mut original = record(1, "integration");
    original
        .field_set("ext.integration.case", "value", FieldRing::Ring3)
        .unwrap();
    original.compute_content_hash();
    let frame = encode_record(&original).unwrap();
    assert!(frame.len() > KV_HEADER_LEN);
    let (decoded, kind) = decode_any(&frame, DecodeOptions::untrusted()).unwrap();
    assert_eq!(kind, FrameKind::Kv);
    assert_eq!(
        decoded
            .field_get("ext.integration.case", FieldRing::Ring3)
            .unwrap(),
        "value"
    );
    assert_eq!(decoded.content_hash, original.content_hash);
}

#[test]
fn public_legacy_sif_decode_is_explicitly_compatibility_only() {
    let original = record(2, "legacy");
    let sif = dologger_core::sif::encode_record(&original);
    let (decoded, kind) = decode_any(&sif, DecodeOptions::default()).unwrap();
    assert_eq!(kind, FrameKind::Sif);
    assert_eq!(decoded.message.as_utf8().unwrap(), "legacy");
}

#[test]
fn stream_scanner_handles_fragmented_multi_record_input() {
    let mut bytes = encode_length_prefixed(&record(1, "one")).unwrap();
    bytes.extend_from_slice(&encode_length_prefixed(&record(2, "two")).unwrap());
    let mut scanner = FrameScanner::new(1024 * 1024).unwrap();
    for chunk in bytes.chunks(7) {
        scanner.feed(chunk).unwrap();
    }
    let first = scanner
        .next_record(DecodeOptions::default())
        .unwrap()
        .unwrap();
    let second = scanner
        .next_record(DecodeOptions::default())
        .unwrap()
        .unwrap();
    assert_eq!((first.lsn, second.lsn), (1, 2));
    assert!(scanner
        .next_record(DecodeOptions::default())
        .unwrap()
        .is_none());
}

#[test]
fn reusable_encoder_keeps_bounded_capacity() {
    let mut encoder = ReusableEncoder::new(1024 * 1024).unwrap();
    let first = encoder.encode(&record(1, "first")).unwrap().to_vec();
    let capacity = encoder.capacity();
    let second = encoder.encode(&record(2, "second")).unwrap().to_vec();
    assert!(encoder.capacity() >= capacity);
    assert_ne!(first, second);
    assert_eq!(encoder.as_bytes(), second.as_slice());
}

#[test]
fn encoding_policy_is_independent_from_localization() {
    let snapshot = resolve_from_detection(
        EncodingPolicy {
            preference: EncodingPreference::Utf8,
            ..Default::default()
        },
        EncodingDetection {
            locale: Some("zh-CN".into()),
            codeset: Some("GBK".into()),
            console_code_page: Some(936),
        },
    )
    .unwrap();
    let mut output = TextOutputEncoder::new(
        snapshot,
        TextOutputPolicy {
            line_ending: LineEnding::Lf,
            nul: NulPolicy::Escape,
            ..Default::default()
        },
    )
    .unwrap();
    output.encode("日志\0消息").unwrap();
    assert_eq!(output.bytes(), "日志\\0消息\n".as_bytes());
}

#[test]
fn replay_guard_protects_strict_audit_order() {
    let guard = ReplayGuard::new(ReplayPolicy {
        window_size: 16,
        reject_gaps: true,
        reject_duplicates: true,
    })
    .unwrap();
    assert_eq!(guard.observe(1, [1; 32]), ReplayDecision::Accepted);
    assert_eq!(guard.observe(1, [1; 32]), ReplayDecision::Duplicate);
    assert_eq!(guard.observe(1, [2; 32]), ReplayDecision::HashConflict);
    assert_eq!(
        guard.observe(3, [3; 32]),
        ReplayDecision::Gap {
            expected: 2,
            found: 3
        }
    );
    assert_eq!(guard.observe(2, [2; 32]), ReplayDecision::Accepted);
    assert_eq!(guard.snapshot().accepted, 2);
}

#[test]
fn replay_window_bounds_memory() {
    let guard = ReplayGuard::new(ReplayPolicy {
        window_size: 4,
        reject_gaps: false,
        reject_duplicates: false,
    })
    .unwrap();
    for lsn in 1..=32 {
        assert_eq!(
            guard.observe(lsn, [lsn as u8; 32]),
            ReplayDecision::Accepted
        );
    }
    assert_eq!(guard.snapshot().retained, 4);
}

#[test]
fn audit_hash_round_trip_is_verifiable() {
    let mut value = record(8, "audit");
    value.level = LogLevel::Audit;
    value.compute_content_hash();
    let frame = encode_record(&value).unwrap();
    let decoded =
        dologger_core::record::wire::decode_record_with(&frame, DecodeOptions::audit()).unwrap();
    assert_eq!(decoded.content_hash, value.content_hash);
}
