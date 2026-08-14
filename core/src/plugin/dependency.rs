//! Field dependency validation.
//!
//! At plugin load time, validates that all `requires_fields` declared
//! in each plugin's manifest are satisfied by the current pipeline.
//! Detects missing fields and circular dependencies.

use std::collections::{HashMap, HashSet};

/// A field dependency declaration from a plugin.
#[derive(Debug, Clone)]
pub struct FieldDependency {
    /// The plugin that requires the field
    pub plugin_name: String,
    /// Fields the plugin requires (MUST be present before this plugin runs)
    pub requires: Vec<String>,
    /// Fields the plugin optionally depends on (best-effort)
    pub requires_optional: Vec<String>,
    /// Fields the plugin provides (adds to the record)
    pub provides: Vec<String>,
    /// Pipeline stage index this plugin runs at (0=PreFilter .. 6=Sink).
    /// Used by `validate_pipeline_ordering` to detect ordering violations.
    pub pipeline_stage: Option<u32>,
}

/// Result of dependency validation.
#[derive(Debug)]
pub struct ValidationResult {
    /// Whether all dependencies are satisfied
    pub satisfied: bool,
    /// Missing required fields
    pub missing_required: Vec<MissingField>,
    /// Detected circular dependencies
    pub circular_deps: Vec<CircularDep>,
}

/// A missing required field.
#[derive(Debug)]
pub struct MissingField {
    /// Plugin that requires the field
    pub plugin: String,
    /// Name of the missing field
    pub field: String,
}

/// A circular dependency chain.
#[derive(Debug)]
pub struct CircularDep {
    /// The cycle of plugin names
    pub chain: Vec<String>,
}

/// Validates field dependencies across all loaded plugins.
pub struct DependencyValidator {
    plugins: Vec<FieldDependency>,
}

impl DependencyValidator {
    /// Create a new validator.
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
        }
    }

    /// Register a plugin's field dependencies.
    pub fn register(&mut self, dep: FieldDependency) {
        self.plugins.push(dep);
    }

    /// Validate all registered dependencies.
    pub fn validate(&self) -> ValidationResult {
        let mut missing_required = Vec::new();

        // Build a map of which plugin provides which fields
        let mut providers: HashMap<String, Vec<String>> = HashMap::new();
        for dep in &self.plugins {
            let entry = providers.entry(dep.plugin_name.clone()).or_default();
            entry.extend(dep.provides.clone());
        }

        // Collect all available fields (in pipeline order)
        let all_fields: HashSet<String> =
            providers.values().flat_map(|v| v.iter().cloned()).collect();

        // Check each plugin's required fields
        for dep in &self.plugins {
            for field in &dep.requires {
                if !all_fields.contains(field) {
                    // Check if any plugin provides it
                    let provided = providers.values().any(|fields| fields.contains(field));
                    if !provided {
                        missing_required.push(MissingField {
                            plugin: dep.plugin_name.clone(),
                            field: field.clone(),
                        });
                    }
                }
            }
        }

        // Check for circular dependencies
        let circular_deps = self.detect_cycles();

        ValidationResult {
            satisfied: missing_required.is_empty() && circular_deps.is_empty(),
            missing_required,
            circular_deps,
        }
    }

    /// Validate pipeline stage ordering for field dependencies.
    ///
    /// For every plugin that requires a field, this checks that the provider
    /// plugin runs at an earlier pipeline stage. If a consumer depends on a
    /// field from a provider at the same or later stage, that is an ordering
    /// violation — the provider hasn't run yet when the consumer needs the field.
    ///
    /// This is a simplified check for now that reports warnings via diag
    /// rather than rejecting plugins. A full enforcement is planned
    /// when the plugin dependency graph is formalised.
    pub fn validate_pipeline_ordering(&self) {
        // Build a map: field_name -> Vec<(provider_name, stage_index)>
        let mut field_providers: HashMap<&str, Vec<(&str, u32)>> = HashMap::new();
        for dep in &self.plugins {
            let stage = match dep.pipeline_stage {
                Some(s) => s,
                None => continue,
            };
            for field in &dep.provides {
                field_providers
                    .entry(field.as_str())
                    .or_default()
                    .push((dep.plugin_name.as_str(), stage));
            }
        }

        // Check each consumer plugin
        for dep in &self.plugins {
            let consumer_stage = match dep.pipeline_stage {
                Some(s) => s,
                None => continue,
            };
            for field in &dep.requires {
                if let Some(providers) = field_providers.get(field.as_str()) {
                    for &(provider_name, provider_stage) in providers {
                        if provider_stage >= consumer_stage {
                            // Provider is at the same or later stage — ordering violation
                            crate::sys::diag::warn(
                                "dependency",
                                &format!(
                                    "Pipeline ordering violation: plugin '{}' (stage {}) requires field '{}' \
                                     provided by '{}' (stage {}), but provider must be at an earlier stage. \
                                     The field will not be available at the time the consumer runs.",
                                    dep.plugin_name,
                                    consumer_stage,
                                    field,
                                    provider_name,
                                    provider_stage,
                                ),
                            );
                        }
                    }
                }
            }
        }
    }

    /// Detect circular dependencies between plugins.
    fn detect_cycles(&self) -> Vec<CircularDep> {
        let mut cycles = Vec::new();

        // Build adjacency: plugin A depends on plugin B if A requires a field B provides
        let mut depends_on: HashMap<String, HashSet<String>> = HashMap::new();
        let mut provides_map: HashMap<String, HashSet<String>> = HashMap::new();

        for dep in &self.plugins {
            let provides: HashSet<String> = dep.provides.iter().cloned().collect();
            provides_map.insert(dep.plugin_name.clone(), provides);
        }

        for dep in &self.plugins {
            for field in &dep.requires {
                for (provider, fields) in &provides_map {
                    if fields.contains(field) && provider != &dep.plugin_name {
                        depends_on
                            .entry(dep.plugin_name.clone())
                            .or_default()
                            .insert(provider.clone());
                    }
                }
            }
        }

        // DFS to find cycles
        let mut visited = HashSet::new();
        let mut in_stack = HashSet::new();
        let mut path = Vec::new();

        for plugin in depends_on.keys() {
            if !visited.contains(plugin) {
                Self::dfs(
                    plugin,
                    &depends_on,
                    &mut visited,
                    &mut in_stack,
                    &mut path,
                    &mut cycles,
                );
            }
        }

        cycles
    }

    fn dfs(
        node: &str,
        graph: &HashMap<String, HashSet<String>>,
        visited: &mut HashSet<String>,
        in_stack: &mut HashSet<String>,
        path: &mut Vec<String>,
        cycles: &mut Vec<CircularDep>,
    ) {
        visited.insert(node.to_string());
        in_stack.insert(node.to_string());
        path.push(node.to_string());

        if let Some(neighbors) = graph.get(node) {
            for neighbor in neighbors {
                if !visited.contains(neighbor.as_str()) {
                    Self::dfs(neighbor, graph, visited, in_stack, path, cycles);
                } else if in_stack.contains(neighbor.as_str()) {
                    // Found a cycle
                    let start = path.iter().position(|p| p == neighbor).unwrap();
                    let cycle: Vec<String> = path[start..].to_vec();
                    cycles.push(CircularDep { chain: cycle });
                }
            }
        }

        path.pop();
        in_stack.remove(node);
    }
}

