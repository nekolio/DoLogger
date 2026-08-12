//! Configuration and domain management.
//!
//! Contains the main configuration types, domain system, config file
//! watcher, and hot-reload infrastructure.

pub mod domain;
pub mod hot_reload;
pub mod settings;
pub mod watcher;

pub use domain::{ArrayMergePolicy, Domain, DomainManager, NonDowngradableCheck};
pub use hot_reload::{HotReloadManager, PluginState, ReloadResult};
pub use settings::{
    resolve_config_path, ApiOverrides, ComplianceProfile, DologgerConfig, PerformanceProfile,
    RecordTagOverrides,
};
pub use watcher::{ConfigWatcher, ReloadCallback, ReloadEvent, WatcherBackend, WatcherConfig};
