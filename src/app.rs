use crate::config::{edit_config_file, Config};
use crate::foreground::ForegroundWatcher;
use crate::keyboard::{take_pending_app_delta, KeyboardListener};
use crate::painter::GdiAAPainter;
use crate::startup::Startup;
use crate::trayicon::TrayIcon;
use crate::utils::{
    check_error, get_activation_window, get_app_icon, get_foreground_window, get_quick_app_icon,
    get_window_user_data, is_running_as_admin, is_switchable_window, list_all_windows,
    list_windows, set_foreground_window, set_window_user_data, PerfSpan,
};

use anyhow::{anyhow, Result};
use indexmap::IndexSet;
use std::{
    collections::{HashMap, HashSet},
    ptr::NonNull,
    sync::atomic::{AtomicU32, Ordering},
};
use windows::core::{w, PCWSTR};
use windows::Win32::{
    Foundation::{GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM},
    System::{
        Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED},
        LibraryLoader::GetModuleHandleW,
    },
    UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyIcon, DispatchMessageW, GetMessageW,
        GetWindowLongPtrW, KillTimer, LoadCursorW, PostMessageW, PostQuitMessage, RegisterClassW,
        RegisterWindowMessageW, SetTimer, SetWindowLongPtrW, TranslateMessage, CS_HREDRAW,
        CS_VREDRAW, CW_USEDEFAULT, GWL_STYLE, HICON, HTCLIENT, IDC_ARROW, MSG, WINDOW_STYLE,
        WM_COMMAND, WM_ERASEBKGND, WM_LBUTTONUP, WM_NCHITTEST, WM_RBUTTONUP, WM_SETTINGCHANGE,
        WM_THEMECHANGED, WM_TIMER, WNDCLASSW, WS_CAPTION, WS_EX_LAYERED, WS_EX_TOOLWINDOW,
        WS_EX_TOPMOST,
    },
};

pub const NAME: PCWSTR = w!("Window Switcher");
pub const WM_USER_TRAYICON: u32 = 6000;
pub const WM_USER_SWITCH_APPS: u32 = 6010;
pub const WM_USER_SWITCH_APPS_DONE: u32 = 6011;
pub const WM_USER_SWITCH_APPS_CANCEL: u32 = 6012;
pub const WM_USER_SWITCH_WINDOWS: u32 = 6020;
pub const WM_USER_SWITCH_WINDOWS_DONE: u32 = 6021;
pub const WM_USER_ICON_READY: u32 = 6030;
pub const IDM_EXIT: u32 = 1;
pub const IDM_STARTUP: u32 = 2;
pub const IDM_CONFIGURE: u32 = 3;
const TRAYICON_RETRY_TIMER_ID: usize = 1;
const TRAYICON_RETRY_DELAY_MS: u32 = 3_000;

pub fn start(config: &Config) -> Result<()> {
    info!("start config={config:?}");
    App::start(config)
}

/// Listen to this message to recreate the tray icon since the taskbar has been recreated.
static WM_TASKBARCREATED: AtomicU32 = AtomicU32::new(0);

pub struct App {
    hwnd: HWND,
    is_admin: bool,
    trayicon: Option<TrayIcon>,
    trayicon_retry_pending: bool,
    startup: Startup,
    config: Config,
    switch_windows_state: SwitchWindowsState,
    switch_apps_state: Option<SwitchAppsState>,
    window_buffer: Vec<WindowEntry>,
    cached_icons: HashMap<String, HICON>,
    pending_icon_jobs: HashSet<String>,
    painter: GdiAAPainter,
}

