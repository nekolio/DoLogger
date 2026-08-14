//! Integration tests for M4+M5: `[sinks.*]` TOML config → registry →
//! `FanoutSink` double-write consistency.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use dologger_core::config::DologgerConfig;
use dologger_core::sink::{build_fanout, Sink};

/// A per-process-unique temp log path for the given label.
fn temp_path(label: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "dologger_fanout_{}_{n}_{label}.log",
        std::process::id()
    ));
    p
}

#[test]
fn two_file_sinks_receive_identical_writes() {
    let path_a = temp_path("a");
    let path_b = temp_path("b");
    let _ = std::fs::remove_file(&path_a);
    let _ = std::fs::remove_file(&path_b);

    // Escape backslashes: in TOML basic strings they start escape sequences.
    let path_a_str = path_a.display().to_string().replace('\\', "\\\\");
    let path_b_str = path_b.display().to_string().replace('\\', "\\\\");
    let toml_str = format!(
        r#"
[dologger]
level = "INFO"

[sinks.alpha]
type = "file"
path = "{path_a_str}"

[sinks.beta]
type = "file"
path = "{path_b_str}"
"#
    );

    let (config, warnings) = DologgerConfig::parse(&toml_str, None).expect("config parses");
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    assert_eq!(config.sinks.len(), 2, "both sinks must be registered");

    let mut fanout = build_fanout(&config.sinks).expect("sinks build");
    fanout.open().expect("fanout opens");
    fanout.write("hello fanout").expect("write 1");
    fanout.write("second line").expect("write 2");
    fanout
        .write_batch(&["batch one".into(), "batch two".into()])
        .expect("batch write");
    fanout.flush().expect("flush");
    fanout.close().expect("close");

    let content_a = std::fs::read_to_string(&path_a).expect("read sink a");
    let content_b = std::fs::read_to_string(&path_b).expect("read sink b");
    assert_eq!(
        content_a, "hello fanout\nsecond line\nbatch one\nbatch two\n",
        "sink a received all writes in order"
    );
    assert_eq!(content_a, content_b, "fanout must double-write identically");

    let _ = std::fs::remove_file(&path_a);
    let _ = std::fs::remove_file(&path_b);
}

#[test]
fn absent_sinks_section_defaults_to_console() {
    let (config, warnings) =
        DologgerConfig::parse("[dologger]\nlevel = \"INFO\"\n", None).expect("config parses");
    assert!(warnings.is_empty());
    assert_eq!(config.sinks.len(), 1, "console default must be present");
    assert!(matches!(
        config.sinks[0],
        dologger_core::sink::SinkKindConfig::Console(_)
    ));
}

#[test]
fn invalid_sink_entry_is_warned_not_fatal() {
    // `use_stderr` is a bool; the string value fails deserialisation, so the
    // bad entry is warned and skipped while the good one survives.
    let (config, warnings) = DologgerConfig::parse(
        r#"
[sinks.bad]
type = "console"
use_stderr = "not-a-bool"

[sinks.good]
type = "console"
"#,
        None,
    )
    .expect("config parses despite a bad sink");
    assert_eq!(config.sinks.len(), 1, "good sink only");
    assert!(matches!(
        config.sinks[0],
        dologger_core::sink::SinkKindConfig::Console(_)
    ));
    assert_eq!(warnings.len(), 1, "one warning for the bad sink");
}

#[test]
fn unknown_sink_type_is_warned_and_console_default_applies() {
    let (config, warnings) = DologgerConfig::parse(
        r#"
[sinks.mystery]
type = "nonexistent"
"#,
        None,
    )
    .expect("config parses");
    // The only sink is invalid → nothing valid remains → console default.
    assert_eq!(config.sinks.len(), 1);
    assert!(matches!(
        config.sinks[0],
        dologger_core::sink::SinkKindConfig::Console(_)
    ));
    assert_eq!(warnings.len(), 1);
}
