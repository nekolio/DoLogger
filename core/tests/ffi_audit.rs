use std::ffi::CString;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use dologger_core::error::DO_LOG_OK;
use dologger_core::ffi::{dologger_init, dologger_log, dologger_log_params, dologger_shutdown};
use dologger_core::record::LogLevel;
use dologger_core::DologgerError;

fn test_directory() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("dologger-ffi-audit-{nonce}"))
}

#[test]
fn c_abi_audit_uses_configured_worm_and_security_paths() {
    let directory = test_directory();
    fs::create_dir_all(&directory).expect("create test directory");
    let config_path = directory.join("dologger.toml");
    let worm_path = directory.join("audit.worm");
    let security_path = directory.join("security.log");
    let worm_config_path = worm_path.to_string_lossy().replace('\\', "/");
    let security_config_path = security_path.to_string_lossy().replace('\\', "/");
    let config = format!(
        "[dologger]\nperformance_profile = \"balanced\"\nring_buffer_size = 1024\nbatch_size = 1\nenable_audit = true\nenable_signature = false\nring_buffer_coop_helping = false\naudit_worm_path = \"{}\"\naudit_security_path = \"{}\"\n",
        worm_config_path, security_config_path
    );
    fs::write(&config_path, config).expect("write test config");

    let config_c = CString::new(config_path.to_string_lossy().as_bytes())
        .expect("config path must not contain NUL");
    let mut error = DologgerError::new();
    let handle = dologger_init(config_c.as_ptr(), &mut error);
    assert!(!handle.is_null(), "dologger_init failed: {error:?}");
    assert_eq!(error.code, DO_LOG_OK);

    let message = CString::new("ffi audit integration").expect("message must not contain NUL");
    let mut params: dologger_log_params = unsafe { std::mem::zeroed() };
    params.level = LogLevel::Audit as u8;
    params.message = message.as_ptr();
    let result = dologger_log(handle, &params);
    assert_eq!(result, DO_LOG_OK);

    dologger_shutdown(handle);

    let worm = fs::read_to_string(&worm_path).expect("configured WORM file must be written");
    let security =
        fs::read_to_string(&security_path).expect("configured Security file must be written");
    let envelope = worm.lines().next().expect("WORM record line");
    let content_hash = envelope
        .split("content_hash=")
        .nth(1)
        .and_then(|value| value.split(';').next())
        .expect("WORM envelope content hash")
        .to_string();
    assert_eq!(content_hash.len(), 64);
    assert!(content_hash.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert!(envelope.starts_with("lsn=1;"));
    assert!(envelope.contains("record="));

    let fields: Vec<_> = security
        .lines()
        .next()
        .expect("Security record line")
        .split('|')
        .collect();
    assert_eq!(fields.first().copied(), Some("1"));
    assert_eq!(fields.get(2).copied(), Some("AUDIT"));
    assert_eq!(fields.get(6).copied(), Some("ffi audit integration"));
    assert_eq!(fields.get(7).map(|value| value.len()), Some(16));
    assert_eq!(fields.get(7).copied(), Some(&content_hash[..16]));

    fs::remove_dir_all(directory).expect("remove test directory");
}