impl App {
    pub fn start(config: &Config) -> Result<()> {
        let hwnd = Self::create_window()?;
        let painter = GdiAAPainter::new(hwnd)?;

        let _foreground_watcher = ForegroundWatcher::init(&config.switch_windows_blacklist)?;
        let _keyboard_listener = KeyboardListener::init(hwnd, &config.to_hotkeys())?;

        let trayicon = match config.trayicon {
            true => Some(TrayIcon::create()),
            false => None,
        };

        let is_admin = is_running_as_admin()?;
        debug!("is_admin {is_admin}");

        let startup = Startup::init(is_admin)?;

        let mut app = App {
            hwnd,
            is_admin,
            trayicon,
            trayicon_retry_pending: false,
            startup,
            config: config.clone(),
            switch_windows_state: SwitchWindowsState {
                cache: None,
                modifier_released: true,
            },
            switch_apps_state: None,
            window_buffer: Vec::new(),
            cached_icons: Default::default(),
            pending_icon_jobs: Default::default(),
            painter,
        };

        app.set_trayicon();

        let app_ptr = Box::into_raw(Box::new(app)) as _;
        check_error(|| set_window_user_data(hwnd, app_ptr))
            .map_err(|err| anyhow!("Failed to set window ptr, {err}"))?;

        Self::eventloop()
    }

    fn eventloop() -> Result<()> {
        let mut message = MSG::default();
        loop {
            let ret = unsafe { GetMessageW(&mut message, None, 0, 0) };
            match ret.0 {
                -1 => {
                    unsafe { GetLastError() }.ok()?;
                }
                0 => break,
                _ => unsafe {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                },
            }
        }

        Ok(())
    }

