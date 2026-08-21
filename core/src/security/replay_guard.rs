//! Bounded audit replay protection.
//!
//! KV frames make record transport explicit, but transport integrity is not
//! replay protection. This module provides a small, allocation-bounded guard
//! for audit consumers. It tracks a sliding LSN window and the content hash
//! associated with each accepted LSN. It does not replace signatures or the
//! WORM chain; it prevents duplicate delivery and obvious reordering from
//! being silently treated as fresh audit evidence.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::Mutex;

/// Result of submitting one audit record to a replay guard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum ReplayDecision {
    /// The record is new and was inserted into the active window.
    Accepted,
    /// The exact LSN/hash pair was already accepted.
    Duplicate,
    /// The LSN was seen before with a different hash.
    HashConflict,
    /// The record is older than the retained window.
    TooOld,
    /// The record skips one or more LSNs.
    Gap { expected: u64, found: u64 },
    /// LSN zero is reserved and cannot enter the audit chain.
    InvalidLsn,
}

impl ReplayDecision {
    /// Whether the record may proceed under a permissive consumer policy.
    pub const fn is_accepted(self) -> bool {
        matches!(self, Self::Accepted)
    }

    /// Whether the decision indicates a security-relevant conflict.
    pub const fn is_security_event(self) -> bool {
        matches!(
            self,
            Self::HashConflict | Self::TooOld | Self::Gap { .. } | Self::InvalidLsn
        )
    }
}

/// Policy for a replay window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplayPolicy {
    /// Number of LSNs retained for duplicate detection.
    pub window_size: usize,
    /// Whether a forward LSN gap is rejected instead of accepted.
    pub reject_gaps: bool,
    /// Whether a duplicate exact pair is treated as an error by callers.
    pub reject_duplicates: bool,
}

impl Default for ReplayPolicy {
    fn default() -> Self {
        Self {
            window_size: 4096,
            reject_gaps: true,
            reject_duplicates: false,
        }
    }
}

impl ReplayPolicy {
    /// Validate policy limits before a guard is created.
    pub const fn validate(self) -> Result<(), ReplayPolicyError> {
        if self.window_size == 0 || self.window_size > 1_000_000 {
            return Err(ReplayPolicyError::InvalidWindow);
        }
        Ok(())
    }
}

/// Invalid replay policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayPolicyError {
    /// Window must be between one and one million entries.
    InvalidWindow,
}

impl fmt::Display for ReplayPolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidWindow => f.write_str("replay window must be between 1 and 1000000"),
        }
    }
}

impl std::error::Error for ReplayPolicyError {}

#[derive(Debug)]
struct ReplayState {
    highest_lsn: u64,
    entries: HashMap<u64, [u8; 32]>,
    order: VecDeque<u64>,
    accepted: u64,
    duplicates: u64,
    conflicts: u64,
    too_old: u64,
    gaps: u64,
}

impl ReplayState {
    fn new(capacity: usize) -> Self {
        Self {
            highest_lsn: 0,
            entries: HashMap::with_capacity(capacity),
            order: VecDeque::with_capacity(capacity),
            accepted: 0,
            duplicates: 0,
            conflicts: 0,
            too_old: 0,
            gaps: 0,
        }
    }

    fn evict_to(&mut self, capacity: usize) {
        while self.order.len() > capacity {
            if let Some(lsn) = self.order.pop_front() {
                self.entries.remove(&lsn);
            }
        }
    }
}

/// Thread-safe, bounded replay guard.
pub struct ReplayGuard {
    policy: ReplayPolicy,
    state: Mutex<ReplayState>,
}

impl fmt::Debug for ReplayGuard {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let snapshot = self.snapshot();
        formatter
            .debug_struct("ReplayGuard")
            .field("policy", &self.policy)
            .field("snapshot", &snapshot)
            .finish()
    }
}

/// Immutable statistics snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplayStats {
    /// Highest LSN observed.
    pub highest_lsn: u64,
    /// Number of active entries.
    pub retained: usize,
    /// Accepted new records.
    pub accepted: u64,
    /// Exact duplicates.
    pub duplicates: u64,
    /// Same LSN with a different hash.
    pub conflicts: u64,
    /// Records older than the window.
    pub too_old: u64,
    /// Forward gaps observed.
    pub gaps: u64,
}

impl ReplayGuard {
    /// Construct a bounded guard.
    pub fn new(policy: ReplayPolicy) -> Result<Self, ReplayPolicyError> {
        policy.validate()?;
        Ok(Self {
            state: Mutex::new(ReplayState::new(policy.window_size)),
            policy,
        })
    }

    /// Construct the default production guard.
    pub fn production() -> Self {
        Self::new(ReplayPolicy::default()).expect("default replay policy is valid")
    }

    /// Submit one LSN/hash pair.
    pub fn observe(&self, lsn: u64, content_hash: [u8; 32]) -> ReplayDecision {
        if lsn == 0 {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.conflicts = state.conflicts.saturating_add(1);
            return ReplayDecision::InvalidLsn;
        }
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(previous) = state.entries.get(&lsn) {
            if previous == &content_hash {
                state.duplicates = state.duplicates.saturating_add(1);
                return ReplayDecision::Duplicate;
            }
            state.conflicts = state.conflicts.saturating_add(1);
            return ReplayDecision::HashConflict;
        }
        if state.highest_lsn > self.policy.window_size as u64
            && lsn.saturating_add(self.policy.window_size as u64) <= state.highest_lsn
        {
            state.too_old = state.too_old.saturating_add(1);
            return ReplayDecision::TooOld;
        }
        if self.policy.reject_gaps && lsn > state.highest_lsn.saturating_add(1) {
            state.gaps = state.gaps.saturating_add(1);
            return ReplayDecision::Gap {
                expected: state.highest_lsn.saturating_add(1),
                found: lsn,
            };
        }
        if lsn > state.highest_lsn {
            state.highest_lsn = lsn;
        }
        state.entries.insert(lsn, content_hash);
        state.order.push_back(lsn);
        state.evict_to(self.policy.window_size);
        state.accepted = state.accepted.saturating_add(1);
        ReplayDecision::Accepted
    }

