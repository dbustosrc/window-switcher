use anyhow::{anyhow, bail, Context, Result};
use std::{
    process::{Child, Command},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use window_switcher::utils::set_foreground_window;
use windows::{
    core::w,
    Win32::{
        Foundation::{FILETIME, HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM},
        Graphics::Gdi::UpdateWindow,
        System::{
            LibraryLoader::GetModuleHandleW,
            ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX},
            Threading::{
                GetGuiResources, GetProcessHandleCount, GetProcessTimes, OpenProcess,
                GR_GDIOBJECTS, GR_USEROBJECTS, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ,
            },
        },
        UI::{
            Input::KeyboardAndMouse::{
                SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
                KEYEVENTF_SCANCODE, VK_MENU, VK_TAB,
            },
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, FindWindowW,
                GetMessageW, GetWindowLongPtrW, GetWindowThreadProcessId, IsWindowVisible,
                PostMessageW, PostQuitMessage, RegisterClassW, ShowWindow, TranslateMessage,
                CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, GWL_USERDATA, MSG, SW_SHOW, WINDOW_EX_STYLE,
                WM_CLOSE, WM_COMMAND, WM_DESTROY, WNDCLASSW, WS_OVERLAPPEDWINDOW,
            },
        },
    },
};

const SWITCHER_NAME: windows::core::PCWSTR = w!("Window Switcher");
const BENCHMARK_CLASS: windows::core::PCWSTR = w!("Window Switcher Benchmark Target");
const BENCHMARK_TITLE: windows::core::PCWSTR = w!("Window Switcher Benchmark Window");
const SWITCHER_EXIT_COMMAND: usize = 1;
const DEFAULT_WARMUP_CYCLES: usize = 20;
const DEFAULT_MEASURED_CYCLES: usize = 500;
const DEFAULT_EXTRA_TABS: usize = 3;
const VISIBILITY_TIMEOUT: Duration = Duration::from_millis(500);

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let exe_path = args
        .get(1)
        .context("usage: benchmark-switcher <window-switcher.exe> [cycles] [extra-tabs]")?;
    let cycles = parse_arg(&args, 2, DEFAULT_MEASURED_CYCLES)?;
    let extra_tabs = parse_arg(&args, 3, DEFAULT_EXTRA_TABS)?;

    let (benchmark_hwnd, benchmark_thread) = start_benchmark_window()?;
    let mut switcher = SwitcherProcess::start(exe_path)?;

    let (cold_opening, cold_closing) = run_cycle(benchmark_hwnd, switcher.hwnd, extra_tabs)?;
    for _ in 1..DEFAULT_WARMUP_CYCLES {
        run_cycle(benchmark_hwnd, switcher.hwnd, extra_tabs)?;
    }
    thread::sleep(Duration::from_millis(250));

    let before = ProcessSnapshot::capture(switcher.process)?;
    let mut opening_samples = Vec::with_capacity(cycles);
    let mut closing_samples = Vec::with_capacity(cycles);
    for _ in 0..cycles {
        let (opening, closing) = run_cycle(benchmark_hwnd, switcher.hwnd, extra_tabs)?;
        opening_samples.push(opening);
        closing_samples.push(closing);
    }
    thread::sleep(Duration::from_millis(250));
    let after = ProcessSnapshot::capture(switcher.process)?;

    opening_samples.sort_unstable();
    closing_samples.sort_unstable();
    println!("warmup cycles: {DEFAULT_WARMUP_CYCLES}");
    println!("measured cycles: {cycles}");
    println!("extra Tab presses per cycle: {extra_tabs}");
    println!("cold open: {:.3} ms", millis(cold_opening));
    println!("cold close: {:.3} ms", millis(cold_closing));
    print_latency("open", &opening_samples);
    print_latency("close", &closing_samples);
    println!(
        "handles: {} -> {} (delta {:+})",
        before.handles,
        after.handles,
        after.handles as i64 - before.handles as i64
    );
    println!(
        "GDI objects: {} -> {} (delta {:+})",
        before.gdi,
        after.gdi,
        after.gdi as i64 - before.gdi as i64
    );
    println!(
        "USER objects: {} -> {} (delta {:+})",
        before.user,
        after.user,
        after.user as i64 - before.user as i64
    );
    println!(
        "private memory: {:.3} -> {:.3} MiB (delta {:+.3} MiB)",
        mib(before.private_bytes),
        mib(after.private_bytes),
        signed_mib(after.private_bytes as i64 - before.private_bytes as i64)
    );
    println!(
        "working set: {:.3} -> {:.3} MiB (delta {:+.3} MiB)",
        mib(before.working_set),
        mib(after.working_set),
        signed_mib(after.working_set as i64 - before.working_set as i64)
    );
    println!(
        "CPU time during measured cycles: {:.3} ms",
        ticks_to_millis(after.cpu_ticks.saturating_sub(before.cpu_ticks))
    );

    switcher.stop()?;
    unsafe {
        let _ = PostMessageW(Some(benchmark_hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
    }
    benchmark_thread
        .join()
        .map_err(|_| anyhow!("benchmark window thread panicked"))??;
    Ok(())
}

fn parse_arg(args: &[String], index: usize, default: usize) -> Result<usize> {
    args.get(index)
        .map(|value| value.parse().context("invalid numeric argument"))
        .transpose()
        .map(|value| value.unwrap_or(default).max(1))
}

fn start_benchmark_window() -> Result<(HWND, thread::JoinHandle<Result<()>>)> {
    let (sender, receiver) = mpsc::sync_channel(1);
    let handle = thread::Builder::new()
        .name("benchmark-window".into())
        .spawn(move || benchmark_window_loop(sender))?;
    let raw_hwnd = receiver
        .recv_timeout(Duration::from_secs(5))
        .context("benchmark window was not created")??;
    Ok((HWND(raw_hwnd as _), handle))
}

fn benchmark_window_loop(sender: mpsc::SyncSender<Result<isize>>) -> Result<()> {
    let hmodule = unsafe { GetModuleHandleW(None) }?;
    let class = WNDCLASSW {
        hInstance: HINSTANCE(hmodule.0),
        lpszClassName: BENCHMARK_CLASS,
        lpfnWndProc: Some(benchmark_window_proc),
        style: CS_HREDRAW | CS_VREDRAW,
        ..Default::default()
    };
    if unsafe { RegisterClassW(&class) } == 0 {
        let error = windows::core::Error::from_win32();
        let _ = sender.send(Err(anyhow!("failed to register benchmark window: {error}")));
        return Ok(());
    }
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            BENCHMARK_CLASS,
            BENCHMARK_TITLE,
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            800,
            600,
            None,
            None,
            Some(hmodule.into()),
            None,
        )
    }?;
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = UpdateWindow(hwnd);
    }
    sender
        .send(Ok(hwnd.0 as isize))
        .map_err(|_| anyhow!("failed to publish benchmark window"))?;

    let mut message = MSG::default();
    loop {
        let result = unsafe { GetMessageW(&mut message, None, 0, 0) };
        if result.0 <= 0 {
            break;
        }
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    Ok(())
}

