//! Core text codec service and platform encoding detection.
//!
//! This boundary owns text encode/decode contracts, UTF-8 canonicalization,
//! explicit Windows code-page support, and environment detection. It is
//! independent of localization and is not a plugin extension point.
//! Human-facing console writes are delegated to [`crate::sys::io`].

use std::fmt;

pub mod log_output;
pub mod policy;

/// Supported text encoding selectors for core encode/decode operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextEncoding {
    /// Canonical UTF-8 used by persisted text and all non-Windows pipes/files.
    Utf8,
    /// Little-endian UTF-16 input or output.
    Utf16Le,
    /// Big-endian UTF-16 input or output.
    Utf16Be,
    /// A Windows code page used only when a caller explicitly requests it.
    CodePage(u32),
}

/// Errors returned by the core text codec boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncodingError {
    /// The selected code page is outside the supported numeric range.
    InvalidCodePage(InvalidCodePage),
    /// The byte sequence is not valid for the selected encoding.
    InvalidBytes,
    /// The current platform does not provide the requested codec.
    UnsupportedEncoding(TextEncoding),
    /// The conversion would lose information.
    LossyConversion,
}

impl fmt::Display for EncodingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCodePage(error) => error.fmt(formatter),
            Self::InvalidBytes => formatter.write_str("invalid encoded bytes"),
            Self::UnsupportedEncoding(encoding) => {
                write!(formatter, "unsupported encoding: {encoding:?}")
            }
            Self::LossyConversion => formatter.write_str("lossy encoding conversion rejected"),
        }
    }
}

impl std::error::Error for EncodingError {}

/// Encode text using a core codec without involving localization policy.
pub fn encode(text: &str, encoding: TextEncoding) -> Result<Vec<u8>, EncodingError> {
    match encoding {
        TextEncoding::Utf8 => Ok(text.as_bytes().to_vec()),
        TextEncoding::Utf16Le => Ok(text.encode_utf16().flat_map(u16::to_le_bytes).collect()),
        TextEncoding::Utf16Be => Ok(text.encode_utf16().flat_map(u16::to_be_bytes).collect()),
        TextEncoding::CodePage(code_page) => encode_code_page(text, code_page),
    }
}

/// Decode bytes using a core codec without involving localization policy.
pub fn decode(bytes: &[u8], encoding: TextEncoding) -> Result<String, EncodingError> {
    match encoding {
        TextEncoding::Utf8 => {
            String::from_utf8(bytes.to_vec()).map_err(|_| EncodingError::InvalidBytes)
        }
        TextEncoding::Utf16Le => decode_utf16(bytes, true),
        TextEncoding::Utf16Be => decode_utf16(bytes, false),
        TextEncoding::CodePage(code_page) => decode_code_page(bytes, code_page),
    }
}

/// Result of an explicitly requested, deterministic input detection pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectedInputEncoding {
    /// Strictly valid UTF-8 bytes.
    Utf8,
    /// UTF-16 little-endian bytes, with or without a BOM.
    Utf16Le,
    /// UTF-16 big-endian bytes, with or without a BOM.
    Utf16Be,
    /// A code page that was the only candidate to decode successfully.
    CodePage(u32),
}

impl DetectedInputEncoding {
    /// Convert the detection result to the core codec selector.
    pub const fn as_text_encoding(self) -> TextEncoding {
        match self {
            Self::Utf8 => TextEncoding::Utf8,
            Self::Utf16Le => TextEncoding::Utf16Le,
            Self::Utf16Be => TextEncoding::Utf16Be,
            Self::CodePage(code_page) => TextEncoding::CodePage(code_page),
        }
    }
}

/// Fail-closed outcomes from an explicit input `AUTO` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoDetectionError {
    /// No deterministic text interpretation was found.
    Unknown,
    /// More than one explicitly supplied code page decoded the bytes.
    Ambiguous(Vec<u32>),
    /// A BOM was present but the payload failed strict validation.
    InvalidBom,
}

