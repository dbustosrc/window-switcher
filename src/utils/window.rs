use crate::utils::{get_process_elevation_info, HandleWrapper};

use anyhow::{anyhow, Result};
use parking_lot::Mutex;
use std::{
    collections::{HashMap, HashSet},
    ffi::c_void,
    mem::size_of,
    path::PathBuf,
    sync::LazyLock,
    time::Instant,
};
use windows::core::{BOOL, PCWSTR, PWSTR};
use windows::Win32::{
    Foundation::{
        ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS, FILETIME, HWND, LPARAM, MAX_PATH, POINT, RECT,
        WAIT_TIMEOUT,
    },
    Graphics::{
        Dwm::{
            DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DWM_CLOAKED_SHELL,
        },
        Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST},
    },
    Storage::{
        EnhancedStorage::PKEY_AppUserModel_ID,
        Packaging::Appx::{GetPackagePathByFullName, GetPackagesByPackageFamily},
    },
    System::{
        LibraryLoader::GetModuleFileNameW,
        Threading::{
            GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, WaitForSingleObject,
            PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
        },
    },
    UI::{
        HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
        Input::KeyboardAndMouse::{IsWindowEnabled, SendInput, INPUT, INPUT_MOUSE},
        Shell::PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow},
        WindowsAndMessaging::{
            EnumWindows, GetAncestor, GetCursorPos, GetForegroundWindow, GetWindow,
            GetWindowLongPtrW, GetWindowPlacement, GetWindowRect, GetWindowTextW,
            GetWindowThreadProcessId, IsIconic, IsWindow, SetForegroundWindow, ShowWindow,
            GA_ROOTOWNER, GWL_EXSTYLE, GWL_STYLE, GWL_USERDATA, GW_ENABLEDPOPUP, GW_OWNER,
            SW_RESTORE, WINDOWPLACEMENT, WS_EX_TOOLWINDOW, WS_ICONIC, WS_VISIBLE,
        },
    },
};

const PROCESS_CACHE_LIMIT: usize = 128;
const AUMID_CACHE_LIMIT: usize = 256;

#[derive(Clone)]
struct ProcessMetadata {
    module_path: Option<String>,
    elevated: Option<bool>,
    creation_time: u64,
}

struct ProcessCacheEntry {
    process: HandleWrapper,
    metadata: ProcessMetadata,
    last_used: Instant,
}

type WindowIdentity = (isize, u32, u64);
pub type WindowList = Vec<(HWND, String)>;
type OwnedWindows = HashMap<isize, HWND>;

struct AumidCacheEntry {
    value: Option<String>,
    last_used: Instant,
}

static PROCESS_CACHE: LazyLock<Mutex<HashMap<u32, ProcessCacheEntry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static AUMID_CACHE: LazyLock<Mutex<HashMap<WindowIdentity, AumidCacheEntry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn get_window_state(hwnd: HWND) -> (bool, bool, bool) {
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32;
    let exstyle = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;

    let is_visible = style & WS_VISIBLE.0 != 0;
    let is_iconic = style & WS_ICONIC.0 != 0;
    let is_tool = exstyle & WS_EX_TOOLWINDOW.0 != 0;
    (is_visible, is_iconic, is_tool)
}

pub fn is_iconic_window(hwnd: HWND) -> bool {
    unsafe { IsIconic(hwnd) }.as_bool()
}

pub fn get_window_cloak_type(hwnd: HWND) -> u32 {
    let mut cloak_type = 0u32;
    let _ = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloak_type as *mut u32 as *mut c_void,
            size_of::<u32>() as u32,
        )
    };
    cloak_type
}

fn is_cloaked_window(hwnd: HWND, only_current_desktop: bool) -> bool {
    let cloak_type = get_window_cloak_type(hwnd);

    if only_current_desktop {
        // Any kind of cloaking counts against a window
        cloak_type != 0
    } else {
        // Windows from other desktops will be cloaked as SHELL, so we treat them
        // as if they are uncloaked. All other cloak types count against the window
        cloak_type | DWM_CLOAKED_SHELL != DWM_CLOAKED_SHELL
    }
}

