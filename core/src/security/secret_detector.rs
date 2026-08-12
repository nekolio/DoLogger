//! Secret leak detection Processor.
//!
//! Scans log messages for potential credential leaks before they reach
//! any Sink. Masks or blocks messages containing secrets. This is a
//! critical defense-in-depth measure per the design document's security
//! priorities (Security > Performance > Ecosystem).
//!
//! # Detection approach
//!
//! Uses prefix-pattern matching (no regex dependency — keeps the
//! core engine lean per the dependency strategy). Upgrade to regex-based
//! patterns when a suitable lightweight crate is identified.
//!
//! # Behavior per trust level
//!
//! - Blue plugins: apply all rules, mask on high confidence, warn on medium
//! - Yellow plugins: apply only high confidence rules (strict mode)
//! - Red plugins: CRITICAL rules only (critical_only mode)

use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Detection rules
// ---------------------------------------------------------------------------

/// Severity and action for a detection rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleAction {
    /// Block the entire log entry
    Block,
    /// Mask the matching text with ***
    Mask,
    /// Warn only, don't modify
    Warn,
}

/// A single detection rule using prefix/suffix matching.
struct DetectionRule {
    name: &'static str,
    /// Patterns to search for (case-sensitive prefix match)
    prefixes: &'static [&'static str],
    /// Substrings that confirm the match (e.g., "=", ":")
    delimiters: &'static [&'static str],
    action: RuleAction,
    is_critical: bool,
}

/// Pre-defined detection rules (no regex dependency).
static RULES: &[DetectionRule] = &[
    // ---- CRITICAL: block outright ----
    DetectionRule {
        name: "private_key_pem",
        prefixes: &[
            "-----BEGIN RSA PRIVATE KEY",
            "-----BEGIN EC PRIVATE KEY",
            "-----BEGIN DSA PRIVATE KEY",
            "-----BEGIN OPENSSH PRIVATE KEY",
            "-----BEGIN PRIVATE KEY",
        ],
        delimiters: &[],
        action: RuleAction::Block,
        is_critical: true,
    },
    // ---- HIGH: mask ----
    DetectionRule {
        name: "aws_access_key",
        prefixes: &["AKIA"],
        delimiters: &[" ", "=", ":", "'", "\"", ","],
        action: RuleAction::Mask,
        is_critical: false,
    },
    DetectionRule {
        name: "github_token",
        prefixes: &["ghp_", "gho_", "ghu_", "ghs_", "ghr_"],
        delimiters: &[" ", "=", ":", "'"],
        action: RuleAction::Mask,
        is_critical: false,
    },
    DetectionRule {
        name: "jwt_header",
        prefixes: &["eyJ"],
        delimiters: &[" "],
        action: RuleAction::Mask,
        is_critical: false,
    },
    DetectionRule {
        name: "stripe_live_key",
        prefixes: &["sk_live_", "rk_live_"],
        delimiters: &[" ", "=", ":", "'"],
        action: RuleAction::Mask,
        is_critical: false,
    },
    DetectionRule {
        name: "google_api_key",
        prefixes: &["AIza"],
        delimiters: &[" ", "=", ":", "'"],
        action: RuleAction::Mask,
        is_critical: false,
    },
    DetectionRule {
        name: "slack_token",
        prefixes: &["xoxb-", "xoxp-", "xoxa-", "xoxr-", "xoxs-"],
        delimiters: &[" ", "=", ":"],
        action: RuleAction::Mask,
        is_critical: false,
    },
    DetectionRule {
        name: "auth_bearer",
        prefixes: &["Authorization: Bearer ", "authorization: bearer "],
        delimiters: &[],
        action: RuleAction::Mask,
        is_critical: false,
    },
    // ---- MEDIUM: mask ----
    DetectionRule {
        name: "password_assignment",
        prefixes: &["password=", "password =", "password:", "password :"],
        delimiters: &[" ", "'", "\""],
        action: RuleAction::Mask,
        is_critical: false,
    },
    DetectionRule {
        name: "secret_key",
        prefixes: &[
            "secret=",
            "secret =",
            "secret:",
            "secret :",
            "secret_key=",
            "secret key=",
            "aws_secret_access_key=",
        ],
        delimiters: &[" ", "'", "\""],
        action: RuleAction::Mask,
        is_critical: false,
    },
    DetectionRule {
        name: "api_key",
        prefixes: &[
            "api_key=", "apikey=", "api-key=", "API_KEY=", "api_key:", "apikey:",
        ],
        delimiters: &[" ", "'", "\""],
        action: RuleAction::Mask,
        is_critical: false,
    },
    DetectionRule {
        name: "token_assignment",
        prefixes: &[
            "token=",
            "token =",
            "token:",
            "token :",
            "access_token=",
            "auth_token=",
        ],
        delimiters: &[" ", "'", "\""],
        action: RuleAction::Mask,
        is_critical: false,
    },
    DetectionRule {
        name: "connection_string",
        prefixes: &[
            "mongodb://",
            "postgres://",
            "postgresql://",
            "mysql://",
            "redis://",
        ],
        delimiters: &["@"],
        action: RuleAction::Mask,
        is_critical: false,
    },
];

