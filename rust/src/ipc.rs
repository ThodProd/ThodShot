use anyhow::{Context, Result};
use std::{
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::{
            CloseHandle, ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, GetLastError, HANDLE,
            WAIT_OBJECT_0,
        },
        System::Threading::{
            CreateEventW, CreateMutexW, INFINITE, MUTEX_MODIFY_STATE, OpenMutexW,
            SYNCHRONIZATION_ACCESS_RIGHTS, SetEvent, WaitForMultipleObjects,
        },
    },
    core::PCWSTR,
};

pub const MUTEX_NAME: &str = "Local\\ThodShot.Instance.v1";
pub const CAPTURE_EVENT: &str = "Local\\ThodShot.Capture";
pub const SETTINGS_EVENT: &str = "Local\\ThodShot.Settings";
pub const RELOAD_EVENT: &str = "Local\\ThodShot.Reload";
pub const EXIT_EVENT: &str = "Local\\ThodShot.Exit";

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
fn close(handle: HANDLE) {
    unsafe {
        let _ = CloseHandle(handle);
    }
}

pub fn is_running() -> Result<bool> {
    let name = wide(MUTEX_NAME);
    let access = SYNCHRONIZATION_ACCESS_RIGHTS(
        SYNCHRONIZATION_ACCESS_RIGHTS::default().0 | MUTEX_MODIFY_STATE.0,
    );
    match unsafe { OpenMutexW(access, false, PCWSTR(name.as_ptr())) } {
        Ok(handle) => {
            close(handle);
            Ok(true)
        }
        Err(error) if error.code() == ERROR_FILE_NOT_FOUND.to_hresult() => Ok(false),
        Err(error) => Err(error.into()),
    }
}

pub fn signal(name: &str) -> Result<()> {
    let name_wide = wide(name);
    let handle = unsafe { CreateEventW(None, false, false, PCWSTR(name_wide.as_ptr())) }
        .with_context(|| format!("could not create IPC event {name}"))?;
    let result = unsafe { SetEvent(handle) };
    close(handle);
    result.context("could not signal IPC event")?;
    Ok(())
}

pub fn stop_running(timeout_ms: u64) -> Result<bool> {
    if !is_running()? {
        return Ok(true);
    }
    let request = Listener::open(EXIT_EVENT)?;
    unsafe {
        SetEvent(request.handle())?;
    }
    wait_for_exit(timeout_ms)
}

pub fn wait_for_exit(timeout_ms: u64) -> Result<bool> {
    let until = Instant::now() + Duration::from_millis(timeout_ms);
    while Instant::now() < until {
        if !is_running()? {
            return Ok(true);
        }
        thread::sleep(Duration::from_millis(100));
    }
    Ok(!is_running()?)
}

pub struct Instance(HANDLE);

impl Instance {
    pub fn open() -> Result<(Self, bool)> {
        let name = wide(MUTEX_NAME);
        let handle = unsafe { CreateMutexW(None, false, PCWSTR(name.as_ptr())) }?;
        let exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        Ok((Self(handle), exists))
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        close(self.0);
    }
}

pub fn listeners() -> Result<[Listener; 4]> {
    Ok([
        Listener::open(CAPTURE_EVENT)?,
        Listener::open(SETTINGS_EVENT)?,
        Listener::open(RELOAD_EVENT)?,
        Listener::open(EXIT_EVENT)?,
    ])
}

pub struct Listener(OwnedHandle);

impl Listener {
    pub fn open(name: &str) -> Result<Self> {
        let name_wide = wide(name);
        let name_ptr = if name.is_empty() {
            PCWSTR::null()
        } else {
            PCWSTR(name_wide.as_ptr())
        };
        let handle = unsafe { CreateEventW(None, false, false, name_ptr) }
            .with_context(|| format!("could not create IPC event {name}"))?;
        // CreateEventW returned an owned, process-wide kernel handle.
        Ok(Self(unsafe { OwnedHandle::from_raw_handle(handle.0) }))
    }

    fn handle(&self) -> HANDLE {
        HANDLE(self.0.as_raw_handle())
    }
}

pub struct Watcher {
    pending: Arc<AtomicU8>,
    stop: Listener,
}

impl Watcher {
    pub fn new(events: [Listener; 4], wake: impl Fn() + Send + 'static) -> Result<Self> {
        let stop = Listener::open("")?;
        let thread_stop = Listener(stop.0.try_clone()?);
        let pending = Arc::new(AtomicU8::new(0));
        let thread_pending = pending.clone();
        thread::Builder::new()
            .name("ThodShot IPC".into())
            .spawn(move || {
                let [capture, settings, reload, exit] = events.each_ref().map(Listener::handle);
                let handles = [capture, settings, reload, exit, thread_stop.handle()];
                loop {
                    let event = unsafe { WaitForMultipleObjects(&handles, false, INFINITE) };
                    let index = event.0.wrapping_sub(WAIT_OBJECT_0.0);
                    if index >= events.len() as u32 {
                        break;
                    }
                    thread_pending.fetch_or(1 << index, Ordering::Release);
                    wake();
                }
            })?;
        Ok(Self { pending, stop })
    }

    pub fn take(&self) -> [bool; 4] {
        let pending = self.pending.swap(0, Ordering::AcqRel);
        std::array::from_fn(|i| pending & (1 << i) != 0)
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        let _ = unsafe { SetEvent(self.stop.handle()) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_event_wakes_the_resident_once() {
        let capture = Listener::open("").unwrap();
        let sender = Listener(capture.0.try_clone().unwrap());
        let (wake, notification) = std::sync::mpsc::channel();
        let watcher = Watcher::new(
            [
                capture,
                Listener::open("").unwrap(),
                Listener::open("").unwrap(),
                Listener::open("").unwrap(),
            ],
            move || {
                let _ = wake.send(());
            },
        )
        .unwrap();
        for _ in 0..3 {
            unsafe {
                SetEvent(sender.handle()).unwrap();
            }
            notification.recv_timeout(Duration::from_secs(2)).unwrap();
            assert_eq!(watcher.take(), [true, false, false, false]);
            assert_eq!(watcher.take(), [false; 4]);
        }
    }
}