    fn create_window() -> Result<HWND> {
        WM_TASKBARCREATED.store(
            unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) },
            Ordering::Relaxed,
        );

        let hinstance = unsafe { GetModuleHandleW(None) }
            .map_err(|err| anyhow!("Failed to get current module handle, {err}"))?;

        let hcursor = unsafe { LoadCursorW(None, IDC_ARROW) }
            .map_err(|err| anyhow!("Failed to load arrow cursor, {err}"))?;

        let window_class = WNDCLASSW {
            hCursor: hcursor,
            hInstance: HINSTANCE(hinstance.0),
            lpszClassName: NAME,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(App::window_proc),
            ..Default::default()
        };

        let atom = check_error(|| unsafe { RegisterClassW(&window_class) })
            .map_err(|err| anyhow!("Failed to register class, {err}"))?;

        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                PCWSTR(atom as _),
                NAME,
                WINDOW_STYLE(0),
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                None,
                None,
                Some(hinstance.into()),
                None,
            )
        }
        .map_err(|err| anyhow!("Failed to create windows, {err}"))?;

        // hide caption
        let mut style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32;
        style &= !WS_CAPTION.0;
        unsafe { SetWindowLongPtrW(hwnd, GWL_STYLE, style as _) };

        Ok(hwnd)
    }

    fn set_trayicon(&mut self) {
        if let Some(trayicon) = self.trayicon.as_mut() {
            match trayicon.register(self.hwnd) {
                Ok(()) => {
                    info!("trayicon registered");
                    if self.trayicon_retry_pending {
                        unsafe {
                            let _ = KillTimer(Some(self.hwnd), TRAYICON_RETRY_TIMER_ID);
                        }
                        self.trayicon_retry_pending = false;
                    }
                }
                Err(err) => {
                    if !trayicon.exist() && !self.trayicon_retry_pending {
                        error!("{err}, retrying in 3 second");
                        let timer = unsafe {
                            SetTimer(
                                Some(self.hwnd),
                                TRAYICON_RETRY_TIMER_ID,
                                TRAYICON_RETRY_DELAY_MS,
                                None,
                            )
                        };
                        self.trayicon_retry_pending = timer != 0;
                    }
                }
            }
        }
    }

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match Self::handle_message(hwnd, msg, wparam, lparam) {
            Ok(ret) => ret,
            Err(err) => {
                error!("{err}");
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
        }
    }

    fn handle_message(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Result<LRESULT> {
        match msg {
            WM_USER_TRAYICON => {
                with_app(hwnd, |app| {
                    if let Some(trayicon) = app.trayicon.as_mut() {
                        let keycode = lparam.0 as u32;
                        if keycode == WM_LBUTTONUP || keycode == WM_RBUTTONUP {
                            trayicon.show(app.startup.is_enable)?;
                        }
                    }
                    Ok(())
                })?;
                return Ok(LRESULT(0));
            }
            WM_USER_SWITCH_APPS => {
                debug!("message WM_USER_SWITCH_APPS");
                let delta = take_pending_app_delta();
                with_app(hwnd, |app| {
                    app.switch_apps(delta)?;
                    if let Some(state) = &app.switch_apps_state {
                        app.painter.paint(state);
                    }
                    Ok(())
                })?;
            }
            WM_USER_SWITCH_APPS_DONE => {
                debug!("message WM_USER_SWITCH_APPS_DONE");
                with_app(hwnd, |app| {
                    app.do_switch_app();
                    Ok(())
                })?;
            }
            WM_USER_SWITCH_APPS_CANCEL => {
                debug!("message WM_USER_SWITCH_APPS_CANCEL");
                with_app(hwnd, |app| {
                    app.cancel_switch_app();
                    Ok(())
                })?;
            }
            WM_USER_SWITCH_WINDOWS => {
                debug!("message WM_USER_SWITCH_WINDOWS");
                let reverse = lparam.0 == 1;
                with_app(hwnd, |app| {
                    let target = app
                        .switch_apps_state
                        .as_ref()
                        .and_then(|state| state.windows.get(state.index).map(|window| window.hwnd))
                        .unwrap_or_else(get_foreground_window);
                    app.switch_windows(target, reverse)?;
                    app.cancel_switch_app();
                    Ok(())
                })?;
            }
            WM_USER_SWITCH_WINDOWS_DONE => {
                debug!("message WM_USER_SWITCH_WINDOWS_DONE");
                with_app(hwnd, |app| {
                    app.switch_windows_state.modifier_released = true;
                    Ok(())
                })?;
            }
            WM_USER_ICON_READY => {
                if lparam.0 == 0 {
                    return Ok(LRESULT(0));
                }
                let result = unsafe { Box::from_raw(lparam.0 as *mut IconLoadResult) };
                with_app(hwnd, |app| {
                    app.pending_icon_jobs.remove(&result.key);
                    let mut used = false;
                    if let Some(cached_icon) = app.cached_icons.get_mut(&result.key) {
                        unsafe {
                            let _ = DestroyIcon(*cached_icon);
                        }
                        *cached_icon = result.icon;
                        used = true;
                    }

                    let mut repaint = false;
                    if used {
                        if let Some(state) = app.switch_apps_state.as_mut() {
                            for window in &mut state.windows {
                                if window.icon_key == result.key {
                                    window.icon = result.icon;
                                    repaint = true;
                                }
                            }
                        }
                    } else {
                        unsafe {
                            let _ = DestroyIcon(result.icon);
                        }
                    }

                    if repaint {
                        app.painter.invalidate_session();
                        if let Some(state) = &app.switch_apps_state {
                            app.painter.paint(state);
                        }
                    }
                    Ok(())
                })?;
                return Ok(LRESULT(0));
            }
            WM_NCHITTEST => {
                return Ok(LRESULT(HTCLIENT as _));
            }
            WM_LBUTTONUP => {
                with_app(hwnd, |app| {
                    app.click();
                    Ok(())
                })?;
            }
            WM_COMMAND => {
                let value = wparam.0 as u32;
                let kind = ((value >> 16) & 0xffff) as u16;
                let id = value & 0xffff;
                if kind == 0 {
                    match id {
                        IDM_EXIT => {
                            drop_app(hwnd)?;
                            unsafe { PostQuitMessage(0) }
                        }
                        IDM_STARTUP => {
                            with_app(hwnd, |app| app.startup.toggle())?;
                        }
                        IDM_CONFIGURE => {
                            if let Err(err) = edit_config_file() {
                                alert!("{err}");
                            }
                        }
                        _ => {}
                    }
                }
            }
            WM_ERASEBKGND => {
                return Ok(LRESULT(0));
            }
            WM_TIMER if wparam.0 == TRAYICON_RETRY_TIMER_ID => {
                unsafe {
                    let _ = KillTimer(Some(hwnd), TRAYICON_RETRY_TIMER_ID);
                }
                with_app(hwnd, |app| {
                    app.trayicon_retry_pending = false;
                    app.set_trayicon();
                    Ok(())
                })?;
                return Ok(LRESULT(0));
            }
            WM_SETTINGCHANGE | WM_THEMECHANGED => {
                Config::refresh_system_settings();
                with_app(hwnd, |app| {
                    if app.painter.refresh_theme() {
                        if let Some(state) = &app.switch_apps_state {
                            app.painter.paint(state);
                        }
                    }
                    Ok(())
                })?;
            }
            _ if msg == WM_TASKBARCREATED.load(Ordering::Relaxed) => {
                with_app(hwnd, |app| {
                    app.set_trayicon();
                    Ok(())
                })?;
            }
            _ => {}
        }
        Ok(unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) })
    }

    fn switch_windows(&mut self, hwnd: HWND, reverse: bool) -> Result<bool> {
        if !self.switch_windows_state.modifier_released {
            let ignore_minimal = self.config.switch_windows_ignore_minimal;
            let only_current_desktop = self.config.switch_windows_only_current_desktop();
            let mut next_hwnd = None;
            if let Some((_, _, cache_index, cache_windows)) =
                self.switch_windows_state.cache.as_mut()
            {
                let windows_len = cache_windows.len();
                if cache_windows.contains(&(hwnd.0 as isize)) {
                    for _ in 0..windows_len.saturating_sub(1) {
                        *cache_index = if reverse {
                            if *cache_index == 0 {
                                windows_len - 1
                            } else {
                                *cache_index - 1
                            }
                        } else if *cache_index >= windows_len - 1 {
                            0
                        } else {
                            *cache_index + 1
                        };
                        let candidate = HWND(cache_windows[*cache_index] as _);
                        let activation = get_activation_window(candidate);
                        if is_switchable_window(activation, ignore_minimal, only_current_desktop) {
                            next_hwnd = Some(activation);
                            break;
                        }
                    }
                }
            }
            if let Some(hwnd) = next_hwnd {
                set_foreground_window(hwnd);
                return Ok(true);
            }
        }

        let sibling_windows = {
            let _perf = PerfSpan::new("enumerate_same_application");
            list_windows(
                hwnd,
                self.config.switch_windows_ignore_minimal,
                self.config.switch_windows_only_current_desktop(),
                self.is_admin,
            )?
        };
        let Some((module_path, windows)) = sibling_windows else {
            return Ok(false);
        };
        debug!(
            "switch windows: hwnd:{hwnd:?} reverse:{reverse} state:{:?}",
            self.switch_windows_state
        );
        let windows_len = windows.len();
        if windows_len <= 1 {
            return Ok(false);
        }
        let current_id = windows[0].0;
        let mut index = 1;
        let mut state_id = current_id;
        let mut state_windows = vec![];
        if windows_len > 2 {
            if let Some((cache_module_path, cache_id, cache_index, cache_windows)) =
                self.switch_windows_state.cache.as_ref()
            {
                if cache_module_path == &module_path {
                    if self.switch_windows_state.modifier_released {
                        if *cache_id != current_id {
                            if let Some((i, _)) =
                                windows.iter().enumerate().find(|(_, (v, _))| v == cache_id)
                            {
                                index = i;
                            }
                        }
                    } else {
                        state_id = *cache_id;
                        let mut windows_set: IndexSet<isize> =
                            windows.iter().map(|(v, _)| v.0 as _).collect();
                        for id in cache_windows {
                            if windows_set.contains(id) {
                                state_windows.push(*id);
                                windows_set.swap_remove(id);
                            }
                        }
                        state_windows.extend(windows_set);
                        index = if reverse {
                            if *cache_index == 0 {
                                windows_len - 1
                            } else {
                                cache_index - 1
                            }
                        } else if *cache_index >= windows_len - 1 {
                            0
                        } else {
                            cache_index + 1
                        };
                    }
                }
            }
        }
        if state_windows.is_empty() {
            state_windows = windows.iter().map(|(v, _)| v.0 as _).collect();
        }
        let hwnd = get_activation_window(HWND(state_windows[index] as _));
        self.switch_windows_state = SwitchWindowsState {
            cache: Some((module_path, state_id, index, state_windows)),
            modifier_released: false,
        };
        set_foreground_window(hwnd);

        Ok(true)
    }

    fn switch_apps(&mut self, delta: isize) -> Result<()> {
        debug!(
            "switch apps: delta:{delta}, state:{:?}",
            self.switch_apps_state
        );
        if delta == 0 {
            return Ok(());
        }
        if let Some(state) = self.switch_apps_state.as_mut() {
            state.index = advance_index(state.index, state.windows.len(), delta);
            debug!("switch apps: new index:{}", state.index);
            return Ok(());
        }
        let windows = {
            let _perf = PerfSpan::new("enumerate_all_windows");
            list_all_windows(
                self.config.switch_apps_ignore_minimal,
                self.config.switch_apps_only_current_desktop(),
                self.is_admin,
            )?
        };
        let active_modules: HashSet<&str> = windows
            .iter()
            .map(|(module_path, _, _)| module_path.as_str())
            .collect();
        self.cached_icons.retain(|module_path, icon| {
            if active_modules.contains(module_path.as_str()) {
                true
            } else {
                unsafe {
                    let _ = DestroyIcon(*icon);
                }
                false
            }
        });
        drop(active_modules);
        let mut entries = std::mem::take(&mut self.window_buffer);
        entries.clear();
        entries.reserve(windows.len());
        let mut icon_jobs = Vec::new();
        for (module_path, hwnd, title) in windows {
            let icon = if let Some(icon) = self.cached_icons.get(&module_path) {
                *icon
            } else {
                let icon = get_quick_app_icon(hwnd);
                self.cached_icons.insert(module_path.clone(), icon);
                if self.pending_icon_jobs.insert(module_path.clone()) {
                    icon_jobs.push((module_path.clone(), hwnd.0 as isize));
                }
                icon
            };
            entries.push(WindowEntry {
                icon,
                hwnd,
                title,
                icon_key: module_path,
            });
        }
        let num_apps = entries.len() as i32;
        if num_apps == 0 {
            self.window_buffer = entries;
            return Ok(());
        }

        let index = advance_index(0, entries.len(), delta);

        let state = SwitchAppsState {
            windows: entries,
            index,
        };
        self.switch_apps_state = Some(state);
        self.start_icon_jobs(icon_jobs);
        debug!("switch apps, new state:{:?}", self.switch_apps_state);
        Ok(())
    }

    fn start_icon_jobs(&mut self, jobs: Vec<(String, isize)>) {
        if jobs.is_empty() {
            return;
        }

        let pending_keys: Vec<String> = jobs.iter().map(|(key, _)| key.clone()).collect();
        let overrides = self.config.switch_apps_override_icons.clone();
        let target = self.hwnd.0 as isize;
        let spawn_result = std::thread::Builder::new()
            .name("window-switcher-icons".into())
            .stack_size(256 * 1024)
            .spawn(move || {
                let com_initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
                for (key, raw_hwnd) in jobs {
                    let _perf = PerfSpan::new("resolve_icon");
                    let hwnd = HWND(raw_hwnd as _);
                    let icon = get_app_icon(&overrides, &key, hwnd);
                    let result = Box::new(IconLoadResult { key, icon });
                    let result_ptr = Box::into_raw(result);
                    if unsafe {
                        PostMessageW(
                            Some(HWND(target as _)),
                            WM_USER_ICON_READY,
                            WPARAM(0),
                            LPARAM(result_ptr as isize),
                        )
                    }
                    .is_err()
                    {
                        let result = unsafe { Box::from_raw(result_ptr) };
                        unsafe {
                            let _ = DestroyIcon(result.icon);
                        }
                    }
                }
                if com_initialized {
                    unsafe { CoUninitialize() };
                }
            });

        if let Err(error) = spawn_result {
            for key in pending_keys {
                self.pending_icon_jobs.remove(&key);
            }
            error!("failed to start icon loader: {error}");
        }
    }

    fn click(&mut self) {
        if let Some(state) = self.switch_apps_state.as_mut() {
            if let Some(i) = self.painter.find_clicked_app_index(state) {
                state.index = i;
                self.do_switch_app();
            }
        }
    }

    fn do_switch_app(&mut self) {
        if let Some(mut state) = self.switch_apps_state.take() {
            let ignore_minimal = self.config.switch_apps_ignore_minimal;
            let only_current_desktop = self.config.switch_apps_only_current_desktop();
            for offset in 0..state.windows.len() {
                let index = (state.index + offset) % state.windows.len();
                let hwnd = get_activation_window(state.windows[index].hwnd);
                if is_switchable_window(hwnd, ignore_minimal, only_current_desktop)
                    && set_foreground_window(hwnd)
                {
                    break;
                }
            }
            self.painter.unpaint();
            state.windows.clear();
            self.window_buffer = state.windows;
        }
    }

    fn cancel_switch_app(&mut self) {
        if let Some(mut state) = self.switch_apps_state.take() {
            self.painter.unpaint();
            state.windows.clear();
            self.window_buffer = state.windows;
        }
    }
}