// ---------------------------------------------------------------------------
// Finding and result types
// ---------------------------------------------------------------------------

/// A finding from secret detection.
#[derive(Debug, Clone)]
pub struct Finding {
    /// Name of the rule that matched
    pub rule: String,
    /// Action taken (Block/Mask/Warn)
    pub action: RuleAction,
    /// Byte offset where the match starts
    pub start: usize,
    /// Byte offset where the match ends
    pub end: usize,
}

/// Result of scanning a message for secrets.
#[derive(Debug, Clone)]
pub struct DetectionResult {
    /// The (possibly masked) message
    pub message: String,
    /// Whether any secrets were detected
    pub detected: bool,
    /// Whether the message should be blocked entirely
    pub should_block: bool,
    /// List of individual findings
    pub findings: Vec<Finding>,
}

// ---------------------------------------------------------------------------
// SecretDetector
// ---------------------------------------------------------------------------

/// Secret detector with configurable strictness per trust color.
pub struct SecretDetector {
    enabled: bool,
    /// Only apply critical rules
    critical_only: bool,
    /// Only apply critical + high rules (skip medium)
    strict: bool,
    // Statistics
    total_scanned: u64,
    total_detected: u64,
    total_blocked: u64,
    total_masked: u64,
}

impl SecretDetector {
    /// Full detection — all rules active (Blue plugins).
    pub fn new() -> Self {
        Self {
            enabled: true,
            critical_only: false,
            strict: false,
            total_scanned: 0,
            total_detected: 0,
            total_blocked: 0,
            total_masked: 0,
        }
    }

    /// Strict mode — only critical + high rules (Yellow plugins).
    pub fn strict() -> Self {
        Self {
            enabled: true,
            critical_only: false,
            strict: true,
            total_scanned: 0,
            total_detected: 0,
            total_blocked: 0,
            total_masked: 0,
        }
    }

    /// Critical-only mode (Red plugins).
    pub fn critical_only() -> Self {
        Self {
            enabled: true,
            critical_only: true,
            strict: false,
            total_scanned: 0,
            total_detected: 0,
            total_blocked: 0,
            total_masked: 0,
        }
    }

    /// Disable detection entirely.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Scan a message and return the (possibly masked) result.
    pub fn scan(&mut self, message: &str) -> DetectionResult {
        self.total_scanned += 1;
        let mut result = DetectionResult {
            message: message.to_string(),
            detected: false,
            should_block: false,
            findings: Vec::new(),
        };

        if !self.enabled || message.is_empty() {
            return result;
        }

        for rule in RULES {
            if self.critical_only && !rule.is_critical {
                continue;
            }
            if self.strict && !rule.is_critical && !rule.name.starts_with("auth") {
                // In strict mode, skip medium rules (password=, secret=, token=, api_key=, connection_string)
                if matches!(
                    rule.name,
                    "password_assignment"
                        | "secret_key"
                        | "api_key"
                        | "token_assignment"
                        | "connection_string"
                ) {
                    continue;
                }
            }

            for &prefix in rule.prefixes {
                let mut search_start = 0usize;
                while let Some(pos) = message[search_start..].find(prefix) {
                    let abs_pos = search_start + pos;
                    let after_prefix = abs_pos + prefix.len();

                    // Find the end of the secret value (up to next delimiter or space)
                    let mut end = message.len();
                    for &delim in rule.delimiters {
                        if delim.is_empty() {
                            continue;
                        }
                        if let Some(dpos) = message[after_prefix..].find(delim) {
                            let candidate = after_prefix + dpos;
                            if candidate < end {
                                end = candidate;
                            }
                        }
                    }
                    // If no delimiter found, capture up to next whitespace or 80 chars
                    if end == message.len() {
                        if let Some(space) = message[after_prefix..].find(' ') {
                            end = after_prefix + space;
                        } else {
                            end = (after_prefix + 80).min(message.len());
                        }
                    }

                    let matched = after_prefix..end;

                    match rule.action {
                        RuleAction::Block => {
                            result.detected = true;
                            result.should_block = true;
                            self.total_blocked += 1;
                        }
                        RuleAction::Mask => {
                            result.detected = true;
                            // SAFETY: We only write ASCII '*' bytes (0x2A) which is
                            // always valid UTF-8. The byte indices come from
                            // str::find() which returns valid UTF-8 boundaries.
                            unsafe {
                                let bytes = result.message.as_bytes_mut();
                                for b in &mut bytes[matched.start..matched.end] {
                                    *b = b'*';
                                }
                            }
                            self.total_masked += 1;
                        }
                        RuleAction::Warn => {
                            result.detected = true;
                        }
                    }

                    result.findings.push(Finding {
                        rule: rule.name.to_string(),
                        action: rule.action,
                        start: matched.start,
                        end: matched.end,
                    });

                    search_start = end;
                }
            }
        }

        if result.detected {
            self.total_detected += 1;
        }
        result
    }

