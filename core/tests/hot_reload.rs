//! Integration tests for `Engine::reload_config` hot reload.
//!
//! These exercise the swappable `SinkRef` path: a successful reload must swap
//! the active config, while a reload whose sink fails to build/open must be
//! rejected and leave the previous config untouched.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dologger_core::config::DologgerConfig;
use dologger_core::config::InputEncodingMode;
use dologger_core::config::{ConfigWatcher, WatcherBackend, WatcherConfig};
use dologger_core::error::{
    DO_LOG_ERR_CONFIG_HOT_RELOAD_FAILED, DO_LOG_ERR_CONFIG_RESTART_REQUIRED,
};
use dologger_core::sink::{DurabilityLevel, FileSinkConfig, SinkKindConfig};
use dologger_core::Engine;

fn config_with_level(level: &str) -> DologgerConfig {
    let mut config = DologgerConfig::dev_profile();
    config.level = level.to_string();
    config
}

fn config_with_bad_file_sink(level: &str) -> DologgerConfig {
    let mut config = config_with_level(level);
    // Point the file sink at an existing directory: opening it as a file fails,
    // which must surface as DO_LOG_ERR_CONFIG_HOT_RELOAD_FAILED on reload.
    config.sinks = vec![SinkKindConfig::File(FileSinkConfig {
        path: std::env::temp_dir(),
        max_size: 1024 * 1024,
        fsync_on_write: false,
        durability_level: DurabilityLevel::Media,
        buffer_size: 1024,
    })];
    config
}

#[test]
fn reload_config_success_swaps_config() {
    let engine = Engine::init(config_with_level("INFO")).expect("engine init");
    let mut engine = engine;

    let new_config = config_with_level("DEBUG");
    engine
        .reload_config(new_config)
        .expect("reload with a valid sink must succeed");

    assert_eq!(engine.config.level, "DEBUG");
}

#[test]
fn reload_config_failure_keeps_previous_config() {
    let engine = Engine::init(config_with_level("INFO")).expect("engine init");
    let mut engine = engine;

    let err = engine
        .reload_config(config_with_bad_file_sink("DEBUG"))
        .expect_err("reload with an un-openable sink must fail");

    assert_eq!(
        err, DO_LOG_ERR_CONFIG_HOT_RELOAD_FAILED,
        "expected DO_LOG_ERR_CONFIG_HOT_RELOAD_FAILED, got {err}"
    );

    // The previous config must be preserved on failure.
    assert_eq!(engine.config.level, "INFO");
}

#[test]
fn reload_config_applies_hot_fields_but_protects_encoding() {
    let engine = Engine::init(config_with_level("INFO")).expect("engine init");
    let mut engine = engine;
    let mut new_config = config_with_level("DEBUG");
    new_config.encoding.input = InputEncodingMode::Auto;

    let err = engine
        .reload_config(new_config)
        .expect_err("protected encoding change must require restart");

    assert_eq!(err, DO_LOG_ERR_CONFIG_RESTART_REQUIRED);
    assert_eq!(engine.config.level, "DEBUG");
    assert_eq!(engine.config.encoding.input, InputEncodingMode::Utf8);
}

/// End-to-end wiring check: a `ConfigWatcher` that re-parses a config file and
/// calls `Engine::reload_config` on change. Uses the polling backend (short
/// interval) so the test is portable and not tied to a native kernel backend.
#[test]
fn config_watcher_drives_engine_reload() {
    use std::fs;
    use std::io::Write;

    let dir = std::env::temp_dir().join(format!("dologger_reload_{}", std::process::id()));
    let _ = fs::create_dir_all(&dir);
    let config_path = dir.join("dologger.toml");
    fs::write(&config_path, "[dologger]\nlevel = \"INFO\"\n").expect("write config");

    let (initial, _) = DologgerConfig::parse(
        &fs::read_to_string(&config_path).unwrap(),
        Some(PathBuf::from(&config_path)),
    )
    .expect("parse");
    let engine = Arc::new(Mutex::new(Engine::init(initial).expect("engine init")));

    let engine_for_reload = Arc::clone(&engine);
    let watch = config_path.clone();
    let watcher_config = WatcherConfig {
        backend: WatcherBackend::Polling,
        poll_interval_ms: 50,
        debounce_ms: 50,
        enabled: true,
    };
    let _watcher = ConfigWatcher::start(
        vec![config_path.clone()],
        Box::new(move |_p: &std::path::Path| {
            let content = fs::read_to_string(&watch).map_err(|e| e.to_string())?;
            let (config, _) =
                DologgerConfig::parse(&content, Some(watch.clone())).map_err(|(_, m)| m)?;
            engine_for_reload
                .lock()
                .unwrap()
                .reload_config(config)
                .map_err(|e| format!("reload: {e}"))
        }),
        watcher_config,
    )
    .expect("watcher starts");

    // Give the watcher a beat to arm, then change the config file.
    std::thread::sleep(Duration::from_millis(150));
    let mut f = fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&config_path)
        .unwrap();
    writeln!(f, "[dologger]\nlevel = \"WARN\"\n").unwrap();
    drop(f);

    // Poll until the reload lands (generous budget; polling + debounce).
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if engine.lock().unwrap().config.level == "WARN" {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "reload did not propagate to the engine"
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    let _ = fs::remove_dir_all(&dir);
}
