//! `dologctl` — DoLogger CLI management tool.
//!
//! All output uses `dologger_core::io` platform-native
//! syscalls (WriteFile on Windows, write(2) on POSIX), NOT libc stdio.
//! This ensures the CLI dogfoods the same I/O architecture as the core engine.

mod banner;
mod commands;
pub mod output;

use clap::{Parser, Subcommand};

// ---------------------------------------------------------------------------
// Consistent exit codes — inspired by ripgrep, git, and cargo conventions.
// ---------------------------------------------------------------------------

/// Success — everything worked as expected.
pub const EXIT_SUCCESS: i32 = 0;
/// Generic error — something went wrong (I/O, parse error, etc.).
pub const EXIT_ERR: i32 = 1;
/// Verification / check failed — the data did NOT pass validation.
pub const EXIT_VERIFY_FAILED: i32 = 2;
/// Configuration error — config file missing, invalid syntax, broken
/// invariants detected by strict mode.
pub const EXIT_CONFIG_ERR: i32 = 3;

// ---------------------------------------------------------------------------
// Output macros — delegate to the centralized output module so they
// automatically respect `--quiet`, `--color`, and `--output`.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// CLI struct with global output flags (available on every subcommand).
// ---------------------------------------------------------------------------

/// DoLogger CLI — manage, verify, and control the DoLogger logging engine.
#[derive(Parser)]
#[command(
    name = "dologctl",
    version = env!("CARGO_PKG_VERSION"),
    about = "DoLogger CLI management tool",
    long_about = "A CLI tool for managing DoLogger: configuration, plugins, log verification, and operational control.",
    // Ensure consistent help styling — `--output`, `--color`, and `--quiet`
    // appear under a "Global" options section.
    next_display_order = None,
)]
struct Cli {
    /// Output format: text (default, human-readable) or json (machine-readable)
    #[arg(short = 'o', long, global = true, default_value = "text", value_enum)]
    output: output::OutputFormat,

    /// Controls when to use ANSI colour escapes
    #[arg(long, global = true, default_value = "auto", value_enum)]
    color: output::ColorMode,

    /// Suppress non-error output (verify commands exit silently on success)
    #[arg(short = 'q', long, global = true, default_value_t = false)]
    quiet: bool,

    /// Display third-party license attributions (use with version/about)
    #[arg(long, global = true, default_value_t = false)]
    licenses: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate a configuration file from a template
    Init {
        /// Template to use: dev, prod, audit
        #[arg(short, long, default_value = "dev")]
        template: String,
    },
    /// Run the DoLogger engine
    Run {
        /// Validate configuration only (dry-run), do not start engine
        #[arg(long)]
        dry_run: bool,

        /// Path to configuration file
        #[arg(short, long)]
        config: Option<String>,

        /// Enable per-record pipeline stage timing trace
        #[arg(long)]
        trace: bool,
    },
    /// Manage plugins
    Plugin {
        #[command(subcommand)]
        action: PluginAction,
    },
    /// Validate configuration file
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Verify log integrity
    VerifyLog {
        /// Path to the log file
        path: String,
        /// Public key hex (32 bytes) for Ed25519 signature verification
        #[arg(long)]
        pubkey: Option<String>,
    },
    /// Verify external anchor JSON file
    VerifyAnchor {
        /// Path to the anchor JSON file
        path: String,
        /// Public key hex (32 bytes) for signature verification
        #[arg(long)]
        pubkey: Option<String>,
    },
    /// Scan *.worm files and report LSN continuity
    RecoveryReport {
        /// Directory containing .worm files
        #[arg(default_value = ".")]
        worm_dir: String,
    },
    /// Generate synthetic SIF test records
    Record {
        /// Domain name for test records
        domain: String,
        /// Output SIF file path
        // Short flag is -f: -o is taken by the global --output format flag.
        #[arg(short = 'f', long)]
        output: String,
        /// Duration in seconds
        #[arg(short, long, default_value = "10")]
        duration: u64,
    },
    /// Replay records from a SIF file
    Replay {
        /// Input SIF file path
        input: String,
        /// Replay speed: "1" = real-time stalling, "max" = full speed (default)
        #[arg(short, long, default_value = "max")]
        speed: String,
    },
    /// Check recording session status
    RecordStop {
        /// Domain name
        domain: String,
    },
    /// Shared memory management
    Shm {
        #[command(subcommand)]
        action: ShmAction,
    },
    /// Run local performance benchmark
    Perf {
        /// Number of records to push (default: 100000)
        #[arg(long, default_value = "100000")]
        count: usize,

        /// Size of each log message in bytes (default: 80, max: 255)
        #[arg(long, default_value = "80")]
        message_size: usize,
    },
    /// Show full project information and system details
    About,
    /// Show version information
    Version,
    /// Generate shell completion scripts
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
}

