//! End-to-end plugin-bundle tests through `PluginManager`.
//!
//! These exercise the real host path (`PluginManager::load_plugin` → dlopen →
//! `plugin_query_multi` → register every entry from one library handle)
//! against the actual `dologger-official-plugins` cdylib.
//!
//! Requires the bundle to be built first: `cargo build --workspace`
//! (`cargo test --workspace` builds all members first, so this is satisfied
//! in CI). If the artifact is missing, the tests skip with a hint instead of
//! failing.

use std::path::PathBuf;

use dologger_core::plugin::{PluginManager, TrustLevel};

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

#[test]
fn manager_registers_all_bundle_plugins_from_one_library() {
    let Some(path) = bundle_library_path() else {
        eprintln!("SKIP: bundle not built yet — run `cargo build --workspace`");
        return;
    };

    let mut mgr = PluginManager::new(vec![], true);
    let names = mgr.load_plugin(&path).expect("bundle loads via dlopen");
    assert_eq!(
        names.len(),
        4,
        "ONE library must register all 4 official plugins"
    );
    for expected in [
        "formatter-json",
        "formatter-text",
        "filter-level",
        "field-container",
    ] {
        assert!(
            names.iter().any(|n| n == expected),
            "missing {expected}: {names:?}"
        );
    }

    for name in &names {
        let plugin = mgr.get(name).expect("registered plugin is queryable");
        assert_eq!(
            plugin.info.abi_version,
            dologger_core::plugin::CORE_ABI_VERSION,
            "{name} declares the matching core ABI"
        );
        assert_ne!(plugin.info.phase, 0, "{name} has a non-empty phase mask");
        assert_eq!(
            plugin.trust_level,
            TrustLevel::Red,
            "dev_mode assigns Red (unsigned) trust to every bundle plugin"
        );
        assert_eq!(plugin.info.name, *name);
    }
}

#[test]
fn manager_duplicate_bundle_load_is_rejected() {
    let Some(path) = bundle_library_path() else {
        eprintln!("SKIP: bundle not built yet — run `cargo build --workspace`");
        return;
    };

    let mut mgr = PluginManager::new(vec![], true);
    mgr.load_plugin(&path).expect("first load succeeds");
    let err = mgr
        .load_plugin(&path)
        .expect_err("second load of the same library must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("already loaded"),
        "expected AlreadyLoaded, got: {msg}"
    );
}

#[test]
fn manager_discover_picks_up_bundle_in_plugins_dir() {
    // Full discovery path: place the bundle in a temp plugins dir and let
    // `discover` find + load it, proving the search-path integration.
    let Some(path) = bundle_library_path() else {
        eprintln!("SKIP: bundle not built yet — run `cargo build --workspace`");
        return;
    };

    let dir = std::env::temp_dir().join(format!("dologger-plugin-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let staged = dir.join(path.file_name().unwrap());
    std::fs::copy(&path, &staged).unwrap();

    let mut mgr = PluginManager::new(vec![dir.clone()], true);
    let errors = mgr.discover();
    assert!(errors.is_empty(), "discover reported errors: {errors:?}");
    assert_eq!(
        mgr.plugin_count(),
        4,
        "discover loaded all 4 bundle plugins"
    );
    assert_eq!(mgr.plugin_names().len(), 4);

    // Cleanup the temp dir.
    let _ = std::fs::remove_dir_all(&dir);
}
