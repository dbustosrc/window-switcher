#![windows_subsystem = "windows"]

use anyhow::{anyhow, bail, Result};
use std::{
    backtrace::Backtrace,
    fs::{File, OpenOptions},
    io::Write,
    panic::PanicHookInfo,
    path::Path,
    sync::OnceLock,
    time::{SystemTime, UNIX_EPOCH},
};

use window_switcher::{alert, load_config, start, utils::SingleInstance};

const CRASH_LOG_NAME: &str = "window-switcher-crash.log";
const SESSION_MARKER_NAME: &str = "window-switcher-running.marker";
static CRASH_LOG_PATH: OnceLock<std::path::PathBuf> = OnceLock::new();

fn main() {
    install_panic_logging();
    if let Err(err) = run() {
        let details = format!("{err:#}\nbacktrace:\n{}", Backtrace::force_capture());
        log::error!("fatal error: {details}");
        append_crash_log("fatal error", &details);
        alert!("{err}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    unsafe {
        let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
            windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );
    }

    let config = load_config().map_err(|err| anyhow!("Failed to load configuration: {err:#}"))?;
    if let Some(log_file) = &config.log_file {
        let file = prepare_log_file(log_file).map_err(|err| {
            anyhow!(
                "Failed to prepare log file at {}, {err}",
                log_file.display()
            )
        })?;
        simple_logging::log_to(file, config.log_level);
        log::info!(
            "window-switcher {} started, pid={}, log={}",
            env!("CARGO_PKG_VERSION"),
            std::process::id(),
            log_file.display()
        );
    }
    let instance = SingleInstance::create("WindowSwitcherMutex")?;
    if !instance.is_single() {
        bail!("Another instance is running. This instance will abort.")
    }
    let session_marker = begin_session_marker();
    let result = start(&config);
    if result.is_ok() {
        log::info!("window-switcher stopped normally");
        clear_session_marker(session_marker.as_deref());
    }
    result
}

fn prepare_log_file(path: &Path) -> std::io::Result<File> {
    if path.exists() {
        OpenOptions::new().append(true).open(path)
    } else {
        File::create(path)
    }
}

fn install_panic_logging() {
    let crash_log_path = diagnostic_path(CRASH_LOG_NAME);
    let _ = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&crash_log_path);
    let _ = CRASH_LOG_PATH.set(crash_log_path);
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let details = format_panic(panic_info);
        log::error!("panic: {details}");
        append_crash_log("panic", &details);
        previous_hook(panic_info);
    }));
}

fn format_panic(panic_info: &PanicHookInfo<'_>) -> String {
    let payload = panic_info
        .payload()
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| {
            panic_info
                .payload()
                .downcast_ref::<String>()
                .map(String::as_str)
        })
        .unwrap_or("non-string panic payload");
    let location = panic_info
        .location()
        .map(|location| {
            format!(
                "{}:{}:{}",
                location.file(),
                location.line(),
                location.column()
            )
        })
        .unwrap_or_else(|| "unknown location".into());
    let thread = std::thread::current();
    let thread_name = thread.name().unwrap_or("unnamed");
    format!(
        "thread={thread_name}, location={location}, message={payload}\nbacktrace:\n{}",
        Backtrace::force_capture()
    )
}

fn append_crash_log(kind: &str, details: &str) {
    let Some(path) = CRASH_LOG_PATH.get() else {
        return;
    };
    append_diagnostic(path, kind, details);
}

fn append_diagnostic(path: &Path, kind: &str, details: &str) {
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let timestamp = unix_timestamp();
    let _ = writeln!(
        file,
        "\n[{timestamp}] {kind}; version={}; pid={}\n{details}",
        env!("CARGO_PKG_VERSION"),
        std::process::id()
    );
    let _ = file.flush();
}

fn begin_session_marker() -> Option<std::path::PathBuf> {
    let path = diagnostic_path(SESSION_MARKER_NAME);
    if let Ok(previous_session) = std::fs::read_to_string(&path) {
        let details = format!(
            "A previous session marker was found. The process did not record a normal shutdown.\n{}",
            previous_session.trim()
        );
        log::warn!("{details}");
        append_crash_log("unclean previous shutdown", &details);
    }

    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&path)
        .ok()?;
    let timestamp = unix_timestamp();
    writeln!(
        file,
        "version={} pid={} started_at_unix={timestamp}",
        env!("CARGO_PKG_VERSION"),
        std::process::id()
    )
    .ok()?;
    file.flush().ok()?;
    Some(path)
}

fn clear_session_marker(path: Option<&Path>) {
    if let Some(path) = path {
        let _ = std::fs::remove_file(path);
    }
}

fn diagnostic_path(name: &str) -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.join(name)))
        .unwrap_or_else(|| name.into())
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_log_contains_failure_context() {
        let path = std::env::temp_dir().join(format!(
            "window-switcher-diagnostic-{}-{}.log",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        append_diagnostic(&path, "test panic", "thread=test, message=diagnostic");
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("test panic"));
        assert!(content.contains(&format!("version={}", env!("CARGO_PKG_VERSION"))));
        assert!(content.contains("thread=test, message=diagnostic"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn normal_shutdown_removes_session_marker() {
        let path = std::env::temp_dir().join(format!(
            "window-switcher-session-{}-{}.marker",
            std::process::id(),
            unix_timestamp()
        ));
        std::fs::write(&path, "test marker").unwrap();
        clear_session_marker(Some(&path));
        assert!(!path.exists());
    }
}
