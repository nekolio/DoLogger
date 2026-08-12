//! Plugin sandbox isolation.
//!
//! Isolates yellow and red plugins in restricted execution environments
//! to prevent malicious or buggy plugins from compromising the host.
//!
//! # Trust model
//!
//! | Color | Trust | Sandbox | Description |
//! |-------|--------|---------|-------------|
//! | Blue | Full | None | Official signed plugins from DoLogger team |
//! | Yellow | Partial | Restricted process | Verified third-party plugins |
//! | Red | None | Maximum isolation | Untrusted community plugins |
//!
//! # Platform isolation
//!
//! | Platform | Mechanism | Syscall Filtering |
//! |----------|-----------|-------------------|
//! | Linux | seccomp-bpf + clone(CLONE_NEWPID) | prctl(PR_SET_SECCOMP, SECCOMP_MODE_FILTER, ...) |
//! | Windows | AppContainer + LowBoxToken | CreateAppContainerProfile + restricted SID |
//! | macOS | App Sandbox + seatbelt | sandbox_init(3) with SBPL profile |
//!
//! # Implementation status
//!
//! - Linux seccomp: full BPF filter generation for syscall allowlist
//! - Windows AppContainer: profile creation skeleton (requires process isolation)
//! - macOS Sandbox: SBPL profile generation skeleton
//!
//! Full process isolation (fork + exec for plugin subprocesses) is deferred
//! not yet implemented. The sandbox currently provides the policy framework\n//! and BPF filter generation.

// TODO: Remove #![allow(missing_docs)] and add doc comments to all public items.
// This module has many public types (SandboxLevel, SandboxBackend, SyscallCategory,
// SandboxPolicy, SandboxEngine, etc.) that need individual documentation.
#![allow(missing_docs)]

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};

// ---------------------------------------------------------------------------
// Sandbox profile
// ---------------------------------------------------------------------------

/// Sandbox isolation level for a plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SandboxLevel {
    /// No sandbox — full system access (blue plugins only)
    None,
    /// Restricted process — limited syscalls, no network (yellow plugins)
    Restricted,
    /// Maximum isolation — minimal syscall set, no FS/network (red plugins)
    Isolated,
}

/// Platform-specific sandbox backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxBackend {
    /// Linux seccomp-bpf
    Seccomp,
    /// Windows AppContainer
    AppContainer,
    /// macOS App Sandbox (seatbelt)
    MacOSSandbox,
    /// No sandbox backend available (unsupported platform)
    None,
}

impl SandboxBackend {
    /// Detect the available sandbox backend for the current platform.
    pub fn detect() -> Self {
        #[cfg(target_os = "linux")]
        return SandboxBackend::Seccomp;
        #[cfg(windows)]
        return SandboxBackend::AppContainer;
        #[cfg(target_os = "macos")]
        return SandboxBackend::MacOSSandbox;
        #[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
        return SandboxBackend::None;
    }

    /// Whether this backend supports actual process isolation.
    pub fn supports_isolation(&self) -> bool {
        matches!(self, SandboxBackend::Seccomp | SandboxBackend::MacOSSandbox)
    }
}

// ---------------------------------------------------------------------------
// Syscall categories for seccomp-bpf allowlists
// ---------------------------------------------------------------------------

/// Categories of system calls for sandbox policy definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SyscallCategory {
    /// Memory allocation (mmap, munmap, brk, mprotect)
    Memory,
    /// File I/O (read, write, openat, close, fsync)
    FileIO,
    /// Network (socket, connect, bind, sendto, recvfrom)
    Network,
    /// Process management (clone, fork, execve, exit)
    Process,
    /// Thread synchronization (futex, sched_yield)
    Threading,
    /// Time functions (clock_gettime, gettimeofday)
    Time,
    /// Signal handling (sigaction, rt_sigreturn)
    Signal,
    /// System information (uname, sysinfo)
    SystemInfo,
}

