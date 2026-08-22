//! System-level operations and utilities.
//!
//! Contains system monitoring, diagnostics, internal logging, control plane,
//! I/O helpers, time source, host info, and thread pool.

pub mod control_plane;
pub mod diagnostics;
pub mod host_info;
pub mod internal_log;
pub mod io;
pub mod system_monitor;
pub mod thread_pool;
pub mod time;

pub use control_plane::{ControlPlane, ControlPlaneConfig, ControlPlaneStats, LevelCb, ReloadCb};
pub use host_info::HostInfoProvider;
pub use internal_log::{DiagLevel, InternalLog};
pub use system_monitor::{Sysmon, SysmonEvent};
pub use thread_pool::{PoolSet, ThreadPool};
pub use time::{MonotonicClock, TimeSource};
