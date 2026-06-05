//! Panic hook that swallows panics inside the loader and writes them to the
//! log file. The loader must never abort the host process.

use std::panic;
use std::panic::PanicHookInfo;

/// Installs a panic hook that logs the panic payload and location, then
/// returns normally. Combined with `panic = "unwind"` (default in workspace),
/// the worker thread will unwind to its `extern "system"` boundary and the
/// thread simply exits — Tuanjie keeps running.
pub fn install() {
    panic::set_hook(Box::new(report));
}

fn report(info: &PanicHookInfo<'_>) {
    let payload = info
        .payload()
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "<non-string payload>".to_string());
    let location = info
        .location()
        .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
        .unwrap_or_else(|| "<no location>".to_string());
    log::error!("loader panic at {location}: {payload}");
}