    /// Submit a Record's audit identity.
    pub fn observe_record(&self, record: &crate::record::Record) -> ReplayDecision {
        self.observe(record.lsn, record.content_hash)
    }

    /// Return current bounded statistics.
    pub fn snapshot(&self) -> ReplayStats {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        ReplayStats {
            highest_lsn: state.highest_lsn,
            retained: state.entries.len(),
            accepted: state.accepted,
            duplicates: state.duplicates,
            conflicts: state.conflicts,
            too_old: state.too_old,
            gaps: state.gaps,
        }
    }

    /// Clear all retained identities while preserving the configured policy.
    pub fn reset(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *state = ReplayState::new(self.policy.window_size);
    }

    /// Return the configured window size.
    pub const fn window_size(&self) -> usize {
        self.policy.window_size
    }

    /// Return whether the caller should reject exact duplicates.
    pub const fn reject_duplicates(&self) -> bool {
        self.policy.reject_duplicates
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(value: u8) -> [u8; 32] {
        [value; 32]
    }

    #[test]
    fn production_policy_is_bounded() {
        let guard = ReplayGuard::production();
        assert_eq!(guard.window_size(), 4096);
        assert_eq!(guard.observe(1, hash(1)), ReplayDecision::Accepted);
    }

    #[test]
    fn zero_lsn_is_rejected() {
        let guard = ReplayGuard::production();
        assert_eq!(guard.observe(0, hash(0)), ReplayDecision::InvalidLsn);
        assert_eq!(guard.snapshot().accepted, 0);
    }

    #[test]
    fn exact_duplicate_is_distinguished_from_conflict() {
        let guard = ReplayGuard::production();
        assert_eq!(guard.observe(1, hash(1)), ReplayDecision::Accepted);
        assert_eq!(guard.observe(1, hash(1)), ReplayDecision::Duplicate);
        assert_eq!(guard.observe(1, hash(2)), ReplayDecision::HashConflict);
        let stats = guard.snapshot();
        assert_eq!(stats.duplicates, 1);
        assert_eq!(stats.conflicts, 1);
    }

    #[test]
    fn strict_policy_rejects_forward_gaps() {
        let guard = ReplayGuard::new(ReplayPolicy {
            window_size: 8,
            reject_gaps: true,
            reject_duplicates: false,
        })
        .unwrap();
        assert_eq!(
            guard.observe(2, hash(2)),
            ReplayDecision::Gap {
                expected: 1,
                found: 2
            }
        );
        assert_eq!(guard.observe(1, hash(1)), ReplayDecision::Accepted);
        assert_eq!(guard.observe(2, hash(2)), ReplayDecision::Accepted);
    }

    #[test]
    fn permissive_policy_accepts_forward_gaps() {
        let guard = ReplayGuard::new(ReplayPolicy {
            window_size: 8,
            reject_gaps: false,
            reject_duplicates: false,
        })
        .unwrap();
        assert_eq!(guard.observe(10, hash(10)), ReplayDecision::Accepted);
        assert_eq!(guard.observe(9, hash(9)), ReplayDecision::Accepted);
    }

    #[test]
    fn old_entries_are_evicted_and_rejected() {
        let guard = ReplayGuard::new(ReplayPolicy {
            window_size: 2,
            reject_gaps: false,
            reject_duplicates: false,
        })
        .unwrap();
        for lsn in 1..=4 {
            assert_eq!(
                guard.observe(lsn, hash(lsn as u8)),
                ReplayDecision::Accepted
            );
        }
        assert_eq!(guard.snapshot().retained, 2);
        assert_eq!(guard.observe(1, hash(1)), ReplayDecision::TooOld);
    }

    #[test]
    fn reset_removes_history() {
        let guard = ReplayGuard::production();
        guard.observe(1, hash(1));
        guard.reset();
        assert_eq!(
            guard.snapshot(),
            ReplayStats {
                highest_lsn: 0,
                retained: 0,
                accepted: 0,
                duplicates: 0,
                conflicts: 0,
                too_old: 0,
                gaps: 0
            }
        );
    }

    #[test]
    fn invalid_policy_is_rejected() {
        assert!(matches!(
            ReplayGuard::new(ReplayPolicy {
                window_size: 0,
                ..Default::default()
            }),
            Err(ReplayPolicyError::InvalidWindow)
        ));
        assert!(matches!(
            ReplayGuard::new(ReplayPolicy {
                window_size: 1_000_001,
                ..Default::default()
            }),
            Err(ReplayPolicyError::InvalidWindow)
        ));
    }

    #[test]
    fn decision_helpers_are_stable() {
        assert!(ReplayDecision::Accepted.is_accepted());
        assert!(!ReplayDecision::Duplicate.is_accepted());
        assert!(ReplayDecision::HashConflict.is_security_event());
        assert!(ReplayDecision::Gap {
            expected: 1,
            found: 2
        }
        .is_security_event());
    }
}
