//! Plugin system for DoLogger.
//!
//! Contains the plugin manager, sandbox enforcement, pipeline phase
//! definitions, dependency validation, and resource quota tracking.

pub mod dependency;
pub mod manager;
pub mod phase;
pub mod quota;
pub mod sandbox;

pub use dependency::{
    CircularDep, DependencyValidator, FieldDependency, MissingField, ValidationResult,
};
pub use manager::{
    default_plugin_paths, validate_plugin_name, LoadedPlugin, PluginError, PluginManager,
    PluginMeta, PluginResult, TrustLevel, CORE_ABI_VERSION,
};
#[allow(deprecated)]
pub use phase::PHASE_POLICY;
pub use phase::{
    phase_name, PHASE_ALL, PHASE_ASSEMBLY, PHASE_CONFIG, PHASE_FILTER, PHASE_FORMATTING,
    PHASE_HOSTINFO, PHASE_KEY, PHASE_NAMES, PHASE_PRE_FILTER, PHASE_PROCESSING, PHASE_SINK,
    PHASE_SYSCALL,
};
pub use quota::{PluginQuota, QuotaAction, QuotaConfig, QuotaManager};
pub use sandbox::{
    SandboxBackend, SandboxEngine, SandboxLevel, SandboxPolicy, SandboxResult, SyscallCategory,
};
