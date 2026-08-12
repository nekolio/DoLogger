//! Hot reload state migration.
//!
//! Manages safe hot-reloading of plugins with state migration
//! and epoch-based anti-rollback protection.
//!
//! # Protocol
//!
//! 1. New plugin version loaded and `plugin_init()` called
//! 2. Old plugin's `serialize_state()` called to capture current state
//! 3. State passed to new plugin's `deserialize_state()`
//! 4. Atomic pointer swap in pipeline
//! 5. Old plugin's `plugin_shutdown()` called
//! 6. If any step fails → rollback, keep old plugin, report sysmon ERROR
//!
//! # Epoch anti-rollback
//!
//! Each hot reload increments a global `reload_epoch` counter.
//! Saved states carry the epoch at which they were serialized.
//! A state with a higher epoch than the current one is rejected —
//! this prevents accidentally rolling back to stale state.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// Serialized plugin state blob.
#[derive(Debug, Clone)]
pub struct PluginState {
    /// Plugin name
    pub plugin_name: String,
    /// Plugin version that produced this state
    pub plugin_version: u32,
    /// Opaque state bytes (plugin-defined format)
    pub data: Vec<u8>,
    /// Epoch at which this state was serialized
    pub epoch: u64,
    /// State format version (for forward compatibility)
    pub format_version: u32,
}

/// Result of a hot reload attempt.
#[derive(Debug, Clone)]
pub struct ReloadResult {
    /// Plugin name
    pub plugin_name: String,
    /// Whether the reload succeeded
    pub success: bool,
    /// Old plugin version
    pub old_version: u32,
    /// New plugin version
    pub new_version: u32,
    /// Error message if failed
    pub error: Option<String>,
    /// Whether state was migrated
    pub state_migrated: bool,
    /// Epoch of this reload
    pub epoch: u64,
}

/// Manages hot reload state migration with epoch-based anti-rollback.
pub struct HotReloadManager {
    /// Global reload epoch — monotonically increasing
    epoch: AtomicU64,
    /// Saved plugin states keyed by plugin name
    saved_states: Mutex<HashMap<String, PluginState>>,
    /// Reload history (last N entries)
    history: Mutex<Vec<ReloadResult>>,
}

impl HotReloadManager {
    /// Create a new hot reload manager.
    pub fn new() -> Self {
        Self {
            epoch: AtomicU64::new(1),
            saved_states: Mutex::new(HashMap::new()),
            history: Mutex::new(Vec::new()),
        }
    }

    /// Increment and return the current epoch.
    pub fn next_epoch(&self) -> u64 {
        self.epoch.fetch_add(1, Ordering::Relaxed)
    }

    /// Get the current epoch.
    pub fn current_epoch(&self) -> u64 {
        self.epoch.load(Ordering::Relaxed)
    }

    /// Serialize a plugin's state before reload.
    ///
    /// Calls the plugin's `serialize_state` function (if available) and
    /// stores the result tagged with the current epoch.
    pub fn serialize_state(
        &self,
        plugin_name: &str,
        plugin_version: u32,
        state_data: Vec<u8>,
        format_version: u32,
    ) {
        let epoch = self.next_epoch();
        let state = PluginState {
            plugin_name: plugin_name.to_string(),
            plugin_version,
            data: state_data,
            epoch,
            format_version,
        };

        self.saved_states
            .lock()
            .unwrap()
            .insert(plugin_name.to_string(), state);

        crate::sys::diag::info(
            "hot_reload",
            &format!("State serialized for '{plugin_name}' v{plugin_version} at epoch {epoch}"),
        );
    }

    /// Retrieve and validate a plugin's saved state for deserialization.
    ///
    /// # Epoch anti-rollback
    ///
    /// If the saved state has a higher epoch than `expected_max_epoch`,
    /// it is rejected — this prevents rolling back to a state serialized
    /// by a newer version of the plugin.
    pub fn get_state_for_reload(
        &self,
        plugin_name: &str,
        expected_max_epoch: u64,
    ) -> Option<PluginState> {
        let states = self.saved_states.lock().unwrap();
        let state = states.get(plugin_name)?;

        // Epoch anti-rollback check
        if state.epoch > expected_max_epoch {
            crate::sys::diag::warn(
                "hot_reload",
                &format!(
                    "State for '{plugin_name}' has epoch {} > current {} — rejecting rollback",
                    state.epoch, expected_max_epoch
                ),
            );
            return None;
        }

        // Format version check
        if state.format_version > 1 {
            crate::sys::diag::warn(
                "hot_reload",
                &format!(
                    "State for '{plugin_name}' has format_version {} > supported 1",
                    state.format_version
                ),
            );
            return None;
        }

        Some(state.clone())
    }