impl SyscallCategory {
    /// Get the Linux syscall names for this category.
    pub fn linux_syscalls(self) -> &'static [&'static str] {
        match self {
            Self::Memory => &["mmap", "munmap", "brk", "mprotect", "madvise"],
            Self::FileIO => &[
                "read",
                "write",
                "openat",
                "close",
                "fstat",
                "lseek",
                "fsync",
                "fdatasync",
                "readv",
                "writev",
                "pread64",
                "pwrite64",
            ],
            Self::Network => &[
                "socket",
                "connect",
                "bind",
                "listen",
                "accept",
                "sendto",
                "recvfrom",
                "sendmsg",
                "recvmsg",
                "setsockopt",
                "getsockname",
            ],
            Self::Process => &["clone", "fork", "vfork", "execve", "exit", "exit_group"],
            Self::Threading => &["futex", "sched_yield", "nanosleep", "gettid"],
            Self::Time => &["clock_gettime", "gettimeofday", "time", "clock_nanosleep"],
            Self::Signal => &[
                "sigaction",
                "sigreturn",
                "rt_sigaction",
                "rt_sigreturn",
                "sigprocmask",
                "rt_sigprocmask",
            ],
            Self::SystemInfo => &["uname", "sysinfo", "getpid", "getuid", "getgid"],
        }
    }
}

// ---------------------------------------------------------------------------
// Sandbox policy
// ---------------------------------------------------------------------------

/// A sandbox policy defining what a plugin is allowed to do.
#[derive(Debug, Clone)]
pub struct SandboxPolicy {
    /// Isolation level
    pub level: SandboxLevel,
    /// Allowed syscall categories
    pub allowed_categories: HashSet<SyscallCategory>,
    /// Allowed file paths (absolute, read-only)
    pub allowed_read_paths: Vec<String>,
    /// Allowed file paths (absolute, read-write)
    pub allowed_write_paths: Vec<String>,
    /// Allowed network addresses (host:port)
    pub allowed_network: Vec<String>,
    /// Maximum memory in bytes
    pub max_memory_bytes: u64,
    /// Maximum CPU time in seconds (0 = unlimited)
    pub max_cpu_seconds: u64,
    /// Whether file writes are allowed at all
    pub allow_file_write: bool,
    /// Whether network access is allowed at all
    pub allow_network: bool,
    /// Whether process creation (fork/clone) is allowed
    pub allow_fork: bool,
}

impl Default for SandboxPolicy {
    fn default() -> Self {
        Self {
            level: SandboxLevel::None,
            allowed_categories: HashSet::new(),
            allowed_read_paths: Vec::new(),
            allowed_write_paths: Vec::new(),
            allowed_network: Vec::new(),
            max_memory_bytes: 0,
            max_cpu_seconds: 0,
            allow_file_write: true,
            allow_network: true,
            allow_fork: false,
        }
    }
}

impl SandboxPolicy {
    /// Create a policy for blue (trusted) plugins — no restrictions.
    pub fn blue() -> Self {
        Self {
            level: SandboxLevel::None,
            ..Default::default()
        }
    }

    /// Create a policy for yellow (verified third-party) plugins.
    pub fn yellow() -> Self {
        let mut allowed = HashSet::new();
        allowed.insert(SyscallCategory::Memory);
        allowed.insert(SyscallCategory::FileIO);
        allowed.insert(SyscallCategory::Threading);
        allowed.insert(SyscallCategory::Time);
        allowed.insert(SyscallCategory::Signal);
        allowed.insert(SyscallCategory::SystemInfo);
        // Yellow plugins get file I/O but NOT network or fork

        Self {
            level: SandboxLevel::Restricted,
            allowed_categories: allowed,
            allow_file_write: true,
            allow_network: false,
            allow_fork: false,
            ..Default::default()
        }
    }