pub fn is_small_window(hwnd: HWND) -> bool {
    is_small_window_with_state(hwnd, is_iconic_window(hwnd))
}

fn is_small_window_with_state(hwnd: HWND, is_iconic: bool) -> bool {
    if is_iconic {
        return false;
    }
    let (width, height) = get_window_size_with_state(hwnd, false);
    width < 120 || height < 90
}

pub fn is_switchable_window(hwnd: HWND, ignore_minimal: bool, only_current_desktop: bool) -> bool {
    let (is_visible, is_iconic, is_tool) = get_window_state(hwnd);
    if !is_visible
        || (ignore_minimal && is_iconic)
        || is_tool
        || is_cloaked_window(hwnd, only_current_desktop)
        || is_small_window_with_state(hwnd, is_iconic)
    {
        return false;
    }

    let title = get_window_title(hwnd);
    !title.is_empty() && title != "Windows Input Experience"
}

pub fn get_monitor_rect_and_dpi() -> (RECT, u32) {
    unsafe {
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..MONITORINFO::default()
        };
        let mut cursor = POINT::default();
        let _ = GetCursorPos(&mut cursor);

        let hmonitor = MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST);
        let _ = GetMonitorInfoW(hmonitor, &mut mi);
        let mut dpi_x = 96;
        let mut dpi_y = 96;
        let _ = GetDpiForMonitor(hmonitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
        (mi.rcMonitor, dpi_x.max(96))
    }
}

pub fn get_window_size(hwnd: HWND) -> (i32, i32) {
    get_window_size_with_state(hwnd, is_iconic_window(hwnd))
}

fn get_window_size_with_state(hwnd: HWND, is_iconic: bool) -> (i32, i32) {
    if !is_iconic {
        if let Some(size) = get_visual_window_size(hwnd) {
            return size;
        }
    }

    let mut placement = WINDOWPLACEMENT::default();
    if unsafe { GetWindowPlacement(hwnd, &mut placement) }.is_ok() {
        if let Some(size) = rect_size(placement.rcNormalPosition) {
            return size;
        }
    }

    get_visual_window_size(hwnd).unwrap_or_default()
}

fn get_visual_window_size(hwnd: HWND) -> Option<(i32, i32)> {
    let mut rect = RECT::default();
    if unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut rect as *mut RECT as *mut c_void,
            size_of::<RECT>() as u32,
        )
    }
    .is_ok()
    {
        if let Some(size) = rect_size(rect) {
            return Some(size);
        }
    }

    let mut rect = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut rect) }.is_ok() {
        return rect_size(rect);
    }
    None
}

fn rect_size(rect: RECT) -> Option<(i32, i32)> {
    let width = rect.right.checked_sub(rect.left)?;
    let height = rect.bottom.checked_sub(rect.top)?;
    (width > 0 && height > 0).then_some((width, height))
}

pub fn get_exe_folder() -> Result<PathBuf> {
    let path =
        std::env::current_exe().map_err(|err| anyhow!("Failed to get binary path, {err}"))?;
    path.parent()
        .ok_or_else(|| anyhow!("Failed to get binary folder"))
        .map(|v| v.to_path_buf())
}

pub fn get_exe_path() -> Vec<u16> {
    let mut path = vec![0u16; MAX_PATH as _];
    let size = unsafe { GetModuleFileNameW(None, &mut path) } as usize;
    path[..size].to_vec()
}

pub fn get_window_pid(hwnd: HWND) -> u32 {
    let mut pid: u32 = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid as *mut u32)) };
    pid
}

pub fn get_module_path(pid: u32) -> Option<String> {
    get_process_metadata(pid).module_path
}