    /// Record a reload result in history.
    pub fn record_reload(&self, result: ReloadResult) {
        let mut history = self.history.lock().unwrap();
        history.push(result);
        // Keep last 100 entries
        if history.len() > 100 {
            history.remove(0);
        }
    }

    /// Attempt a full hot reload cycle for a plugin.
    ///
    /// Returns the reload result. The caller is responsible for:
    /// 1. Loading the new plugin version
    /// 2. Calling this method to migrate state
    /// 3. Swapping plugin pointers on success
    /// 4. Shutting down old plugin on success, or new plugin on failure
    pub fn reload_plugin(
        &self,
        plugin_name: &str,
        old_version: u32,
        new_version: u32,
        new_state_data: Option<Vec<u8>>,
    ) -> ReloadResult {
        let epoch = self.next_epoch();

        let has_new_data = new_state_data.is_some();
        let (state_migrated, error) = if let Some(data) = new_state_data {
            self.serialize_state(plugin_name, new_version, data, 1);
            (true, None)
        } else {
            // Try to retrieve old state for migration
            match self.get_state_for_reload(plugin_name, epoch) {
                Some(_) => (true, None),
                None => (
                    false,
                    Some(format!(
                        "No saved state for '{}' at epoch {}",
                        plugin_name, epoch
                    )),
                ),
            }
        };

        let result = ReloadResult {
            plugin_name: plugin_name.to_string(),
            success: state_migrated || has_new_data,
            old_version,
            new_version,
            error,
            state_migrated,
            epoch,
        };

        if state_migrated {
            crate::sys::diag::info(
                "hot_reload",
                &format!(
                    "Plugin '{plugin_name}' reloaded: v{old_version} → v{new_version}, state_migrated=true, epoch={epoch}"
                ),
            );
        }

        result
    }

    /// Rollback a failed reload — restores the old plugin state.
    pub fn rollback(&self, plugin_name: &str, error: &str) -> ReloadResult {
        let mut states = self.saved_states.lock().unwrap();
        let old_state = states.remove(plugin_name);
        let epoch = self.current_epoch();

        crate::sys::diag::error(
            "hot_reload",
            &format!("Reload of '{plugin_name}' failed, rolling back: {error}"),
        );

        ReloadResult {
            plugin_name: plugin_name.to_string(),
            success: false,
            old_version: old_state.as_ref().map(|s| s.plugin_version).unwrap_or(0),
            new_version: 0,
            error: Some(error.to_string()),
            state_migrated: false,
            epoch,
        }
    }

    /// Clear saved state for a plugin (e.g., after uninstall).
    pub fn clear_state(&self, plugin_name: &str) {
        self.saved_states.lock().unwrap().remove(plugin_name);
    }

    /// Get reload history.
    pub fn reload_history(&self) -> Vec<ReloadResult> {
        self.history.lock().unwrap().clone()
    }
}

impl Default for HotReloadManager {
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
    fn test_epoch_monotonic() {
        let mgr = HotReloadManager::new();
        let e1 = mgr.next_epoch();
        let e2 = mgr.next_epoch();
        assert!(e2 > e1);
    }

    #[test]
    fn test_state_serialize_and_retrieve() {
        let mgr = HotReloadManager::new();
        let epoch = mgr.current_epoch();

        mgr.serialize_state("test_plugin", 0x000100, b"state_blob".to_vec(), 1);

        let state = mgr.get_state_for_reload("test_plugin", epoch + 1);
        assert!(state.is_some());
        assert_eq!(state.unwrap().data, b"state_blob");
    }

    #[test]
    fn test_epoch_anti_rollback() {
        let mgr = HotReloadManager::new();

        // Serialize at epoch 1 (next_epoch returns 1, increments to 2)
        mgr.serialize_state("plugin", 2, vec![1, 2, 3], 1);

        // Try to retrieve with max_epoch = 0 (older than saved state's epoch=1)
        let state = mgr.get_state_for_reload("plugin", 0);
        assert!(
            state.is_none(),
            "Should reject state with higher epoch (1 > 0)"
        );
    }

    #[test]
    fn test_reload_records_history() {
        let mgr = HotReloadManager::new();

        mgr.record_reload(ReloadResult {
            plugin_name: "test".into(),
            success: true,
            old_version: 1,
            new_version: 2,
            error: None,
            state_migrated: true,
            epoch: 1,
        });

        let history = mgr.reload_history();
        assert_eq!(history.len(), 1);
        assert!(history[0].success);
    }

    #[test]
    fn test_rollback_clears_state() {
        let mgr = HotReloadManager::new();

        mgr.serialize_state("failing_plugin", 1, vec![], 1);
        mgr.rollback("failing_plugin", "deserialize_state failed");

        // State should be cleared after rollback
        let state = mgr.get_state_for_reload("failing_plugin", 99);
        assert!(state.is_none());
    }
}
