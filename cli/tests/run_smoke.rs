//! Integration tests for `dologctl run` — verifies the engine startup
//! → graceful-shutdown path that `cmd_run` exercises, minus the
//! blocking signal-wait (we don't actually send Ctrl-C in tests).

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use dologger_core::config::DologgerConfig;
use dologger_core::Engine;

/// A per-process-unique temp config path so parallel tests don't collide.
fn temp_config_path(label: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "dologctl_run_{}_{n}_{label}.toml",
        std::process::id()
    ));
    p
}

#[test]
fn engine_init_and_shutdown_round_trip_with_minimal_config() {
    // What `dologctl run` does after the signal arrives: load config,
    // construct Engine, then shutdown.  Mirrors the body of cmd_run.
    let config = DologgerConfig::dev_profile();

    let mut engine = Engine::init(config).expect("engine init succeeds with dev profile");
    engine.shutdown();
}

#[test]
fn engine_init_succeeds_with_parsed_toml_config() {
    let path = temp_config_path("parsed");
    let toml_str = r#"
[dologger]
level = "INFO"
performance_profile = "dev"
ring_buffer_size = 4096
batch_size = 32
enable_signature = false

[sinks.console]
type = "console"
use_stderr = false
"#;
    std::fs::write(&path, toml_str).expect("write tmp config");

    let content = std::fs::read_to_string(&path).expect("read tmp config");
    let (config, warnings) =
        DologgerConfig::parse(&content, Some(path.clone())).expect("config parses");
    assert!(warnings.is_empty(), "no warnings: {warnings:?}");
    assert_eq!(config.ring_buffer_size, 4096);

    let mut engine = Engine::init(config).expect("engine init");
    engine.shutdown();

    let _ = std::fs::remove_file(&path);
}

#[test]
fn malformed_config_surfaces_parse_error() {
    let bad = "this is not valid [toml";
    let result = DologgerConfig::parse(bad, None);
    assert!(
        result.is_err(),
        "malformed TOML must be rejected, got {result:?}"
    );
}