    /// Get detection statistics.
    pub fn stats(&self) -> HashMap<String, u64> {
        let mut s = HashMap::new();
        s.insert("total_scanned".into(), self.total_scanned);
        s.insert("total_detected".into(), self.total_detected);
        s.insert("total_blocked".into(), self.total_blocked);
        s.insert("total_masked".into(), self.total_masked);
        s
    }
}

impl Default for SecretDetector {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_aws_key() {
        let mut d = SecretDetector::new();
        let r = d.scan("config key=AKIAIOSFODNN7EXAMPLE for service");
        assert!(r.detected);
        assert!(!r.should_block);
    }

    #[test]
    fn test_block_private_key() {
        let mut d = SecretDetector::new();
        let r = d.scan(
            "Found key: -----BEGIN RSA PRIVATE KEY----- abc123 -----END RSA PRIVATE KEY-----",
        );
        assert!(r.detected);
        assert!(r.should_block);
    }

    #[test]
    fn test_github_token() {
        let mut d = SecretDetector::new();
        let r = d.scan("export GITHUB_TOKEN=ghp_1234567890abcdefghijklmnopqrstuv");
        assert!(r.detected);
    }

    #[test]
    fn test_jwt() {
        let mut d = SecretDetector::new();
        let r = d.scan(
            "Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.signature",
        );
        assert!(r.detected);
        assert!(r.message.contains("***"));
    }

    #[test]
    fn test_password_mask() {
        let mut d = SecretDetector::new();
        let r = d.scan("login with username=admin password=hunter2 to proceed");
        assert!(r.detected);
        assert!(!r.message.contains("hunter2"));
        assert!(r.message.contains("***"));
    }

    #[test]
    fn test_no_false_positive() {
        let mut d = SecretDetector::new();
        let r = d.scan("User admin logged in successfully from 192.168.1.1");
        assert!(!r.detected);
    }

    #[test]
    fn test_strict_mode() {
        let mut d = SecretDetector::strict();
        let r = d.scan("password=hunter2 for login");
        assert!(!r.detected, "Strict mode should skip medium-severity rules");
        let r2 = d.scan("key=AKIAIOSFODNN7EXAMPLE for AWS");
        assert!(
            r2.detected,
            "Strict mode should still catch high-severity rules"
        );
    }

    #[test]
    fn test_critical_only() {
        let mut d = SecretDetector::critical_only();
        let r = d.scan("key=AKIAIOSFODNN7EXAMPLE for AWS");
        assert!(!r.detected, "Critical-only should skip high severity");
        let r2 = d.scan("-----BEGIN RSA PRIVATE KEY----- data");
        assert!(r2.detected, "Critical-only should catch private keys");
    }

    #[test]
    fn test_connection_string() {
        let mut d = SecretDetector::new();
        let r = d.scan("Connecting to mongodb://admin:secretpass@db.example.com:27017/db");
        assert!(r.detected);
    }

    #[test]
    fn test_disabled() {
        let mut d = SecretDetector::new();
        d.set_enabled(false);
        let r = d.scan("password=hunter2");
        assert!(!r.detected);
    }
}