fn get_module_path_from_handle(handle: windows::Win32::Foundation::HANDLE) -> Option<String> {
    let mut len: u32 = MAX_PATH;
    let mut name = vec![0u16; len as usize];
    let ret = unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(name.as_mut_ptr()),
            &mut len,
        )
    };
    if ret.is_err() || len == 0 {
        return None;
    }
    unsafe { name.set_len(len as usize) };
    let module_path = String::from_utf16_lossy(&name);
    if module_path.is_empty() {
        return None;
    }
    Some(module_path)
}

fn get_process_module_path(pid: u32, is_admin: bool) -> Option<String> {
    let metadata = get_process_metadata(pid);
    if !is_admin && metadata.elevated == Some(true) {
        return None;
    }
    metadata.module_path
}

fn get_process_metadata(pid: u32) -> ProcessMetadata {
    if pid == 0 {
        return ProcessMetadata {
            module_path: None,
            elevated: None,
            creation_time: 0,
        };
    }

    {
        let mut cache = PROCESS_CACHE.lock();
        if let Some(entry) = cache.get_mut(&pid) {
            if unsafe { WaitForSingleObject(entry.process.get_handle(), 0) } == WAIT_TIMEOUT {
                entry.last_used = Instant::now();
                return entry.metadata.clone();
            }
            cache.remove(&pid);
        }
    }

    let Some(process_handle) = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            false,
            pid,
        )
    }
    .ok() else {
        return ProcessMetadata {
            module_path: None,
            elevated: None,
            creation_time: 0,
        };
    };
    let process = HandleWrapper::new(process_handle);
    let metadata = ProcessMetadata {
        module_path: get_module_path_from_handle(process.get_handle()),
        elevated: get_process_elevation_info(process.get_handle()).ok(),
        creation_time: get_process_creation_time(process.get_handle()),
    };

    let mut cache = PROCESS_CACHE.lock();
    cache.retain(
        |_, entry| unsafe { WaitForSingleObject(entry.process.get_handle(), 0) } == WAIT_TIMEOUT,
    );
    if cache.len() >= PROCESS_CACHE_LIMIT {
        if let Some(oldest_pid) = cache
            .iter()
            .min_by_key(|(_, entry)| (entry.last_used, entry.metadata.creation_time))
            .map(|(pid, _)| *pid)
        {
            cache.remove(&oldest_pid);
        }
    }
    cache.insert(
        pid,
        ProcessCacheEntry {
            process,
            metadata: metadata.clone(),
            last_used: Instant::now(),
        },
    );
    metadata
}

fn get_process_creation_time(process: windows::Win32::Foundation::HANDLE) -> u64 {
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    if unsafe { GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user) }
        .is_err()
    {
        return 0;
    }
    ((creation.dwHighDateTime as u64) << 32) | creation.dwLowDateTime as u64
}

fn is_chrome_browser(module_path: &str) -> bool {
    PathBuf::from(module_path)
        .file_name()
        .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("chrome.exe"))
}

fn is_edge_browser(module_path: &str) -> bool {
    PathBuf::from(module_path)
        .file_name()
        .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("msedge.exe"))
}

fn get_aumid(hwnd: HWND) -> Option<String> {
    let pid = get_window_pid(hwnd);
    let creation_time = get_process_metadata(pid).creation_time;
    let key = (hwnd.0 as isize, pid, creation_time);
    {
        let mut cache = AUMID_CACHE.lock();
        if let Some(entry) = cache.get_mut(&key) {
            entry.last_used = Instant::now();
            return entry.value.clone();
        }
    }

    let aumid = unsafe {
        SHGetPropertyStoreForWindow(hwnd)
            .ok()
            .and_then(|store: IPropertyStore| store.GetValue(&PKEY_AppUserModel_ID).ok())
            .map(|propvar| propvar.to_string())
    };

    let mut cache = AUMID_CACHE.lock();
    if cache.len() >= AUMID_CACHE_LIMIT {
        if let Some(oldest_hwnd) = cache
            .iter()
            .min_by_key(|(_, entry)| entry.last_used)
            .map(|(hwnd, _)| *hwnd)
        {
            cache.remove(&oldest_hwnd);
        }
    }
    cache.insert(
        key,
        AumidCacheEntry {
            value: aumid.clone(),
            last_used: Instant::now(),
        },
    );
    aumid
}