#[derive(Subcommand)]
enum PluginAction {
    /// Install a plugin
    Install { source: String },
    /// List installed plugins
    List,
    /// Remove a plugin
    Remove { name: String },
    /// Verify plugin integrity (ABI, trust, symbol resolution)
    Verify {
        /// Plugin name to verify (omit to verify all)
        name: Option<String>,
    },
    /// Scan plugins for suspicious symbols
    Scan,
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Validate a configuration file
    Validate {
        #[arg(long)]
        strict: bool,
        #[arg(short, long)]
        config: Option<String>,
    },
}

#[derive(Subcommand)]
enum ShmAction {
    /// Display shared memory region metadata
    Status {
        /// Shared memory path
        path: String,
    },
    /// Clean up orphaned shared memory
    Clear {
        /// Shared memory path
        path: String,
        /// Force cleanup even if producer is alive
        #[arg(long)]
        force: bool,
    },
}

// ---------------------------------------------------------------------------
// Template content
// ---------------------------------------------------------------------------

const DEV_TEMPLATE: &str = r#"# DoLogger Development Configuration
# Generated by: dologctl init --template dev

[dologger]
level = "DEBUG"
performance_profile = "dev"
ring_buffer_size = 65536    # 64K records
batch_size = 32
enable_signature = false
"#;

const PROD_TEMPLATE: &str = r#"# DoLogger Production Configuration
# Generated by: dologctl init --template prod

[dologger]
level = "INFO"
performance_profile = "prod-performance"
ring_buffer_size = 262144   # 256K records
batch_size = 256
enable_signature = false    # Enable for audit domains

# Domain-specific configuration
# [dologger.domain.app]
# level = "INFO"
# sinks = ["console", "file"]

# Console sink definition
# [sinks.console]
# type = "console"
# use_stderr = false
"#;

const AUDIT_TEMPLATE: &str = r#"# DoLogger Audit Configuration
# Generated by: dologctl init --template audit

[dologger]
level = "INFO"
performance_profile = "prod-audit"
ring_buffer_size = 262144   # 256K records
batch_size = 128
enable_signature = true

# [dologger.compliance]
# template = "hipaa"
"#;

// ---------------------------------------------------------------------------
// main — parse, configure output, dispatch
// ---------------------------------------------------------------------------

fn main() {
    let cli = Cli::parse();

    // Build output config and initialise global state so every command
    // module can query `output::color_enabled()` / `output::is_quiet()`.
    let cfg = output::OutputConfig {
        format: cli.output,
        color: cli.color,
        quiet: cli.quiet,
    };
    output::init(&cfg);

    match cli.command {
        Commands::Init { template } => cmd_init(&template),
        Commands::Run {
            dry_run,
            config,
            trace,
        } => cmd_run(dry_run, config.as_deref(), trace),
        Commands::Plugin { action } => cmd_plugin(action),
        Commands::Config { action } => cmd_config(action),
        Commands::VerifyLog { path, pubkey } => {
            commands::verify::cmd_verify_log(&path, pubkey.as_deref(), cfg.format)
        }
        Commands::VerifyAnchor { path, pubkey } => {
            commands::verify::cmd_verify_anchor(&path, pubkey.as_deref(), cfg.format)
        }
        Commands::RecoveryReport { worm_dir } => {
            commands::verify::cmd_recovery_report(&worm_dir, cfg.format)
        }
        Commands::Record {
            domain,
            output,
            duration,
        } => commands::record::cmd_record(&domain, &output, duration, cfg.format),
        Commands::Replay { input, speed } => {
            commands::record::cmd_replay(&input, &speed, cfg.format)
        }
        Commands::RecordStop { domain } => commands::record::cmd_record_stop(&domain, cfg.format),
        Commands::Shm { action } => cmd_shm(action, cfg.format),
        Commands::Perf {
            count,
            message_size,
        } => commands::perf::cmd_perf(count, message_size, cfg.format),
        Commands::About | Commands::Version => {
            if cli.licenses {
                banner::print_licenses();
            } else {
                banner::print_banner(&cfg);
            }
        }
        Commands::Completions { shell } => cmd_completions(shell),
    }
}

// ---------------------------------------------------------------------------
// Completions
// ---------------------------------------------------------------------------

/// Generate shell completion script for the given shell and print it to
/// stdout so the user can source or eval it.
fn cmd_completions(shell: clap_complete::Shell) {
    use clap::CommandFactory;
    let mut cmd = Cli::command();
    let name = cmd.get_name().to_string();
    clap_complete::generate(shell, &mut cmd, &name, &mut std::io::stdout());
}

// ---------------------------------------------------------------------------
// Command handlers
// ---------------------------------------------------------------------------

