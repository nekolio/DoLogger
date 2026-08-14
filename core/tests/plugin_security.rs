//! Plugin security-model integration tests.
//!
//! These exercise the Ed25519 trust model end-to-end against the real
//! `dologger-official-plugins` cdylib: `.sig` sidecar verification against a
//! configured trust anchor, the Red gate (unsigned plugins in production),
//! and the dev-mode / allow-red escape hatches.
//!
//! Requires the bundle to be built first: `cargo build --workspace`. If the
//! artifact is missing, the tests skip with a hint instead of failing.

use std::path::{Path, PathBuf};

use dologger_core::plugin::{PluginError, PluginManager, TrustLevel};
use dologger_core::security::fingerprint_key;
use ed25519_dalek::{Signer, SigningKey};

/// Locate the built `dologger_official_plugins` cdylib in the target dir.
fn bundle_library_path() -> Option<PathBuf> {
    let manifest = env!("CARGO_MANIFEST_DIR"); // = core/
    let profile = option_env!("PROFILE").unwrap_or("debug");
    let stem = if cfg!(windows) {
        "dologger_official_plugins"
    } else {
        "libdologger_official_plugins"
    };
    let ext = if cfg!(windows) {
        "dll"
    } else if cfg!(target_os = "macos") {
        "dylib"
    } else {
        "so"
    };

    let mut candidates: Vec<PathBuf> = vec![
        PathBuf::from(manifest).join(format!("../target/{profile}/{stem}.{ext}")),
        PathBuf::from(manifest).join(format!("../target/debug/{stem}.{ext}")),
        PathBuf::from(manifest).join(format!("../target/release/{stem}.{ext}")),
    ];
    if let Ok(td) = std::env::var("CARGO_TARGET_DIR") {
        candidates.push(PathBuf::from(&td).join(format!("{profile}/{stem}.{ext}")));
        candidates.push(PathBuf::from(&td).join(format!("debug/{stem}.{ext}")));
    }
    candidates.into_iter().find(|p| p.exists())
}