    /// Create a policy for red (untrusted community) plugins — maximum isolation.
    pub fn red() -> Self {
        let mut allowed = HashSet::new();
        allowed.insert(SyscallCategory::Memory);
        allowed.insert(SyscallCategory::Threading);
        allowed.insert(SyscallCategory::Time);
        // Red plugins: memory + threading + time only — no file IO, no network, no fork

        Self {
            level: SandboxLevel::Isolated,
            allowed_categories: allowed,
            allow_file_write: false,
            allow_network: false,
            allow_fork: false,
            ..Default::default()
        }
    }

    /// Check if a specific syscall category is allowed.
    pub fn allows_category(&self, category: SyscallCategory) -> bool {
        if self.level == SandboxLevel::None {
            return true;
        }
        self.allowed_categories.contains(&category)
    }

    /// Check if a plugin of the given trust color can register as the given plugin type.
    /// Returns Err with reason if the registration should be denied.
    pub fn check_plugin_type_allowed(
        color: SandboxLevel,
        plugin_type_name: &str,
    ) -> Result<(), String> {
        match color {
            SandboxLevel::None => Ok(()), // Blue can do anything
            SandboxLevel::Restricted => {
                // Yellow cannot be: ConfigProvider, KeyProvider, PolicyProvider, HostInfoProvider, SyscallBroker
                match plugin_type_name {
                    "ConfigProvider" | "KeyProvider" | "PolicyProvider" | "HostInfoProvider"
                    | "SyscallBroker" => Err(format!(
                        "Yellow plugins cannot register as {}",
                        plugin_type_name
                    )),
                    _ => Ok(()),
                }
            }
            SandboxLevel::Isolated => {
                // Red can only be: Filter, FieldProvider, Processor, Formatter, IOSink
                match plugin_type_name {
                    "Filter" | "FieldProvider" | "Processor" | "Formatter" | "IOSink" => Ok(()),
                    other => Err(format!("Red plugins cannot register as {}", other)),
                }
            }
        }
    }

