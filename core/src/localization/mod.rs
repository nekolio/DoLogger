//! Localization services for human-facing messages.
//!
//! Localization consumes core codec capabilities at its outer display boundary,
//! but it does not own encoding policy and never transforms persisted records,
//! SIF/WORM bytes, signatures, hashes, or audit-chain material.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, RwLock};

/// Default locale used when detection and explicit configuration are absent.
pub const DEFAULT_LOCALE: &str = "en-US";
/// Maximum accepted BCP-47 tag length for the built-in detector.
pub const MAX_LOCALE_LENGTH: usize = 32;
/// Maximum catalog key length accepted by the runtime registry.
pub const MAX_MESSAGE_KEY_LENGTH: usize = 128;
/// Maximum translated message length accepted by the runtime registry.
pub const MAX_MESSAGE_LENGTH: usize = 4096;

/// Errors raised while validating or installing a message catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalizationError {
    /// The locale tag is not a supported ASCII BCP-47 subset.
    InvalidLocale(String),
    /// A message key is empty, too long, or contains an unsafe character.
    InvalidMessageKey(String),
    /// A translated message is empty, too long, or contains a NUL byte.
    InvalidMessage(String),
    /// The catalog contains the same key more than once.
    DuplicateMessageKey(String),
    /// A concurrent catalog snapshot could not be read or replaced.
    RegistryPoisoned,
}

impl fmt::Display for LocalizationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLocale(value) => write!(formatter, "invalid locale: {value}"),
            Self::InvalidMessageKey(value) => write!(formatter, "invalid message key: {value}"),
            Self::InvalidMessage(value) => write!(formatter, "invalid localized message: {value}"),
            Self::DuplicateMessageKey(value) => write!(formatter, "duplicate message key: {value}"),
            Self::RegistryPoisoned => formatter.write_str("localization registry poisoned"),
        }
    }
}

impl std::error::Error for LocalizationError {}

/// An ordered locale fallback chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocaleChain {
    tags: Vec<String>,
}

impl LocaleChain {
    /// Build a chain from an optional locale tag.
    pub fn new(requested: Option<&str>) -> Result<Self, LocalizationError> {
        let requested = requested.unwrap_or(DEFAULT_LOCALE);
        let normalized = normalize_locale(requested)?;
        let mut tags = vec![normalized.clone()];

        if let Some((language, _)) = normalized.split_once('-') {
            tags.push(language.to_string());
        }
        if normalized != DEFAULT_LOCALE {
            tags.push(DEFAULT_LOCALE.to_string());
        }

        Ok(Self { tags })
    }

    /// Detect the locale without consulting platform-specific unsafe APIs.
    ///
    /// Precedence is explicit value, `DOLOGGER_LOCALE`, `LC_ALL`, `LANG`, then
    /// the built-in English fallback. Environment input is treated as
    /// untrusted and validated before it enters the chain.
    pub fn detect(explicit: Option<&str>) -> Result<Self, LocalizationError> {
        let detected = explicit
            .map(str::to_owned)
            .or_else(|| std::env::var("DOLOGGER_LOCALE").ok())
            .or_else(|| std::env::var("LC_ALL").ok())
            .or_else(|| std::env::var("LANG").ok())
            .map(|value| value.split('.').next().unwrap_or(&value).replace('_', "-"));
        Self::new(detected.as_deref())
    }

    /// Return the ordered tags from most specific to least specific.
    pub fn tags(&self) -> &[String] {
        &self.tags
    }
}

/// An immutable, validated catalog snapshot.
#[derive(Debug, Clone)]
pub struct MessageCatalog {
    locale: String,
    messages: Arc<HashMap<String, Arc<str>>>,
}

impl MessageCatalog {
    /// Build a catalog from trusted application/plugin entries.
    pub fn from_entries<I, K, V>(locale: &str, entries: I) -> Result<Self, LocalizationError>
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let locale = normalize_locale(locale)?;
        let mut messages = HashMap::new();
        for (key, value) in entries {
            let key = key.into();
            let value = value.into();
            validate_message_key(&key)?;
            validate_message(&value)?;
            if messages
                .insert(key.clone(), Arc::<str>::from(value))
                .is_some()
            {
                return Err(LocalizationError::DuplicateMessageKey(key));
            }
        }
        Ok(Self {
            locale,
            messages: Arc::new(messages),
        })
    }

    /// Return the normalized locale tag owned by this catalog.
    pub fn locale(&self) -> &str {
        &self.locale
    }

    /// Look up a key without allocating or formatting a fallback.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.messages.get(key).map(AsRef::as_ref)
    }
}

/// Thread-safe catalog registry used only by human-facing output paths.
pub struct LocalizationRegistry {
    chain: LocaleChain,
    catalogs: RwLock<HashMap<String, MessageCatalog>>,
}

