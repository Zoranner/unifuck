//! Initialises the loader log file at `%LOCALAPPDATA%\Tuanjie\unifuck-loader.log`.
//!
//! Uses `simplelog::WriteLogger` for a single-writer, append-only log. Any
//! initialisation failure is swallowed: `log::*` macros become no-ops, which
//! is acceptable because the log is observational only.

use std::env;
use std::fs::{File, OpenOptions};
use std::io;
use std::path::PathBuf;

use simplelog::{ConfigBuilder, LevelFilter, WriteLogger};

/// Returns the resolved log file path or an io::Error explaining why it could
/// not be created.
fn log_path() -> io::Result<PathBuf> {
    let local = env::var_os("LOCALAPPDATA").ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "LOCALAPPDATA environment variable not set",
        )
    })?;
    let mut p = PathBuf::from(local);
    p.push("Tuanjie");
    std::fs::create_dir_all(&p)?;
    p.push("unifuck-loader.log");
    Ok(p)
}

/// Initialises the global logger. Subsequent calls are no-ops (returns Ok)
/// because `WriteLogger::init` may only be called once per process; if it has
/// already been initialised by something else we treat it as success.
pub fn init() -> io::Result<()> {
    let path = log_path()?;
    let file: File = OpenOptions::new().create(true).append(true).open(&path)?;
    let config = ConfigBuilder::new()
        .set_time_format_rfc3339()
        .set_thread_level(LevelFilter::Error)
        .build();
    // If init fails (already-set logger), we silently swallow it: there is no
    // recovery and `log::*` macros simply use the existing logger.
    let _ = WriteLogger::init(LevelFilter::Info, config, file);
    Ok(())
}

/// Emits a one-line banner identifying this loader instance.
pub fn banner() {
    log::info!(
        "unifuck-loader v{} pid={} tid={}",
        env!("CARGO_PKG_VERSION"),
        std::process::id(),
        unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() },
    );
}
