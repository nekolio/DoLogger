//! Hexadecimal encoding and decoding.
//!
//! Provides [`encode`], [`encode_upper`], [`decode`], and
//! [`encode_to_slice`]/[`decode_to_slice`] for working with hex strings.
//! Lower-case is the default; both cases are accepted on decode and may be
//! freely mixed.
//!
//! # Example
//!
//! ```
//! use dologger_core::hex;
//!
//! assert_eq!(hex::encode("Hello world!"), "48656c6c6f20776f726c6421");
//! assert_eq!(hex::encode_upper("Hi"), "4869");
//! assert_eq!(hex::decode("48656c6c6f20776f726c6421").unwrap(),
//!            b"Hello world!".to_vec());
//! ```

#![allow(clippy::unreadable_literal)]

use core::fmt;
use core::iter;

/// The error type for decoding a hex string into `Vec<u8>` or `[u8; N]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FromHexError {
    /// An invalid character was found. Valid ones are: `0...9`, `a...f`
    /// or `A...F`.
    InvalidHexCharacter {
        /// The offending character.
        c: char,
        /// Byte index of the offending character in the input.
        index: usize,
    },

    /// A hex string's length needs to be even, as two digits correspond
    /// to one byte.
    OddLength,

    /// If the hex string is decoded into a fixed sized container, such
    /// as an array, the hex string's length * 2 has to match the
    /// container's length.
    InvalidStringLength,
}

impl fmt::Display for FromHexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            FromHexError::InvalidHexCharacter { c, index } => {
                write!(f, "Invalid character {:?} at position {}", c, index)
            }
            FromHexError::OddLength => write!(f, "Odd number of digits"),
            FromHexError::InvalidStringLength => write!(f, "Invalid string length"),
        }
    }
}

impl std::error::Error for FromHexError {}

const HEX_CHARS_LOWER: &[u8; 16] = b"0123456789abcdef";
const HEX_CHARS_UPPER: &[u8; 16] = b"0123456789ABCDEF";

/// Encoding values as hex string.
///
/// This trait is implemented for all `T` which implement `AsRef<[u8]>`.
/// This includes `String`, `str`, `Vec<u8>` and `[u8]`.
pub trait ToHex {
    /// Encode the hex strict representing `self` into the result. Lower
    /// case letters are used (e.g. `f9b4ca`).
    fn encode_hex<T: iter::FromIterator<char>>(&self) -> T;

    /// Encode the hex strict representing `self` into the result. Upper
    /// case letters are used (e.g. `F9B4CA`).
    fn encode_hex_upper<T: iter::FromIterator<char>>(&self) -> T;
}

struct BytesToHexChars<'a> {
    inner: core::slice::Iter<'a, u8>,
    table: &'static [u8; 16],
    next: Option<char>,
}

impl<'a> BytesToHexChars<'a> {
    fn new(inner: &'a [u8], table: &'static [u8; 16]) -> BytesToHexChars<'a> {
        BytesToHexChars {
            inner: inner.iter(),
            table,
            next: None,
        }
    }
}

impl<'a> Iterator for BytesToHexChars<'a> {
    type Item = char;

    fn next(&mut self) -> Option<Self::Item> {
        match self.next.take() {
            Some(current) => Some(current),
            None => self.inner.next().map(|byte| {
                let current = self.table[(byte >> 4) as usize] as char;
                self.next = Some(self.table[(byte & 0x0F) as usize] as char);
                current
            }),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let length = self.len();
        (length, Some(length))
    }
}

impl<'a> iter::ExactSizeIterator for BytesToHexChars<'a> {
    fn len(&self) -> usize {
        let mut length = self.inner.len() * 2;
        if self.next.is_some() {
            length += 1;
        }
        length
    }
}

#[inline]
fn encode_to_iter<T: iter::FromIterator<char>>(table: &'static [u8; 16], source: &[u8]) -> T {
    BytesToHexChars::new(source, table).collect()
}

impl<T: AsRef<[u8]>> ToHex for T {
    fn encode_hex<U: iter::FromIterator<char>>(&self) -> U {
        encode_to_iter(HEX_CHARS_LOWER, self.as_ref())
    }

