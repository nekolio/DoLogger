//! Integration tests for structured error exits and control-plane parsing.

use dologger_core::error::{
    error_descriptor, ErrorContext, ErrorExit, ErrorOrigin, ErrorReport, DO_LOG_ERR_INVALID_ARG,
    DO_LOG_ERR_KV_INVALID,
};

#[test]
fn error_report_keeps_machine_key_separate_from_detail() {
    let report = ErrorReport::new(
        DO_LOG_ERR_KV_INVALID,
        ErrorContext::new(ErrorOrigin::Serialization, "kv.decode").with_detail("offset=44"),
    );
    assert_eq!(report.key(), "kv.invalid");
    assert_eq!(report.code, DO_LOG_ERR_KV_INVALID);
    assert!(report.diagnostic_message().contains("offset=44"));
    assert_eq!(report.fallback_message(), "KV frame invalid");
}

#[test]
fn result_report_converts_arbitrary_display_errors() {
    let result: Result<(), _> = Err("bad input");
    let report = result
        .report(
            DO_LOG_ERR_INVALID_ARG,
            ErrorContext::new(ErrorOrigin::Api, "ffi.call"),
        )
        .unwrap_err();
    assert_eq!(report.code, DO_LOG_ERR_INVALID_ARG);
    assert!(report.diagnostic_message().contains("bad input"));
}

#[test]
fn descriptors_are_stable_for_wire_codes() {
    assert_eq!(error_descriptor(DO_LOG_ERR_KV_INVALID).key, "kv.invalid");
    assert_eq!(
        error_descriptor(DO_LOG_ERR_KV_INVALID).code,
        DO_LOG_ERR_KV_INVALID
    );
}
