//! Immutable derived message views.
//!
//! A view is an external representation of a Record message. It never becomes
//! the source of truth for hashing, signing, WORM storage, or KV persistence.

use std::fmt;
use std::sync::Arc;

/// Maximum bytes held by one derived view.
pub const MAX_DERIVED_VIEW_BYTES: usize = 8 * 1024 * 1024;
/// Maximum number of transformations in one derived view chain.
pub const MAX_DERIVED_VIEW_DEPTH: u8 = 16;

/// A bounded description of the transformation that produced a view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewTransform {
    /// The view is the result of text decoding.
    Decode {
        /// Encoding identifier used by the decoder.
        encoding: String,
    },
    /// The view is the result of output encoding.
    Encode {
        /// Encoding identifier used by the encoder.
        encoding: String,
    },
    /// The view is a formatter or plugin result.
    Format {
        /// Formatter or plugin identifier.
        formatter: String,
    },
}

impl ViewTransform {
    fn validate(&self) -> Result<(), ViewError> {
        let value = match self {
            Self::Decode { encoding } | Self::Encode { encoding } => encoding,
            Self::Format { formatter } => formatter,
        };
        if value.is_empty() || value.len() > 128 || value.bytes().any(|byte| byte == 0) {
            return Err(ViewError::InvalidTransform);
        }
        Ok(())
    }
}

/// An immutable, bounded representation derived from a Record payload.
#[derive(Debug, Clone)]
pub struct DerivedMessageView {
    bytes: Arc<[u8]>,
    source_content_hash: [u8; 32],
    parent_view_id: Option<[u8; 32]>,
    depth: u8,
    transform: ViewTransform,
}

impl DerivedMessageView {
    /// Create a view rooted at a Record's canonical content hash.
    pub fn from_record(
        source_content_hash: [u8; 32],
        bytes: &[u8],
        transform: ViewTransform,
    ) -> Result<Self, ViewError> {
        Self::new(source_content_hash, None, 0, bytes, transform)
    }

    /// Create a view derived from another immutable view.
    pub fn derive(
        parent: &Self,
        bytes: &[u8],
        transform: ViewTransform,
    ) -> Result<Self, ViewError> {
        let parent_view_id = parent.view_id();
        Self::new(
            parent.source_content_hash,
            Some(parent_view_id),
            parent.depth.saturating_add(1),
            bytes,
            transform,
        )
    }

    fn new(
        source_content_hash: [u8; 32],
        parent_view_id: Option<[u8; 32]>,
        depth: u8,
        bytes: &[u8],
        transform: ViewTransform,
    ) -> Result<Self, ViewError> {
        if bytes.len() > MAX_DERIVED_VIEW_BYTES {
            return Err(ViewError::SizeExceeded {
                actual: bytes.len(),
                maximum: MAX_DERIVED_VIEW_BYTES,
            });
        }
        if depth > MAX_DERIVED_VIEW_DEPTH {
            return Err(ViewError::DepthExceeded {
                actual: depth,
                maximum: MAX_DERIVED_VIEW_DEPTH,
            });
        }
        transform.validate()?;
        Ok(Self {
            bytes: Arc::from(bytes),
            source_content_hash,
            parent_view_id,
            depth,
            transform,
        })
    }

    /// Return the immutable view bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Return the source Record content hash.
    pub fn source_content_hash(&self) -> &[u8; 32] {
        &self.source_content_hash
    }

    /// Return the parent view identifier, if this is a derived view.
    pub fn parent_view_id(&self) -> Option<&[u8; 32]> {
        self.parent_view_id.as_ref()
    }

    /// Return the transformation depth.
    pub fn depth(&self) -> u8 {
        self.depth
    }

    /// Return the transformation descriptor.
    pub fn transform(&self) -> &ViewTransform {
        &self.transform
    }

    /// Return a deterministic identifier for this view metadata and content.
    pub fn view_id(&self) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(self.source_content_hash);
        if let Some(parent_view_id) = self.parent_view_id {
            hasher.update(parent_view_id);
        }
        hasher.update([self.depth]);
        hasher.update(format!("{:?}", self.transform).as_bytes());
        hasher.update((self.bytes.len() as u64).to_le_bytes());
        hasher.update(&self.bytes);
        hasher.finalize().into()
    }
}

/// Errors returned when constructing a derived view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewError {
    /// View bytes exceed the configured hard limit.
    SizeExceeded {
        /// Actual byte count.
        actual: usize,
        /// Configured maximum byte count.
        maximum: usize,
    },
    /// The view chain exceeds the configured hard depth.
    DepthExceeded {
        /// Actual chain depth.
        actual: u8,
        /// Configured maximum chain depth.
        maximum: u8,
    },
    /// Transformation metadata is empty, oversized, or contains NUL.
    InvalidTransform,
}

impl fmt::Display for ViewError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SizeExceeded { actual, maximum } => {
                write!(formatter, "derived view size {actual} exceeds {maximum}")
            }
            Self::DepthExceeded { actual, maximum } => {
                write!(formatter, "derived view depth {actual} exceeds {maximum}")
            }
            Self::InvalidTransform => formatter.write_str("invalid derived view transform"),
        }
    }
}

impl std::error::Error for ViewError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_views_preserve_source_and_parent_chain() {
        let root = DerivedMessageView::from_record(
            [7; 32],
            b"raw",
            ViewTransform::Decode {
                encoding: "utf-8".to_string(),
            },
        )
        .unwrap();
        let child = DerivedMessageView::derive(
            &root,
            b"formatted",
            ViewTransform::Format {
                formatter: "text".to_string(),
            },
        )
        .unwrap();
        assert_eq!(child.source_content_hash(), &[7; 32]);
        assert_eq!(child.parent_view_id(), Some(&root.view_id()));
        assert_eq!(child.depth(), 1);
        assert_ne!(root.view_id(), child.view_id());
    }

    #[test]
    fn derived_views_reject_unbounded_resources() {
        assert!(matches!(
            DerivedMessageView::from_record(
                [0; 32],
                &vec![0; MAX_DERIVED_VIEW_BYTES + 1],
                ViewTransform::Format {
                    formatter: "test".to_string(),
                },
            ),
            Err(ViewError::SizeExceeded { .. })
        ));
    }
}