    fn encode_hex_upper<U: iter::FromIterator<char>>(&self) -> U {
        encode_to_iter(HEX_CHARS_UPPER, self.as_ref())
    }
}

/// Types that can be decoded from a hex string.
pub trait FromHex {
    /// The error type produced by a failed decode.
    type Error;

    /// Decode the hex string into `Self`.
    fn from_hex<T: AsRef<[u8]>>(hex: T) -> Result<Self, Self::Error>
    where
        Self: Sized;
}

/// Decode any byte slice as hex.
#[inline]
fn val(data: &[u8], pos: usize) -> Result<u8, FromHexError> {
    match data[pos] {
        b'0'..=b'9' => Ok(data[pos] - b'0'),
        b'a'..=b'f' => Ok(data[pos] - b'a' + 10),
        b'A'..=b'F' => Ok(data[pos] - b'A' + 10),
        _ => Err(FromHexError::InvalidHexCharacter {
            c: data[pos] as char,
            index: pos,
        }),
    }
}

/// Encodes `data` as hex string using lowercase characters.
///
/// # Example
///
/// ```
/// use dologger_core::hex;
///
/// assert_eq!(hex::encode("foobar"), "666f6f626172");
/// ```
#[must_use]
pub fn encode<T: AsRef<[u8]>>(data: T) -> String {
    data.encode_hex()
}

/// Encodes `data` as hex string using uppercase characters.
///
/// Apart from the characters' casing, this works exactly like
/// [`encode`].
///
/// # Example
///
/// ```
/// use dologger_core::hex;
///
/// assert_eq!(hex::encode_upper("Hello world!"), "48656C6C6F20776F726C6421");
/// assert_eq!(hex::encode_upper(vec![1, 2, 3, 15, 16]), "0102030F10");
/// ```
#[must_use]
pub fn encode_upper<T: AsRef<[u8]>>(data: T) -> String {
    data.encode_hex_upper()
}

/// Encodes `input` as hex string into `output`.
///
/// Returns `InvalidStringLength` if `output.len() != input.len() * 2`.
///
/// # Example
///
/// ```
/// use dologger_core::hex;
///
/// let mut output = [0; 8];
/// hex::encode_to_slice(b"kiwi", &mut output).unwrap();
/// assert_eq!(&output, b"6b697769");
/// ```
pub fn encode_to_slice<T: AsRef<[u8]>>(input: T, output: &mut [u8]) -> Result<(), FromHexError> {
    let input = input.as_ref();

    if input.len() * 2 != output.len() {
        return Err(FromHexError::InvalidStringLength);
    }

    // Two characters per byte: walk the output in pairs and write the
    // hex pair directly.  No allocation, no iterator chaining.
    let mut i = 0;
    for &byte in input {
        output[i] = HEX_CHARS_LOWER[(byte >> 4) as usize];
        output[i + 1] = HEX_CHARS_LOWER[(byte & 0x0F) as usize];
        i += 2;
    }

    Ok(())
}

/// Decodes a hex string into raw bytes.
///
/// Both, upper and lower case characters are valid in the input string
/// and can even be mixed (e.g. `f9b4ca`, `F9B4CA` and `f9B4Ca` are all
/// valid strings).
///
/// # Example
///
/// ```
/// use dologger_core::hex;
///
/// assert_eq!(
///     hex::decode("48656c6c6f20776f726c6421"),
///     Ok("Hello world!".to_owned().into_bytes())
/// );
///
/// assert_eq!(hex::decode("123"), Err(hex::FromHexError::OddLength));
/// assert!(hex::decode("foo").is_err());
/// ```
pub fn decode<T: AsRef<[u8]>>(data: T) -> Result<Vec<u8>, FromHexError> {
    Vec::from_hex(data)
}

/// Decode a hex string into a mutable bytes slice.
///
/// Both, upper and lower case characters are valid in the input string
/// and can even be mixed.
///
/// # Example
///
/// ```
/// use dologger_core::hex;
///
/// let mut bytes = [0u8; 4];
/// assert_eq!(hex::decode_to_slice("6b697769", &mut bytes as &mut [u8]), Ok(()));
/// assert_eq!(&bytes, b"kiwi");
/// ```
pub fn decode_to_slice<T: AsRef<[u8]>>(data: T, out: &mut [u8]) -> Result<(), FromHexError> {
    let data = data.as_ref();

    if data.len() % 2 != 0 {
        return Err(FromHexError::OddLength);
    }
    if data.len() / 2 != out.len() {
        return Err(FromHexError::InvalidStringLength);
    }

    for (i, byte) in out.iter_mut().enumerate() {
        *byte = val(data, 2 * i)? << 4 | val(data, 2 * i + 1)?;
    }

    Ok(())
}

impl FromHex for Vec<u8> {
    type Error = FromHexError;

