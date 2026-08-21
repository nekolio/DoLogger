//! Configuration for input detection and display encoding.

use std::fmt;

/// Input policy for length-delimited byte sources.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum InputEncodingMode {
    /// Require the caller to provide already validated UTF-8.
    #[default]
    Utf8,
    /// Explicitly run fail-closed input detection.
    Auto,
}

impl InputEncodingMode {
    /// Parse the configuration spelling.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "utf8" | "utf-8" => Some(Self::Utf8),
            "auto" => Some(Self::Auto),
            _ => None,
        }
    }

    /// Return the stable configuration spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Utf8 => "utf8",
            Self::Auto => "auto",
        }
    }
}

/// Encoding settings shared by byte ingestion and human-facing outputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodingConfig {
    /// Input policy for explicitly length-delimited byte sources.
    pub input: InputEncodingMode,
    /// Output policy for display adapters.
    pub output: crate::codec::policy::EncodingPreference,
    /// Optional manually selected Windows code page for display output.
    pub output_code_page: Option<u32>,
}

impl Default for EncodingConfig {
    fn default() -> Self {
        Self {
            input: InputEncodingMode::Utf8,
            output: crate::codec::policy::EncodingPreference::Utf8,
            output_code_page: None,
        }
    }
}

impl EncodingConfig {
    /// Return whether this configuration is protected from hot reload.
    pub const fn is_restart_required() -> bool {
        true
    }
}

impl fmt::Display for InputEncodingMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_mode_is_fail_closed_by_default() {
        assert_eq!(InputEncodingMode::default(), InputEncodingMode::Utf8);
        assert_eq!(
            InputEncodingMode::parse("AUTO"),
            Some(InputEncodingMode::Auto)
        );
        assert_eq!(InputEncodingMode::parse("guess"), None);
    }

    #[test]
    fn encoding_config_is_restart_required() {
        assert!(EncodingConfig::is_restart_required());
    }
}