fn get_edge_aumid_info(hwnd: HWND) -> (Option<String>, Option<String>) {
    let aumid = match get_aumid(hwnd) {
        Some(v) => v,
        None => return (None, None),
    };

    if let Some(pkg) = aumid.strip_suffix("!App") {
        if !pkg.is_empty() {
            return (None, Some(pkg.to_string()));
        }
        return (None, None);
    }

    if aumid == "MSEdge" || aumid.is_empty() {
        return (None, None);
    }
    if let Some(profile) = aumid.strip_prefix("MSEdge.UserData.") {
        if !profile.is_empty() {
            return (Some(profile.to_string()), None);
        }
    }
    (None, None)
}

fn get_chrome_aumid_info(hwnd: HWND) -> (Option<String>, Option<String>) {
    let aumid = match get_aumid(hwnd) {
        Some(v) => v,
        None => return (None, None),
    };

    let crx_prefix = "_crx_";
    if let Some(idx) = aumid.find(crx_prefix) {
        let after_crx = &aumid[idx + crx_prefix.len()..];
        let (app_id, profile) = if let Some(dot_idx) = after_crx.find(".UserData.") {
            (
                &after_crx[..dot_idx],
                &after_crx[dot_idx + ".UserData.".len()..],
            )
        } else {
            (after_crx, "Default")
        };
        if !app_id.is_empty() && !profile.is_empty() {
            let profile = if profile == "Default" {
                None
            } else {
                Some(profile.to_string())
            };
            return (profile, Some(app_id.to_string()));
        }
        return (None, None);
    }

    if aumid == "Chrome" || aumid.is_empty() {
        return (None, None);
    }
    if let Some(profile) = aumid.strip_prefix("Chrome.UserData.") {
        if !profile.is_empty() {
            return (Some(profile.to_string()), None);
        }
    }
    (None, None)
}

fn is_app_id_match(full: &str, truncated: &str) -> bool {
    if full.eq_ignore_ascii_case(truncated) {
        return true;
    }
    let mut chars = full.chars();
    for tc in truncated.chars() {
        loop {
            match chars.next() {
                Some(fc) if fc == tc => break,
                Some(_) => continue,
                None => return false,
            }
        }
    }
    true
}

pub(crate) fn pwa_map_profile_dir(aumid_profile: &str) -> String {
    if let Some(num) = aumid_profile.strip_prefix("Profile") {
        if num.chars().all(|c| c.is_ascii_digit()) {
            return format!("Profile {}", num);
        }
    }
    aumid_profile.to_string()
}

pub(crate) fn pwa_find_lnk_path(
    user_data_dir: &str,
    profile: &str,
    app_id: &str,
) -> Option<PathBuf> {
    let profile_dir = pwa_map_profile_dir(profile);
    let web_apps_dir = PathBuf::from(user_data_dir)
        .join(&profile_dir)
        .join("Web Applications");
    if !web_apps_dir.is_dir() {
        return None;
    }
    for entry in std::fs::read_dir(&web_apps_dir).ok()?.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let dir_name = entry.file_name().to_string_lossy().to_string();
        let dir_app_id = dir_name.strip_prefix("_crx_").unwrap_or(&dir_name);
        if is_app_id_match(dir_app_id, app_id) {
            for lnk_entry in std::fs::read_dir(entry.path()).ok()?.flatten() {
                let path = lnk_entry.path();
                if path.extension().map(|e| e.to_string_lossy().to_lowercase())
                    == Some("lnk".into())
                {
                    return Some(path);
                }
            }
            return None;
        }
    }
    None
}

