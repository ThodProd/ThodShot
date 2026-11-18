use anyhow::{Context, Result};
use std::env;
use winreg::{RegKey, enums::HKEY_CURRENT_USER};

use crate::{
    ipc,
    settings::{AppSettings, Paths},
};

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const RUN_NAME: &str = "ThodShot";
const KEYBOARD_KEY: &str = "Control Panel\\Keyboard";
const PRINT_SCREEN_VALUE: &str = "PrintScreenKeyForSnippingEnabled";

pub struct Integration;

impl Integration {
    pub fn apply(paths: &Paths, settings: &mut AppSettings) -> Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (run, _) = hkcu
            .create_subkey(RUN_KEY)
            .context("could not open startup registry key")?;
        if settings.autostart {
            let executable = env::current_exe().context("could not determine executable path")?;
            run.set_value(
                RUN_NAME,
                &format!("\"{}\" --background", executable.display()),
            )
            .context("could not configure autostart")?;
        } else {
            remove_value(&run, RUN_NAME)?;
        }

        let keyboard = hkcu
            .open_subkey_with_flags(
                KEYBOARD_KEY,
                winreg::enums::KEY_READ | winreg::enums::KEY_WRITE,
            )
            .context("could not open keyboard registry key")?;
        if settings.intercept_print_screen {
            if !settings.prt_sc_recorded {
                settings.prt_sc_original = keyboard
                    .get_value::<u32, _>(PRINT_SCREEN_VALUE)
                    .ok()
                    .map(|v| v as i32);
                settings.prt_sc_recorded = true;
                // Preserve the original binding before touching the Windows setting.
                settings.save(paths)?;
            }
            keyboard
                .set_value(PRINT_SCREEN_VALUE, &0u32)
                .context("could not disable Windows PrintScreen binding")?;
        } else {
            Self::restore_print_screen(&keyboard, settings)?;
        }
        settings.save(paths)
    }

    pub fn cleanup(paths: &Paths) -> Result<()> {
        anyhow::ensure!(
            ipc::stop_running(15_000)?,
            "ThodShot не завершился перед удалением интеграций"
        );
        let mut settings = AppSettings::load(paths)?;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if settings.prt_sc_recorded {
            let keyboard = hkcu.open_subkey_with_flags(KEYBOARD_KEY, winreg::enums::KEY_WRITE)?;
            Self::restore_print_screen(&keyboard, &mut settings)?;
        }
        match hkcu.open_subkey_with_flags(RUN_KEY, winreg::enums::KEY_WRITE) {
            Ok(run) => remove_value(&run, RUN_NAME)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        settings.save(paths)?;
        paths.forget_pointer()
    }

    fn restore_print_screen(keyboard: &RegKey, settings: &mut AppSettings) -> Result<()> {
        if !settings.prt_sc_recorded {
            return Ok(());
        }
        match settings.prt_sc_original {
            Some(value) => keyboard.set_value(PRINT_SCREEN_VALUE, &(value as u32))?,
            None => remove_value(keyboard, PRINT_SCREEN_VALUE)?,
        }
        settings.prt_sc_recorded = false;
        settings.prt_sc_original = None;
        Ok(())
    }
}

fn remove_value(key: &RegKey, name: &str) -> std::io::Result<()> {
    match key.delete_value(name) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}
