//! Bounded text output encoding for sinks and adapters.
//!
//! This is deliberately below localization. A localized message is still a
//! Unicode string; this module decides how that string crosses a console/file
//! boundary. It sanitizes NULs and line separators, enforces an output budget,
//! and keeps the selected encoding immutable for the sink lifetime.

use std::fmt;

use super::policy::{EncodingPolicyError, EncodingSnapshot};
use super::EncodingError;

/// Maximum output line size accepted by the default logger boundary.
pub const DEFAULT_MAX_LINE_BYTES: usize = 1024 * 1024;

/// Line-ending policy for text outputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineEnding {
    /// Preserve `\n` and normalize CRLF/CR to LF.
    #[default]
    Lf,
    /// Emit CRLF for Windows-oriented text sinks.
    CrLf,
    /// Preserve the caller's line separators after NUL sanitization.
    Preserve,
}

/// NUL handling policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NulPolicy {
    /// Replace NUL with the visible escape `\\0`.
    #[default]
    Escape,
    /// Reject the line.
    Reject,
    /// Remove NUL bytes.
    Remove,
}

/// Text output policy independent from message localization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextOutputPolicy {
    /// Line separator strategy.
    pub line_ending: LineEnding,
    /// NUL handling.
    pub nul: NulPolicy,
    /// Maximum encoded line size.
    pub max_line_bytes: usize,
    /// Whether a trailing line separator is added.
    pub append_newline: bool,
}

impl Default for TextOutputPolicy {
    fn default() -> Self {
        Self {
            line_ending: LineEnding::Lf,
            nul: NulPolicy::Escape,
            max_line_bytes: DEFAULT_MAX_LINE_BYTES,
            append_newline: true,
        }
    }
}

impl TextOutputPolicy {
    /// Validate safety and resource limits.
    pub const fn validate(self) -> Result<(), TextOutputError> {
        if self.max_line_bytes == 0 || self.max_line_bytes > 64 * 1024 * 1024 {
            return Err(TextOutputError::InvalidLimit);
        }
        Ok(())
    }
}

/// Output conversion failure.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum TextOutputError {
    /// Output policy has an invalid limit.
    InvalidLimit,
    /// Input contains a NUL and policy rejects it.
    NulRejected,
    /// The encoded line exceeds the configured bound.
    LineTooLong { length: usize, max: usize },
    /// The selected codec rejected the input.
    Encoding(EncodingError),
    /// Automatic encoding resolution failed.
    Policy(EncodingPolicyError),
}

impl fmt::Display for TextOutputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLimit => f.write_str("invalid text output limit"),
            Self::NulRejected => f.write_str("NUL byte rejected by text output policy"),
            Self::LineTooLong { length, max } => {
                write!(f, "encoded line length {length} exceeds {max}")
            }
            Self::Encoding(error) => error.fmt(f),
            Self::Policy(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for TextOutputError {}

/// Encoded output metadata returned to sinks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncodedLine {
    /// Number of bytes written.
    pub length: usize,
    /// Whether NUL or line separators were normalized.
    pub normalized: bool,
}

/// Reusable text encoder for one sink.
pub struct TextOutputEncoder {
    snapshot: EncodingSnapshot,
    policy: TextOutputPolicy,
    buffer: Vec<u8>,
}

impl fmt::Debug for TextOutputEncoder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TextOutputEncoder")
            .field("encoding", &self.snapshot.id())
            .field("policy", &self.policy)
            .field("capacity", &self.buffer.capacity())
            .finish()
    }
}

impl TextOutputEncoder {
    /// Construct an encoder from an immutable codec decision.
    pub fn new(
        snapshot: EncodingSnapshot,
        policy: TextOutputPolicy,
    ) -> Result<Self, TextOutputError> {
        policy.validate()?;
        Ok(Self {
            snapshot,
            policy,
            buffer: Vec::with_capacity(256),
        })
    }

    /// Return the selected codec snapshot.
    pub fn snapshot(&self) -> &EncodingSnapshot {
        &self.snapshot
    }

    /// Return the output policy.
    pub const fn policy(&self) -> TextOutputPolicy {
        self.policy
    }

    /// Encode one text line and return metadata.
    pub fn encode(&mut self, text: &str) -> Result<EncodedLine, TextOutputError> {
        self.buffer.clear();
        let normalized_text = sanitize(text, self.policy.nul, self.policy.line_ending)?;
        let normalized = normalized_text != text;
        let mut bytes = self
            .snapshot
            .encode(&normalized_text)
            .map_err(TextOutputError::Encoding)?;
        if self.policy.append_newline
            && !normalized_text.ends_with('\n')
            && !normalized_text.ends_with('\r')
        {
            let newline = match self.policy.line_ending {
                LineEnding::CrLf => "\r\n",
                LineEnding::Lf | LineEnding::Preserve => "\n",
            };
            bytes.extend_from_slice(
                &self
                    .snapshot
                    .encode(newline)
                    .map_err(TextOutputError::Encoding)?,
            );
        }
        if bytes.len() > self.policy.max_line_bytes {
            return Err(TextOutputError::LineTooLong {
                length: bytes.len(),
                max: self.policy.max_line_bytes,
            });
        }
        self.buffer.extend_from_slice(&bytes);
        Ok(EncodedLine {
            length: bytes.len(),
            normalized,
        })
    }

