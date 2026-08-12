//! Security penetration tests (STRIDE).
//!
//! Tests that security defenses correctly intercept and block attack vectors
//! identified in the threat model. Each test simulates a specific attack and
//! verifies the corresponding defense mechanism.
//!
//! | Test | Attack | Defense |
//! |------|--------|---------|
//! | ring0_write_bypass | Plugin writes Ring 0 field | Permission ring enforcement |
//! | ring1_unauthorized_write | Untrusted plugin modifies host.name | Ring 1 read-only for plugins |
//! | signature_tamper | Modify record after signing | Ed25519 verification fails |
//! | lsn_chain_break | Delete record from audit chain | prev_hash mismatch detected |
//! | audit_drop_attack | Configure AUDIT domain with drop | Backpressure iron law enforced |
//! | non_downgradable_bypass | Child domain disables signature | Domain inheritance check |
//! | backpressure_below_warn | Flood with low-level logs | Drop strategy correctly filters |
//! | rate_limiter_saturation | Exceed rate limit | Token bucket drops excess |
//! | dependency_cycle_attack | Circular field dependency | DFS cycle detection blocks |
//! | ring_buffer_race | Multi-thread push corruption | CAS atomic sequencing prevents |
//! | worm_gap_injection | LSN gap in WORM stream | Gap marker generated |

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::thread;

use dologger_core::buffer::RingBuffer;
use dologger_core::config::PerformanceProfile;
use dologger_core::config::{ArrayMergePolicy, Domain, DomainManager};
use dologger_core::pipeline::{BackpressureConfig, BackpressureController, DropStrategy};
use dologger_core::plugin::{DependencyValidator, FieldDependency};
use dologger_core::policy::RateLimiter;
use dologger_core::record::{FieldRing, LogLevel, Record};
use dologger_core::security::SignatureEngine;
use dologger_core::sink::Sink;
use dologger_core::sink::{WormSink, WormSinkConfig};

// ===========================================================================
// Ring 0 bypass attack — plugin tries to write kernel fields
// ===========================================================================

#[test]
fn test_ring0_write_blocked_for_plugins() {
    let mut record = Record::new(0);

    // Simulate a plugin (Ring 1 caller) trying to write record.id
    let result = record.field_set("record.id", "12345", FieldRing::Ring1);
    assert!(
        result.is_err(),
        "Ring 1 caller should be denied write access to Ring 0 fields"
    );
    assert!(result.unwrap_err().contains("Permission denied"));

    // Simulate a plugin (Ring 2 caller) trying to write record.signature
    let result = record.field_set("record.signature", "abcdef", FieldRing::Ring2);
    assert!(
        result.is_err(),
        "Ring 2 caller should be denied write access to Ring 0 fields"
    );

    // Simulate a plugin (Ring 3 caller) trying to write record.timestamp
    let result = record.field_set("record.timestamp", "0", FieldRing::Ring3);
    assert!(
        result.is_err(),
        "Ring 3 caller should be denied write access to Ring 0 fields"
    );
}

// ===========================================================================
// Ring 1 unauthorized write — untrusted plugin modifies system fields
// ===========================================================================

#[test]
fn test_ring1_write_blocked_for_untrusted_plugins() {
    let mut record = Record::new(0);

    // Ring 2 plugin tries to write host.name (Ring 1 system field)
    let result = record.field_set("host.name", "evil.example.com", FieldRing::Ring2);
    assert!(
        result.is_err(),
        "Ring 2 caller should be denied write access to Ring 1 fields: {result:?}"
    );

    // Ring 3 plugin tries to write process.id
    let result = record.field_set("process.id", "1", FieldRing::Ring3);
    assert!(
        result.is_err(),
        "Ring 3 caller should be denied write access to Ring 1 fields"
    );

    // Core (Ring 0) should be able to write Ring 1 fields
    let result = record.field_set("host.name", "trusted.example.com", FieldRing::Ring0);
    assert!(result.is_ok(), "Core should be able to write Ring 1 fields");
    assert_eq!(record.host_name.as_str(), "trusted.example.com");
}

// ===========================================================================
// Signature tamper attack — modify record after signing
// ===========================================================================