impl Default for DependencyValidator {
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
    fn test_all_dependencies_satisfied() {
        let mut validator = DependencyValidator::new();

        validator.register(FieldDependency {
            plugin_name: "host_info".into(),
            requires: vec![],
            requires_optional: vec![],
            provides: vec!["host.name".into(), "process.id".into()],
            pipeline_stage: Some(2), // FieldProvider stage
        });

        validator.register(FieldDependency {
            plugin_name: "audit_enricher".into(),
            requires: vec!["host.name".into()],
            requires_optional: vec!["user.id".into()],
            provides: vec!["audit.tag".into()],
            pipeline_stage: Some(4), // Processing stage — comes after FieldProvider
        });

        let result = validator.validate();
        assert!(result.satisfied);
        assert!(result.missing_required.is_empty());
        assert!(result.circular_deps.is_empty());
    }

    #[test]
    fn test_missing_dependency() {
        let mut validator = DependencyValidator::new();

        validator.register(FieldDependency {
            plugin_name: "host_info".into(),
            requires: vec![],
            requires_optional: vec![],
            provides: vec!["host.name".into()],
            pipeline_stage: Some(2),
        });

        validator.register(FieldDependency {
            plugin_name: "audit_enricher".into(),
            requires: vec!["host.name".into(), "missing.field".into()],
            requires_optional: vec![],
            provides: vec![],
            pipeline_stage: Some(4),
        });

        let result = validator.validate();
        assert!(!result.satisfied);
        // host.name is provided by host_info, only missing.field is missing
        assert_eq!(result.missing_required.len(), 1);
        assert_eq!(result.missing_required[0].field, "missing.field");
        assert_eq!(result.missing_required[0].plugin, "audit_enricher");
    }

    #[test]
    fn test_circular_dependency() {
        let mut validator = DependencyValidator::new();

        validator.register(FieldDependency {
            plugin_name: "plugin_a".into(),
            requires: vec!["field_b".into()],
            requires_optional: vec![],
            provides: vec!["field_a".into()],
            pipeline_stage: Some(4),
        });

        validator.register(FieldDependency {
            plugin_name: "plugin_b".into(),
            requires: vec!["field_a".into()],
            requires_optional: vec![],
            provides: vec!["field_b".into()],
            pipeline_stage: Some(4),
        });

        let result = validator.validate();
        assert!(!result.satisfied);
        assert!(!result.circular_deps.is_empty());
    }
}
