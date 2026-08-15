//! C ABI phase constants (mirrors `dologger_core.h` DO_LOG_PHASE_*).
//!
//! These bitmask values identify which pipeline stage(s) a plugin
//! mounts into. A plugin may mount at multiple stages by OR-ing values.
//!
//! # Correspondence with C header
//!
//! Each constant here matches the C `#define` in `core/include/dologger_core.h`.
//! They MUST be kept in sync — any change here requires a corresponding
//! change in the C header and vice versa.
//!
//! The Sink stage (pipeline stage 6) is intentionally NOT a phase constant:
//! Sink is a core built-in output executor, not a plugin mount point.

/// Pre-filter stage: rate limiting, drop policy
pub const PHASE_PRE_FILTER: u32 = 0x0001;
/// Filter stage: domain-based filtering, custom rules
pub const PHASE_FILTER: u32 = 0x0002;
/// Assembly stage: signature, LSN, prev_hash
pub const PHASE_ASSEMBLY: u32 = 0x0004;
/// Processing stage: transformation, enrichment
pub const PHASE_PROCESSING: u32 = 0x0008;
/// Formatting stage: text/json/csv/sif encoding
pub const PHASE_FORMATTING: u32 = 0x0010;
/// Config provider: configuration loading/saving
pub const PHASE_CONFIG: u32 = 0x0040;
/// Key provider: Ed25519 key generation/storage
pub const PHASE_KEY: u32 = 0x0080;
/// Host info provider: system/process metadata injection
pub const PHASE_HOSTINFO: u32 = 0x0100;
/// Syscall broker: platform-specific system call interception
pub const PHASE_SYSCALL: u32 = 0x0200;
/// Policy provider (deprecated, same as PRE_FILTER)
#[deprecated(note = "Use PHASE_PRE_FILTER instead")]
pub const PHASE_POLICY: u32 = 0x0400;
/// FieldProvider stage: custom key-value field injection (pipeline stage 2).
///
/// Distinct from [`PHASE_HOSTINFO`]: host-info injection is a core-provided
/// enrichment, whereas a `FieldProvider` plugin mounts the FieldProvider
/// pipeline stage to inject its own fields. The dispatch treats both bits as
/// field-provider mount points (a plugin may declare either).
pub const PHASE_FIELD_PROVIDER: u32 = 0x0800;

/// All valid phase bits.
pub const PHASE_ALL: u32 = PHASE_PRE_FILTER
    | PHASE_FILTER
    | PHASE_ASSEMBLY
    | PHASE_PROCESSING
    | PHASE_FORMATTING
    | PHASE_CONFIG
    | PHASE_KEY
    | PHASE_HOSTINFO
    | PHASE_SYSCALL
    | PHASE_FIELD_PROVIDER;

/// Human-readable name for a phase constant.
pub fn phase_name(phase: u32) -> &'static str {
    match phase {
        PHASE_PRE_FILTER => "PRE_FILTER",
        PHASE_FILTER => "FILTER",
        PHASE_ASSEMBLY => "ASSEMBLY",
        PHASE_PROCESSING => "PROCESSING",
        PHASE_FORMATTING => "FORMATTING",
        PHASE_CONFIG => "CONFIG",
        PHASE_KEY => "KEY",
        PHASE_HOSTINFO => "HOSTINFO",
        PHASE_SYSCALL => "SYSCALL",
        PHASE_FIELD_PROVIDER => "FIELD_PROVIDER",
        _ => "UNKNOWN",
    }
}

/// All phase names in pipeline order.
pub const PHASE_NAMES: &[(&str, u32)] = &[
    ("PRE_FILTER", PHASE_PRE_FILTER),
    ("FILTER", PHASE_FILTER),
    ("ASSEMBLY", PHASE_ASSEMBLY),
    ("PROCESSING", PHASE_PROCESSING),
    ("FORMATTING", PHASE_FORMATTING),
    ("CONFIG", PHASE_CONFIG),
    ("KEY", PHASE_KEY),
    ("HOSTINFO", PHASE_HOSTINFO),
    ("SYSCALL", PHASE_SYSCALL),
    ("FIELD_PROVIDER", PHASE_FIELD_PROVIDER),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_phase_constants_match_c_header() {
        // Verify the values match dologger_core.h
        assert_eq!(PHASE_PRE_FILTER, 0x0001);
        assert_eq!(PHASE_FILTER, 0x0002);
        assert_eq!(PHASE_ASSEMBLY, 0x0004);
        assert_eq!(PHASE_PROCESSING, 0x0008);
        assert_eq!(PHASE_FORMATTING, 0x0010);
        assert_eq!(PHASE_CONFIG, 0x0040);
        assert_eq!(PHASE_KEY, 0x0080);
        assert_eq!(PHASE_HOSTINFO, 0x0100);
        assert_eq!(PHASE_SYSCALL, 0x0200);
        assert_eq!(PHASE_FIELD_PROVIDER, 0x0800);
    }

    #[test]
    fn test_phase_bits_are_unique() {
        let phases = [
            PHASE_PRE_FILTER,
            PHASE_FILTER,
            PHASE_ASSEMBLY,
            PHASE_PROCESSING,
            PHASE_FORMATTING,
            PHASE_CONFIG,
            PHASE_KEY,
            PHASE_HOSTINFO,
            PHASE_SYSCALL,
            PHASE_FIELD_PROVIDER,
        ];
        for i in 0..phases.len() {
            for j in (i + 1)..phases.len() {
                assert_ne!(phases[i], phases[j], "Phase bits must be unique");
            }
        }
    }

    #[test]
    fn test_phase_name_known() {
        assert_eq!(phase_name(PHASE_KEY), "KEY");
        assert_eq!(phase_name(PHASE_HOSTINFO), "HOSTINFO");
    }

    #[test]
    fn test_phase_name_unknown() {
        assert_eq!(phase_name(0xDEAD), "UNKNOWN");
    }
}