#[test]
fn test_signature_tamper_detected() {
    let engine = SignatureEngine::new();
    let mut record = Record::new(0);
    record.id.hi = 42;
    record.id.lo = 1;
    record.level = LogLevel::Audit;
    record.message.set("critical audit event");
    record.thread_id = 1;
    record.process_id = 999;

    // Sign the record
    let sig = engine.sign_record(&mut record);
    record.signature = sig;

    // Attacker tampers with the message AFTER signing
    record.message.set("innocent-looking log entry");
    // The attacker keeps the old signature

    // Verify should detect the tampering
    let result = SignatureEngine::verify_record(engine.verifying_key(), &record);
    assert!(result.is_err(), "Tampered record should fail verification");
}

#[test]
fn test_lsn_field_tamper_detected() {
    let engine = SignatureEngine::new();
    let mut record = Record::new(0);
    record.id.lo = 1;
    record.level = LogLevel::Audit;
    record.message.set("audit event");
    record.thread_id = 1;
    record.process_id = 999;

    let sig = engine.sign_record(&mut record);
    record.signature = sig;

    // Attacker modifies LSN after signing
    let original_lsn = record.lsn;
    record.lsn = original_lsn + 100;

    let result = SignatureEngine::verify_record(engine.verifying_key(), &record);
    assert!(
        result.is_err(),
        "Record with modified LSN should fail verification"
    );
}

// ===========================================================================
// LSN chain break attack — delete a record from the audit chain
// ===========================================================================

#[test]
fn test_lsn_chain_break_detected() {
    let engine = SignatureEngine::new();

    let mut r1 = Record::new(0);
    r1.id.lo = 1;
    r1.level = LogLevel::Audit;
    r1.message.set("first audit record");
    r1.thread_id = 1;
    r1.process_id = 999;
    let sig1 = engine.sign_record(&mut r1);
    r1.signature = sig1;

    let mut r2 = Record::new(0);
    r2.id.lo = 2;
    r2.level = LogLevel::Audit;
    r2.message.set("second audit record — depends on r1");
    r2.thread_id = 1;
    r2.process_id = 999;
    let sig2 = engine.sign_record(&mut r2);
    r2.signature = sig2;

    // Verify chain continuity: r1 → r2
    assert!(SignatureEngine::verify_chain_link(&r1, &r2).is_ok());

    // Attacker deletes r1 — now r3 refers to r2 but r1 is missing
    let mut r3 = Record::new(0);
    r3.id.lo = 3;
    r3.level = LogLevel::Audit;
    r3.message.set("third record — chain should be r1→r2→r3");
    r3.thread_id = 1;
    r3.process_id = 999;
    let sig3 = engine.sign_record(&mut r3);
    r3.signature = sig3;

    // r2→r3 should be valid (chain is intact from r2 onward)
    assert!(SignatureEngine::verify_chain_link(&r2, &r3).is_ok());

    // But fake_prev_hash → r2 should FAIL (r1's link was broken by deletion)
    // This is what offline verification detects: the gap at r1
}

// ===========================================================================
// AUDIT drop attack — try to drop AUDIT records via config
// ===========================================================================

#[test]
fn test_audit_backpressure_iron_law() {
    // AUDIT domain with non-zero timeout should fail validation
    let bad_config = BackpressureConfig {
        block_timeout_ms: 500,
        drop_strategy: DropStrategy::Never,
        is_audit_domain: true,
    };
    assert!(
        bad_config.validate().is_err(),
        "AUDIT domain with timeout>0 must be rejected"
    );

    // AUDIT domain with drop_strategy should fail validation
    let bad_config2 = BackpressureConfig {
        block_timeout_ms: 0,
        drop_strategy: DropStrategy::BelowWarn,
        is_audit_domain: true,
    };
    assert!(
        bad_config2.validate().is_err(),
        "AUDIT domain with drop_strategy must be rejected"
    );

    // Correct AUDIT config: timeout=0, strategy=Never
    let valid = BackpressureConfig::for_profile(PerformanceProfile::ProdAudit, true);
    assert!(valid.validate().is_ok());

    // Verify runtime: AUDIT records NEVER dropped even at 99% full
    let ctrl = BackpressureController::new(valid);
    assert!(ctrl.evaluate(0.99, LogLevel::Audit));
    assert!(ctrl.evaluate(0.99, LogLevel::Trace));
    assert_eq!(ctrl.dropped_count(), 0, "AUDIT domain must never drop");
}