impl fmt::Display for AutoDetectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown => formatter.write_str("input encoding is not uniquely detectable"),
            Self::Ambiguous(code_pages) => {
                write!(formatter, "input encoding is ambiguous: {code_pages:?}")
            }
            Self::InvalidBom => formatter.write_str("input encoding BOM is invalid"),
        }
    }
}

impl std::error::Error for AutoDetectionError {}

/// Detect an input encoding only when the caller explicitly requests `AUTO`.
///
/// The function is intentionally fail-closed. It never chooses a statistical
/// best match and never mutates the supplied bytes. Code-page candidates are
/// caller-provided so the core does not silently invent a platform policy.
pub fn detect_input(
    bytes: &[u8],
    code_page_candidates: &[u32],
) -> Result<DetectedInputEncoding, AutoDetectionError> {
    if bytes.starts_with(&[0xff, 0xfe]) {
        decode_utf16(&bytes[2..], true).map_err(|_| AutoDetectionError::InvalidBom)?;
        return Ok(DetectedInputEncoding::Utf16Le);
    }
    if bytes.starts_with(&[0xfe, 0xff]) {
        decode_utf16(&bytes[2..], false).map_err(|_| AutoDetectionError::InvalidBom)?;
        return Ok(DetectedInputEncoding::Utf16Be);
    }
    if std::str::from_utf8(bytes).is_ok() {
        return Ok(DetectedInputEncoding::Utf8);
    }

    let mut utf16_candidates = Vec::with_capacity(2);
    if has_utf16_shape(bytes, true) && decode_utf16(bytes, true).is_ok() {
        utf16_candidates.push(DetectedInputEncoding::Utf16Le);
    }
    if has_utf16_shape(bytes, false) && decode_utf16(bytes, false).is_ok() {
        utf16_candidates.push(DetectedInputEncoding::Utf16Be);
    }
    if utf16_candidates.len() == 1 {
        return Ok(utf16_candidates[0]);
    }
    if utf16_candidates.len() > 1 {
        return Err(AutoDetectionError::Unknown);
    }

    let mut decoded_code_pages = Vec::new();
    for &code_page in code_page_candidates {
        if validate_code_page(code_page).is_err() {
            continue;
        }
        if decode_code_page(bytes, code_page).is_ok() {
            decoded_code_pages.push(code_page);
        }
    }
    match decoded_code_pages.as_slice() {
        [code_page] => Ok(DetectedInputEncoding::CodePage(*code_page)),
        [] => Err(AutoDetectionError::Unknown),
        candidates => Err(AutoDetectionError::Ambiguous(candidates.to_vec())),
    }
}

fn has_utf16_shape(bytes: &[u8], little_endian: bool) -> bool {
    if bytes.len() < 4 || !bytes.len().is_multiple_of(2) {
        return false;
    }
    let zero_count = bytes
        .chunks_exact(2)
        .filter(|pair| {
            if little_endian {
                pair[1] == 0 && pair[0] != 0
            } else {
                pair[0] == 0 && pair[1] != 0
            }
        })
        .count();
    zero_count * 2 >= bytes.len() / 2
}

fn decode_utf16(bytes: &[u8], little_endian: bool) -> Result<String, EncodingError> {
    if !bytes.len().is_multiple_of(2) {
        return Err(EncodingError::InvalidBytes);
    }
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| {
            if little_endian {
                u16::from_le_bytes([pair[0], pair[1]])
            } else {
                u16::from_be_bytes([pair[0], pair[1]])
            }
        })
        .collect();
    String::from_utf16(&units).map_err(|_| EncodingError::InvalidBytes)
}

/// A detected locale and display encoding snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodingDetection {
    /// Normalized locale-like value from the process environment, if present.
    pub locale: Option<String>,
    /// Codeset token parsed from the environment, if present.
    pub codeset: Option<String>,
    /// Windows console code page when it can be queried safely.
    pub console_code_page: Option<u32>,
}

/// Invalid explicit code-page configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidCodePage(pub u32);

impl fmt::Display for InvalidCodePage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid code page: {}", self.0)
    }
}

impl std::error::Error for InvalidCodePage {}