unsafe extern "system" fn benchmark_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

struct SwitcherProcess {
    child: Child,
    hwnd: HWND,
    process: HANDLE,
    stopped: bool,
}

impl SwitcherProcess {
    fn start(exe_path: &str) -> Result<Self> {
        let child = Command::new(exe_path)
            .spawn()
            .with_context(|| format!("failed to launch {exe_path}"))?;
        let hwnd = wait_for_switcher_window(child.id(), Duration::from_secs(5))?;
        let process = unsafe {
            OpenProcess(
                PROCESS_QUERY_INFORMATION | PROCESS_VM_READ,
                false,
                child.id(),
            )
        }?;
        Ok(Self {
            child,
            hwnd,
            process,
            stopped: false,
        })
    }

    fn stop(&mut self) -> Result<()> {
        if self.stopped {
            return Ok(());
        }
        unsafe {
            PostMessageW(
                Some(self.hwnd),
                WM_COMMAND,
                WPARAM(SWITCHER_EXIT_COMMAND),
                LPARAM(0),
            )?;
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if self.child.try_wait()?.is_some() {
                self.stopped = true;
                return Ok(());
            }
            thread::sleep(Duration::from_millis(10));
        }
        self.child.kill()?;
        let _ = self.child.wait();
        self.stopped = true;
        Ok(())
    }
}

impl Drop for SwitcherProcess {
    fn drop(&mut self) {
        let _ = self.stop();
        if !self.process.is_invalid() {
            unsafe {
                let _ = windows::Win32::Foundation::CloseHandle(self.process);
            }
        }
    }
}

