//! Encoding policy snapshots for log I/O.
//!
//! The codec service owns bytes; localization owns human messages. This module
//! only chooses a codec at an explicit boundary (console, configured file
//! sink, or adapter). It never changes process locale, console state, or the
//! canonical KV/SIF byte contract.

use std::fmt;
use std::sync::Arc;

use super::{
    decode, detect, encode, parse_code_page, EncodingDetection, EncodingError, TextEncoding,
};

/// Source that selected an output encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodingSource {
    /// Caller supplied a concrete encoding.
    Explicit,
    /// A recognized locale/codeset selected a code page.
    Environment,
    /// The platform console reported a code page.
    Platform,
    /// No safe platform signal was available.
    Utf8Fallback,
}

/// Explicit or automatic encoding selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EncodingPreference {
    /// Always use canonical UTF-8 at this boundary.
    Utf8,
    /// Use a concrete Windows code page.
    CodePage(u32),
    /// Detect from environment and platform, then fall back to UTF-8.
    #[default]
    Auto,
}

/// Policy controlling conversion strictness and fallback behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncodingPolicy {
    /// Preferred encoding mode.
    pub preference: EncodingPreference,
    /// If true, conversion errors fall back to UTF-8 instead of returning an
    /// error. The fallback is observable in the resulting snapshot.
    pub allow_utf8_fallback: bool,
    /// If false, Windows conversions that substitute a default character fail.
    pub allow_lossy: bool,
}

impl Default for EncodingPolicy {
    fn default() -> Self {
        Self {
            preference: EncodingPreference::Auto,
            allow_utf8_fallback: true,
            allow_lossy: false,
        }
    }
}

/// Immutable decision used by a logger or adapter for one output lifetime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodingSnapshot {
    /// Selected core codec.
    pub encoding: TextEncoding,
    /// Why this codec won.
    pub source: EncodingSource,
    /// Detected locale token, if any.
    pub locale: Option<String>,
    /// Detected codeset token, if any.
    pub codeset: Option<String>,
    /// Platform console code page, if available.
    pub console_code_page: Option<u32>,
    /// Whether UTF-8 fallback is permitted for conversion failures.
    pub allow_utf8_fallback: bool,
    /// Whether lossy code-page output is permitted.
    pub allow_lossy: bool,
}

impl EncodingSnapshot {
    /// Return a compact stable identifier for diagnostics and status output.
    pub fn id(&self) -> String {
        match self.encoding {
            TextEncoding::Utf8 => "utf-8".to_string(),
            TextEncoding::Utf16Le => "utf-16le".to_string(),
            TextEncoding::Utf16Be => "utf-16be".to_string(),
            TextEncoding::CodePage(code_page) => format!("cp{code_page}"),
        }
    }

    /// Encode text using this snapshot.
    pub fn encode(&self, text: &str) -> Result<Vec<u8>, EncodingError> {
        match encode(text, self.encoding) {
            Ok(bytes) => Ok(bytes),
            Err(error)
                if self.allow_utf8_fallback && !matches!(self.encoding, TextEncoding::Utf8) =>
            {
                encode(text, TextEncoding::Utf8).map_err(|_| error)
            }
            Err(error) => Err(error),
        }
    }

    /// Decode bytes using this snapshot.
    pub fn decode(&self, bytes: &[u8]) -> Result<String, EncodingError> {
        match decode(bytes, self.encoding) {
            Ok(text) => Ok(text),
            Err(error)
                if self.allow_utf8_fallback && !matches!(self.encoding, TextEncoding::Utf8) =>
            {
                decode(bytes, TextEncoding::Utf8).map_err(|_| error)
            }
            Err(error) => Err(error),
        }
    }
}

/// Errors produced while resolving an encoding policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncodingPolicyError {
    /// The explicit code page is invalid.
    InvalidCodePage(u32),
    /// A policy requested a platform-specific code page that is unavailable.
    Unsupported(TextEncoding),
}

impl fmt::Display for EncodingPolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCodePage(code_page) => write!(f, "invalid code page {code_page}"),
            Self::Unsupported(encoding) => write!(f, "unsupported encoding {encoding:?}"),
        }
    }
}

impl std::error::Error for EncodingPolicyError {}

/// Resolve a policy once and share the immutable result across sinks.
pub fn resolve(policy: EncodingPolicy) -> Result<EncodingSnapshot, EncodingPolicyError> {
    let detection = detect();
    resolve_from_detection(policy, detection)
}

