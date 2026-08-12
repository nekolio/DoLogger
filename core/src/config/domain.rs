//! Logger domain management with inheritance.
//!
//! Domains allow separate logging configurations for different subsystems.
//! Child domains inherit from parents, with array merging strategies.
//! Non-downgradable items are enforced: children can only tighten, never loosen.

use std::collections::HashMap;

/// A logger domain with its configuration.
#[derive(Debug, Clone)]
pub struct Domain {
    /// Domain name (e.g. "app", "security_audit")
    pub name: String,
    /// Parent domain name (None = root)
    pub inherits: Option<String>,
    /// Log level override
    pub level: Option<String>,
    /// Sinks assigned to this domain
    pub sinks: Vec<String>,
    /// Whether Ed25519 signing is enabled (non-downgradable)
    pub enable_signature: Option<bool>,
    /// Whether HTML escaping is enabled (non-downgradable)
    pub escape_html: Option<bool>,
    /// Whether WORM storage is enabled (non-downgradable)
    pub worm_enabled: Option<bool>,
    /// Whether fsync is forced on every write (non-downgradable)
    pub fsync_on_write: Option<bool>,
    /// Whether TLS is required for remote sinks (non-downgradable)
    pub require_tls: Option<bool>,
    /// Whether Ring 2 fields are included in signature coverage (non-downgradable)
    pub sign_ring2: Option<bool>,
    /// Performance profile for this domain
    pub performance_profile: Option<String>,
    /// Array merge policy: "replace" | "append" | "unique_append"
    pub array_merge_policy: ArrayMergePolicy,
}

/// How arrays are merged during domain inheritance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ArrayMergePolicy {
    /// Child completely replaces parent's array
    Replace,
    /// Child appends to parent's array (may produce duplicates)
    Append,
    /// Child appends only unique items not already in parent
    #[default]
    UniqueAppend,
}

/// Non-downgradable check result.
#[derive(Debug, Clone)]
pub struct NonDowngradableCheck {
    /// Item that was checked
    pub item_name: String,
    /// Expected safety direction
    pub expected_direction: String,
    /// Parent's value
    pub parent_value: String,
    /// Child's attempted value
    pub child_value: String,
    /// Whether the check passed
    pub passed: bool,
    /// Diagnostic message
    pub message: String,
}

/// Manages all logger domains with inheritance resolution.
#[derive(Debug, Clone, Default)]
pub struct DomainManager {
    /// All domains keyed by name
    domains: HashMap<String, Domain>,
    /// The default domain name
    default_domain: String,
}

/// Items that CANNOT be downgraded by child domains (security-sensitive).
/// Children can only tighten (false→true), never loosen (true→false).
pub(crate) const NON_DOWNGRADABLE_ITEMS: &[&str] = &[
    "enable_signature",
    "escape_html",
    "worm_enabled",
    "fsync_on_write",
    "require_tls",
    "sign_ring2",
];

impl DomainManager {
    /// Create a new domain manager with a default domain.
    pub fn new() -> Self {
        let mut domains = HashMap::new();
        domains.insert(
            "default".to_string(),
            Domain {
                name: "default".to_string(),
                inherits: None,
                level: Some("INFO".to_string()),
                sinks: vec!["console".to_string()],
                enable_signature: Some(false),
                escape_html: Some(false),
                worm_enabled: Some(false),
                fsync_on_write: Some(false),
                require_tls: Some(false),
                sign_ring2: Some(false),
                performance_profile: Some("prod-performance".to_string()),
                array_merge_policy: ArrayMergePolicy::UniqueAppend,
            },
        );

        Self {
            domains,
            default_domain: "default".to_string(),
        }
    }

