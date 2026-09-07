use crate::{
    app::{
        WM_USER_SWITCH_APPS, WM_USER_SWITCH_APPS_CANCEL, WM_USER_SWITCH_APPS_DONE,
        WM_USER_SWITCH_WINDOWS, WM_USER_SWITCH_WINDOWS_DONE,
    },
    config::{Hotkey, SWITCH_APPS_HOTKEY_ID, SWITCH_WINDOWS_HOTKEY_ID},
    foreground::IS_FOREGROUND_IN_BLACKLIST,
};

use anyhow::{anyhow, Result};
use std::{
    cell::RefCell,
    sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering},
};
use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Input::KeyboardAndMouse::{
            SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
            SCANCODE_LSHIFT, SCANCODE_RSHIFT, VK_CONTROL,
        },
        WindowsAndMessaging::{
            CallNextHookEx, PostMessageW, SetWindowsHookExW, UnhookWindowsHookEx, HHOOK,
            KBDLLHOOKSTRUCT, LLKHF_UP, WH_KEYBOARD_LL,
        },
    },
};

thread_local! {
    static KEYBOARD_STATE: RefCell<Vec<HotKeyState>> = const { RefCell::new(Vec::new()) };
}

static WINDOW: AtomicIsize = AtomicIsize::new(0);
static IS_SHIFT_PRESSED: AtomicBool = AtomicBool::new(false);
static IS_SWITCHING_APPS: AtomicBool = AtomicBool::new(false);
static PREVIOUS_KEYCODE: AtomicU32 = AtomicU32::new(0);
static PENDING_APP_DELTA: AtomicIsize = AtomicIsize::new(0);
static APP_MESSAGE_PENDING: AtomicBool = AtomicBool::new(false);

#[derive(Debug)]
pub struct KeyboardListener {
    hook: HHOOK,
}

impl KeyboardListener {
    pub fn init(hwnd: HWND, hotkeys: &[&Hotkey]) -> Result<Self> {
        WINDOW.store(hwnd.0 as isize, Ordering::Relaxed);

        let keyboard_state = hotkeys
            .iter()
            .map(|hotkey| HotKeyState {
                hotkey: (*hotkey).clone(),
                is_modifier_pressed: false,
            })
            .collect();
        KEYBOARD_STATE.with(|state| *state.borrow_mut() = keyboard_state);

        let hook = unsafe {
            let hinstance = GetModuleHandleW(None)
                .map_err(|err| anyhow!("Failed to get module handle, {err}"))?;
            SetWindowsHookExW(
                WH_KEYBOARD_LL,
                Some(keyboard_proc),
                Some(hinstance.into()),
                0,
            )
        }
        .map_err(|err| anyhow!("Failed to set windows hook, {err}"))?;
        info!("keyboard listener start");

        Ok(Self { hook })
    }
}

impl Drop for KeyboardListener {
    fn drop(&mut self) {
        debug!("keyboard listener destroyed");
        if !self.hook.is_invalid() {
            let _ = unsafe { UnhookWindowsHookEx(self.hook) };
        }
    }
}

#[derive(Debug)]
struct HotKeyState {
    hotkey: Hotkey,
    is_modifier_pressed: bool,
}

pub fn take_pending_app_delta() -> isize {
    APP_MESSAGE_PENDING.store(false, Ordering::Release);
    PENDING_APP_DELTA.swap(0, Ordering::AcqRel)
}

unsafe fn post_message(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> bool {
    PostMessageW(Some(hwnd), msg, wparam, lparam).is_ok()
}

fn window() -> HWND {
    HWND(WINDOW.load(Ordering::Relaxed) as _)
}

unsafe fn queue_switch_apps(reverse: bool) {
    PENDING_APP_DELTA.fetch_add(if reverse { -1 } else { 1 }, Ordering::Relaxed);
    if !APP_MESSAGE_PENDING.swap(true, Ordering::AcqRel)
        && !post_message(window(), WM_USER_SWITCH_APPS, WPARAM(0), LPARAM(0))
    {
        APP_MESSAGE_PENDING.store(false, Ordering::Release);
        PENDING_APP_DELTA.store(0, Ordering::Release);
    }
}

unsafe fn mask_alt_menu() {
    let control_down = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VK_CONTROL,
                ..Default::default()
            },
        },
    };
    let control_up = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VK_CONTROL,
                dwFlags: KEYEVENTF_KEYUP,
                ..Default::default()
            },
        },
    };
    SendInput(
        &[control_down, control_up],
        std::mem::size_of::<INPUT>() as i32,
    );
}