fn wait_for_switcher_window(pid: u32, timeout: Duration) -> Result<HWND> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Ok(hwnd) = unsafe { FindWindowW(SWITCHER_NAME, SWITCHER_NAME) } {
            let mut window_pid = 0;
            unsafe { GetWindowThreadProcessId(hwnd, Some(&mut window_pid)) };
            if window_pid == pid && unsafe { GetWindowLongPtrW(hwnd, GWL_USERDATA) } != 0 {
                return Ok(hwnd);
            }
        }
        thread::sleep(Duration::from_millis(10));
    }
    bail!("window-switcher did not create its hidden window")
}

fn run_cycle(target: HWND, switcher: HWND, extra_tabs: usize) -> Result<(Duration, Duration)> {
    if !set_foreground_window(target) {
        bail!("failed to focus benchmark window");
    }
    wait_visibility(switcher, false, VISIBILITY_TIMEOUT)?;
    send_key(VK_MENU, false)?;
    let opening_started = Instant::now();
    send_key(VK_TAB, false)?;
    send_key(VK_TAB, true)?;
    wait_visibility(switcher, true, VISIBILITY_TIMEOUT)?;
    let opening = opening_started.elapsed();

    for _ in 0..extra_tabs {
        send_key(VK_TAB, false)?;
        send_key(VK_TAB, true)?;
        thread::sleep(Duration::from_millis(1));
    }
    let closing_started = Instant::now();
    send_key(VK_MENU, true)?;
    wait_visibility(switcher, false, VISIBILITY_TIMEOUT)?;
    Ok((opening, closing_started.elapsed()))
}

fn send_key(key: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY, up: bool) -> Result<()> {
    let scan_code = match key {
        VK_MENU => 0x38,
        VK_TAB => 0x0f,
        _ => bail!("unsupported benchmark key {}", key.0),
    };
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wScan: scan_code,
                dwFlags: KEYEVENTF_SCANCODE
                    | if up {
                        KEYEVENTF_KEYUP
                    } else {
                        Default::default()
                    },
                ..Default::default()
            },
        },
    };
    let sent = unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
    if sent != 1 {
        bail!("SendInput sent {sent} of 1 keyboard events");
    }
    Ok(())
}

fn wait_visibility(hwnd: HWND, visible: bool, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if unsafe { IsWindowVisible(hwnd) }.as_bool() == visible {
            return Ok(());
        }
        std::hint::spin_loop();
        thread::yield_now();
    }
    bail!("timed out waiting for selector visibility={visible}")
}

#[derive(Clone, Copy)]
struct ProcessSnapshot {
    handles: u32,
    gdi: u32,
    user: u32,
    private_bytes: usize,
    working_set: usize,
    cpu_ticks: u64,
}

impl ProcessSnapshot {
    fn capture(process: HANDLE) -> Result<Self> {
        let mut handles = 0;
        unsafe { GetProcessHandleCount(process, &mut handles)? };
        let gdi = unsafe { GetGuiResources(process, GR_GDIOBJECTS) };
        let user = unsafe { GetGuiResources(process, GR_USEROBJECTS) };

        let mut memory = PROCESS_MEMORY_COUNTERS_EX {
            cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
            ..Default::default()
        };
        unsafe {
            GetProcessMemoryInfo(
                process,
                &mut memory as *mut _ as *mut _,
                std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
            )?;
        }

        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user_time = FILETIME::default();
        unsafe {
            GetProcessTimes(
                process,
                &mut creation,
                &mut exit,
                &mut kernel,
                &mut user_time,
            )?;
        }
        Ok(Self {
            handles,
            gdi,
            user,
            private_bytes: memory.PrivateUsage,
            working_set: memory.WorkingSetSize,
            cpu_ticks: filetime_ticks(kernel) + filetime_ticks(user_time),
        })
    }
}

fn print_latency(label: &str, samples: &[Duration]) {
    println!("{label} min: {:.3} ms", millis(samples[0]));
    println!("{label} p50: {:.3} ms", millis(percentile(samples, 50)));
    println!("{label} p95: {:.3} ms", millis(percentile(samples, 95)));
    println!("{label} p99: {:.3} ms", millis(percentile(samples, 99)));
    println!("{label} max: {:.3} ms", millis(*samples.last().unwrap()));
}

fn percentile(samples: &[Duration], percentile: usize) -> Duration {
    let index = ((samples.len() - 1) * percentile / 100).min(samples.len() - 1);
    samples[index]
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn mib(bytes: usize) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

fn signed_mib(bytes: i64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

fn filetime_ticks(value: FILETIME) -> u64 {
    ((value.dwHighDateTime as u64) << 32) | value.dwLowDateTime as u64
}

fn ticks_to_millis(ticks: u64) -> f64 {
    ticks as f64 / 10_000.0
}
