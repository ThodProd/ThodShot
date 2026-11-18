use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::{
    Foundation::{LPARAM, LRESULT, WPARAM},
    UI::{Input::KeyboardAndMouse::GetAsyncKeyState, WindowsAndMessaging::*},
};
static ENABLED: AtomicBool = AtomicBool::new(false);
static DOWN: AtomicBool = AtomicBool::new(false);
static CAPTURE: AtomicBool = AtomicBool::new(false);
pub struct Hook(HHOOK);
impl Hook {
    pub fn new() -> windows::core::Result<Self> {
        Ok(Self(unsafe {
            SetWindowsHookExW(WH_KEYBOARD_LL, Some(callback), None, 0)?
        }))
    }
    pub fn enable(&self, enabled: bool) {
        ENABLED.store(enabled, Ordering::Relaxed);
    }
    pub fn take(&self) -> bool {
        CAPTURE.swap(false, Ordering::Relaxed)
    }
}
impl Drop for Hook {
    fn drop(&mut self) {
        unsafe {
            let _ = UnhookWindowsHookEx(self.0);
        }
    }
}
unsafe extern "system" fn callback(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code >= 0 && unsafe { (*(l.0 as *const KBDLLHOOKSTRUCT)).vkCode } == 0x2c {
        if (w.0 as u32 == WM_KEYUP || w.0 as u32 == WM_SYSKEYUP)
            && DOWN.swap(false, Ordering::Relaxed)
        {
            return LRESULT(1);
        }
        let mods = [0x12, 0x5b, 0x5c, 0x11, 0x10]
            .iter()
            .any(|v| unsafe { GetAsyncKeyState(*v) < 0 });
        if ENABLED.load(Ordering::Relaxed) && !mods && w.0 as u32 == WM_KEYDOWN {
            if !DOWN.swap(true, Ordering::Relaxed) {
                CAPTURE.store(true, Ordering::Relaxed);
            }
            return LRESULT(1);
        }
    }
    unsafe { CallNextHookEx(None, code, w, l) }
}