unsafe extern "system" fn keyboard_proc(code: i32, w_param: WPARAM, l_param: LPARAM) -> LRESULT {
    let kbd_data: &KBDLLHOOKSTRUCT = &*(l_param.0 as *const _);
    let scan_code = kbd_data.scanCode;
    let is_key_pressed = || kbd_data.flags.0 & LLKHF_UP.0 == 0;
    if [SCANCODE_LSHIFT, SCANCODE_RSHIFT].contains(&scan_code) {
        IS_SHIFT_PRESSED.store(is_key_pressed(), Ordering::Relaxed);
    }

    let mut is_modifier = false;
    let mut send_done_hotkeys = 0u32;
    let mut send_action_message: Option<(u32, bool, bool)> = None;

    KEYBOARD_STATE.with(|keyboard_state| {
        let mut keyboard_state = keyboard_state.borrow_mut();
        for state in keyboard_state.iter_mut() {
            if state.hotkey.modifier.contains(&scan_code) {
                is_modifier = true;
                if is_key_pressed() {
                    state.is_modifier_pressed = true;
                } else {
                    state.is_modifier_pressed = false;
                    if PREVIOUS_KEYCODE.load(Ordering::Relaxed) == state.hotkey.code {
                        send_done_hotkeys |= 1 << state.hotkey.id;
                    }
                }
            }
        }

        if !is_modifier {
            for state in keyboard_state.iter_mut() {
                if !is_key_pressed() || !state.is_modifier_pressed {
                    continue;
                }

                let id = state.hotkey.id;
                if scan_code == state.hotkey.code {
                    let reverse = IS_SHIFT_PRESSED.load(Ordering::Relaxed);
                    if id == SWITCH_APPS_HOTKEY_ID
                        || (id == SWITCH_WINDOWS_HOTKEY_ID
                            && !IS_FOREGROUND_IN_BLACKLIST.load(Ordering::Relaxed))
                    {
                        send_action_message = Some((id, reverse, false));
                        PREVIOUS_KEYCODE.store(scan_code, Ordering::Relaxed);
                        break;
                    }
                } else if id == SWITCH_APPS_HOTKEY_ID {
                    if scan_code == 0x01 {
                        send_action_message = Some((id, false, true));
                        PREVIOUS_KEYCODE.store(scan_code, Ordering::Relaxed);
                        break;
                    }
                    if [0x48, 0x4b, 0x4d, 0x50].contains(&scan_code)
                        && IS_SWITCHING_APPS.load(Ordering::Relaxed)
                    {
                        let reverse = scan_code == 0x48 || scan_code == 0x4b;
                        send_action_message = Some((id, reverse, false));
                        break;
                    }
                }
            }
        }
    });

    if send_done_hotkeys & (1 << SWITCH_WINDOWS_HOTKEY_ID) != 0 {
        let _ = post_message(window(), WM_USER_SWITCH_WINDOWS_DONE, WPARAM(0), LPARAM(0));
    }
    if send_done_hotkeys & (1 << SWITCH_APPS_HOTKEY_ID) != 0 {
        let _ = post_message(window(), WM_USER_SWITCH_APPS_DONE, WPARAM(0), LPARAM(0));
        IS_SWITCHING_APPS.store(false, Ordering::Relaxed);
    }

    if let Some((id, reverse, is_cancel)) = send_action_message {
        if id == SWITCH_APPS_HOTKEY_ID {
            if is_cancel {
                let _ = post_message(window(), WM_USER_SWITCH_APPS_CANCEL, WPARAM(0), LPARAM(0));
                IS_SWITCHING_APPS.store(false, Ordering::Relaxed);
            } else {
                if !IS_SWITCHING_APPS.load(Ordering::Relaxed) {
                    mask_alt_menu();
                }
                queue_switch_apps(reverse);
                IS_SWITCHING_APPS.store(true, Ordering::Relaxed);
            }
            return LRESULT(1);
        }
        if id == SWITCH_WINDOWS_HOTKEY_ID {
            let _ = post_message(
                window(),
                WM_USER_SWITCH_WINDOWS,
                WPARAM(0),
                LPARAM(reverse as isize),
            );
            IS_SWITCHING_APPS.store(false, Ordering::Relaxed);
            return LRESULT(1);
        }
    }
    CallNextHookEx(None, code, w_param, l_param)
}