    /// Add a domain.
    pub fn add_domain(&mut self, domain: Domain) -> Result<(), String> {
        // Validate parent exists
        if let Some(ref parent) = domain.inherits {
            if !self.domains.contains_key(parent) {
                return Err(format!(
                    "Domain '{}' inherits from unknown parent '{}'",
                    domain.name, parent
                ));
            }
        }

        // Validate non-downgradable items
        if let Some(ref parent_name) = domain.inherits {
            let parent = &self.domains[parent_name];
            let checks = self.check_non_downgradable(parent, &domain);
            let failures: Vec<_> = checks.iter().filter(|c| !c.passed).collect();
            if !failures.is_empty() {
                let msgs: Vec<_> = failures.iter().map(|c| c.message.clone()).collect();
                return Err(format!(
                    "Domain '{}' violates non-downgradable constraints:\n  {}",
                    domain.name,
                    msgs.join("\n  ")
                ));
            }
        }

        self.domains.insert(domain.name.clone(), domain);
        Ok(())
    }

    /// Resolve a domain's effective configuration by merging with all ancestors.
    pub fn resolve(&self, domain_name: &str) -> Result<Domain, String> {
        let domain = self
            .domains
            .get(domain_name)
            .ok_or_else(|| format!("Domain '{domain_name}' not found"))?;

        // Collect the inheritance chain
        let mut chain = vec![domain.clone()];
        let mut current = domain.clone();
        while let Some(ref parent_name) = current.inherits {
            let parent = self
                .domains
                .get(parent_name)
                .ok_or_else(|| format!("Parent domain '{parent_name}' not found"))?;
            chain.push(parent.clone());
            current = parent.clone();
        }

        // Merge from root (last) to child (first)
        chain.reverse();
        let mut resolved = chain[0].clone();
        for ancestor in &chain[1..] {
            resolved = self.merge_domains(&resolved, ancestor);
        }

        Ok(resolved)
    }

    /// Merge child into parent, with non-downgradable enforcement.
    ///
    /// Child domains can only tighten constraints (false→true for
    /// security booleans), never loosen them. All 6 non-downgradable items
    /// AND the 5 additional domain fields are propagated from child.
    fn merge_domains(&self, parent: &Domain, child: &Domain) -> Domain {
        let mut merged = parent.clone();
        merged.name = child.name.clone();

        // Override with child values (all optional fields)
        if child.level.is_some() {
            merged.level = child.level.clone();
        }
        if child.enable_signature.is_some() {
            merged.enable_signature = child.enable_signature;
        }
        if child.performance_profile.is_some() {
            merged.performance_profile = child.performance_profile.clone();
        }
        // non-downgradable items — child can only tighten
        if child.escape_html.is_some() {
            merged.escape_html = child.escape_html;
        }
        if child.worm_enabled.is_some() {
            merged.worm_enabled = child.worm_enabled;
        }
        if child.sign_ring2.is_some() {
            merged.sign_ring2 = child.sign_ring2;
        }
        if child.fsync_on_write.is_some() {
            merged.fsync_on_write = child.fsync_on_write;
        }
        if child.require_tls.is_some() {
            merged.require_tls = child.require_tls;
        }

        // Merge sinks according to policy
        merged.sinks = match child.array_merge_policy {
            ArrayMergePolicy::Replace => child.sinks.clone(),
            ArrayMergePolicy::Append => {
                let mut s = parent.sinks.clone();
                s.extend(child.sinks.clone());
                s
            }
            ArrayMergePolicy::UniqueAppend => {
                let mut s = parent.sinks.clone();
                for sink in &child.sinks {
                    if !s.contains(sink) {
                        s.push(sink.clone());
                    }
                }
                s
            }
        };

        merged
    }