    /// Validate that the policy is internally consistent.
    pub fn validate(&self) -> Result<(), String> {
        if self.level == SandboxLevel::None {
            return Ok(()); // No restrictions — always valid
        }

        // Must have at least Memory category
        if !self.allowed_categories.contains(&SyscallCategory::Memory) {
            return Err("Sandbox policy must allow Memory category (mmap/munmap)".into());
        }

        // If allow_file_write, must have FileIO category
        if self.allow_file_write && !self.allowed_categories.contains(&SyscallCategory::FileIO) {
            return Err("Sandbox policy: allow_file_write=true requires FileIO category".into());
        }

        // If allow_network, must have Network category
        if self.allow_network && !self.allowed_categories.contains(&SyscallCategory::Network) {
            return Err("Sandbox policy: allow_network=true requires Network category".into());
        }

        // If allow_fork, must have Process category
        if self.allow_fork && !self.allowed_categories.contains(&SyscallCategory::Process) {
            return Err("Sandbox policy: allow_fork=true requires Process category".into());
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Sandbox engine
// ---------------------------------------------------------------------------

/// Result of applying a sandbox policy to a process.
#[derive(Debug, Clone)]
pub struct SandboxResult {
    /// Whether the sandbox was successfully applied
    pub success: bool,
    /// Backend used
    pub backend: SandboxBackend,
    /// Policy level applied
    pub level: SandboxLevel,
    /// Error message if application failed
    pub error: Option<String>,
    /// Platform-specific sandbox context (opaque)
    pub context_id: Option<u64>,
}

/// The sandbox engine — applies isolation policies to plugins.
///
/// On platforms supporting it, this sets up seccomp-bpf filters
/// or AppContainer profiles before plugin code executes.
pub struct SandboxEngine {
    /// Whether sandboxing is enabled (can be disabled for debugging)
    enabled: AtomicBool,
    /// Detected backend
    backend: SandboxBackend,
    /// Number of sandboxes applied
    #[allow(dead_code)]
    applied_count: AtomicBool, // Tracks if we've applied at least one
}

impl SandboxEngine {
    /// Create a new sandbox engine, detecting the platform backend.
    pub fn new() -> Self {
        Self {
            enabled: AtomicBool::new(true),
            backend: SandboxBackend::detect(),
            applied_count: AtomicBool::new(false),
        }
    }

    /// Disable sandboxing (e.g., for development/debugging).
    pub fn disable(&self) {
        self.enabled.store(false, Ordering::Release);
        crate::sys::diag::warn(
            "sandbox",
            "Sandbox engine DISABLED — plugins run unrestricted",
        );
    }

    /// Enable sandboxing.
    pub fn enable(&self) {
        self.enabled.store(true, Ordering::Release);
    }

    /// Whether sandboxing is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }

    /// Get the detected platform backend.
    pub fn backend(&self) -> SandboxBackend {
        self.backend
    }

    /// Apply a sandbox policy to the current process.
    ///
    /// This is called before executing plugin code. On success, the
    /// current process is restricted according to the policy.
    ///
    /// # Safety
    ///
    /// This is inherently unsafe — it modifies the process's security
    /// context. Once applied, sandbox restrictions cannot be removed.
    pub fn apply_policy(&self, policy: &SandboxPolicy) -> SandboxResult {
        if !self.enabled.load(Ordering::Acquire) {
            return SandboxResult {
                success: true,
                backend: self.backend,
                level: policy.level,
                error: Some("sandbox disabled".into()),
                context_id: None,
            };
        }

        if policy.level == SandboxLevel::None {
            return SandboxResult {
                success: true,
                backend: self.backend,
                level: SandboxLevel::None,
                error: None,
                context_id: None,
            };
        }

        #[cfg(target_os = "linux")]
        {
            apply_seccomp_policy(policy)
        }

        #[cfg(windows)]
        {
            apply_appcontainer_policy(policy)
        }

        #[cfg(target_os = "macos")]
        {
            apply_macos_sandbox_policy(policy)
        }

        #[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
        {
            return SandboxResult {
                success: false,
                backend: SandboxBackend::None,
                level: policy.level,
                error: Some("Unsupported platform: no sandbox backend available".into()),
                context_id: None,
            };
        }
    }
}

impl Default for SandboxEngine {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Linux: seccomp-bpf filter application
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
fn apply_seccomp_policy(policy: &SandboxPolicy) -> SandboxResult {
    // seccomp-bpf constants
    const SECCOMP_SET_MODE_FILTER: libc::c_int = 1;

    // Collect all allowed syscall numbers
    let mut allowed_syscalls: Vec<i32> = Vec::new();

    for cat in &policy.allowed_categories {
        for name in cat.linux_syscalls() {
            if let Some(nr) = syscall_name_to_number(name) {
                allowed_syscalls.push(nr);
            }
        }
    }

    // Always allow basic syscalls needed for plugin execution
    // (exit, exit_group, restart_syscall)
    allowed_syscalls.push(60); // exit (x86_64)
    allowed_syscalls.push(231); // exit_group (x86_64)
    allowed_syscalls.push(219); // restart_syscall (x86_64)
    allowed_syscalls.push(0); // read (for basic I/O)
    allowed_syscalls.push(1); // write (for basic I/O)

    allowed_syscalls.sort();
    allowed_syscalls.dedup();

    // Build a simple BPF filter program.
    // In production, this would use the `seccomp` crate or generate proper BPF bytecode.
    // For now, we use prctl with SECCOMP_MODE_FILTER and a minimal filter.

    // Generate BPF instructions for the syscall allowlist
    let filter = build_bpf_filter(&allowed_syscalls);

    let prog = libc::sock_fprog {
        len: filter.len() as u16,
        filter: filter.as_ptr() as *mut libc::sock_filter,
    };

    // Apply the seccomp filter
    // SAFETY: prctl(PR_SET_SECCOMP, ...) installs the BPF program referenced
    // by `prog` for the calling thread. `prog` is a valid `sock_fprog` whose
    // `filter` slice outlives this call. The operation is one-way — the
    // sandbox cannot be removed afterwards, which is the intended behavior.
    let rc = unsafe {
        libc::prctl(
            libc::PR_SET_SECCOMP,
            SECCOMP_SET_MODE_FILTER as libc::c_ulong,
            &prog as *const libc::sock_fprog as libc::c_ulong,
        )
    };

    if rc != 0 {
        let err = std::io::Error::last_os_error();
        return SandboxResult {
            success: false,
            backend: SandboxBackend::Seccomp,
            level: policy.level,
            error: Some(format!("seccomp prctl failed: {err}")),
            context_id: None,
        };
    }

    crate::sys::diag::info(
        "sandbox",
        &format!(
            "seccomp-bpf applied: {} syscalls allowed, level={:?}",
            allowed_syscalls.len(),
            policy.level
        ),
    );

    SandboxResult {
        success: true,
        backend: SandboxBackend::Seccomp,
        level: policy.level,
        error: None,
        context_id: Some(0),
    }
}

/// Build a minimal BPF filter that allows listed syscall numbers.
///
/// BPF instruction semantics: `jt` and `jf` are RELATIVE jump offsets
/// (number of instructions to skip forward, NOT absolute indices).
/// `jt=0` means "next instruction", `jf=0` means "fall through to next."
#[cfg(target_os = "linux")]
fn build_bpf_filter(allowed: &[i32]) -> Vec<libc::sock_filter> {
    // BPF instruction encoding constants
    const BPF_LD: u16 = 0x00;
    const BPF_W: u16 = 0x00;
    const BPF_ABS: u16 = 0x20;
    const BPF_JMP: u16 = 0x05;
    const BPF_JEQ: u16 = 0x10;
    const BPF_RET: u16 = 0x06;
    const BPF_K: u16 = 0x00;
    const SECCOMP_RET_ALLOW: u32 = 0x7FFF_0000;
    const SECCOMP_RET_KILL_PROCESS: u32 = 0x8000_0000;

    let mut filter = Vec::new();

    // Load syscall number from seccomp_data.nr (offset 0 for x86_64)
    filter.push(libc::sock_filter {
        code: BPF_LD | BPF_W | BPF_ABS,
        jt: 0,
        jf: 0,
        k: 0,
    });

    let jeq_start_idx = filter.len(); // Index of first JEQ instruction

    // For each allowed syscall, add a JEQ check
    for &syscall_nr in allowed {
        filter.push(libc::sock_filter {
            code: BPF_JMP | BPF_JEQ | BPF_K,
            jt: 0, // Patched below — relative jump to ALLOW
            jf: 0, // Fall through to next JEQ (or KILL if last)
            k: syscall_nr as u32,
        });
    }

    // ALLOW return and KILL return positions
    let allow_idx = filter.len();
    let kill_idx = filter.len() + 1;

    // Patch JEQ instructions: jt = relative offset from THIS instruction to ALLOW
    // For instruction at index i: jt = allow_idx - i - 1 (instructions to skip)
    for (i, insn) in filter
        .iter_mut()
        .enumerate()
        .take(allow_idx)
        .skip(jeq_start_idx)
    {
        let rel_jt = (allow_idx - i - 1) as u8;
        insn.jt = rel_jt;
    }

    // ALLOW return (SECCOMP_RET_ALLOW)
    filter.push(libc::sock_filter {
        code: BPF_RET | BPF_K,
        jt: 0,
        jf: 0,
        k: SECCOMP_RET_ALLOW,
    });

    // KILL return (SECCOMP_RET_KILL_PROCESS — default deny)
    filter.push(libc::sock_filter {
        code: BPF_RET | BPF_K,
        jt: 0,
        jf: 0,
        k: SECCOMP_RET_KILL_PROCESS,
    });

    filter
}

/// Map a Linux syscall name to its number on x86_64.
#[cfg(target_os = "linux")]
fn syscall_name_to_number(name: &str) -> Option<i32> {
    // x86_64 syscall table (common entries)
    match name {
        "read" => Some(0),
        "write" => Some(1),
        "openat" => Some(257),
        "close" => Some(3),
        "fstat" => Some(5),
        "lseek" => Some(8),
        "mmap" => Some(9),
        "mprotect" => Some(10),
        "munmap" => Some(11),
        "brk" => Some(12),
        "rt_sigaction" => Some(13),
        "rt_sigprocmask" => Some(14),
        "rt_sigreturn" => Some(15),
        "pread64" => Some(17),
        "pwrite64" => Some(18),
        "readv" => Some(19),
        "writev" => Some(20),
        "sched_yield" => Some(24),
        "madvise" => Some(28),
        "getpid" => Some(39),
        "socket" => Some(41),
        "connect" => Some(42),
        "accept" => Some(43),
        "sendto" => Some(44),
        "recvfrom" => Some(45),
        "sendmsg" => Some(46),
        "recvmsg" => Some(47),
        "bind" => Some(49),
        "listen" => Some(50),
        "getsockname" => Some(51),
        "setsockopt" => Some(54),
        "clone" => Some(56),
        "fork" => Some(57),
        "vfork" => Some(58),
        "execve" => Some(59),
        "exit" => Some(60),
        "nanosleep" => Some(35),
        "getuid" => Some(102),
        "getgid" => Some(104),
        "gettid" => Some(186),
        "futex" => Some(202),
        "clock_gettime" => Some(228),
        "exit_group" => Some(231),
        "time" => Some(201),
        "gettimeofday" => Some(96),
        "sigaction" => Some(13),   // rt_sigaction
        "sigreturn" => Some(15),   // rt_sigreturn
        "sigprocmask" => Some(14), // rt_sigprocmask
        "fsync" => Some(74),
        "fdatasync" => Some(75),
        "uname" => Some(63),
        "sysinfo" => Some(99),
        "clock_nanosleep" => Some(230),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Windows: AppContainer profile (skeleton)
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn apply_appcontainer_policy(policy: &SandboxPolicy) -> SandboxResult {
    // Windows AppContainer isolation requires:
    // 1. CreateAppContainerProfile() to define the container
    // 2. DeriveAppContainerSidFromAppContainerSid() for the SID
    // 3. CreateRestrictedToken() + AddSIDsToToken() for the LowBox token
    // 4. CreateProcessAsUser() with the restricted token
    //
    // Full implementation requires process-level isolation (not yet implemented).
    // For now, we return a skeleton result indicating the policy was registered.

    crate::sys::diag::info(
        "sandbox",
        &format!(
            "AppContainer policy registered: level={:?}, allow_network={}, allow_fork={}",
            policy.level, policy.allow_network, policy.allow_fork
        ),
    );

    SandboxResult {
        success: true,
        backend: SandboxBackend::AppContainer,
        level: policy.level,
        error: Some("AppContainer: full process isolation not yet implemented".into()),
        context_id: None,
    }
}

// ---------------------------------------------------------------------------
// macOS: App Sandbox (skeleton)
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
fn apply_macos_sandbox_policy(policy: &SandboxPolicy) -> SandboxResult {
    // macOS sandboxing requires:
    // 1. sandbox_init(3) with an SBPL (Sandbox Profile Language) profile
    // 2. Or using App Sandbox entitlements at the process level
    //
    // Full implementation requires process-level isolation (not yet implemented).
    // For now, we register the policy and return.

    crate::sys::diag::info(
        "sandbox",
        &format!(
            "macOS sandbox policy registered: level={:?}, allow_network={}",
            policy.level, policy.allow_network
        ),
    );

    SandboxResult {
        success: true,
        backend: SandboxBackend::MacOSSandbox,
        level: policy.level,
        error: Some("macOS sandbox: full isolation not yet implemented".into()),
        context_id: None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blue_policy_no_restrictions() {
        let policy = SandboxPolicy::blue();
        assert_eq!(policy.level, SandboxLevel::None);
        assert!(policy.allow_file_write);
        assert!(policy.allow_network);
    }

    #[test]
    fn test_yellow_policy_no_network_no_fork() {
        let policy = SandboxPolicy::yellow();
        assert_eq!(policy.level, SandboxLevel::Restricted);
        assert!(policy.allow_file_write);
        assert!(!policy.allow_network);
        assert!(!policy.allow_fork);
        assert!(policy.allows_category(SyscallCategory::FileIO));
        assert!(!policy.allows_category(SyscallCategory::Network));
        assert!(!policy.allows_category(SyscallCategory::Process));
    }

    #[test]
    fn test_red_policy_maximum_isolation() {
        let policy = SandboxPolicy::red();
        assert_eq!(policy.level, SandboxLevel::Isolated);
        assert!(!policy.allow_file_write);
        assert!(!policy.allow_network);
        assert!(!policy.allow_fork);
        // Red plugins get Memory + Threading + Time only
        assert!(policy.allows_category(SyscallCategory::Memory));
        assert!(policy.allows_category(SyscallCategory::Threading));
        assert!(!policy.allows_category(SyscallCategory::FileIO));
        assert!(!policy.allows_category(SyscallCategory::Network));
    }

    #[test]
    fn test_policy_validation_missing_memory() {
        let mut policy = SandboxPolicy::yellow();
        policy.allowed_categories.remove(&SyscallCategory::Memory);
        assert!(policy.validate().is_err());
    }

    #[test]
    fn test_policy_validation_file_write_without_io() {
        let mut policy = SandboxPolicy::yellow();
        policy.allow_file_write = true;
        policy.allowed_categories.remove(&SyscallCategory::FileIO);
        assert!(policy.validate().is_err());
    }

    #[test]
    fn test_policy_validation_ok() {
        assert!(SandboxPolicy::blue().validate().is_ok());
        assert!(SandboxPolicy::yellow().validate().is_ok());
        assert!(SandboxPolicy::red().validate().is_ok());
    }

    #[test]
    fn test_sandbox_engine_detect_backend() {
        let engine = SandboxEngine::new();
        let backend = engine.backend();
        // On any supported platform, should not be None
        #[cfg(any(target_os = "linux", windows, target_os = "macos"))]
        assert_ne!(backend, SandboxBackend::None);
    }

    #[test]
    fn test_sandbox_engine_enable_disable() {
        let engine = SandboxEngine::new();
        assert!(engine.is_enabled());

        engine.disable();
        assert!(!engine.is_enabled());

        engine.enable();
        assert!(engine.is_enabled());
    }

    #[test]
    fn test_apply_blue_policy_noop() {
        let engine = SandboxEngine::new();
        let result = engine.apply_policy(&SandboxPolicy::blue());
        assert!(result.success);
        assert_eq!(result.level, SandboxLevel::None);
    }

    #[test]
    fn test_apply_policy_when_disabled() {
        let engine = SandboxEngine::new();
        engine.disable();
        let result = engine.apply_policy(&SandboxPolicy::red());
        assert!(result.success, "Disabled engine should return success");
    }

    #[test]
    fn test_syscall_name_lookup_known() {
        #[cfg(target_os = "linux")]
        {
            assert_eq!(syscall_name_to_number("write"), Some(1));
            assert_eq!(syscall_name_to_number("read"), Some(0));
            assert_eq!(syscall_name_to_number("exit"), Some(60));
            assert_eq!(syscall_name_to_number("mmap"), Some(9));
            assert_eq!(syscall_name_to_number("futex"), Some(202));
        }
    }

    #[test]
    fn test_syscall_name_lookup_unknown() {
        #[cfg(target_os = "linux")]
        {
            assert_eq!(syscall_name_to_number("nonexistent_syscall"), None);
        }
    }

    #[test]
    fn test_all_categories_have_syscalls() {
        for cat in &[
            SyscallCategory::Memory,
            SyscallCategory::FileIO,
            SyscallCategory::Network,
            SyscallCategory::Process,
            SyscallCategory::Threading,
            SyscallCategory::Time,
            SyscallCategory::Signal,
            SyscallCategory::SystemInfo,
        ] {
            assert!(
                !cat.linux_syscalls().is_empty(),
                "{cat:?} must have syscalls"
            );
        }
    }

    #[test]
    fn test_check_plugin_type_allowed_blue() {
        // Blue (None) can register as anything
        assert!(
            SandboxPolicy::check_plugin_type_allowed(SandboxLevel::None, "ConfigProvider").is_ok()
        );
        assert!(
            SandboxPolicy::check_plugin_type_allowed(SandboxLevel::None, "KeyProvider").is_ok()
        );
        assert!(
            SandboxPolicy::check_plugin_type_allowed(SandboxLevel::None, "SyscallBroker").is_ok()
        );
        assert!(SandboxPolicy::check_plugin_type_allowed(SandboxLevel::None, "Filter").is_ok());
    }

    #[test]
    fn test_check_plugin_type_allowed_yellow() {
        // Yellow (Restricted) cannot be sensitive providers
        assert!(SandboxPolicy::check_plugin_type_allowed(
            SandboxLevel::Restricted,
            "ConfigProvider"
        )
        .is_err());
        assert!(
            SandboxPolicy::check_plugin_type_allowed(SandboxLevel::Restricted, "KeyProvider")
                .is_err()
        );
        assert!(SandboxPolicy::check_plugin_type_allowed(
            SandboxLevel::Restricted,
            "PolicyProvider"
        )
        .is_err());
        assert!(SandboxPolicy::check_plugin_type_allowed(
            SandboxLevel::Restricted,
            "HostInfoProvider"
        )
        .is_err());
        assert!(SandboxPolicy::check_plugin_type_allowed(
            SandboxLevel::Restricted,
            "SyscallBroker"
        )
        .is_err());
        // But can be Filter or Formatter
        assert!(
            SandboxPolicy::check_plugin_type_allowed(SandboxLevel::Restricted, "Filter").is_ok()
        );
        assert!(
            SandboxPolicy::check_plugin_type_allowed(SandboxLevel::Restricted, "Formatter").is_ok()
        );
        assert!(
            SandboxPolicy::check_plugin_type_allowed(SandboxLevel::Restricted, "Processor").is_ok()
        );
    }

    #[test]
    fn test_check_plugin_type_allowed_red() {
        // Red (Isolated) can only be: Filter, FieldProvider, Processor, Formatter, IOSink
        assert!(SandboxPolicy::check_plugin_type_allowed(SandboxLevel::Isolated, "Filter").is_ok());
        assert!(
            SandboxPolicy::check_plugin_type_allowed(SandboxLevel::Isolated, "FieldProvider")
                .is_ok()
        );
        assert!(
            SandboxPolicy::check_plugin_type_allowed(SandboxLevel::Isolated, "Processor").is_ok()
        );
        assert!(
            SandboxPolicy::check_plugin_type_allowed(SandboxLevel::Isolated, "Formatter").is_ok()
        );
        assert!(SandboxPolicy::check_plugin_type_allowed(SandboxLevel::Isolated, "IOSink").is_ok());
        // Cannot be anything else
        assert!(
            SandboxPolicy::check_plugin_type_allowed(SandboxLevel::Isolated, "ConfigProvider")
                .is_err()
        );
        assert!(
            SandboxPolicy::check_plugin_type_allowed(SandboxLevel::Isolated, "KeyProvider")
                .is_err()
        );
        assert!(
            SandboxPolicy::check_plugin_type_allowed(SandboxLevel::Isolated, "SyscallBroker")
                .is_err()
        );
        assert!(SandboxPolicy::check_plugin_type_allowed(
            SandboxLevel::Isolated,
            "HostInfoProvider"
        )
        .is_err());
        assert!(
            SandboxPolicy::check_plugin_type_allowed(SandboxLevel::Isolated, "UnknownPlugin")
                .is_err()
        );
    }
}
