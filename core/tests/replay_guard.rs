//! Replay guard integration coverage.

use dologger_core::security::{ReplayDecision, ReplayGuard, ReplayPolicy};

#[test]
fn out_of_order_records_within_window_are_supported_when_configured() {
    let guard = ReplayGuard::new(ReplayPolicy {
        window_size: 32,
        reject_gaps: false,
        reject_duplicates: false,
    })
    .unwrap();
    assert_eq!(guard.observe(10, [10; 32]), ReplayDecision::Accepted);
    assert_eq!(guard.observe(9, [9; 32]), ReplayDecision::Accepted);
    assert_eq!(guard.observe(8, [8; 32]), ReplayDecision::Accepted);
    assert_eq!(guard.snapshot().retained, 3);
}

#[test]
fn conflict_is_not_downgraded_to_duplicate() {
    let guard = ReplayGuard::production();
    guard.observe(1, [1; 32]);
    assert!(guard.observe(1, [2; 32]).is_security_event());
    assert_eq!(guard.snapshot().conflicts, 1);
}