pub(crate) fn find_appx_pkg_dir(package_family_name: &str) -> Option<String> {
    unsafe {
        let pfn: Vec<u16> = package_family_name.encode_utf16().chain(Some(0)).collect();

        let mut count = 0u32;
        let mut buffer_len = 0u32;

        let rc = GetPackagesByPackageFamily(
            PCWSTR(pfn.as_ptr()),
            &mut count,
            None,
            &mut buffer_len,
            None,
        );

        if rc.0 != ERROR_INSUFFICIENT_BUFFER.0 {
            return None;
        }

        let mut names = vec![PWSTR::null(); count as usize];
        let mut buffer = vec![0u16; buffer_len as usize];

        if GetPackagesByPackageFamily(
            PCWSTR(pfn.as_ptr()),
            &mut count,
            Some(names.as_mut_ptr()),
            &mut buffer_len,
            Some(PWSTR(buffer.as_mut_ptr())),
        )
        .0 != ERROR_SUCCESS.0
        {
            return None;
        }

        for name in names {
            let full_name = name.to_string().ok()?;
            let full_w: Vec<u16> = full_name.encode_utf16().chain(Some(0)).collect();

            let mut path_len = 0u32;

            let _ = GetPackagePathByFullName(PCWSTR(full_w.as_ptr()), &mut path_len, None);

            let mut path_buf = vec![0u16; path_len as usize];

            if GetPackagePathByFullName(
                PCWSTR(full_w.as_ptr()),
                &mut path_len,
                Some(PWSTR(path_buf.as_mut_ptr())),
            )
            .0 != ERROR_SUCCESS.0
            {
                continue;
            }

            let path = String::from_utf16_lossy(&path_buf[..(path_len as usize - 1)]);
            return Some(path);
        }
    }
    None
}

pub(crate) fn get_default_user_data_dir(module_path: &str) -> Option<String> {
    let local_app_data = std::env::var("LOCALAPPDATA").ok()?;
    let browser_path = if module_path.to_lowercase().contains("chrome.exe") {
        r"Google\Chrome\User Data"
    } else if module_path.to_lowercase().contains("msedge.exe") {
        r"Microsoft\Edge\User Data"
    } else {
        return None;
    };
    Some(
        PathBuf::from(local_app_data)
            .join(browser_path)
            .to_string_lossy()
            .to_string(),
    )
}

pub fn get_window_exe(hwnd: HWND) -> Option<String> {
    let pid = get_window_pid(hwnd);
    if pid == 0 {
        return None;
    }
    let module_path = get_module_path(pid)?;
    module_path.split('\\').map(|v| v.to_string()).next_back()
}

pub fn set_foreground_window(hwnd: HWND) -> bool {
    // ref https://github.com/microsoft/PowerToys/blob/4cb72ee126caf1f720c507f6a1dbe658cd515366/src/modules/fancyzones/FancyZonesLib/WindowUtils.cpp#L191
    unsafe {
        if !IsWindow(Some(hwnd)).as_bool() {
            return false;
        }
        if is_iconic_window(hwnd) {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }

        let input = INPUT {
            r#type: INPUT_MOUSE,
            ..Default::default()
        };

        SendInput(&[input], std::mem::size_of::<INPUT>() as i32);

        SetForegroundWindow(hwnd).as_bool()
    }
}

pub fn get_foreground_window() -> HWND {
    unsafe { GetForegroundWindow() }
}

pub fn get_window_title(hwnd: HWND) -> String {
    let mut buf = [0u16; 512];
    let len = unsafe { GetWindowTextW(hwnd, buf.as_mut_slice()) };
    if len == 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..len as usize])
}

fn get_selector_title(hwnd: HWND, modal: Option<HWND>) -> String {
    if let Some(modal) = modal {
        let modal_title = get_window_title(modal);
        if !modal_title.is_empty() {
            return modal_title;
        }
    }
    get_window_title(hwnd)
}

