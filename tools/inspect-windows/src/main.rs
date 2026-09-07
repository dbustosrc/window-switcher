use anyhow::{Context, Result};
use std::time::{Duration, Instant};
use window_switcher::utils::*;

use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::Graphics::Dwm::{DWM_CLOAKED_APP, DWM_CLOAKED_INHERITED, DWM_CLOAKED_SHELL};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetGuiResources, GetProcessHandleCount, GR_GDIOBJECTS,
};
use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindow, GW_OWNER};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|arg| arg == "--benchmark") {
        let iterations = args
            .get(2)
            .and_then(|value| value.parse().ok())
            .unwrap_or(100);
        return benchmark_enumeration(iterations);
    }
    let mut hwnds: Vec<HWND> = Default::default();
    unsafe { EnumWindows(Some(enum_window), LPARAM(&mut hwnds as *mut _ as isize)) }
        .with_context(|| "Fail to enum windows".to_string())?;
    for hwnd in hwnds {
        let title = get_window_title(hwnd);
        let cloak_type = get_window_cloak_type(hwnd);
        let (is_visible, is_iconic, is_tool) = get_window_state(hwnd);
        let (width, height) = get_window_size(hwnd);
        let owner_hwnd: HWND = unsafe { GetWindow(hwnd, GW_OWNER) }.unwrap_or_default();
        let owner_title = if !owner_hwnd.is_invalid() {
            get_window_title(owner_hwnd)
        } else {
            "".into()
        };
        println!(
            "visible:{}iconic:{}tool:{}cloak:{} {:>10} {:>10}:{} {}:{}",
            pretty_bool(is_visible),
            pretty_bool(is_iconic),
            pretty_bool(is_tool),
            pretty_cloak(cloak_type),
            format!("{}x{}", width, height),
            hwnd.0 as isize,
            title,
            owner_hwnd.0 as isize,
            owner_title
        );
    }
    Ok(())
}

fn benchmark_enumeration(iterations: usize) -> Result<()> {
    let iterations = iterations.max(1);
    let is_admin = is_running_as_admin().unwrap_or(false);
    let first_started = Instant::now();
    let first_windows = list_all_windows(false, true, is_admin)?;
    let first_latency = first_started.elapsed();
    let process = unsafe { GetCurrentProcess() };
    let mut handles_before = 0;
    unsafe { GetProcessHandleCount(process, &mut handles_before)? };
    let gdi_before = unsafe { GetGuiResources(process, GR_GDIOBJECTS) };

    let mut samples = Vec::with_capacity(iterations);
    let mut window_count = first_windows.len();
    for _ in 0..iterations {
        let started = Instant::now();
        window_count = list_all_windows(false, true, is_admin)?.len();
        samples.push(started.elapsed());
    }
    let mut handles_after = 0;
    unsafe { GetProcessHandleCount(process, &mut handles_after)? };
    let gdi_after = unsafe { GetGuiResources(process, GR_GDIOBJECTS) };
    samples.sort_unstable();
    println!("iterations: {iterations}");
    println!("windows: {window_count}");
    println!("first: {:.3} ms", millis(first_latency));
    println!("p50: {:.3} ms", millis(percentile(&samples, 50)));
    println!("p95: {:.3} ms", millis(percentile(&samples, 95)));
    println!("max: {:.3} ms", millis(*samples.last().unwrap()));
    println!(
        "handles after warm-up: {handles_before} -> {handles_after} (delta {})",
        handles_after as i64 - handles_before as i64
    );
    println!(
        "GDI objects after warm-up: {gdi_before} -> {gdi_after} (delta {})",
        gdi_after as i64 - gdi_before as i64
    );
    Ok(())
}

fn percentile(samples: &[Duration], percentile: usize) -> Duration {
    let index = ((samples.len() - 1) * percentile / 100).min(samples.len() - 1);
    samples[index]
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn pretty_bool(value: bool) -> String {
    if value {
        "*".into()
    } else {
        " ".into()
    }
}

fn pretty_cloak(value: u32) -> &'static str {
    match value {
        0 => " ",
        DWM_CLOAKED_SHELL => "S",
        DWM_CLOAKED_APP => "A",
        DWM_CLOAKED_INHERITED => "I",
        _ => "?",
    }
}

extern "system" fn enum_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let windows: &mut Vec<HWND> = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
    windows.push(hwnd);
    BOOL(1)
}