    fn from_hex<T: AsRef<[u8]>>(hex: T) -> Result<Self, Self::Error> {
        let hex = hex.as_ref();
        if hex.len() % 2 != 0 {
            return Err(FromHexError::OddLength);
        }
        let mut out = Vec::with_capacity(hex.len() / 2);
        let mut i = 0;
        while i < hex.len() {
            let hi = val(hex, i)?;
            let lo = val(hex, i + 1)?;
            out.push((hi << 4) | lo);
            i += 2;
        }
        Ok(out)
    }
}

// `decode_to_array` and array impls intentionally omitted: callers in
// this codebase only need `Vec<u8>` and `&mut [u8]` targets.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode() {
        assert_eq!(encode("foobar"), "666f6f626172");
        assert_eq!(encode(vec![1, 2, 3, 15, 16]), "0102030f10");
    }

    #[test]
    fn test_encode_upper() {
        assert_eq!(encode_upper("Hello world!"), "48656C6C6F20776F726C6421");
        assert_eq!(encode_upper(vec![1, 2, 3, 15, 16]), "0102030F10");
    }

    #[test]
    fn test_decode() {
        assert_eq!(
            decode("666f6f626172"),
            Ok(String::from("foobar").into_bytes())
        );
        assert_eq!(decode("123"), Err(FromHexError::OddLength));
        assert!(decode("foo").is_err());
    }

    #[test]
    fn test_decode_mixed_case() {
        assert_eq!(Vec::from_hex("666f6F626172").unwrap(), b"foobar");
        assert_eq!(Vec::from_hex("666F6F626172").unwrap(), b"foobar");
    }

    #[test]
    fn test_encode_to_slice() {
        let mut out = [0u8; 8];
        encode_to_slice(b"kiwi", &mut out).unwrap();
        assert_eq!(&out, b"6b697769");

        let mut out = [0u8; 10];
        encode_to_slice(b"kiwis", &mut out).unwrap();
        assert_eq!(&out, b"6b69776973");

        let mut out = [0u8; 100];
        assert_eq!(
            encode_to_slice(b"kiwis", &mut out),
            Err(FromHexError::InvalidStringLength)
        );
    }

    #[test]
    fn test_decode_to_slice() {
        let mut out = [0u8; 4];
        decode_to_slice(b"6b697769", &mut out).unwrap();
        assert_eq!(&out, b"kiwi");

        let mut out = [0u8; 5];
        decode_to_slice(b"6b69776973", &mut out).unwrap();
        assert_eq!(&out, b"kiwis");

        let mut out = [0u8; 4];
        assert_eq!(
            decode_to_slice(b"6", &mut out),
            Err(FromHexError::OddLength)
        );
    }

    #[test]
    fn test_error_display() {
        assert_eq!(
            FromHexError::InvalidHexCharacter { c: '\n', index: 5 }.to_string(),
            "Invalid character '\\n' at position 5"
        );
        assert_eq!(FromHexError::OddLength.to_string(), "Odd number of digits");
        assert_eq!(
            FromHexError::InvalidStringLength.to_string(),
            "Invalid string length"
        );
    }

    #[test]
    fn test_to_hex_trait_string() {
        let s: String = "foobar".encode_hex();
        assert_eq!(s, "666f6f626172");
    }

    #[test]
    fn test_to_hex_trait_array() {
        let arr: [u8; 4] = [0xde, 0xad, 0xbe, 0xef];
        let s: String = arr.encode_hex();
        assert_eq!(s, "deadbeef");
    }
}