/// Copy the bundle into a fresh temp dir so a `.sig` sidecar can be attached
/// without mutating the build artifact. `tag` must be unique per test — the
/// tests run in parallel and share one process id.
fn stage_bundle(tag: &str) -> (PathBuf, PathBuf) {
    let path = bundle_library_path()
        .expect("bundle artifact should be built (run `cargo build --workspace`)");
    let dir = std::env::temp_dir().join(format!("dologger-sec-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let staged = dir.join(path.file_name().unwrap());
    std::fs::copy(&path, &staged).unwrap();
    (staged, dir)
}

/// Write `<library>.sig` — an Ed25519 signature over the library bytes.
fn sign(path: &Path, key: &SigningKey) {
    let bytes = std::fs::read(path).unwrap();
    let sig = key.sign(&bytes);
    let file_name = path.file_name().unwrap().to_str().unwrap();
    let sig_path = path.with_file_name(format!("{file_name}.sig"));
    std::fs::write(sig_path, sig.to_bytes()).unwrap();
}

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

#[test]
fn signed_plugin_is_blue_against_matching_anchor() {
    let (staged, dir) = stage_bundle("blue");
    let signing_key = key(7);
    sign(&staged, &signing_key);

    // Production mode (dev_mode = false), anchor set, signature valid.
    let mut mgr = PluginManager::new(vec![], false);
    mgr.set_trust_anchor(signing_key.verifying_key().to_bytes());

    let names = mgr
        .load_plugin(&staged)
        .expect("signed bundle loads in production");
    assert_eq!(names.len(), 4, "ONE library still registers all 4 plugins");
    for name in &names {
        assert_eq!(
            mgr.get(name).expect("registered").trust_level,
            TrustLevel::Blue,
            "{name} verifies → Blue"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn wrong_anchor_rejects_signed_plugin() {
    let (staged, dir) = stage_bundle("wrong_anchor");
    let signing_key = key(7);
    sign(&staged, &signing_key);

    // A different key is the anchor → signature must be rejected outright.
    let mut mgr = PluginManager::new(vec![], false);
    mgr.set_trust_anchor(key(9).verifying_key().to_bytes());

    let err = mgr
        .load_plugin(&staged)
        .expect_err("mismatched anchor must reject");
    assert!(
        matches!(err, PluginError::SignatureInvalid { .. }),
        "expected SignatureInvalid, got: {err:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unsigned_plugin_rejected_outside_dev_mode() {
    let (staged, dir) = stage_bundle("unsigned_prod");

    let mut mgr = PluginManager::new(vec![], false);
    let err = mgr
        .load_plugin(&staged)
        .expect_err("unsigned plugin must be rejected in production");
    assert!(
        matches!(err, PluginError::UnsignedRejected { .. }),
        "expected UnsignedRejected, got: {err:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn anchor_without_sig_keeps_plugin_red_and_gated() {
    let (staged, dir) = stage_bundle("anchor_no_sig");

    // Anchor is set but no `.sig` sidecar → Red, therefore gated in prod.
    let mut mgr = PluginManager::new(vec![], false);
    mgr.set_trust_anchor(key(7).verifying_key().to_bytes());
    let err = mgr
        .load_plugin(&staged)
        .expect_err("anchor + missing sidecar must stay Red and be rejected");
    assert!(
        matches!(err, PluginError::UnsignedRejected { .. }),
        "expected UnsignedRejected, got: {err:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unsigned_plugin_allowed_when_red_explicitly_permitted() {
    let (staged, dir) = stage_bundle("allow_red");

    // Production mode but the operator explicitly allows unsigned plugins.
    let mut mgr = PluginManager::new(vec![], false);
    mgr.set_allow_red_plugins(true);
    let names = mgr
        .load_plugin(&staged)
        .expect("allow_red_plugins permits unsigned plugins");
    assert_eq!(names.len(), 4);
    for name in &names {
        assert_eq!(
            mgr.get(name).expect("registered").trust_level,
            TrustLevel::Red,
            "{name} stays Red when permitted"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unsigned_plugin_allowed_in_dev_mode_as_red() {
    let (staged, dir) = stage_bundle("dev_mode");

    let mut mgr = PluginManager::new(vec![], true);
    let names = mgr
        .load_plugin(&staged)
        .expect("dev_mode permits unsigned plugins");
    assert_eq!(names.len(), 4);
    for name in &names {
        assert_eq!(
            mgr.get(name).expect("registered").trust_level,
            TrustLevel::Red,
            "dev mode assigns Red trust"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn multi_anchor_any_matching_key_grants_blue() {
    let (staged, dir) = stage_bundle("multi_anchor");
    let signing_key = key(7);
    sign(&staged, &signing_key);

    // Two anchors active; the bundle is signed by the second one.
    let mut mgr = PluginManager::new(vec![], false);
    mgr.set_trust_anchors(vec![
        key(9).verifying_key().to_bytes(),
        signing_key.verifying_key().to_bytes(),
    ]);

    let names = mgr
        .load_plugin(&staged)
        .expect("signature verifies against one of the active anchors");
    for name in &names {
        assert_eq!(
            mgr.get(name).expect("registered").trust_level,
            TrustLevel::Blue,
            "{name} verifies → Blue"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn revoked_anchor_rejects_signature() {
    let (staged, dir) = stage_bundle("revoked");
    let signing_key = key(7);
    sign(&staged, &signing_key);
    let fp = fingerprint_key(&signing_key.verifying_key());

    let mut mgr = PluginManager::new(vec![], false);
    mgr.set_trust_anchor(signing_key.verifying_key().to_bytes());
    mgr.revoke_trust_anchor(fp);

    // Revoking the only anchor empties the active set → the plugin is Red and
    // the Red gate rejects it outside dev mode. A revoked key can never grant
    // Blue.
    let err = mgr
        .load_plugin(&staged)
        .expect_err("revoked key's signature must not grant Blue");
    assert!(
        matches!(err, PluginError::UnsignedRejected { .. }),
        "expected UnsignedRejected (no active anchor remains), got: {err:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn load_trust_store_parses_active_and_revoked() {
    let (staged, dir) = stage_bundle("trust_store");
    let key_a = key(7);
    let key_b = key(8);

    // key_b is listed as ACTIVE but its fingerprint is on the CRL — the CRL
    // wins (overlap defense). key_a is untouched.
    let mut active_pub = String::new();
    for k in [&key_a, &key_b] {
        let hex_pub = k.verifying_key().to_bytes();
        active_pub.push_str(&hex::encode(hex_pub));
        active_pub.push('\n');
    }
    std::fs::write(dir.join("active.pub"), active_pub).unwrap();
    let fp_b = fingerprint_key(&key_b.verifying_key());
    std::fs::write(
        dir.join("revoked.txt"),
        format!("{} compromised 1750000000\n", hex::encode(fp_b)),
    )
    .unwrap();

    let mut mgr = PluginManager::new(vec![], false);
    mgr.load_trust_store(&dir).expect("store loads");

    // Signed by the revoked key → rejected with a "revoked" reason.
    sign(&staged, &key_b);
    let err = mgr
        .load_plugin(&staged)
        .expect_err("key on the CRL can never grant Blue");
    match err {
        PluginError::SignatureInvalid { reason, .. } => assert!(
            reason.contains("revoked"),
            "reason should mention revocation, got: {reason}"
        ),
        other => panic!("expected SignatureInvalid, got: {other:?}"),
    }

    // Re-signed by the clean key → Blue. (Signing never mutates the bundle
    // bytes, so overwriting the sidecar is enough.)
    sign(&staged, &key_a);
    let names = mgr
        .load_plugin(&staged)
        .expect("key not on the CRL grants Blue");
    for name in &names {
        assert_eq!(
            mgr.get(name).expect("registered").trust_level,
            TrustLevel::Blue,
            "{name} verifies → Blue"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn load_trust_store_rejects_malformed_lines() {
    let dir = std::env::temp_dir().join(format!("dologger-store-bad-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // 63 hex chars — not a valid Ed25519 public key.
    std::fs::write(dir.join("active.pub"), "a3f8b2c1\n").unwrap();
    std::fs::write(dir.join("revoked.txt"), "").unwrap();

    let mut mgr = PluginManager::new(vec![], false);
    let err = mgr.load_trust_store(&dir).expect_err("bad store must fail");
    assert!(
        err.contains("expected 32 bytes") || err.contains("hex"),
        "unexpected error: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn set_trust_anchor_compat_replaces_active_set() {
    let (staged, dir) = stage_bundle("compat_shim");
    let signing_key = key(7);
    sign(&staged, &signing_key);

    let mut mgr = PluginManager::new(vec![], false);
    mgr.set_trust_anchor(key(9).verifying_key().to_bytes());
    mgr.set_trust_anchor(signing_key.verifying_key().to_bytes());

    // The shim replaced the earlier anchor; the bundle signed by key(7) is Blue.
    let names = mgr
        .load_plugin(&staged)
        .expect("latest set_trust_anchor wins");
    for name in &names {
        assert_eq!(
            mgr.get(name).expect("registered").trust_level,
            TrustLevel::Blue
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