fn cmd_init(template: &str) {
    let content = match template {
        "dev" => DEV_TEMPLATE,
        "prod" => PROD_TEMPLATE,
        "audit" => AUDIT_TEMPLATE,
        other => {
            stderr!("Error: Unknown template '{other}'");
            stderr!("Available templates: dev, prod, audit");
            std::process::exit(EXIT_ERR);
        }
    };

    let filename = "dologger.toml";
    if std::path::Path::new(filename).exists() {
        stderr!("Warning: '{filename}' already exists. Use a different name or remove the existing file.");
        std::process::exit(EXIT_ERR);
    }

    match std::fs::write(filename, content) {
        Ok(()) => stdout!("Created '{filename}' from template '{template}'"),
        Err(e) => {
            stderr!("Error writing '{filename}': {e}");
            std::process::exit(EXIT_ERR);
        }
    }
}

fn cmd_run(dry_run: bool, config_path: Option<&str>, trace: bool) {
    if dry_run {
        stdout!("=== DoLogger Dry-Run Configuration Validation ===\n");
        if trace {
            stdout!("Trace mode enabled — will trace pipeline stages during run.\n");
        }
        validate_config(config_path)
    } else if trace {
        commands::run::cmd_run_trace(config_path);
    } else {
        stderr!("Engine startup not yet implemented (M3+)");
        stderr!("Use --dry-run to validate configuration without starting.");
        stderr!("Use --trace to run with pipeline stage timing.");
        std::process::exit(EXIT_ERR);
    }
}

fn validate_config(config_path: Option<&str>) {
    // Load and validate the configuration
    let config = if let Some(path) = config_path {
        match std::fs::read_to_string(path) {
            Ok(content) => {
                stdout!("Configuration file: {path}");
                stdout!("File size: {} bytes\n", content.len());
                content
            }
            Err(e) => {
                stderr!("Error: Cannot read config file '{path}': {e}");
                std::process::exit(EXIT_CONFIG_ERR);
            }
        }
    } else {
        // Search for config in default locations
        let candidates = ["dologger.toml", ".dologger.toml"];
        let mut found = None;
        for c in &candidates {
            if std::path::Path::new(c).exists() {
                found = Some(*c);
                break;
            }
        }
        match found {
            Some(path) => {
                stdout!("Configuration file: {path} (auto-detected)\n");
                std::fs::read_to_string(path).unwrap_or_default()
            }
            None => {
                stdout!("No configuration file found.");
                stdout!("Searched: {}", candidates.join(", "));
                stdout!("Using hardcoded defaults.\n");
                stdout!("Configuration summary (defaults):");
                stdout!("  level: INFO");
                stdout!("  performance_profile: prod-performance");
                stdout!("  ring_buffer_size: 262144");
                stdout!("  batch_size: 256");
                stdout!("  enable_signature: false");
                stdout!("\nRun 'dologctl init --template dev' to generate a config file.");
                return;
            }
        }
    };

    // Try to parse as TOML
    match config.parse::<toml::Table>() {
        Ok(table) => {
            stdout!("TOML syntax: VALID");

            if let Some(dologger) = table.get("dologger").and_then(|v| v.as_table()) {
                stdout!("\n[dologger] section:");
                for (key, value) in dologger {
                    stdout!("  {key} = {value}");
                }
            } else {
                stdout!("\nNote: No [dologger] section found — all values will use defaults.");
            }

            stdout!("\nConfiguration validation: PASSED");
        }
        Err(e) => {
            stderr!("\nTOML syntax: INVALID");
            stderr!("Error: {e}");
            std::process::exit(EXIT_CONFIG_ERR);
        }
    }
}

fn cmd_plugin(action: PluginAction) {
    match action {
        PluginAction::Install { source } => commands::plugin::cmd_plugin_install(&source),
        PluginAction::List => commands::plugin::cmd_plugin_list(),
        PluginAction::Remove { name } => commands::plugin::cmd_plugin_remove(&name),
        PluginAction::Verify { name } => commands::plugin::cmd_plugin_verify(name.as_deref()),
        PluginAction::Scan => commands::plugin::cmd_plugin_scan(),
    }
}

fn cmd_config(action: ConfigAction) {
    match action {
        ConfigAction::Validate { strict, config } => {
            if strict {
                let passed = commands::config::cmd_config_validate_strict(config.as_deref());
                if !passed {
                    std::process::exit(EXIT_CONFIG_ERR);
                }
            } else {
                commands::config::cmd_config_validate_normal(config.as_deref());
            }
        }
    }
}

fn cmd_shm(action: ShmAction, format: output::OutputFormat) {
    match action {
        ShmAction::Status { path } => commands::shm::cmd_shm_status(&path, format),
        ShmAction::Clear { path, force } => commands::shm::cmd_shm_clear(&path, force, format),
    }
}
