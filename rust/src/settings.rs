use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{env, fs, path::PathBuf};
use std::{fs::OpenOptions, io::Write};

#[derive(Debug, Clone)]
pub struct Paths {
    pub data_dir: PathBuf,
    pointer: PathBuf,
}

impl Paths {
    pub fn initialize() -> Result<Self> {
        let local = env::var_os("LOCALAPPDATA").context("LOCALAPPDATA is not set")?;
        let pointer = PathBuf::from(&local).join("ThodShot").join("datadir.txt");
        let data_dir = match env::var("THODHUB_DATA_DIR")
            .ok()
            .filter(|v| !v.trim().is_empty())
        {
            Some(value) => {
                if let Some(parent) = pointer.parent() {
                    fs::create_dir_all(parent)?;
                }
                if fs::read_to_string(&pointer).ok().as_deref().map(str::trim) != Some(value.trim())
                {
                    fs::write(&pointer, &value)?;
                }
                PathBuf::from(value)
            }
            None => fs::read_to_string(&pointer)
                .ok()
                .map(|v| PathBuf::from(v.trim_start_matches('\u{feff}').trim()))
                .filter(|v| !v.as_os_str().is_empty())
                .unwrap_or_else(|| {
                    PathBuf::from(local)
                        .join("ThodHUB")
                        .join("Data")
                        .join("thodshot")
                }),
        };
        fs::create_dir_all(&data_dir)?;
        Ok(Self { data_dir, pointer })
    }

    pub fn settings_file(&self) -> PathBuf {
        self.data_dir.join("settings.json")
    }

    pub fn log(&self, message: &str) {
        let dir = self.data_dir.join("logs");
        let _ = fs::create_dir_all(&dir);
        let log = dir.join("thodshot.log");
        let truncate = fs::metadata(&log).is_ok_and(|metadata| metadata.len() >= 512 * 1024);
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .write(true)
            .append(!truncate)
            .truncate(truncate)
            .open(log)
        {
            let _ = writeln!(
                file,
                "{} {message}",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
            );
        }
    }
    pub fn forget_pointer(&self) -> Result<()> {
        if self.pointer.exists() {
            fs::remove_file(&self.pointer)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct AppSettings {
    pub save_folder: String,
    pub autostart: bool,
    pub show_tray_icon: bool,
    pub intercept_print_screen: bool,
    pub default_mode: String,
    pub theme: String,
    pub open_folder_after_save: bool,
    pub format: String,
    pub ocr_language: String,
    pub setup_version: i32,
    #[serde(rename = "PrtScRecorded")]
    pub prt_sc_recorded: bool,
    #[serde(rename = "PrtScOriginal")]
    pub prt_sc_original: Option<i32>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            save_folder: String::new(),
            autostart: true,
            show_tray_icon: false,
            intercept_print_screen: true,
            default_mode: "region".into(),
            theme: "dark".into(),
            open_folder_after_save: true,
            format: "png".into(),
            ocr_language: "auto".into(),
            setup_version: 0,
            prt_sc_recorded: false,
            prt_sc_original: None,
        }
    }
}

impl AppSettings {
    pub fn effective_folder(&self) -> PathBuf {
        if !self.save_folder.trim().is_empty() {
            return PathBuf::from(&self.save_folder);
        }
        use windows::Win32::{
            System::Com::CoTaskMemFree,
            UI::Shell::{FOLDERID_Pictures, KF_FLAG_DEFAULT, SHGetKnownFolderPath},
        };
        if let Ok(path) = unsafe { SHGetKnownFolderPath(&FOLDERID_Pictures, KF_FLAG_DEFAULT, None) }
        {
            let result = unsafe { path.to_string() }.ok().map(PathBuf::from);
            unsafe {
                CoTaskMemFree(Some(path.0 as _));
            }
            if let Some(path) = result {
                return path.join("Скриншоты");
            }
        }
        PathBuf::from(env::var_os("USERPROFILE").unwrap_or_default())
            .join("Pictures")
            .join("Скриншоты")
    }
    pub fn load(paths: &Paths) -> Result<Self> {
        let file = paths.settings_file();
        let mut settings = match fs::read_to_string(&file) {
            Ok(json) => match serde_json::from_str(json.trim_start_matches('\u{feff}')) {
                Ok(settings) => settings,
                Err(error) => {
                    paths.log(&format!("Ошибка чтения {}: {error}", file.display()));
                    return Ok(Self::default());
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(error) => return Err(error.into()),
        };
        settings.migrate(paths)?;
        Ok(settings)
    }

    fn migrate(&mut self, paths: &Paths) -> Result<()> {
        if self.setup_version < 1 {
            self.default_mode = "region".into();
            self.setup_version = 1;
            self.save(paths)?;
        }
        Ok(())
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        fs::create_dir_all(&paths.data_dir)?;
        let file = paths.settings_file();
        let temporary = file.with_extension(format!("{}.tmp", std::process::id()));
        fs::write(&temporary, serde_json::to_string_pretty(self)?)?;
        use std::os::windows::ffi::OsStrExt;
        use windows::{
            Win32::Storage::FileSystem::{
                MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
            },
            core::PCWSTR,
        };
        let source: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
        let target: Vec<u16> = file.as_os_str().encode_wide().chain(Some(0)).collect();
        let result = unsafe {
            MoveFileExW(
                PCWSTR(source.as_ptr()),
                PCWSTR(target.as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if let Err(error) = result {
            let _ = fs::remove_file(temporary);
            return Err(error.into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::AppSettings;

    #[test]
    fn uses_the_existing_dotnet_json_field_names() {
        let json = serde_json::to_value(AppSettings::default()).unwrap();
        assert_eq!(json["DefaultMode"], "region");
        assert_eq!(json["InterceptPrintScreen"], true);
        assert!(json.get("PrtScRecorded").is_some());
        assert!(json.get("PrtScOriginal").is_some());
        assert!(json.get("default_mode").is_none());
    }

    #[test]
    fn accepts_an_existing_settings_document() {
        let value = r#"{"SaveFolder":"D:\\shots","Autostart":false,"InterceptPrintScreen":false,"SetupVersion":1}"#;
        let settings: AppSettings = serde_json::from_str(value).unwrap();
        assert_eq!(settings.save_folder, "D:\\shots");
        assert!(!settings.autostart);
        assert!(!settings.intercept_print_screen);
        assert_eq!(settings.default_mode, "region");
    }

    #[test]
    fn preserves_original_windows_printscreen_setting() {
        let settings: AppSettings =
            serde_json::from_str(r#"{"PrtScRecorded":true,"PrtScOriginal":1}"#).unwrap();
        assert!(settings.prt_sc_recorded);
        assert_eq!(settings.prt_sc_original, Some(1));
    }
}
