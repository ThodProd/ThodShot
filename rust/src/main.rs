#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod annotation;
mod app;
mod capture;
mod hook;
mod integration;
mod ipc;
mod ocr;
mod settings;
mod settings_view;

use anyhow::{Result, ensure};
use integration::Integration;
use settings::{AppSettings, Paths};

fn main() {
    let result = Paths::initialize().and_then(|paths| {
        let result = run(&paths);
        if let Err(error) = &result {
            paths.log(&format!("Ошибка запуска или команды: {error:#}"));
        }
        result
    });
    if let Err(error) = result {
        eprintln!("ThodShot: {error:#}");
        std::process::exit(1);
    }
}

fn run(paths: &Paths) -> Result<()> {
    let command = std::env::args().nth(1).unwrap_or_default();
    if command == "--restart" {
        ensure!(
            ipc::wait_for_exit(15_000)?,
            "ThodShot не завершился перед перезапуском"
        );
        std::process::Command::new(std::env::current_exe()?)
            .arg("--settings")
            .spawn()?;
        return Ok(());
    }

    match command.as_str() {
        "--register-integrations" => apply_integrations(paths, AppSettings::load(paths)?)?,
        "--cleanup" => Integration::cleanup(paths)?,
        "--stop-background" => {
            if !ipc::stop_running(15_000)? {
                anyhow::bail!("the resident ThodShot process did not stop within 15 seconds");
            }
        }
        "--enable-printscreen" | "--disable-printscreen" => {
            let mut settings = AppSettings::load(paths)?;
            settings.intercept_print_screen = command == "--enable-printscreen";
            apply_integrations(paths, settings)?;
        }
        _ => {
            // Create the listeners before publishing the mutex so clients can queue startup commands.
            let listeners = ipc::listeners()?;
            let (_instance, exists) = ipc::Instance::open()?;
            if exists {
                if command == "--capture" {
                    ipc::signal(ipc::CAPTURE_EVENT)?;
                } else if command != "--background" {
                    ipc::signal(ipc::SETTINGS_EVENT)?;
                }
                return Ok(());
            }
            app::run(paths.clone(), command, listeners)?;
        }
    }
    Ok(())
}

fn apply_integrations(paths: &Paths, mut settings: AppSettings) -> Result<()> {
    Integration::apply(paths, &mut settings)?;
    if ipc::is_running()? {
        ipc::signal(ipc::RELOAD_EVENT)?;
    } else if settings.intercept_print_screen {
        // ThodHUB stops the previous resident before installing the update.
        std::process::Command::new(std::env::current_exe()?)
            .arg("--background")
            .spawn()?;
    }
    Ok(())
}