/// Detect locale/codeset input without changing process or console state.
pub fn detect() -> EncodingDetection {
    let raw = std::env::var("DOLOGGER_LOCALE")
        .ok()
        .or_else(|| std::env::var("LC_ALL").ok())
        .or_else(|| std::env::var("LC_MESSAGES").ok())
        .or_else(|| std::env::var("LANG").ok());
    let (locale, codeset) = raw
        .as_deref()
        .map(parse_locale_value)
        .unwrap_or((None, None));
    EncodingDetection {
        locale,
        codeset,
        console_code_page: crate::sys::io::detected_console_code_page(),
    }
}

/// Validate a manually selected code page before it reaches platform I/O.
pub const fn validate_code_page(code_page: u32) -> Result<(), InvalidCodePage> {
    if code_page == 0 || code_page > 65_535 {
        Err(InvalidCodePage(code_page))
    } else {
        Ok(())
    }
}

/// Parse common locale codeset spellings into a Windows-compatible code page.
pub fn parse_code_page(codeset: &str) -> Option<u32> {
    let normalized = codeset.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "utf8" | "utf-8" => Some(65001),
        "ascii" | "us-ascii" => Some(20127),
        "gbk" | "gb2312" | "cp936" | "windows-936" => Some(936),
        "big5" | "cp950" | "windows-950" => Some(950),
        "shift_jis" | "shift-jis" | "cp932" | "windows-932" => Some(932),
        "cp437" | "ibm437" => Some(437),
        value if value.strip_prefix("cp").is_some() => value
            .strip_prefix("cp")
            .and_then(|value| value.parse().ok())
            .filter(|code_page| validate_code_page(*code_page).is_ok()),
        value if value.strip_prefix("windows-").is_some() => value
            .strip_prefix("windows-")
            .and_then(|value| value.parse().ok())
            .filter(|code_page| validate_code_page(*code_page).is_ok()),
        _ => None,
    }
}

fn parse_locale_value(value: &str) -> (Option<String>, Option<String>) {
    let value = value.split('@').next().unwrap_or(value);
    let mut parts = value.splitn(2, '.');
    let locale = parts
        .next()
        .filter(|locale| !locale.is_empty())
        .map(|locale| locale.replace('_', "-"));
    let codeset = parts.next().map(str::to_owned);
    (locale, codeset)
}

#[cfg(windows)]
fn encode_code_page(text: &str, code_page: u32) -> Result<Vec<u8>, EncodingError> {
    validate_code_page(code_page).map_err(EncodingError::InvalidCodePage)?;
    extern "system" {
        fn WideCharToMultiByte(
            code_page: u32,
            flags: u32,
            wide: *const u16,
            wide_len: i32,
            output: *mut u8,
            output_len: i32,
            default_char: *const u8,
            used_default: *mut i32,
        ) -> i32;
    }
    let wide: Vec<u16> = text.encode_utf16().collect();
    // SAFETY: the UTF-16 slice is valid for the duration of both Win32 calls;
    // the first call only queries the required output size.
    let size = unsafe {
        WideCharToMultiByte(
            code_page,
            0,
            wide.as_ptr(),
            wide.len() as i32,
            std::ptr::null_mut(),
            0,
            std::ptr::null(),
            std::ptr::null_mut(),
        )
    };
    if size <= 0 {
        return Err(EncodingError::UnsupportedEncoding(TextEncoding::CodePage(
            code_page,
        )));
    }
    let mut output = vec![0u8; size as usize];
    let mut used_default = 0;
    // SAFETY: output has exactly the size returned by the query call.
    let written = unsafe {
        WideCharToMultiByte(
            code_page,
            0,
            wide.as_ptr(),
            wide.len() as i32,
            output.as_mut_ptr(),
            output.len() as i32,
            std::ptr::null(),
            &mut used_default,
        )
    };
    if written <= 0 {
        return Err(EncodingError::InvalidBytes);
    }
    if used_default != 0 {
        return Err(EncodingError::LossyConversion);
    }
    output.truncate(written as usize);
    Ok(output)
}