pub fn get_owner_window(hwnd: HWND) -> HWND {
    unsafe { GetWindow(hwnd, GW_OWNER) }.unwrap_or_default()
}

fn get_root_owner_window(hwnd: HWND) -> HWND {
    let root = unsafe { GetAncestor(hwnd, GA_ROOTOWNER) };
    if root.is_invalid() {
        hwnd
    } else {
        root
    }
}

fn is_modal_popup_relation(
    root: HWND,
    popup: HWND,
    root_enabled: bool,
    popup_visible: bool,
    popup_enabled: bool,
    same_process: bool,
    popup_root: HWND,
) -> bool {
    !root.is_invalid()
        && !popup.is_invalid()
        && root.0 != popup.0
        && !root_enabled
        && popup_visible
        && popup_enabled
        && same_process
        && popup_root.0 == root.0
}

/// Returns the enabled popup that currently blocks the root window, if any.
///
/// `GW_ENABLEDPOPUP` is combined with the disabled-root check and root-owner/
/// process validation. This collapses modal dialogs without grouping unrelated
/// top-level windows that happen to belong to the same application.
fn get_blocking_modal_popup_for_root(root: HWND) -> Option<HWND> {
    if root.is_invalid() || unsafe { IsWindowEnabled(root) }.as_bool() {
        return None;
    }

    let popup = unsafe { GetWindow(root, GW_ENABLEDPOPUP) }.unwrap_or_default();
    if popup.is_invalid() || popup.0 == root.0 || !unsafe { IsWindow(Some(popup)) }.as_bool() {
        return None;
    }

    let (popup_visible, _, _) = get_window_state(popup);
    let popup_enabled = unsafe { IsWindowEnabled(popup) }.as_bool();
    let root_pid = get_window_pid(root);
    let same_process = root_pid != 0 && root_pid == get_window_pid(popup);
    let popup_root = get_root_owner_window(popup);

    is_modal_popup_relation(
        root,
        popup,
        false,
        popup_visible,
        popup_enabled,
        same_process,
        popup_root,
    )
    .then_some(popup)
}

fn get_blocking_modal_popup(hwnd: HWND) -> Option<HWND> {
    get_blocking_modal_popup_for_root(get_root_owner_window(hwnd))
}

/// Resolves a selector entry to the window that can actually receive input.
/// A modal may have opened or closed since the selector list was built, so this
/// is evaluated again immediately before activating the selection.
pub fn get_activation_window(hwnd: HWND) -> HWND {
    get_blocking_modal_popup(hwnd).unwrap_or(hwnd)
}

#[cfg(target_arch = "x86")]
pub fn get_window_user_data(hwnd: HWND) -> i32 {
    unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowLongW(hwnd, GWL_USERDATA) }
}

#[cfg(not(target_arch = "x86"))]
pub fn get_window_user_data(hwnd: HWND) -> isize {
    unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(hwnd, GWL_USERDATA) }
}

#[cfg(target_arch = "x86")]
pub fn set_window_user_data(hwnd: HWND, ptr: i32) -> i32 {
    unsafe { windows::Win32::UI::WindowsAndMessaging::SetWindowLongW(hwnd, GWL_USERDATA, ptr) }
}

#[cfg(not(target_arch = "x86"))]
pub fn set_window_user_data(hwnd: HWND, ptr: isize) -> isize {
    unsafe { windows::Win32::UI::WindowsAndMessaging::SetWindowLongPtrW(hwnd, GWL_USERDATA, ptr) }
}

