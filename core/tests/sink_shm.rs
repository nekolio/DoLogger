//! Integration tests for the WS-2 `[shm]` wiring: TOML config → Engine →
//! shared-memory ring buffer, verified through the core `read_status` API.

use std::time::{Duration, Instant};

use dologger_core::buffer::RecordPtr;
use dologger_core::config::DologgerConfig;
use dologger_core::record::{thread_id_u64, LogLevel};
use dologger_core::sink::shm::read_status;
use dologger_core::sink::{ShmFullPolicy, SHM_MAGIC, SHM_VERSION};
use dologger_core::sys::TimeSource;
use dologger_core::Engine;

/// Per-process-unique shared-memory path for the given label.
fn shm_path(label: &str) -> String {
    format!("/dologger_test_{}_{label}.shm", std::process::id())
}

/// Wait until the ring buffer drains (or timeout) so shm writes have landed.
fn wait_drain(engine: &Engine, timeout: Duration) {
    let start = Instant::now();
    while !engine.ring_buffer.is_empty() {
        if start.elapsed() > timeout {
            panic!("timeout waiting for ring buffer to drain");
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn push_info_record(engine: &Engine, ts: &TimeSource, msg: &str) {
    let tid = thread_id_u64();
    let pid = std::process::id();
    let record_ptr = engine.pool.alloc().expect("pool exhausted");
    // SAFETY: record_ptr was allocated from engine.pool and is exclusively owned.
    unsafe {
        let record = &mut *record_ptr;
        let id = ts.next_id();
        record.set_id(id.hi, id.lo);
        record.timestamp = ts.now_nanos();
        record.level = LogLevel::Info;
        record.message.set(msg);
        record.thread_id = tid as u32;
        record.process_id = pid;
        record.set_process_name("sink_shm_test");
        record.set_host_name("localhost");
        record.set_environment("test");
    }
    // SAFETY: record_ptr is a live engine pool allocation and ownership moves
    // into the ring token exactly once.
    engine
        .ring_buffer
        .try_push(unsafe { RecordPtr::from_raw(record_ptr) })
        .expect("ring buffer accepts record");
}

// ---------------------------------------------------------------------------
// Config parsing
// ---------------------------------------------------------------------------

#[test]
fn config_parses_shm_section() {
    let toml = r#"
[dologger]
level = "INFO"

[shm]
path = "/custom.shm"
buffer_size_mb = 16
slot_size_kb = 128
full_policy = "drop_oldest"
"#;
    let (config, warnings) = DologgerConfig::parse(toml, None).expect("config parses");
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");

    let shm = config.shm.expect("[shm] must be parsed");
    assert_eq!(shm.path, "/custom.shm");
    assert_eq!(shm.buffer_size_mb, 16);
    assert_eq!(shm.slot_size_kb, 128);
    assert_eq!(shm.full_policy, ShmFullPolicy::DropOldest);
    assert_eq!(shm.input_format, "sif", "input_format is forced to sif");
}

#[test]
fn config_absent_shm_section_is_disabled() {
    let (config, warnings) =
        DologgerConfig::parse("[dologger]\nlevel = \"INFO\"\n", None).expect("config parses");
    assert!(warnings.is_empty());
    assert!(config.shm.is_none(), "no [shm] section -> shm disabled");
}

#[test]
fn config_invalid_shm_section_is_warned_not_fatal() {
    let (config, warnings) = DologgerConfig::parse(
        r#"
[shm]
full_policy = "not_a_valid_policy"
"#,
        None,
    )
    .expect("config parses despite a bad [shm] section");
    assert!(config.shm.is_none(), "invalid [shm] -> shm disabled");
    assert_eq!(warnings.len(), 1, "one warning for the bad [shm] section");
}

// ---------------------------------------------------------------------------
// Engine wiring + shared-memory watermark
// ---------------------------------------------------------------------------

#[test]
fn engine_writes_records_into_shm_and_advances_watermark() {
    let path = shm_path("seq");
    let toml = format!(
        r#"
[dologger]
level = "INFO"

[shm]
path = "{path}"
"#
    );
    let (config, warnings) = DologgerConfig::parse(&toml, None).expect("config parses");
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");

    let mut engine = Engine::init(config).expect("engine initialises with [shm]");

    let ts = TimeSource::new();
    const N: usize = 100;
    for i in 0..N {
        push_info_record(&engine, &ts, &format!("shm record {i}"));
    }
    wait_drain(&engine, Duration::from_secs(10));

    // Verify the live region through the read-only core API.
    let status = read_status(&path).expect("region is readable");
    assert_eq!(status.magic, SHM_MAGIC, "magic matches");
    assert_eq!(status.version, SHM_VERSION, "version matches");
    assert!(
        status.producer_alive,
        "producer flag is alive while running"
    );
    assert_eq!(
        status.producer_seq, N as u64,
        "every accepted record advanced the producer sequence"
    );
    assert!(
        status.consumer_seq <= status.producer_seq,
        "watermark never exceeds the producer sequence"
    );

    engine.shutdown();
}

#[test]
fn engine_rejects_shm_in_audit_signature_mode() {
    // AUDIT/ProdAudit enables signatures; sink_shm is forbidden in that mode.
    let path = shm_path("audit");
    let toml = format!(
        r#"
[dologger]
performance_profile = "prod-audit"

[shm]
path = "{path}"
"#
    );
    let (config, _) = DologgerConfig::parse(&toml, None).expect("config parses");
    assert!(config.enable_signature, "prod-audit forces signatures on");

    let result = Engine::init(config);
    let err = match result {
        Ok(_) => panic!("engine must reject shm in audit mode"),
        Err(e) => e,
    };
    assert!(
        err.contains("DO_LOG_ERR_AUDIT_SHM_FORBIDDEN"),
        "expected audit-forbidden error, got: {err}"
    );
}

#[test]
fn read_status_errors_gracefully_on_missing_region() {
    let missing = shm_path("missing");
    let err = read_status(&missing).expect_err("missing region must error");
    assert!(
        !err.is_empty(),
        "a descriptive error is returned for a missing region"
    );
}

// ---------------------------------------------------------------------------
// read_status round-trip against a raw ShmSink
// ---------------------------------------------------------------------------

#[test]
fn read_status_round_trips_sink_writes() {
    use dologger_core::sink::ShmSink;
    use dologger_core::sink::ShmSinkConfig;
    use dologger_core::sys::Sysmon;

    let path = shm_path("rt");
    let sysmon = Sysmon::start();
    let sink = ShmSink::new(ShmSinkConfig {
        path: path.clone(),
        buffer_size_mb: 8,
        ..ShmSinkConfig::default()
    });
    sink.open(&sysmon).expect("sink opens");

    // A minimal structurally framed SIF payload for slot transport coverage.
    let mut sif: Vec<u8> = Vec::new();
    sif.extend_from_slice(b"SIF\0");
    sif.extend_from_slice(&32u16.to_le_bytes());
    sif.extend_from_slice(&0u16.to_le_bytes());
    sif.extend_from_slice(&32u32.to_le_bytes());
    sif.extend_from_slice(&0u32.to_le_bytes());
    sif.extend_from_slice(&0u32.to_le_bytes());
    sif.extend_from_slice(&0u32.to_le_bytes());
    sif.extend_from_slice(&0u32.to_le_bytes());

    const N: usize = 10;
    for _ in 0..N {
        assert!(sink.write(&sif), "write accepted");
    }

    let status = read_status(&path).expect("region readable after writes");
    assert_eq!(
        status.producer_seq, N as u64,
        "producer seq reflects writes"
    );
    assert_eq!(status.buffer_size_bytes, 8 * 1024 * 1024);

    sink.close(&sysmon);
}