/// Resolve deterministically from a supplied detection snapshot.
///
/// This function is the testable boundary between platform probing and policy.
/// Callers that need a stable process-wide choice should call it once during
/// startup and pass the resulting `Arc<EncodingSnapshot>` to sinks.
pub fn resolve_from_detection(
    policy: EncodingPolicy,
    detection: EncodingDetection,
) -> Result<EncodingSnapshot, EncodingPolicyError> {
    let (encoding, source) = match policy.preference {
        EncodingPreference::Utf8 => (TextEncoding::Utf8, EncodingSource::Explicit),
        EncodingPreference::CodePage(code_page) => {
            if code_page == 0 || code_page > 65_535 {
                return Err(EncodingPolicyError::InvalidCodePage(code_page));
            }
            (TextEncoding::CodePage(code_page), EncodingSource::Explicit)
        }
        EncodingPreference::Auto => {
            if let Some(codeset) = detection.codeset.as_deref() {
                if let Some(code_page) = parse_code_page(codeset) {
                    (
                        TextEncoding::CodePage(code_page),
                        EncodingSource::Environment,
                    )
                } else {
                    (TextEncoding::Utf8, EncodingSource::Utf8Fallback)
                }
            } else if let Some(code_page) = detection.console_code_page {
                (TextEncoding::CodePage(code_page), EncodingSource::Platform)
            } else {
                (TextEncoding::Utf8, EncodingSource::Utf8Fallback)
            }
        }
    };
    Ok(EncodingSnapshot {
        encoding,
        source,
        locale: detection.locale,
        codeset: detection.codeset,
        console_code_page: detection.console_code_page,
        allow_utf8_fallback: policy.allow_utf8_fallback,
        allow_lossy: policy.allow_lossy,
    })
}

/// Shared immutable snapshot for sinks and adapters.
pub type SharedEncodingSnapshot = Arc<EncodingSnapshot>;

/// Build the default shared snapshot used by opt-in text outputs.
pub fn default_shared_snapshot() -> Result<SharedEncodingSnapshot, EncodingPolicyError> {
    Ok(Arc::new(resolve(EncodingPolicy::default())?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detection(codeset: Option<&str>, console: Option<u32>) -> EncodingDetection {
        EncodingDetection {
            locale: Some("zh-CN".to_string()),
            codeset: codeset.map(str::to_string),
            console_code_page: console,
        }
    }

    #[test]
    fn explicit_utf8_beats_platform_detection() {
        let snapshot = resolve_from_detection(
            EncodingPolicy {
                preference: EncodingPreference::Utf8,
                ..Default::default()
            },
            detection(Some("GBK"), Some(936)),
        )
        .unwrap();
        assert_eq!(snapshot.encoding, TextEncoding::Utf8);
        assert_eq!(snapshot.source, EncodingSource::Explicit);
    }

    #[test]
    fn environment_codeset_beats_console_code_page() {
        let snapshot = resolve_from_detection(
            EncodingPolicy::default(),
            detection(Some("GBK"), Some(1252)),
        )
        .unwrap();
        assert_eq!(snapshot.encoding, TextEncoding::CodePage(936));
        assert_eq!(snapshot.source, EncodingSource::Environment);
        assert_eq!(snapshot.id(), "cp936");
    }

    #[test]
    fn console_code_page_is_used_without_codeset() {
        let snapshot =
            resolve_from_detection(EncodingPolicy::default(), detection(None, Some(1252))).unwrap();
        assert_eq!(snapshot.encoding, TextEncoding::CodePage(1252));
        assert_eq!(snapshot.source, EncodingSource::Platform);
    }

    #[test]
    fn unknown_codeset_is_safe_utf8_fallback() {
        let snapshot = resolve_from_detection(
            EncodingPolicy::default(),
            detection(Some("x-unknown"), None),
        )
        .unwrap();
        assert_eq!(snapshot.encoding, TextEncoding::Utf8);
        assert_eq!(snapshot.source, EncodingSource::Utf8Fallback);
    }

    #[test]
    fn invalid_explicit_code_page_is_rejected() {
        let result = resolve_from_detection(
            EncodingPolicy {
                preference: EncodingPreference::CodePage(65_536),
                ..Default::default()
            },
            detection(None, None),
        );
        assert_eq!(result, Err(EncodingPolicyError::InvalidCodePage(65_536)));
    }

    #[test]
    fn snapshot_utf8_round_trip_is_locale_independent() {
        let snapshot = resolve_from_detection(
            EncodingPolicy {
                preference: EncodingPreference::Utf8,
                ..Default::default()
            },
            detection(Some("CP936"), Some(936)),
        )
        .unwrap();
        let bytes = snapshot.encode("日志 😀").unwrap();
        assert_eq!(snapshot.decode(&bytes).unwrap(), "日志 😀");
    }

    #[test]
    fn snapshot_fields_are_stable_for_status_reporting() {
        let snapshot = resolve_from_detection(
            EncodingPolicy::default(),
            detection(Some("UTF-8"), Some(65001)),
        )
        .unwrap();
        assert_eq!(snapshot.locale.as_deref(), Some("zh-CN"));
        assert_eq!(snapshot.codeset.as_deref(), Some("UTF-8"));
        assert_eq!(snapshot.console_code_page, Some(65001));
        assert!(snapshot.allow_utf8_fallback);
        assert!(!snapshot.allow_lossy);
    }
}