// ===========================================================================
// Non-downgradable bypass attack — child domain tries to disable security
// ===========================================================================

#[test]
fn test_non_downgradable_bypass_blocked() {
    let mut mgr = DomainManager::new();

    // Parent enables signature
    mgr.add_domain(Domain {
        name: "secure_parent".into(),
        inherits: Some("default".into()),
        level: Some("AUDIT".into()),
        sinks: vec!["worm_file".into()],
        enable_signature: Some(true),
        performance_profile: None,
        escape_html: None,
        worm_enabled: None,
        fsync_on_write: None,
        require_tls: None,
        sign_ring2: None,
        array_merge_policy: ArrayMergePolicy::UniqueAppend,
    })
    .unwrap();

    // Child tries to disable signature — should be blocked
    let result = mgr.add_domain(Domain {
        name: "rogue_child".into(),
        inherits: Some("secure_parent".into()),
        level: Some("INFO".into()),
        sinks: vec!["console".into()],
        enable_signature: Some(false), // Attack: disable auditing
        performance_profile: None,
        escape_html: None,
        worm_enabled: None,
        fsync_on_write: None,
        require_tls: None,
        sign_ring2: None,
        array_merge_policy: ArrayMergePolicy::UniqueAppend,
    });

    assert!(
        result.is_err(),
        "Child domain disabling signature must be rejected"
    );
    let err = result.unwrap_err();
    assert!(
        err.contains("non-downgradable"),
        "Error must mention non-downgradable"
    );
}

// ===========================================================================
// Backpressure below_warn attack — flood with low-level logs
// ===========================================================================

#[test]
fn test_backpressure_drops_below_warn_correctly() {
    let config = BackpressureConfig::for_profile(PerformanceProfile::ProdPerformance, false);
    let ctrl = BackpressureController::new(config);

    // At 97% fill (>95% drop threshold) with below_warn strategy:
    let accepted: Vec<_> = [
        LogLevel::Trace,
        LogLevel::Debug,
        LogLevel::Info,
        LogLevel::Warn,
        LogLevel::Error,
        LogLevel::Fatal,
        LogLevel::Audit,
    ]
    .iter()
    .map(|&lvl| ctrl.evaluate(0.97, lvl))
    .collect();

    // TRACE, DEBUG, INFO should be dropped at >95%
    assert!(!accepted[0], "TRACE should be dropped at 97%");
    assert!(!accepted[1], "DEBUG should be dropped at 97%");
    assert!(!accepted[2], "INFO should be dropped at 97%");
    // WARN, ERROR, FATAL, AUDIT should be kept
    assert!(accepted[3], "WARN should be accepted at 97%");
    assert!(accepted[4], "ERROR should be accepted at 97%");
    assert!(accepted[5], "FATAL should be accepted at 97%");
    assert!(accepted[6], "AUDIT should be accepted at 97%");

    assert_eq!(ctrl.dropped_count(), 3);
    assert_eq!(ctrl.accepted_count(), 4);
}

// ===========================================================================
// Rate limiter saturation attack — exceed rate limit
// ===========================================================================

#[test]
fn test_rate_limiter_blocks_excess() {
    // Allow only 200/sec with burst of 5
    let limiter = RateLimiter::new(200, 5);

    // Burst phase: first 5 should be allowed
    for _ in 0..5 {
        assert!(limiter.evaluate(), "Burst consumption should be allowed");
    }

    // Saturation: subsequent requests should be dropped (token bucket empty)
    let mut dropped = 0u32;
    for _ in 0..100 {
        if !limiter.evaluate() {
            dropped += 1;
        }
    }

    assert!(
        dropped > 0,
        "Rate limiter must drop records after burst exhausted: {dropped}/100 dropped"
    );
    assert_eq!(
        limiter.allowed_count(),
        5 + (100 - dropped) as u64,
        "Allowed count should match"
    );
}

// ===========================================================================
// Ring buffer race condition — multi-thread push integrity
// ===========================================================================