    /// Check non-downgradable constraints between parent and child domain.
    pub fn check_non_downgradable(
        &self,
        parent: &Domain,
        child: &Domain,
    ) -> Vec<NonDowngradableCheck> {
        let mut results = Vec::new();

        for &item in NON_DOWNGRADABLE_ITEMS {
            let (parent_val, child_val) = match item {
                "enable_signature" => (parent.enable_signature, child.enable_signature),
                "escape_html" => (parent.escape_html, child.escape_html),
                "worm_enabled" => (parent.worm_enabled, child.worm_enabled),
                "fsync_on_write" => (parent.fsync_on_write, child.fsync_on_write),
                "require_tls" => (parent.require_tls, child.require_tls),
                "sign_ring2" => (parent.sign_ring2, child.sign_ring2),
                _ => continue,
            };

            if let (Some(p_val), Some(c_val)) = (parent_val, child_val) {
                // Non-downgradable: child can only tighten (false→true), never loosen (true→false)
                if p_val && !c_val {
                    results.push(NonDowngradableCheck {
                        item_name: item.into(),
                        expected_direction: "must be >= parent (non-downgradable)".into(),
                        parent_value: p_val.to_string(),
                        child_value: c_val.to_string(),
                        passed: false,
                        message: format!(
                            "Domain '{}' attempts to disable {} (parent '{}' has it enabled) — this item is non-downgradable",
                            child.name, item, parent.name
                        ),
                    });
                }
            }
        }

        results
    }

    /// List all domain names.
    pub fn domain_names(&self) -> Vec<&str> {
        self.domains.keys().map(|s| s.as_str()).collect()
    }

    /// Get the default domain name.
    pub fn default_domain(&self) -> &str {
        &self.default_domain
    }

    /// Check if a domain exists.
    pub fn contains(&self, name: &str) -> bool {
        self.domains.contains_key(name)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_domain() {
        let mgr = DomainManager::new();
        let resolved = mgr.resolve("default").unwrap();
        assert_eq!(resolved.level.as_deref(), Some("INFO"));
        assert_eq!(resolved.sinks, vec!["console"]);
    }

    #[test]
    fn test_inheritance() {
        let mut mgr = DomainManager::new();

        mgr.add_domain(Domain {
            name: "security".into(),
            inherits: Some("default".into()),
            level: Some("AUDIT".into()),
            sinks: vec!["worm_file".into()],
            enable_signature: Some(true),
            performance_profile: None,
            escape_html: None,
            worm_enabled: None,
            fsync_on_write: None,
            require_tls: None,
            sign_ring2: None,
            array_merge_policy: ArrayMergePolicy::UniqueAppend,
        })
        .unwrap();

        let resolved = mgr.resolve("security").unwrap();
        assert_eq!(resolved.level.as_deref(), Some("AUDIT"));
        assert!(resolved.enable_signature.unwrap());
        // Should have both default's console + security's worm_file
        assert!(resolved.sinks.contains(&"console".to_string()));
        assert!(resolved.sinks.contains(&"worm_file".to_string()));
    }

    #[test]
    fn test_non_downgradable_signature() {
        let mut mgr = DomainManager::new();

        // Enable signature on parent
        mgr.add_domain(Domain {
            name: "audit_parent".into(),
            inherits: Some("default".into()),
            level: Some("AUDIT".into()),
            sinks: vec!["worm_file".into()],
            enable_signature: Some(true),
            performance_profile: None,
            escape_html: None,
            worm_enabled: None,
            fsync_on_write: None,
            require_tls: None,
            sign_ring2: None,
            array_merge_policy: ArrayMergePolicy::UniqueAppend,
        })
        .unwrap();

        // Try to disable signature on child
        let result = mgr.add_domain(Domain {
            name: "audit_child".into(),
            inherits: Some("audit_parent".into()),
            level: Some("INFO".into()),
            sinks: vec![],
            enable_signature: Some(false),
            performance_profile: None,
            escape_html: None,
            worm_enabled: None,
            fsync_on_write: None,
            require_tls: None,
            sign_ring2: None,
            array_merge_policy: ArrayMergePolicy::UniqueAppend,
        });

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("non-downgradable"));
    }

    #[test]
    fn test_unknown_parent() {
        let mut mgr = DomainManager::new();
        let result = mgr.add_domain(Domain {
            name: "orphan".into(),
            inherits: Some("nonexistent".into()),
            level: None,
            sinks: vec![],
            enable_signature: None,
            performance_profile: None,
            escape_html: None,
            worm_enabled: None,
            fsync_on_write: None,
            require_tls: None,
            sign_ring2: None,
            array_merge_policy: ArrayMergePolicy::UniqueAppend,
        });
        assert!(result.is_err());
    }
}
