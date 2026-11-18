use anyhow::{Context, Result};
use std::{
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
            CreateEventW, CreateMutexW, MUTEX_MODIFY_STATE, OpenMutexW,
            SYNCHRONIZATION_ACCESS_RIGHTS, SetEvent, WaitForSingleObject,
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
        SetEvent(request.0)?;
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

pub struct Listener(HANDLE);

impl Listener {
    pub fn open(name: &str) -> Result<Self> {
        let name_wide = wide(name);
        let handle = unsafe { CreateEventW(None, false, false, PCWSTR(name_wide.as_ptr())) }
            .with_context(|| format!("could not create IPC event {name}"))?;
        Ok(Self(handle))
    }

    pub fn is_signaled(&self) -> bool {
        unsafe { WaitForSingleObject(self.0, 0) == WAIT_OBJECT_0 }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        close(self.0);
    }
}