/// Lists switchable windows that belong to the same logical application as `target`.
pub fn list_windows(
    target: HWND,
    ignore_minimal: bool,
    only_current_desktop: bool,
    is_admin: bool,
) -> Result<Option<(String, WindowList)>> {
    let (windows, owned_windows) =
        enumerate_window_candidates(ignore_minimal, only_current_desktop)?;
    let mut module_paths = HashMap::new();
    let Some(target_module) =
        resolve_base_module_path(target, &owned_windows, &mut module_paths, is_admin)
    else {
        return Ok(None);
    };
    let target_key = build_module_key(target_module.clone(), target);
    let mut result = Vec::new();
    for (hwnd, title) in windows {
        let Some(module_path) =
            resolve_base_module_path(hwnd, &owned_windows, &mut module_paths, is_admin)
        else {
            continue;
        };
        if module_path != target_module {
            continue;
        }
        if build_module_key(module_path, hwnd) == target_key {
            result.push((hwnd, title));
        }
    }
    debug!("list windows for {target:?}: {result:?}");
    Ok(Some((target_key, result)))
}

/// Lists every available window in the global window order without grouping
/// windows that belong to the same application.
pub fn list_all_windows(
    ignore_minimal: bool,
    only_current_desktop: bool,
    is_admin: bool,
) -> Result<Vec<(String, HWND, String)>> {
    let (windows, owned_windows) =
        enumerate_window_candidates(ignore_minimal, only_current_desktop)?;
    let mut result = Vec::with_capacity(windows.len());
    let mut module_paths = HashMap::new();
    for (hwnd, title) in windows {
        let Some(module_path) =
            resolve_base_module_path(hwnd, &owned_windows, &mut module_paths, is_admin)
        else {
            continue;
        };
        let key = build_module_key(module_path, hwnd);
        result.push((key, hwnd, title));
    }
    debug!("list all windows {result:?}");
    Ok(result)
}

fn enumerate_window_candidates(
    ignore_minimal: bool,
    only_current_desktop: bool,
) -> Result<(WindowList, OwnedWindows)> {
    let mut hwnds: Vec<HWND> = Default::default();
    unsafe { EnumWindows(Some(enum_window), LPARAM(&mut hwnds as *mut _ as isize)) }
        .map_err(|e| anyhow!("Fail to get windows {}", e))?;
    let active_hwnds: HashSet<isize> = hwnds.iter().map(|hwnd| hwnd.0 as isize).collect();
    AUMID_CACHE
        .lock()
        .retain(|(hwnd, _, _), _| active_hwnds.contains(hwnd));
    let mut valid_hwnds = Vec::with_capacity(hwnds.len());
    let mut seen_candidates = HashSet::with_capacity(hwnds.len());
    let mut modal_popups = HashMap::new();
    let mut owned_windows = OwnedWindows::new();
    for hwnd in hwnds {
        let (is_visible, is_iconic, is_tool) = get_window_state(hwnd);
        let ok = is_visible
            && (if ignore_minimal { !is_iconic } else { true })
            && !is_tool
            && !is_cloaked_window(hwnd, only_current_desktop)
            && !is_small_window_with_state(hwnd, is_iconic);
        if ok {
            let root = get_root_owner_window(hwnd);
            let blocking_modal = *modal_popups
                .entry(root.0 as isize)
                .or_insert_with(|| get_blocking_modal_popup_for_root(root));
            let (candidate, title_modal) = if let Some(modal) = blocking_modal {
                if is_switchable_window(root, ignore_minimal, only_current_desktop) {
                    (root, Some(modal))
                } else {
                    (hwnd, None)
                }
            } else {
                (hwnd, None)
            };
            if seen_candidates.insert(candidate.0 as isize) {
                let title = get_selector_title(candidate, title_modal);
                if !title.is_empty() && title != "Windows Input Experience" {
                    valid_hwnds.push((candidate, title));
                }
            }
        }
        let owner = get_owner_window(hwnd);
        if !owner.is_invalid() {
            owned_windows.entry(owner.0 as isize).or_insert(hwnd);
        }
    }
    Ok((valid_hwnds, owned_windows))
}