impl LocalizationRegistry {
    /// Create an empty registry for a locale chain.
    pub fn new(chain: LocaleChain) -> Self {
        Self {
            chain,
            catalogs: RwLock::new(HashMap::new()),
        }
    }

    /// Install or replace an immutable catalog snapshot.
    pub fn install(&self, catalog: MessageCatalog) -> Result<(), LocalizationError> {
        let mut catalogs = self
            .catalogs
            .write()
            .map_err(|_| LocalizationError::RegistryPoisoned)?;
        catalogs.insert(catalog.locale().to_string(), catalog);
        Ok(())
    }

    /// Resolve a localized message with deterministic fallback.
    pub fn resolve(&self, key: &str, fallback: &str) -> Result<String, LocalizationError> {
        validate_message_key(key)?;
        let catalogs = self
            .catalogs
            .read()
            .map_err(|_| LocalizationError::RegistryPoisoned)?;
        for locale in self.chain.tags() {
            if let Some(message) = catalogs.get(locale).and_then(|catalog| catalog.get(key)) {
                return Ok(message.to_string());
            }
        }
        Ok(fallback.to_string())
    }

    /// Return the active fallback chain.
    pub fn chain(&self) -> &LocaleChain {
        &self.chain
    }
}

fn normalize_locale(value: &str) -> Result<String, LocalizationError> {
    let value = value.trim();
    if value.is_empty() || value.len() > MAX_LOCALE_LENGTH || !value.is_ascii() {
        return Err(LocalizationError::InvalidLocale(value.to_string()));
    }

    let mut parts = value.split('-');
    let language = parts.next().unwrap_or_default();
    if !(2..=8).contains(&language.len())
        || !language.bytes().all(|byte| byte.is_ascii_alphabetic())
    {
        return Err(LocalizationError::InvalidLocale(value.to_string()));
    }

    let mut normalized = language.to_ascii_lowercase();
    for part in parts {
        if part.is_empty()
            || part.len() > 8
            || !part.bytes().all(|byte| byte.is_ascii_alphanumeric())
        {
            return Err(LocalizationError::InvalidLocale(value.to_string()));
        }
        normalized.push('-');
        if part.len() == 4 && part.bytes().all(|byte| byte.is_ascii_alphabetic()) {
            let mut chars = part.chars();
            if let Some(first) = chars.next() {
                normalized.push(first.to_ascii_uppercase());
                normalized.extend(chars.map(|character| character.to_ascii_lowercase()));
            }
        } else if part.len() == 2 && part.bytes().all(|byte| byte.is_ascii_alphabetic()) {
            normalized.push_str(&part.to_ascii_uppercase());
        } else {
            normalized.push_str(&part.to_ascii_lowercase());
        }
    }
    Ok(normalized)
}

fn validate_message_key(key: &str) -> Result<(), LocalizationError> {
    if key.is_empty()
        || key.len() > MAX_MESSAGE_KEY_LENGTH
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(LocalizationError::InvalidMessageKey(key.to_string()));
    }
    Ok(())
}

fn validate_message(message: &str) -> Result<(), LocalizationError> {
    if message.is_empty() || message.len() > MAX_MESSAGE_LENGTH || message.contains('\0') {
        return Err(LocalizationError::InvalidMessage(message.to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_chain_prefers_specific_language_then_english() {
        let chain = LocaleChain::new(Some("zh-CN")).unwrap();
        assert_eq!(chain.tags(), &["zh-CN", "zh", "en-US"]);
    }

    #[test]
    fn catalog_rejects_unsafe_message_keys() {
        let result = MessageCatalog::from_entries("en-US", [("../secret", "value")]);
        assert!(matches!(
            result,
            Err(LocalizationError::InvalidMessageKey(_))
        ));
    }

    #[test]
    fn registry_resolves_specific_catalog_before_fallback() {
        let registry = LocalizationRegistry::new(LocaleChain::new(Some("zh-CN")).unwrap());
        registry
            .install(
                MessageCatalog::from_entries("en-US", [("error.invalid_arg", "invalid")]).unwrap(),
            )
            .unwrap();
        registry
            .install(
                MessageCatalog::from_entries("zh", [("error.invalid_arg", "参数无效")]).unwrap(),
            )
            .unwrap();
        assert_eq!(
            registry.resolve("error.invalid_arg", "fallback").unwrap(),
            "参数无效"
        );
    }

    #[test]
    fn registry_falls_back_to_default_message_when_key_is_missing() {
        let registry = LocalizationRegistry::new(LocaleChain::new(Some("fr-FR")).unwrap());
        assert_eq!(
            registry
                .resolve("error.invalid_arg", "invalid argument")
                .unwrap(),
            "invalid argument"
        );
    }
}