#[cfg(not(windows))]
fn encode_code_page(_text: &str, code_page: u32) -> Result<Vec<u8>, EncodingError> {
    validate_code_page(code_page).map_err(EncodingError::InvalidCodePage)?;
    Err(EncodingError::UnsupportedEncoding(TextEncoding::CodePage(
        code_page,
    )))
}

#[cfg(windows)]
fn decode_code_page(bytes: &[u8], code_page: u32) -> Result<String, EncodingError> {
    validate_code_page(code_page).map_err(EncodingError::InvalidCodePage)?;
    extern "system" {
        fn MultiByteToWideChar(
            code_page: u32,
            flags: u32,
            input: *const u8,
            input_len: i32,
            output: *mut u16,
            output_len: i32,
        ) -> i32;
    }
    // SAFETY: the input slice is valid for the duration of both Win32 calls;
    // the first call only queries the required UTF-16 length.
    let size = unsafe {
        MultiByteToWideChar(
            code_page,
            8,
            bytes.as_ptr(),
            bytes.len() as i32,
            std::ptr::null_mut(),
            0,
        )
    };
    if size <= 0 {
        return Err(EncodingError::InvalidBytes);
    }
    let mut wide = vec![0u16; size as usize];
    // SAFETY: wide has exactly the size returned by the query call.
    let written = unsafe {
        MultiByteToWideChar(
            code_page,
            8,
            bytes.as_ptr(),
            bytes.len() as i32,
            wide.as_mut_ptr(),
            wide.len() as i32,
        )
    };
    if written <= 0 {
        return Err(EncodingError::InvalidBytes);
    }
    String::from_utf16(&wide[..written as usize]).map_err(|_| EncodingError::InvalidBytes)
}

#[cfg(not(windows))]
fn decode_code_page(_bytes: &[u8], code_page: u32) -> Result<String, EncodingError> {
    validate_code_page(code_page).map_err(EncodingError::InvalidCodePage)?;
    Err(EncodingError::UnsupportedEncoding(TextEncoding::CodePage(
        code_page,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_platform_code_pages() {
        assert_eq!(parse_code_page("UTF-8"), Some(65001));
        assert_eq!(parse_code_page("GBK"), Some(936));
        assert_eq!(parse_code_page("windows-1252"), Some(1252));
    }

    #[test]
    fn rejects_zero_and_out_of_range_code_pages() {
        assert!(validate_code_page(0).is_err());
        assert!(validate_code_page(65_536).is_err());
        assert!(validate_code_page(936).is_ok());
    }

    #[test]
    fn utf8_codec_round_trips_without_locale_state() {
        let bytes = encode("日志", TextEncoding::Utf8).unwrap();
        assert_eq!(decode(&bytes, TextEncoding::Utf8).unwrap(), "日志");
    }

    #[test]
    fn utf16_codecs_round_trip_strictly() {
        let text = "日志";
        let little_endian = encode(text, TextEncoding::Utf16Le).unwrap();
        let big_endian = encode(text, TextEncoding::Utf16Be).unwrap();
        assert_eq!(decode(&little_endian, TextEncoding::Utf16Le).unwrap(), text);
        assert_eq!(decode(&big_endian, TextEncoding::Utf16Be).unwrap(), text);
    }

    #[test]
    fn explicit_auto_accepts_bom_and_rejects_ambiguous_unknown_bytes() {
        let mut utf16 = vec![0xff, 0xfe];
        utf16.extend_from_slice(&encode("hello", TextEncoding::Utf16Le).unwrap());
        assert_eq!(
            detect_input(&utf16, &[]).unwrap(),
            DetectedInputEncoding::Utf16Le
        );
        assert_eq!(
            detect_input(&[0xff, 0x80, 0x00], &[]),
            Err(AutoDetectionError::Unknown)
        );
    }

    #[test]
    fn auto_prefers_strict_utf8_without_guessing_code_pages() {
        assert_eq!(
            detect_input("plain UTF-8".as_bytes(), &[936, 1252]).unwrap(),
            DetectedInputEncoding::Utf8
        );
    }
}