fn resolve_base_module_path(
    hwnd: HWND,
    owned_windows: &HashMap<isize, HWND>,
    module_paths: &mut HashMap<u32, Option<String>>,
    is_admin: bool,
) -> Option<String> {
    let mut pid = get_window_pid(hwnd);
    let mut module_path = module_paths
        .entry(pid)
        .or_insert_with(|| get_process_module_path(pid, is_admin))
        .clone()
        .unwrap_or_default();
    if !is_valid_module_path(&module_path) {
        if let Some(owned_window) = owned_windows.get(&(hwnd.0 as isize)) {
            pid = get_window_pid(*owned_window);
            module_path = module_paths
                .entry(pid)
                .or_insert_with(|| get_process_module_path(pid, is_admin))
                .clone()
                .unwrap_or_default();
        }
    }
    is_valid_module_path(&module_path).then_some(module_path)
}

fn build_module_key(module_path: String, hwnd: HWND) -> String {
    if is_chrome_browser(&module_path) {
        match get_chrome_aumid_info(hwnd) {
            (Some(profile), Some(app_id)) => format!("{module_path}::{profile}::{app_id}"),
            (Some(profile), None) => format!("{module_path}::{profile}"),
            (None, Some(app_id)) => format!("{module_path}::Default::{app_id}"),
            (None, None) => module_path,
        }
    } else if is_edge_browser(&module_path) {
        match get_edge_aumid_info(hwnd) {
            (None, Some(pkg)) => format!("{module_path}::appx::{pkg}"),
            (Some(profile), None) => format!("{module_path}::{profile}"),
            _ => module_path,
        }
    } else {
        module_path
    }
}

fn is_valid_module_path(module_path: &str) -> bool {
    !module_path.is_empty() && module_path != "C:\\Windows\\System32\\ApplicationFrameHost.exe"
}

extern "system" fn enum_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let windows: &mut Vec<HWND> = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
    windows.push(hwnd);
    BOOL(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_detection_is_case_insensitive_and_filename_scoped() {
        assert!(is_chrome_browser(r"C:\Program Files\Google\Chrome.EXE"));
        assert!(is_edge_browser(r"C:\Program Files\Edge\msedge.exe"));
        assert!(!is_chrome_browser(r"C:\Tools\chrome.exe.helper"));
        assert!(!is_edge_browser(r"C:\Tools\not-msedge.exe"));
    }

    #[test]
    fn application_frame_host_is_not_a_final_module_path() {
        assert!(!is_valid_module_path(
            r"C:\Windows\System32\ApplicationFrameHost.exe"
        ));
        assert!(is_valid_module_path(r"C:\Windows\System32\notepad.exe"));
        assert!(!is_valid_module_path(""));
    }

    #[test]
    fn rect_size_accepts_only_positive_dimensions() {
        assert_eq!(
            rect_size(RECT {
                left: -10,
                top: 20,
                right: 190,
                bottom: 120,
            }),
            Some((200, 100))
        );
        assert_eq!(rect_size(RECT::default()), None);
        assert_eq!(
            rect_size(RECT {
                left: 10,
                top: 10,
                right: 5,
                bottom: 20,
            }),
            None
        );
    }

    #[test]
    fn small_window_filter_keeps_minimized_windows() {
        assert!(!is_small_window_with_state(HWND::default(), true));
    }

    #[test]
    fn modal_popup_requires_a_disabled_root_and_matching_ownership() {
        let root = HWND(1 as _);
        let popup = HWND(2 as _);

        assert!(is_modal_popup_relation(
            root, popup, false, true, true, true, root
        ));
        assert!(!is_modal_popup_relation(
            root, popup, true, true, true, true, root
        ));
        assert!(!is_modal_popup_relation(
            root, popup, false, true, true, false, root
        ));
        assert!(!is_modal_popup_relation(
            root, popup, false, true, true, true, popup
        ));
        assert!(!is_modal_popup_relation(
            root, root, false, true, true, true, root
        ));
    }
}
