//! 零依赖诊断日志: `%APPDATA%\unifuck\unifuck.log`。
//! 每次扫描截断重写, 用于远程定位用户机器上的发现问题。

use std::path::PathBuf;

fn log_path() -> Option<PathBuf> {
    let appdata = std::env::var("APPDATA").ok()?;
    let dir = PathBuf::from(appdata).join("unifuck");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("unifuck.log"))
}

/// 扫描开始时调用: 截断旧日志并写入头部。
pub(crate) fn begin_session() {
    let Some(path) = log_path() else {
        return;
    };
    let header = format!("unifuck {} discovery log\n", env!("CARGO_PKG_VERSION"));
    let _ = std::fs::write(path, header);
}

pub(crate) fn log(msg: &str) {
    let Some(path) = log_path() else {
        return;
    };
    if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(path) {
        use std::io::Write;
        let _ = writeln!(f, "{}", msg);
    }
}