fn advance_index(index: usize, len: usize, delta: isize) -> usize {
    debug_assert!(len > 0);
    (index as isize + delta).rem_euclid(len as isize) as usize
}

impl Drop for App {
    fn drop(&mut self) {
        if self.trayicon_retry_pending {
            unsafe {
                let _ = KillTimer(Some(self.hwnd), TRAYICON_RETRY_TIMER_ID);
            }
        }
        for (_, icon) in self.cached_icons.drain() {
            unsafe {
                let _ = DestroyIcon(icon);
            }
        }
    }
}

fn app_ptr(hwnd: HWND) -> Result<NonNull<App>> {
    let ptr = check_error(|| get_window_user_data(hwnd))
        .map_err(|err| anyhow!("Failed to get window ptr, {err}"))?;
    NonNull::new(ptr as *mut App).ok_or_else(|| anyhow!("Window app pointer is null"))
}

fn with_app<T>(hwnd: HWND, callback: impl FnOnce(&mut App) -> Result<T>) -> Result<T> {
    let mut ptr = app_ptr(hwnd)?;
    // Window messages are dispatched serially on the thread that owns `App`.
    // The mutable reference cannot escape this callback.
    callback(unsafe { ptr.as_mut() })
}

fn drop_app(hwnd: HWND) -> Result<()> {
    let ptr = app_ptr(hwnd)?;
    set_window_user_data(hwnd, 0);
    unsafe { drop(Box::from_raw(ptr.as_ptr())) };
    Ok(())
}

#[derive(Debug)]
struct SwitchWindowsState {
    cache: Option<(String, HWND, usize, Vec<isize>)>,
    modifier_released: bool,
}

#[derive(Debug)]
pub struct SwitchAppsState {
    pub windows: Vec<WindowEntry>,
    pub index: usize,
}

#[derive(Debug)]
pub struct WindowEntry {
    pub icon: HICON,
    pub hwnd: HWND,
    pub title: String,
    pub icon_key: String,
}

struct IconLoadResult {
    key: String,
    icon: HICON,
}

#[cfg(test)]
mod tests {
    use super::advance_index;

    #[test]
    fn advance_index_wraps_in_both_directions() {
        assert_eq!(advance_index(0, 5, 1), 1);
        assert_eq!(advance_index(0, 5, -1), 4);
        assert_eq!(advance_index(4, 5, 2), 1);
        assert_eq!(advance_index(1, 5, -3), 3);
        assert_eq!(advance_index(0, 1, 100), 0);
    }
}
