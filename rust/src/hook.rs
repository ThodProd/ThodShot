use crate::{ipc, settings::Paths};
use anyhow::{Context, Result};
use std::{
    cell::Cell,
    sync::mpsc,
    thread::{self, JoinHandle},
};
use windows::Win32::{
    Foundation::{HANDLE, LPARAM, LRESULT, WPARAM},
    System::Threading::{GetCurrentThreadId, SetEvent},
    UI::{
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey, VK_CONTROL, VK_LWIN,
            VK_MENU, VK_RWIN, VK_SHIFT, VK_SNAPSHOT,
        },
        WindowsAndMessaging::*,
    },
};

const UPDATE_BINDING: u32 = WM_APP + 1;
const HOTKEY_ID: i32 = 1;

thread_local! {
    static CAPTURE_EVENT: Cell<HANDLE> = const { Cell::new(HANDLE(std::ptr::null_mut())) };
    static PRINT_SCREEN_DOWN: Cell<bool> = const { Cell::new(false) };
}

pub struct Hook {
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}

impl Hook {
    pub fn new(paths: Paths, enabled: bool) -> Result<Self> {
        let capture = ipc::Listener::open(ipc::CAPTURE_EVENT)?;
        let (ready, thread_id) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("ThodShot PrintScreen".into())
            .spawn(move || {
                CAPTURE_EVENT.set(capture.handle());
                let mut message = MSG::default();
                // The queue must exist before the UI can send configuration or shutdown messages.
                let id = unsafe {
                    let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
                    GetCurrentThreadId()
                };
                let mut binding = enabled.then(|| Binding::new(&paths));
                if ready.send(id).is_err() {
                    return;
                }
                loop {
                    let result = unsafe { GetMessageW(&mut message, None, 0, 0) }.0;
                    if result <= 0 {
                        if result < 0 {
                            paths.log("Поток PrintScreen: не удалось получить сообщение Windows");
                        }
                        break;
                    }
                    match message.message {
                        UPDATE_BINDING => match (message.wParam.0 != 0, binding.is_some()) {
                            (true, false) => binding = Some(Binding::new(&paths)),
                            (false, true) => binding = None,
                            _ => {}
                        },
                        WM_HOTKEY
                            if message.wParam.0 == HOTKEY_ID as usize && binding.is_some() =>
                        {
                            request_capture();
                        }
                        _ => {}
                    }
                }
                drop(binding);
                drop(capture);
            })?;
        Ok(Self {
            thread_id: thread_id
                .recv()
                .context("Поток PrintScreen не запустился")?,
            thread: Some(thread),
        })
    }

    pub fn enable(&self, enabled: bool) -> Result<()> {
        unsafe {
            PostThreadMessageW(
                self.thread_id,
                UPDATE_BINDING,
                WPARAM(enabled as usize),
                LPARAM(0),
            )?;
        }
        Ok(())
    }
}

impl Drop for Hook {
    fn drop(&mut self) {
        if unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) }.is_ok()
            && let Some(thread) = self.thread.take()
        {
            let _ = thread.join();
        }
    }
}

struct Binding {
    registered: bool,
    low_level: Option<HHOOK>,
}

impl Binding {
    fn new(paths: &Paths) -> Self {
        let registered =
            unsafe { RegisterHotKey(None, HOTKEY_ID, MOD_NOREPEAT, u32::from(VK_SNAPSHOT.0)) };
        // Windows may reserve PrintScreen for its own handler. Keep the low-level
        // path for the bare key, with WM_HOTKEY available if the hook is bypassed.
        let low_level = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(callback), None, 0) };
        if let Err(error) = &low_level {
            paths.log(&format!("Перехват PrintScreen: {error}"));
        }
        if registered.is_err() && low_level.is_err() {
            paths.log("Глобальную клавишу PrintScreen зарегистрировать не удалось");
        }
        Self {
            registered: registered.is_ok(),
            low_level: low_level.ok(),
        }
    }
}

impl Drop for Binding {
    fn drop(&mut self) {
        unsafe {
            if let Some(handle) = self.low_level {
                let _ = UnhookWindowsHookEx(handle);
            }
            if self.registered {
                let _ = UnregisterHotKey(None, HOTKEY_ID);
            }
        }
        PRINT_SCREEN_DOWN.set(false);
    }
}

fn request_capture() {
    // The resident's IPC watcher wakes the UI. The hook callback never calls egui.
    let _ = unsafe { SetEvent(CAPTURE_EVENT.get()) };
}

fn handle_print_screen(message: u32, modified: bool, held: &Cell<bool>) -> (bool, bool) {
    match message {
        WM_KEYUP | WM_SYSKEYUP => (held.replace(false), false),
        WM_KEYDOWN | WM_SYSKEYDOWN if held.get() => (true, false),
        WM_KEYDOWN | WM_SYSKEYDOWN if !modified => {
            held.set(true);
            (true, true)
        }
        _ => (false, false),
    }
}

unsafe extern "system" fn callback(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32
        && unsafe { (*(l.0 as *const KBDLLHOOKSTRUCT)).vkCode } == u32::from(VK_SNAPSHOT.0)
    {
        let message = w.0 as u32;
        let modified = matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN)
            && [VK_MENU, VK_LWIN, VK_RWIN, VK_CONTROL, VK_SHIFT]
                .iter()
                .any(|key| unsafe { GetAsyncKeyState(i32::from(key.0)) < 0 });
        let (handled, capture) =
            PRINT_SCREEN_DOWN.with(|held| handle_print_screen(message, modified, held));
        if capture {
            request_capture();
        }
        if handled {
            return LRESULT(1);
        }
    }
    unsafe { CallNextHookEx(None, code, w, l) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn print_screen_rearms_after_each_release_even_without_focus() {
        let held = Cell::new(false);
        for (press, release) in [
            (WM_KEYDOWN, WM_SYSKEYUP),
            (WM_SYSKEYDOWN, WM_KEYUP),
            (WM_SYSKEYDOWN, WM_SYSKEYUP),
        ] {
            assert_eq!(handle_print_screen(press, false, &held), (true, true));
            assert_eq!(handle_print_screen(press, false, &held), (true, false));
            assert_eq!(handle_print_screen(release, false, &held), (true, false));
        }
    }

    #[test]
    fn modified_print_screen_is_left_to_windows() {
        let held = Cell::new(false);
        assert_eq!(
            handle_print_screen(WM_SYSKEYDOWN, true, &held),
            (false, false)
        );
        assert_eq!(handle_print_screen(WM_KEYUP, false, &held), (false, false));
        assert_eq!(handle_print_screen(WM_KEYDOWN, false, &held), (true, true));
    }
}