    /// Borrow the last encoded bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.buffer
    }

    /// Encode and copy directly into a caller-provided byte vector.
    pub fn encode_into(
        &mut self,
        text: &str,
        output: &mut Vec<u8>,
    ) -> Result<EncodedLine, TextOutputError> {
        let metadata = self.encode(text)?;
        output.extend_from_slice(&self.buffer);
        Ok(metadata)
    }

    /// Clear reusable storage without reducing capacity.
    pub fn clear(&mut self) {
        self.buffer.clear();
    }
}

fn sanitize(text: &str, nul: NulPolicy, ending: LineEnding) -> Result<String, TextOutputError> {
    let mut output = String::with_capacity(text.len());
    let mut normalized = false;
    let mut chars = text.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '\0' {
            normalized = true;
            match nul {
                NulPolicy::Escape => output.push_str("\\0"),
                NulPolicy::Remove => {}
                NulPolicy::Reject => return Err(TextOutputError::NulRejected),
            }
            continue;
        }
        if matches!(ending, LineEnding::Lf | LineEnding::CrLf) && character == '\r' {
            normalized = true;
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            output.push('\n');
            continue;
        }
        output.push(character);
    }
    if matches!(ending, LineEnding::CrLf) {
        let mut crlf = String::with_capacity(output.len() + output.matches('\n').count());
        for character in output.chars() {
            if character == '\n' {
                crlf.push('\r');
            }
            crlf.push(character);
        }
        output = crlf;
        normalized = true;
    }
    if normalized {
        Ok(output)
    } else {
        Ok(text.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::policy::{resolve_from_detection, EncodingPreference};
    use crate::codec::EncodingDetection;

    fn encoder(policy: TextOutputPolicy) -> TextOutputEncoder {
        let snapshot = resolve_from_detection(
            crate::codec::policy::EncodingPolicy {
                preference: EncodingPreference::Utf8,
                ..Default::default()
            },
            EncodingDetection {
                locale: None,
                codeset: None,
                console_code_page: None,
            },
        )
        .unwrap();
        TextOutputEncoder::new(snapshot, policy).unwrap()
    }

    #[test]
    fn default_encoder_adds_lf() {
        let mut output = encoder(TextOutputPolicy::default());
        let metadata = output.encode("hello").unwrap();
        assert_eq!(metadata.length, 6);
        assert_eq!(output.bytes(), b"hello\n");
    }

    #[test]
    fn crlf_normalizes_mixed_input() {
        let mut output = encoder(TextOutputPolicy {
            line_ending: LineEnding::CrLf,
            ..Default::default()
        });
        output.encode("a\rb\n").unwrap();
        assert_eq!(output.bytes(), b"a\r\nb\r\n");
    }

    #[test]
    fn nul_escape_is_safe_for_c_strings() {
        let mut output = encoder(TextOutputPolicy::default());
        let metadata = output.encode("a\0b").unwrap();
        assert!(metadata.normalized);
        assert_eq!(output.bytes(), b"a\\0b\n");
    }

    #[test]
    fn nul_reject_is_explicit() {
        let mut output = encoder(TextOutputPolicy {
            nul: NulPolicy::Reject,
            ..Default::default()
        });
        assert_eq!(output.encode("a\0b"), Err(TextOutputError::NulRejected));
    }

    #[test]
    fn line_limit_is_enforced_after_encoding() {
        let mut output = encoder(TextOutputPolicy {
            max_line_bytes: 4,
            ..Default::default()
        });
        assert!(matches!(
            output.encode("hello"),
            Err(TextOutputError::LineTooLong { .. })
        ));
    }

    #[test]
    fn encode_into_appends_exact_bytes() {
        let mut output = encoder(TextOutputPolicy::default());
        let mut target = b"prefix:".to_vec();
        output.encode_into("x", &mut target).unwrap();
        assert_eq!(target, b"prefix:x\n");
    }

    #[test]
    fn policy_rejects_zero_limit() {
        assert_eq!(
            TextOutputPolicy {
                max_line_bytes: 0,
                ..Default::default()
            }
            .validate(),
            Err(TextOutputError::InvalidLimit)
        );
    }
}