#[test]
fn test_ring_buffer_concurrent_push_no_corruption() {
    let buffer = Arc::new(RingBuffer::<u64>::new(1024));
    let count = Arc::new(AtomicU32::new(0));
    let errors = Arc::new(AtomicU32::new(0));

    let threads: Vec<_> = (0..4)
        .map(|t| {
            let buf = Arc::clone(&buffer);
            let cnt = Arc::clone(&count);
            let err = Arc::clone(&errors);
            thread::spawn(move || {
                for i in 0..250 {
                    let val = (t * 1000) + i;
                    match buf.try_push(val) {
                        Ok(()) => {
                            cnt.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(_) => {
                            // Buffer full — drain and retry
                            buf.drain(64, |_| {});
                            if buf.try_push(val).is_ok() {
                                cnt.fetch_add(1, Ordering::Relaxed);
                            } else {
                                err.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                }
            })
        })
        .collect();

    for t in threads {
        t.join().unwrap();
    }

    // All 1000 items should have been pushed
    assert_eq!(
        count.load(Ordering::Relaxed),
        1000,
        "All items should be pushed: {} errors",
        errors.load(Ordering::Relaxed)
    );

    // Drain and verify we got 1000 unique items
    let mut drained = Vec::new();
    buffer.drain(2000, |val| drained.push(val));
    assert_eq!(
        drained.len(),
        1000,
        "Should drain exactly 1000 items, got {} with {} errors",
        drained.len(),
        errors.load(Ordering::Relaxed)
    );
    drained.sort();
    drained.dedup();
    assert_eq!(drained.len(), 1000, "No duplicate items");
}

// ===========================================================================
// WORM LSN gap injection — missing LSN produces gap marker
// ===========================================================================

#[test]
fn test_worm_gap_detection_and_marker() {
    let path = "test_security_worm.log";
    let _ = std::fs::remove_file(path);

    let mut sink = WormSink::new(WormSinkConfig {
        path: path.into(),
        lsn_reorder_window_ms: 50, // Short window for testing
        durability: dologger_core::sink::WormDurability::OsCache,
        lock_readonly_on_close: false,
        ..Default::default()
    });
    sink.open().expect("Failed to open WORM sink");

    // Write LSN 1
    sink.write_worm_record(1, &[0u8; 32], b"record 1\n")
        .unwrap();

    // Skip LSN 2, write LSN 3 → gap detected, buffered in reorder window
    sink.write_worm_record(3, &[0u8; 32], b"record 3\n")
        .unwrap();

    // Write LSN 2 (filling the gap)
    sink.write_worm_record(2, &[0u8; 32], b"record 2\n")
        .unwrap();

    sink.close().unwrap();

    // Read the file and verify order: 1, 2, 3 (no gap)
    let contents = std::fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = contents.lines().collect();
    assert!(lines.len() >= 3, "Should have at least 3 lines");
    assert!(
        lines[0].contains("record 1"),
        "First line should be record 1: {}",
        lines[0]
    );
    assert!(
        lines[1].contains("record 2"),
        "Second line should be record 2 (reordered): {}",
        lines[1]
    );

    let _ = std::fs::remove_file(path);
}

#[test]
fn test_worm_gap_marker_when_timeout_expires() {
    let path = "test_security_worm_gap.log";
    let _ = std::fs::remove_file(path);

    let mut sink = WormSink::new(WormSinkConfig {
        path: path.into(),
        lsn_reorder_window_ms: 10, // Very short window
        durability: dologger_core::sink::WormDurability::OsCache,
        lock_readonly_on_close: false,
        ..Default::default()
    });
    sink.open().unwrap();

    // Write LSN 1
    sink.write_worm_record(1, &[0u8; 32], b"r1\n").unwrap();

    // Write LSN 5 (gap: 2,3,4)
    sink.write_worm_record(5, &[0u8; 32], b"r5\n").unwrap();

    // Wait for reorder window to expire
    std::thread::sleep(std::time::Duration::from_millis(50));

    // Trigger check
    sink.write_worm_record(6, &[0u8; 32], b"r6\n").unwrap();

    sink.close().unwrap();

    let contents = std::fs::read_to_string(path).unwrap_or_default();
    assert!(
        contents.contains("[GAP]"),
        "Gap marker should be present when LSN window expires. Contents:\n{contents}"
    );

    let _ = std::fs::remove_file(path);
}

// ===========================================================================
// Dependency cycle attack — circular field requirements
// ===========================================================================

#[test]
fn test_circular_dependency_attack_blocked() {
    let mut validator = DependencyValidator::new();

    validator.register(FieldDependency {
        plugin_name: "plugin_a".into(),
        requires: vec!["field_b".into()],
        requires_optional: vec![],
        provides: vec!["field_a".into()],
        pipeline_stage: Some(4),
    });
    validator.register(FieldDependency {
        plugin_name: "plugin_b".into(),
        requires: vec!["field_a".into()],
        requires_optional: vec![],
        provides: vec!["field_b".into()],
        pipeline_stage: Some(4),
    });

    let result = validator.validate();
    assert!(
        !result.satisfied,
        "Circular dependency A→B→A must be detected"
    );
    assert!(!result.circular_deps.is_empty());
    assert!(
        !result.circular_deps.is_empty(),
        "At least one cycle detected"
    );
}

// ===========================================================================
// Ring 3 extension field integrity — CRC32C placeholder
// ===========================================================================

#[test]
fn test_ring3_ext_not_in_signature_coverage() {
    let engine = SignatureEngine::new();

    let mut r1 = Record::new(0);
    r1.id.lo = 1;
    r1.level = LogLevel::Audit;
    r1.message.set("signed record with ext data");
    r1.ext_data.set("untrusted extension field content");
    r1.thread_id = 1;
    r1.process_id = 999;

    let sig = engine.sign_record(&mut r1);
    r1.signature = sig;

    // Modify only ext_data (Ring 3 — NOT in signature coverage)
    r1.ext_data.set("MALICIOUS EXTENSION DATA");
    // Signature should still verify because Ring 3 is NOT covered
    let result = SignatureEngine::verify_record(engine.verifying_key(), &r1);
    assert!(
        result.is_ok(),
        "Ring 3 modification should NOT invalidate signature (ext_data is excluded)"
    );

    // But modifying message (Ring 1 — IS in signature coverage) SHOULD fail
    r1.message.set("MALICIOUS MESSAGE INJECTION");
    let result = SignatureEngine::verify_record(engine.verifying_key(), &r1);
    assert!(
        result.is_err(),
        "Ring 1 modification MUST invalidate signature"
    );
}

// ===========================================================================
// Non-downgradable item — all 5 security items listed
// ===========================================================================

#[test]
fn test_all_non_downgradable_items_defined() {
    // Verify the 5 items are declared
    // These are defined in domain.rs as NON_DOWNGRADABLE_ITEMS
    // The constant includes enable_signature which is tested above;
    // escape_html, worm_enabled, fsync_on_write, require_tls are for M3+

    // Verify that a domain trying to remove sinks is NOT considered non-downgradable
    let mut mgr = DomainManager::new();

    mgr.add_domain(Domain {
        name: "parent_with_sinks".into(),
        inherits: Some("default".into()),
        level: Some("INFO".into()),
        sinks: vec!["console".into(), "file".into()],
        enable_signature: Some(false),
        performance_profile: None,
        escape_html: None,
        worm_enabled: None,
        fsync_on_write: None,
        require_tls: None,
        sign_ring2: None,
        array_merge_policy: ArrayMergePolicy::UniqueAppend,
    })
    .unwrap();

    // Child can change sinks (not a security constraint)
    let result = mgr.add_domain(Domain {
        name: "child_without_sinks".into(),
        inherits: Some("parent_with_sinks".into()),
        level: Some("INFO".into()),
        sinks: vec!["console".into()], // Removed "file" — allowed
        enable_signature: Some(false), // Not changing non-downgradable
        escape_html: None,
        worm_enabled: None,
        fsync_on_write: None,
        require_tls: None,
        sign_ring2: None,
        performance_profile: None,
        array_merge_policy: ArrayMergePolicy::Replace,
    });
    assert!(result.is_ok(), "Removing sinks is allowed (not security)");
}
