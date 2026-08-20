//! HostInfoProvider — populates Ring 1 fields with host/process/thread info.
//!
//! # Default Implementation
//!
//! Built-in provider that fills in standard Ring 1 fields:
//! - host.name, process.id, process.name
//! - thread.id, thread.name
//! - app.name, app.version, environment
//!
//! Only writes to the Ring 1 whitelist keys.

use crate::record::{thread_id_u64, Record};

/// Default HostInfoProvider — populates Ring 1 system fields.
pub struct HostInfoProvider {
    /// Host name
    host_name: String,
    /// Application name
    app_name: String,
    /// Application version
    app_version: String,
    /// Environment (dev/test/staging/prod)
    environment: String,
}

impl HostInfoProvider {
    /// Create a new HostInfoProvider with system-detected values.
    pub fn new() -> Self {
        Self {
            host_name: hostname(),
            app_name: env!("CARGO_PKG_NAME").to_string(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            environment: std::env::var("DO_LOG_ENV").unwrap_or_else(|_| "dev".into()),
        }
    }

    /// Create with custom values.
    pub fn with_values(
        host_name: &str,
        app_name: &str,
        app_version: &str,
        environment: &str,
    ) -> Self {
        Self {
            host_name: host_name.into(),
            app_name: app_name.into(),
            app_version: app_version.into(),
            environment: environment.into(),
        }
    }

    /// Populate Ring 1 fields on a record.
    ///
    /// Only writes to the Ring 1 whitelist. Does NOT modify Ring 0, Ring 2, or Ring 3.
    pub fn provide(&self, record: &mut Record) -> usize {
        let mut fields_set = 0;

        record.set_host_name(&self.host_name);
        fields_set += 1;

        record.process_id = std::process::id();
        fields_set += 1;

        record.set_process_name(&self.app_name);
        fields_set += 1;

        record.thread_id = thread_id_u64() as u32;
        fields_set += 1;

        record.set_app_name(&self.app_name);
        fields_set += 1;

        record.set_app_version(&self.app_version);
        fields_set += 1;

        record.set_environment(&self.environment);
        fields_set += 1;

        fields_set
    }
}

impl Default for HostInfoProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// Get the system hostname.
fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown".into())
}
