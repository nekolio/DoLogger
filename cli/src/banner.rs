//! Colourful project banner display.
//!
//! Displays the DoLogger ASCII art logo with system information,
//! inspired by neofetch/screenfetch and modern CLI tools.
//! Uses ANSI terminal colours for a polished, professional appearance.
//!
//! When stdout is not a terminal (piped / redirected), the ASCII art
//! logo is skipped and only a single version line is printed — similar
//! to how `bat` and `eza` handle non-terminal output.

use crate::output::{self, color, OutputConfig};

/// Print the full project banner with environment info.
/// Triggered by `dologctl version`, `dologctl about`, or plain `dologctl`
/// with no subcommand.
///
/// When stdout is piped, the ASCII logo is skipped and only the version
/// line is printed (analogous to `bat --plain` / `rg --no-heading`).
pub fn print_banner(cfg: &OutputConfig) {
    if cfg.is_piped() || cfg.quiet {
        // Piped output — skip ASCII art, print bare version line
        output::stdout_line(&format!("dologctl v{}", env!("CARGO_PKG_VERSION")));
        return;
    }
    print_logo();
    print_info();
}

/// Return `true` when ANSI colour escapes should be emitted.
///
/// This delegates to the global `output::color_enabled()` which is
/// initialised from the CLI's `--color` flag in `main()`.
#[allow(dead_code)]
pub fn use_color() -> bool {
    output::color_enabled()
}

fn print_logo() {
    let bright_cyan = output::when_color(color::BRIGHT_CYAN);
    let cyan = output::when_color(color::CYAN);
    let blue = output::when_color(color::BLUE);
    let reset = output::when_color(color::RESET);

    let logo = format!(
        r#"{bright_cyan}   ___       __                         {reset}
{bright_cyan}  / _ \___  / /  ___  ___ ____ ____ ____{reset}
{cyan} / // / _ \/ /__/ _ \/ _ `/ _ `/ -_) __/{reset}
{blue}/____/\___/____/\___/\_, /\_, /\__/_/   {reset}
{blue}                    /___//___/           {reset}"#
    );
    output::stdout_line(&logo);
}

fn print_info() {
    let version = env!("CARGO_PKG_VERSION");
    let rustc = option_env!("RUSTC_VERSION").unwrap_or("stable");
    let target = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };

    // Gather system info
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let hostname = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    let pid = std::process::id();
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| ".".to_string());

    // Compute colour strings (or empty when colour is off)
    let bold = output::when_color(color::BOLD);
    let dim = output::when_color(color::DIM);
    let green = output::when_color(color::GREEN);
    let cyan = output::when_color(color::CYAN);
    let bright_green = output::when_color(color::BRIGHT_GREEN);
    let bright_magenta = output::when_color(color::BRIGHT_MAGENTA);
    let bright_white = output::when_color(color::BRIGHT_WHITE);
    let reset = output::when_color(color::RESET);

    // Build info lines with key-value alignment
    let info = format!(
        r#"{bold}{bright_white}  DoLogger CLI (dologctl){reset}
{dim}  ───────────────────────────{reset}
  {bold}Project{reset}     {green}DoLogger{reset} — Cross-platform, high-security logging engine
  {bold}Version{reset}     {bright_green}v{version}{reset} ({profile} build)
  {bold}Rustc{reset}       {rustc}
  {bold}Target{reset}      {target}
  {bold}License{reset}     Apache-2.0 OR MIT
  {bold}Repository{reset}  {cyan}https://github.com/Nekolio/DoLogger{reset}

{dim}  ───────────────────────────{reset}
  {bold}System{reset}      {os} / {arch}
  {bold}Hostname{reset}    {hostname}
  {bold}Process{reset}     PID {pid}
  {bold}Directory{reset}   {cwd}

{dim}  ───────────────────────────{reset}
  {bold}Author{reset}      {bright_magenta}@Nekolio{reset} {dim}<https://github.com/Nekolio>{reset}
  {bold}Contact{reset}     dologger@nekolio.dev

  {dim}Plugins: 10 VTable types | Sinks: 9 built-in | Audit: Ed25519 + LSN chain{reset}
  {dim}Performance: 102ns P50 submit | 13.3M rec/s batch throughput{reset}
  {dim}Security: 115 tests | 0 clippy | Sandbox isolation framework{reset}
"#,
        bold = bold,
        dim = dim,
        green = green,
        cyan = cyan,
        bright_green = bright_green,
        bright_magenta = bright_magenta,
        bright_white = bright_white,
        reset = reset,
    );

    output::stdout_line(&info);
}

/// Print third-party license attributions from the NOTICE file.
///
/// For interactive navigation, pipe the output:
/// ```bash
/// dologctl version --licenses | less -R
/// ```
pub fn print_licenses() {
    let bold = output::when_color(color::BOLD);
    let dim = output::when_color(color::DIM);
    let reset = output::when_color(color::RESET);

    let licenses_text = include_str!("../../NOTICE");

    output::stdout_line(&format!(
        "{bold}DoLogger — Third-Party License Attributions{reset}"
    ));
    output::stdout_line(&format!(
        "{dim}Project: Apache-2.0 OR MIT  |  Pipe to `less -R` for interactive paging{reset}\n"
    ));

    for line in licenses_text.lines() {
        output::stdout_line(line);
    }
}
