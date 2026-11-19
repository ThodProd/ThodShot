use crate::{
    annotation::{self, Mark, Tool},
    capture::{Capture, CaptureWorker},
    hook::Hook,
    integration::Integration,
    ipc, ocr,
    settings::{AppSettings, Paths},
    settings_view::{
        Section,
        kit::{self, Icon, Look},
    },
};
use anyhow::{Context, Result};
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};
use image::RgbaImage;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};
use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
    Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint},
    UI::{
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{
            GetCursorPos, GetForegroundWindow, HCURSOR, HTCLIENT, HWND_TOPMOST, IDC_CROSS,
            IsWindowVisible, LoadCursorW, SWP_SHOWWINDOW, SetCursor, SetForegroundWindow,
            SetWindowPos, WM_NCDESTROY, WM_SETCURSOR, WindowFromPoint,
        },
    },
};

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Hidden,
    Settings,
    Overlay,
    Preview,
}
pub fn run(paths: Paths, command: String, events: [ipc::Listener; 4]) -> Result<()> {
    let settings = AppSettings::load(&paths)?;
    let capture_worker = CaptureWorker::new()?;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("ThodShot")
            .with_inner_size([1., 1.])
            .with_min_inner_size([1., 1.])
            .with_position([-32000., -32000.])
            .with_taskbar(false)
            .with_active(false)
            .with_decorations(false)
            .with_icon(icon()),
        ..Default::default()
    };
    eframe::run_native(
        "ThodShot",
        options,
        Box::new(move |cc| {
            Ok(Box::new(App::new(
                cc,
                paths,
                settings,
                command,
                events,
                capture_worker,
            )?))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}
fn icon() -> egui::IconData {
    let i = image::load_from_memory(include_bytes!("../assets/app.ico"))
        .expect("embedded icon")
        .to_rgba8();
    egui::IconData {
        width: i.width(),
        height: i.height(),
        rgba: i.into_raw(),
    }
}

pub(crate) fn configure_fonts(ctx: &egui::Context) {
    if let Some(bytes) = std::env::var_os("WINDIR")
        .and_then(|dir| std::fs::read(PathBuf::from(dir).join("Fonts/segoeui.ttf")).ok())
    {
        let mut fonts = egui::FontDefinitions::default();
        fonts
            .font_data
            .insert("Segoe".into(), egui::FontData::from_owned(bytes).into());
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, "Segoe".into());
        ctx.set_fonts(fonts);
    }
}

fn monitor_at_cursor(capture: &Capture) -> Rect {
    let mut cursor = POINT::default();
    let _ = unsafe { GetCursorPos(&mut cursor) };
    let cursor = Pos2::new(
        (cursor.x - capture.origin[0]) as f32,
        (cursor.y - capture.origin[1]) as f32,
    );
    capture
        .monitors
        .iter()
        .map(|&[x, y, w, h]| {
            Rect::from_min_size(Pos2::new(x as f32, y as f32), Vec2::new(w as f32, h as f32))
        })
        .min_by(|a, b| {
            a.distance_sq_to_pos(cursor)
                .total_cmp(&b.distance_sq_to_pos(cursor))
        })
        .expect("capture has at least one monitor")
}

struct OverlayCursor {
    window: HWND,
    cursor: HCURSOR,
}

impl OverlayCursor {
    fn install(window: HWND) -> Result<Self> {
        let cursor = unsafe { LoadCursorW(None, IDC_CROSS)? };
        let installed =
            unsafe { SetWindowSubclass(window, Some(overlay_cursor_proc), 0, cursor.0 as usize) };
        anyhow::ensure!(installed.as_bool(), "Не удалось установить курсор выбора");
        Ok(Self { window, cursor })
    }

    fn refresh(&self) {
        let mut point = POINT::default();
        unsafe {
            if GetCursorPos(&mut point).is_ok() && WindowFromPoint(point) == self.window {
                let _ = SetCursor(Some(self.cursor));
            }
        }
    }
}

impl Drop for OverlayCursor {
    fn drop(&mut self) {
        let _ = unsafe { RemoveWindowSubclass(self.window, Some(overlay_cursor_proc), 0) };
    }
}

unsafe extern "system" fn overlay_cursor_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    id: usize,
    cursor: usize,
) -> LRESULT {
    if message == WM_SETCURSOR && lparam.0 as u16 == HTCLIENT as u16 {
        let _ = unsafe { SetCursor(Some(HCURSOR(cursor as _))) };
        return LRESULT(1);
    }
    if message == WM_NCDESTROY {
        let _ = unsafe { RemoveWindowSubclass(window, Some(overlay_cursor_proc), id) };
    }
    unsafe { DefSubclassProc(window, message, wparam, lparam) }
}

struct App {
    window: HWND,
    paths: Paths,
    settings: AppSettings,
    page: Page,
    hook: Hook,
    events: ipc::Watcher,
    tray: tray_icon::TrayIcon,
    tray_capture: tray_icon::menu::MenuId,
    tray_settings: tray_icon::menu::MenuId,
    tray_exit: tray_icon::menu::MenuId,
    capture_worker: CaptureWorker,
    capture_rx: Option<Receiver<Result<Capture>>>,
    capture_after_hide: bool,
    capture: Option<Capture>,
    image: Option<RgbaImage>,
    texture: Option<egui::TextureHandle>,
    marks: Vec<Mark>,
    undone_marks: Vec<Mark>,
    pending: Option<Mark>,
    tool: Tool,
    color: Color32,
    width: f32,
    selection: Option<Rect>,
    drag: Option<Pos2>,
    toolbar_monitor: Rect,
    overlay_opened: Instant,
    overlay_was_foreground: bool,
    foreground_before_shot: HWND,
    overlay_cursor: Option<OverlayCursor>,
    full: bool,
    zoom: f32,
    pan: Vec2,
    saved: Option<PathBuf>,
    words: Vec<ocr::Word>,
    selected: BTreeSet<usize>,
    selection_base: BTreeSet<usize>,
    ocr_visible: bool,
    ocr_rx: Option<Receiver<Result<Vec<ocr::Word>>>>,
    notice: String,
    quitting: bool,
    settings_open: bool,
    settings_section: Section,
}
impl App {
    fn new(
        cc: &eframe::CreationContext<'_>,
        paths: Paths,
        settings: AppSettings,
        command: String,
        events: [ipc::Listener; 4],
        capture_worker: CaptureWorker,
    ) -> Result<Self> {
        configure_fonts(&cc.egui_ctx);
        let hook = Hook::new(paths.clone(), settings.intercept_print_screen)?;
        let menu = tray_icon::menu::Menu::new();
        let capture = tray_icon::menu::MenuItem::new("Сделать снимок", true, None);
        let prefs = tray_icon::menu::MenuItem::new("Настройки", true, None);
        let exit = tray_icon::menu::MenuItem::new("Выйти", true, None);
        menu.append_items(&[
            &capture,
            &prefs,
            &tray_icon::menu::PredefinedMenuItem::separator(),
            &exit,
        ])?;
        let i = icon();
        let tray = tray_icon::TrayIconBuilder::new()
            .with_tooltip("ThodShot — скриншоты")
            .with_icon(tray_icon::Icon::from_rgba(i.rgba, i.width, i.height)?)
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build()?;
        tray.set_visible(settings.show_tray_icon)?;
        let RawWindowHandle::Win32(handle) = cc.window_handle()?.as_raw() else {
            anyhow::bail!("Ожидалось окно Win32");
        };
        let repaint = cc.egui_ctx.clone();
        let events = ipc::Watcher::new(events, move || repaint.request_repaint())?;
        let mut app = Self {
            window: HWND(handle.hwnd.get() as _),
            paths,
            settings,
            page: Page::Hidden,
            hook,
            events,
            tray,
            tray_capture: capture.id().clone(),
            tray_settings: prefs.id().clone(),
            tray_exit: exit.id().clone(),
            capture_worker,
            capture_rx: None,
            capture_after_hide: false,
            capture: None,
            image: None,
            texture: None,
            marks: vec![],
            undone_marks: vec![],
            pending: None,
            tool: Tool::Cursor,
            color: Color32::from_rgb(242, 59, 48),
            width: 6.,
            selection: None,
            drag: None,
            toolbar_monitor: Rect::NOTHING,
            overlay_opened: Instant::now(),
            overlay_was_foreground: false,
            foreground_before_shot: HWND::default(),
            overlay_cursor: None,
            full: false,
            zoom: 1.,
            pan: Vec2::ZERO,
            saved: None,
            words: vec![],
            selected: Default::default(),
            selection_base: Default::default(),
            ocr_visible: false,
            ocr_rx: None,
            notice: String::new(),
            quitting: false,
            settings_open: false,
            settings_section: Section::default(),
        };
        app.theme(&cc.egui_ctx);
        if command == "--capture" {
            app.start_capture(&cc.egui_ctx);
        } else if command != "--background" {
            app.show(Page::Settings, &cc.egui_ctx);
        }
        Ok(app)
    }
    fn theme(&self, ctx: &egui::Context) {
        let dark = match self.settings.theme.as_str() {
            "dark" => true,
            "light" => false,
            _ => {
                winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
                    .open_subkey(
                        "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize",
                    )
                    .and_then(|k| k.get_value::<u32, _>("AppsUseLightTheme"))
                    .unwrap_or(1)
                    == 0
            }
        };
        let mut v = if dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        ctx.set_theme(if dark {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        });
        v.panel_fill = if dark {
            Color32::from_rgb(20, 23, 30)
        } else {
            Color32::from_rgb(241, 245, 251)
        };
        v.window_corner_radius = 16.into();
        v.menu_corner_radius = 12.into();
        v.window_fill = if dark {
            Color32::from_rgba_unmultiplied(30, 35, 45, 238)
        } else {
            Color32::from_rgba_unmultiplied(255, 255, 255, 240)
        };
        v.selection.bg_fill = kit::ACCENT.gamma_multiply(0.45);
        v.selection.stroke = Stroke::new(1_f32, kit::ACCENT);
        v.widgets.hovered.weak_bg_fill = if dark {
            Color32::from_rgb(54, 62, 78)
        } else {
            Color32::from_rgb(220, 231, 247)
        };
        v.widgets.noninteractive.bg_stroke.color = if dark {
            Color32::from_rgb(53, 63, 79)
        } else {
            Color32::from_rgb(208, 219, 234)
        };
        v.widgets.inactive.weak_bg_fill = if dark {
            Color32::from_rgb(42, 49, 62)
        } else {
            Color32::from_rgb(231, 238, 249)
        };
        for widget in [
            &mut v.widgets.inactive,
            &mut v.widgets.hovered,
            &mut v.widgets.active,
            &mut v.widgets.open,
        ] {
            widget.corner_radius = 10.into();
        }
        ctx.set_visuals(v);
        ctx.style_mut(|style| {
            style.spacing.button_padding = Vec2::new(10., 6.);
            style
                .text_styles
                .insert(egui::TextStyle::Body, egui::FontId::proportional(14.));
            style
                .text_styles
                .insert(egui::TextStyle::Button, egui::FontId::proportional(14.));
        });
    }
    fn show(&mut self, page: Page, ctx: &egui::Context) {
        if self.page == Page::Overlay && page == Page::Hidden {
            let previous = self.foreground_before_shot;
            unsafe {
                if !previous.is_invalid()
                    && GetForegroundWindow() == self.window
                    && IsWindowVisible(previous).as_bool()
                {
                    let _ = SetForegroundWindow(previous);
                }
            }
        }
        self.page = page;
        if page != Page::Overlay {
            self.overlay_cursor = None;
        }
        if page == Page::Hidden {
            // The root viewport must stay mapped for eframe to handle repaint and IPC.
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(Vec2::splat(1.)));
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(Pos2::new(
                -32000., -32000.,
            )));
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::splat(1.)));
            self.capture = None;
            self.image = None;
            self.texture = None;
            self.marks.clear();
            self.undone_marks.clear();
            self.pending = None;
            self.ocr_rx = None;
            self.words.clear();
            self.selected.clear();
            self.settings_open = false;
        }
        if page != Page::Overlay {
            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
                egui::WindowLevel::Normal,
            ));
        }
        if matches!(page, Page::Settings | Page::Preview) {
            let (title, mut size, minimum) = if page == Page::Settings {
                (
                    "Настройки · ThodShot",
                    Vec2::new(800., 548.),
                    Vec2::new(430., 340.),
                )
            } else {
                (
                    "Скриншот · ThodShot",
                    Vec2::new(1100., 760.),
                    Vec2::new(600., 380.),
                )
            };
            let mut cursor = POINT::default();
            let mut info = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            let ppp = ctx.pixels_per_point();
            if unsafe {
                GetCursorPos(&mut cursor).is_ok()
                    && GetMonitorInfoW(
                        MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST),
                        &mut info,
                    )
                    .as_bool()
            } {
                let work = info.rcWork;
                let available = Vec2::new(
                    (work.right - work.left) as f32,
                    (work.bottom - work.top) as f32,
                ) / ppp;
                size = size.min(available * 0.85);
                let left_top = Pos2::new(work.left as f32, work.top as f32) / ppp;
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(
                    left_top + (available - size) / 2.,
                ));
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.into()));
            ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(minimum.min(size)));
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
    }
    fn update_print_screen(&mut self) {
        if let Err(error) = self.hook.enable(self.settings.intercept_print_screen) {
            self.notice = format!("Не удалось обновить PrintScreen: {error:#}");
            self.paths.log(&self.notice);
        }
    }

    fn start_capture(&mut self, ctx: &egui::Context) {
        if self.page == Page::Overlay || self.capture_rx.is_some() || self.capture_after_hide {
            return;
        }
        let foreground = unsafe { GetForegroundWindow() };
        if foreground != self.window {
            self.foreground_before_shot = foreground;
        }
        self.notice.clear();
        if self.page == Page::Hidden {
            self.request_capture(ctx, false);
        } else {
            self.show(Page::Hidden, ctx);
            // The viewport commands move the preview off screen at the end of this frame.
            self.capture_after_hide = true;
            ctx.request_repaint();
        }
    }

    fn request_capture(&mut self, ctx: &egui::Context, flush_desktop: bool) {
        let repaint = ctx.clone();
        match self
            .capture_worker
            .request(flush_desktop, move || repaint.request_repaint())
        {
            Ok(receiver) => self.capture_rx = Some(receiver),
            Err(error) => self.capture_error(ctx, error),
        }
    }

    fn capture_error(&mut self, ctx: &egui::Context, error: anyhow::Error) {
        self.paths.log(&format!("Захват экрана: {error:#}"));
        self.notice = format!("Не удалось сделать снимок: {error}");
        self.show(Page::Settings, ctx);
    }
    fn accept_capture(&mut self, capture: Capture, ctx: &egui::Context) {
        self.texture = None;
        self.image = None;
        self.marks.clear();
        self.undone_marks.clear();
        self.pending = None;
        self.words.clear();
        self.selected.clear();
        self.ocr_rx = None;
        self.ocr_visible = false;
        self.saved = None;
        self.selection = None;
        self.drag = None;
        self.full = self.settings.default_mode == "full";
        self.toolbar_monitor = monitor_at_cursor(&capture);
        self.overlay_opened = Instant::now();
        self.overlay_was_foreground = false;
        match OverlayCursor::install(self.window) {
            Ok(cursor) => self.overlay_cursor = Some(cursor),
            Err(error) => self.paths.log(&format!("Курсор захвата: {error:#}")),
        }
        if let Err(error) = unsafe {
            SetWindowPos(
                self.window,
                Some(HWND_TOPMOST),
                capture.origin[0],
                capture.origin[1],
                capture.image.width() as i32,
                capture.image.height() as i32,
                SWP_SHOWWINDOW,
            )
        } {
            self.notice = format!("Не удалось открыть область захвата: {error}");
            self.show(Page::Settings, ctx);
            return;
        }
        unsafe {
            self.overlay_was_foreground =
                SetForegroundWindow(self.window).as_bool() || GetForegroundWindow() == self.window;
        }
        // A stationary pointer may not receive a WM_SETCURSOR when the overlay opens.
        if let Some(cursor) = &self.overlay_cursor {
            cursor.refresh();
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        ctx.send_viewport_cmd(egui::ViewportCommand::Title("Съёмка · ThodShot".into()));
        self.capture = Some(capture);
        self.show(Page::Overlay, ctx);
    }
    fn crop(&mut self, r: Rect, ctx: &egui::Context) {
        if let Some(c) = &self.capture {
            let x = r.left().max(0.) as u32;
            let y = r.top().max(0.) as u32;
            let w = (r.right().min(c.image.width() as f32) as u32).saturating_sub(x);
            let h = (r.bottom().min(c.image.height() as f32) as u32).saturating_sub(y);
            if w < 1 || h < 1 {
                return;
            }
            self.image = Some(image::imageops::crop_imm(&c.image, x, y, w, h).to_image());
        }
        self.capture = None;
        self.texture = None;
        self.zoom = 1.;
        self.pan = Vec2::ZERO;
        self.show(Page::Preview, ctx);
    }
    fn upload(&mut self, ctx: &egui::Context) {
        if self.texture.is_none() {
            if let Some(img) = self
                .image
                .as_ref()
                .or_else(|| self.capture.as_ref().map(|c| &c.image))
            {
                let limit = ctx.input(|input| input.max_texture_side) as u32;
                let scaled;
                let img = if img.width().max(img.height()) > limit {
                    scaled = image::imageops::thumbnail(img, limit, limit);
                    &scaled
                } else {
                    img
                };
                self.texture = Some(ctx.load_texture(
                    "shot",
                    egui::ColorImage::from_rgba_unmultiplied(
                        [img.width() as usize, img.height() as usize],
                        img.as_raw(),
                    ),
                    egui::TextureOptions::LINEAR,
                ));
            }
        }
    }
    fn rendered(&self) -> Result<RgbaImage> {
        Ok(annotation::render(
            self.image.as_ref().context("Нет снимка")?,
            &self.marks,
        ))
    }
    fn copy_image(&mut self, close: bool, ctx: &egui::Context) {
        let result = self.rendered().and_then(|image| copy_to_clipboard(&image));
        match result {
            Ok(()) => {
                self.notice = "Изображение скопировано".into();
                if close {
                    self.show(Page::Hidden, ctx);
                }
            }
            Err(e) => self.notice = e.to_string(),
        }
    }
    fn save(&mut self, close: bool, ctx: &egui::Context) {
        let result = (|| -> Result<()> {
            let img = self.rendered()?;
            let path = if let Some(path) = &self.saved {
                path.clone()
            } else {
                let folder = self.settings.effective_folder();
                std::fs::create_dir_all(&folder)?;
                let ext = if self.settings.format == "jpg" {
                    "jpg"
                } else {
                    "png"
                };
                let name = chrono::Local::now()
                    .format("Скриншот %Y-%m-%d %H-%M-%S")
                    .to_string();
                let mut path = folder.join(format!("{name}.{ext}"));
                let mut i = 1;
                while path.exists() {
                    path = folder.join(format!("{name} ({i}).{ext}"));
                    i += 1;
                }
                path
            };
            write_image(&img, &path)?;
            self.saved = Some(path.clone());
            copy_to_clipboard(&img).context("Снимок сохранён, но его не удалось скопировать")?;
            if self.settings.open_folder_after_save {
                std::process::Command::new("explorer.exe")
                    .arg("/select,")
                    .arg(&path)
                    .spawn()
                    .context("Снимок сохранён, но Проводник не открылся")?;
            }
            self.notice = format!("Снимок сохранён · {}", path.display());
            Ok(())
        })();
        match result {
            Ok(()) if close => self.show(Page::Hidden, ctx),
            Ok(()) => {}
            Err(e) => {
                self.paths.log(&format!("Сохранение снимка: {e:#}"));
                self.notice = e.to_string();
            }
        }
    }
    fn copy_text(&mut self) {
        let mut text = String::new();
        let mut line = None;
        for &i in &self.selected {
            if let Some(w) = self.words.get(i) {
                if !text.is_empty() {
                    text.push(if line == Some(w.line) { ' ' } else { '\n' });
                }
                text.push_str(&w.text);
                line = Some(w.line);
            }
        }
        match arboard::Clipboard::new().and_then(|mut c| c.set_text(text)) {
            Ok(()) => {}
            Err(e) => self.notice = e.to_string(),
        }
    }
    fn undo_mark(&mut self) {
        if self.pending.take().is_none()
            && let Some(mark) = self.marks.pop()
        {
            self.undone_marks.push(mark);
        }
    }

    fn redo_mark(&mut self) {
        if let Some(mark) = self.undone_marks.pop() {
            self.marks.push(mark);
        }
    }

    fn header(&mut self, ctx: &egui::Context, title: &str) {
        let fill = ctx.style().visuals.panel_fill;
        egui::TopBottomPanel::top("header")
            .exact_height(48.)
            .frame(
                egui::Frame::new()
                    .fill(fill)
                    .inner_margin(egui::Margin::symmetric(12, 0)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.;
                    let preview = self.page == Page::Preview;
                    if preview {
                        let size = Vec2::new(34., 32.);
                        let can_undo = !self.marks.is_empty() || self.pending.is_some();
                        if kit::icon_btn_sized(
                            ui,
                            Icon::Undo,
                            "Отменить (Ctrl+Z)",
                            Look::Plain,
                            can_undo,
                            size,
                        )
                        .clicked()
                        {
                            self.undo_mark();
                        }
                        if kit::icon_btn_sized(
                            ui,
                            Icon::Redo,
                            "Повторить (Ctrl+Y)",
                            Look::Plain,
                            !self.undone_marks.is_empty(),
                            size,
                        )
                        .clicked()
                        {
                            self.redo_mark();
                        }
                    }
                    let controls_width = if preview { 124. } else { 84. };
                    let r = ui.add_sized(
                        [(ui.available_width() - controls_width).max(0.), 34.],
                        egui::Label::new(egui::RichText::new(title).strong().size(14.5))
                            .halign(egui::Align::Min)
                            .truncate()
                            .sense(Sense::click_and_drag()),
                    );
                    if r.drag_started() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }
                    if r.double_clicked() {
                        let max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
                    }
                    if preview
                        && kit::icon_btn_sized(
                            ui,
                            Icon::Gear,
                            "Настройки",
                            Look::Plain,
                            true,
                            Vec2::new(34., 32.),
                        )
                        .clicked()
                    {
                        self.settings_open = true;
                    }
                    if kit::icon_btn_sized(
                        ui,
                        Icon::Maximize,
                        "Развернуть или восстановить",
                        Look::Plain,
                        true,
                        Vec2::new(34., 32.),
                    )
                    .clicked()
                    {
                        let max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
                    }
                    if kit::icon_btn_sized(
                        ui,
                        Icon::Close,
                        "Закрыть",
                        Look::Danger,
                        true,
                        Vec2::new(34., 32.),
                    )
                    .clicked()
                    {
                        self.show(Page::Hidden, ctx);
                    }
                });
            });
    }
    fn settings_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let before = self.settings.clone();
        self.settings_section.show(
            ui,
            &mut self.settings,
            &self.paths.data_dir,
            &mut self.notice,
        );
        if before != self.settings {
            let changed_integration = before.autostart != self.settings.autostart
                || before.intercept_print_screen != self.settings.intercept_print_screen;
            let result = if changed_integration {
                Integration::apply(&self.paths, &mut self.settings)
            } else {
                self.settings.save(&self.paths)
            };
            if let Err(error) = result {
                self.notice = format!("Не удалось применить настройки: {error}");
                self.paths.log(&self.notice);
                self.settings = AppSettings::load(&self.paths).unwrap_or(before);
            }
            self.update_print_screen();
            if let Err(error) = self.tray.set_visible(self.settings.show_tray_icon) {
                self.notice = format!("Не удалось обновить значок в трее: {error}");
            }
            self.theme(ctx);
        }
    }
    fn overlay(&mut self, ctx: &egui::Context) {
        ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
        self.upload(ctx);
        let Some(c) = &self.capture else {
            return;
        };
        let dimensions = Vec2::new(c.image.width() as f32, c.image.height() as f32);
        let mr = monitor_at_cursor(c);
        let toolbar_monitor = self.toolbar_monitor;
        let mut finish = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                let scale = rect.size() / dimensions;
                let map = |p: Pos2| {
                    let pixel = (p - rect.min) / scale;
                    Pos2::new(
                        pixel.x.clamp(0., dimensions.x),
                        pixel.y.clamp(0., dimensions.y),
                    )
                };
                let response = ui.allocate_rect(rect, Sense::click_and_drag());
                let response = response.on_hover_cursor(egui::CursorIcon::Crosshair);
                let primary = egui::PointerButton::Primary;
                let painter = ui.painter_at(rect);
                if let Some(texture) = &self.texture {
                    painter.image(
                        texture.id(),
                        rect,
                        Rect::from_min_max(Pos2::ZERO, Pos2::new(1., 1.)),
                        Color32::WHITE,
                    );
                }
                painter.rect_filled(rect, 0., Color32::from_black_alpha(96));
                for i in 0..24 {
                    let inset = i as f32 * 4.;
                    let r = rect.shrink(inset);
                    painter.rect_stroke(
                        r,
                        0.,
                        Stroke::new(8.0_f32, Color32::from_black_alpha((24 - i) as u8)),
                        egui::StrokeKind::Inside,
                    );
                }
                if self.full {
                    self.selection = Some(mr);
                    if response.clicked_by(primary) {
                        finish = Some(mr);
                    }
                } else {
                    if response.drag_started_by(primary) {
                        self.drag = ctx.input(|i| i.pointer.press_origin()).map(map);
                        self.selection = None;
                    }
                    if (response.dragged_by(primary) || response.drag_stopped_by(primary))
                        && let (Some(a), Some(b)) = (self.drag, response.interact_pointer_pos())
                    {
                        self.selection = Some(Rect::from_two_pos(a, map(b)));
                    }
                    if response.drag_stopped_by(primary) {
                        finish = self
                            .selection
                            .take()
                            .filter(|r| r.width() >= 5. && r.height() >= 5.);
                        self.drag = None;
                    }
                }
                if let Some(r) = self.selection {
                    let dest = Rect::from_min_max(
                        rect.min + r.min.to_vec2() * scale,
                        rect.min + r.max.to_vec2() * scale,
                    );
                    if let Some(t) = &self.texture {
                        painter.image(
                            t.id(),
                            dest,
                            Rect::from_min_max(
                                Pos2::new(r.left() / dimensions.x, r.top() / dimensions.y),
                                Pos2::new(r.right() / dimensions.x, r.bottom() / dimensions.y),
                            ),
                            Color32::WHITE,
                        );
                    }
                    painter.rect_stroke(
                        dest,
                        0.,
                        Stroke::new(2.0_f32, Color32::from_rgb(54, 118, 232)),
                        egui::StrokeKind::Inside,
                    );
                    painter.text(
                        dest.min + Vec2::new(6., 6.),
                        egui::Align2::LEFT_TOP,
                        format!("{} × {}", r.width() as u32, r.height() as u32),
                        egui::FontId::proportional(13.),
                        Color32::WHITE,
                    );
                }
                let toolbar = rect.min
                    + Vec2::new(toolbar_monitor.center().x, toolbar_monitor.top()) * scale
                    + Vec2::new(0., 18.);
                egui::Area::new("overlay tools".into())
                    .anchor(
                        egui::Align2::CENTER_TOP,
                        toolbar - ctx.screen_rect().center_top(),
                    )
                    .order(egui::Order::Foreground)
                    .show(ctx, |ui| {
                        kit::glass(ui).show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 6.;
                                let region = if self.full {
                                    Look::Plain
                                } else {
                                    Look::Selected
                                };
                                if kit::icon_btn(ui, Icon::Region, "Выделить область", region, true)
                                    .clicked()
                                {
                                    self.full = false;
                                    self.selection = None;
                                }
                                if kit::icon_btn(ui, Icon::Screen, "Весь экран", Look::Plain, true)
                                    .clicked()
                                {
                                    finish = Some(toolbar_monitor);
                                }
                                kit::vsep(ui);
                                if kit::icon_btn(
                                    ui,
                                    Icon::Close,
                                    "Отмена (Esc)",
                                    Look::Danger,
                                    true,
                                )
                                .clicked()
                                {
                                    self.show(Page::Hidden, ctx);
                                }
                            });
                        });
                    });
                if ctx.input(|i| i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Space))
                {
                    finish = Some(mr);
                }
                if ctx.input(|i| i.pointer.secondary_pressed()) {
                    self.show(Page::Hidden, ctx);
                }
            });
        if self.page == Page::Overlay
            && let Some(r) = finish
        {
            self.crop(r, ctx);
        }
    }
    fn preview(&mut self, ctx: &egui::Context) {
        let title = self.image.as_ref().map_or_else(
            || "Скриншот".into(),
            |image| format!("Скриншот · {} × {}", image.width(), image.height()),
        );
        self.header(ctx, &title);
        let max_w = (ctx.screen_rect().width() - 32.).max(300.);
        let tools = egui::Area::new("preview tools".into())
            .anchor(egui::Align2::CENTER_TOP, Vec2::new(0., 56.))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.set_max_width(max_w);
                kit::glass(ui).show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = Vec2::new(6., 6.);
                        for (t, icon, tip) in [
                            (Tool::Cursor, Icon::Cursor, "Курсор"),
                            (Tool::Arrow, Icon::Arrow, "Стрелка"),
                            (Tool::Rectangle, Icon::Rectangle, "Рамка"),
                            (Tool::Highlighter, Icon::Highlighter, "Выделитель"),
                            (Tool::Pen, Icon::Pen, "Маркер"),
                        ] {
                            let look = if self.tool == t {
                                Look::Selected
                            } else {
                                Look::Plain
                            };
                            if kit::icon_btn(ui, icon, tip, look, true).clicked() {
                                self.tool = t;
                                self.pending = None;
                            }
                        }
                        kit::vsep(ui);
                        ui.spacing_mut().slider_width = 84.;
                        ui.add(
                            egui::Slider::new(&mut self.width, 2.0..=20.0)
                                .integer()
                                .show_value(false),
                        )
                        .on_hover_text("Толщина линии");
                        let (dot, _) = ui.allocate_exact_size(Vec2::splat(26.), Sense::hover());
                        let width = if self.tool == Tool::Highlighter {
                            (self.width * 2.4).max(8.)
                        } else {
                            self.width
                        };
                        ui.painter()
                            .circle_filled(dot.center(), (width / 2.).min(12.), self.color);
                        kit::vsep(ui);
                        for (c, name) in kit::PALETTE {
                            if kit::swatch(ui, c, name, self.color == c).clicked() {
                                self.color = c;
                            }
                        }
                        egui::color_picker::color_edit_button_srgba(
                            ui,
                            &mut self.color,
                            egui::color_picker::Alpha::Opaque,
                        );
                        if let Some(mark) = &mut self.pending {
                            mark.color = self.color;
                            mark.width = width;
                        }
                    });
                });
            });
        let ocr = egui::Area::new("preview OCR".into())
            .anchor(egui::Align2::LEFT_BOTTOM, Vec2::new(16., -18.))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                kit::glass(ui).show(ui, |ui| {
                    let busy = self.ocr_rx.is_some();
                    let (label, icon) = match (busy, self.words.is_empty(), self.ocr_visible) {
                        (true, _, _) => ("Распознавание…", Icon::Spinner),
                        (_, true, _) => ("Распознать", Icon::Scan),
                        (_, _, true) => ("Скрыть текст", Icon::Scan),
                        _ => ("Показать текст", Icon::Scan),
                    };
                    let action = ui
                        .add_enabled_ui(!busy, |ui| {
                            kit::seg_button(ui, label, Some(icon), self.ocr_visible)
                        })
                        .inner;
                    if action.clicked() {
                        self.toggle_ocr();
                    }
                });
            });
        let actions = egui::Area::new("preview actions".into())
            .anchor(egui::Align2::RIGHT_BOTTOM, Vec2::new(-16., -18.))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                kit::glass(ui).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.;
                        if kit::icon_btn(ui, Icon::Copy, "Копировать (Ctrl+C)", Look::Plain, true)
                            .clicked()
                        {
                            self.copy_image(true, ctx);
                        }
                        if kit::icon_btn(
                            ui,
                            Icon::Save,
                            "Сохранить (Enter, Space или Ctrl+S)",
                            Look::Accent,
                            true,
                        )
                        .clicked()
                        {
                            self.save(true, ctx);
                        }
                        if kit::icon_btn(
                            ui,
                            Icon::Delete,
                            "Удалить снимок (Esc)",
                            Look::Danger,
                            true,
                        )
                        .clicked()
                        {
                            self.show(Page::Hidden, ctx);
                        }
                    });
                });
            });
        let mut bottom = actions.response.rect.top().min(ocr.response.rect.top());
        if !self.notice.is_empty() {
            let notice = egui::Area::new("preview notice".into())
                .anchor(
                    egui::Align2::CENTER_BOTTOM,
                    Vec2::new(0., bottom - ctx.screen_rect().bottom() - 8.),
                )
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    ui.set_max_width(max_w);
                    kit::toast(ui, &self.notice);
                });
            bottom = bottom.min(notice.response.rect.top());
        }
        self.upload(ctx);
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .inner_margin(16)
                    .fill(if ctx.style().visuals.dark_mode {
                        Color32::from_rgb(14, 17, 23)
                    } else {
                        Color32::from_rgb(220, 228, 240)
                    }),
            )
            .show(ctx, |ui| {
                let Some(image) = &self.image else {
                    return;
                };
                let dimensions = Vec2::new(image.width() as f32, image.height() as f32);
                let mut viewport = ui.max_rect();
                viewport.min.y = viewport.min.y.max(tools.response.rect.bottom() + 8.);
                viewport.max.y = viewport.max.y.min(bottom - 8.);
                if viewport.height() <= 0. {
                    return;
                }
                let fit = (viewport.width() / dimensions.x)
                    .min(viewport.height() / dimensions.y)
                    .clamp(0.001, 1.0 / ctx.pixels_per_point());
                let response = ui.allocate_rect(viewport, Sense::click_and_drag());
                let primary = egui::PointerButton::Primary;
                let old_scale = fit * self.zoom;
                let old_origin = viewport.center() - dimensions * old_scale / 2. + self.pan;
                if response.hovered() {
                    let scroll = ctx.input(|i| i.raw_scroll_delta.y);
                    if scroll != 0. {
                        let next = (self.zoom * (scroll * 0.002).exp()).clamp(1., 32.);
                        if let Some(p) = ctx.pointer_hover_pos() {
                            let pixel = (p - old_origin) / old_scale;
                            self.pan = p - viewport.center() + dimensions * (fit * next) / 2.
                                - pixel * (fit * next);
                        }
                        self.zoom = next;
                        if next == 1. {
                            self.pan = Vec2::ZERO;
                        }
                    }
                }
                let scale = fit * self.zoom;
                if self.tool == Tool::Cursor
                    && response.dragged_by(primary)
                    && (!self.ocr_visible || ctx.input(|i| i.modifiers.alt))
                {
                    self.pan += response.drag_delta();
                }
                let limit = ((dimensions * scale - viewport.size()) / 2.).max(Vec2::ZERO);
                self.pan = self.pan.clamp(-limit, limit);
                let origin = viewport.center() - dimensions * scale / 2. + self.pan;
                let rect = Rect::from_min_size(origin, dimensions * scale);
                ui.painter_at(viewport).rect_stroke(
                    rect.expand(1.),
                    1,
                    Stroke::new(1_f32, ui.visuals().widgets.noninteractive.bg_stroke.color),
                    egui::StrokeKind::Outside,
                );
                let painter = ui.painter_at(viewport.intersect(rect));
                if let Some(t) = &self.texture {
                    painter.image(
                        t.id(),
                        rect,
                        Rect::from_min_max(Pos2::ZERO, Pos2::new(1., 1.)),
                        Color32::WHITE,
                    );
                }
                let map = |p: Pos2| {
                    Pos2::new(
                        ((p.x - origin.x) / scale).clamp(0., dimensions.x),
                        ((p.y - origin.y) / scale).clamp(0., dimensions.y),
                    )
                };
                let point = response
                    .interact_pointer_pos()
                    .or_else(|| ctx.pointer_hover_pos())
                    .map(map);
                let press = ctx
                    .input(|i| i.pointer.press_origin())
                    .filter(|p| rect.contains(*p))
                    .map(map);
                if self.tool != Tool::Cursor {
                    if let Some(mark) =
                        update_mark_drag(&response, press, point, &mut self.pending, |start| {
                            Mark::new(self.tool, start, self.color, self.width)
                        })
                    {
                        self.marks.push(mark);
                        self.undone_marks.clear();
                    }
                } else if self.ocr_visible {
                    if response.secondary_clicked() {
                        self.drag = None;
                    }
                    self.select_text(ctx, &response, point, press, rect, origin, scale, &painter);
                }
                for mark in self.marks.iter().chain(self.pending.iter()) {
                    mark.paint(&painter, origin, scale);
                }
                for (i, w) in self.words.iter().enumerate().filter(|_| self.ocr_visible) {
                    painter.rect_filled(
                        Rect::from_min_size(
                            origin + Vec2::new(w.x, w.y) * scale,
                            Vec2::new(w.w, w.h) * scale,
                        ),
                        2.,
                        Color32::from_rgba_unmultiplied(
                            54,
                            118,
                            232,
                            if self.selected.contains(&i) { 100 } else { 25 },
                        ),
                    );
                }
            });
        if self.settings_open {
            let mut open = true;
            egui::Window::new("Настройки · ThodShot")
                .open(&mut open)
                .order(egui::Order::Foreground)
                .default_size([640., 462.])
                .vscroll(true)
                .show(ctx, |ui| self.settings_ui(ui, ctx));
            self.settings_open = open;
        }
    }

    fn toggle_ocr(&mut self) {
        if !self.words.is_empty() {
            self.ocr_visible = !self.ocr_visible;
            self.selected.clear();
            self.notice = if self.ocr_visible {
                self.tool = Tool::Cursor;
                self.pending = None;
                "Выделите текст мышью · Ctrl+C — копировать".into()
            } else {
                String::new()
            };
        } else if let Some(img) = self.image.clone() {
            let language = self.settings.ocr_language.clone();
            let (tx, rx) = mpsc::channel();
            self.ocr_rx = Some(rx);
            std::thread::spawn(move || {
                let _ = tx.send(ocr::recognize(img, language));
            });
        }
    }

    fn select_text(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        point: Option<Pos2>,
        press: Option<Pos2>,
        image_rect: Rect,
        origin: Pos2,
        scale: f32,
        painter: &egui::Painter,
    ) {
        let primary = egui::PointerButton::Primary;
        let (extend, pan) = ctx.input(|i| (i.modifiers.ctrl, i.modifiers.alt));
        let hit = |p| {
            self.words
                .iter()
                .position(|w| word_rect(w).expand(2.).contains(p))
        };
        if !pan && response.clicked_by(primary) {
            if !extend {
                self.selected.clear();
            }
            if response.triple_clicked() {
                self.selected.extend(0..self.words.len());
            } else if response
                .interact_pointer_pos()
                .is_some_and(|p| image_rect.contains(p))
                && let Some(index) = point.and_then(hit)
            {
                self.selected.insert(index);
                if response.double_clicked() {
                    let line = self.words[index].line;
                    self.selected.extend(
                        self.words
                            .iter()
                            .enumerate()
                            .filter(|(_, word)| word.line == line)
                            .map(|(i, _)| i),
                    );
                }
            }
        }
        if response.drag_started_by(primary) {
            self.drag = if pan { None } else { press };
            self.selection_base = if extend {
                self.selected.clone()
            } else {
                BTreeSet::new()
            };
        }
        if (response.dragged_by(primary) || response.drag_stopped_by(primary))
            && let (Some(start), Some(end)) = (self.drag, point)
        {
            self.selected.clone_from(&self.selection_base);
            if let Some(first) = hit(start) {
                let last = hit(end).unwrap_or(first);
                self.selected.extend(first.min(last)..=first.max(last));
            } else {
                let selection = Rect::from_two_pos(start, end);
                self.selected.extend(
                    self.words
                        .iter()
                        .enumerate()
                        .filter(|(_, word)| selection.intersects(word_rect(word)))
                        .map(|(i, _)| i),
                );
                painter.rect_stroke(
                    Rect::from_min_max(
                        origin + selection.min.to_vec2() * scale,
                        origin + selection.max.to_vec2() * scale,
                    ),
                    0.,
                    Stroke::new(1_f32, Color32::LIGHT_BLUE),
                    egui::StrokeKind::Inside,
                );
            }
        }
        if response.drag_stopped_by(primary) {
            self.drag = None;
        }
        response.context_menu(|ui| {
            if ui
                .add_enabled(
                    !self.selected.is_empty(),
                    egui::Button::new("Копировать текст"),
                )
                .clicked()
            {
                self.copy_text();
                ui.close_menu();
            }
            if ui.button("Выделить всё").clicked() {
                self.selected.extend(0..self.words.len());
                ui.close_menu();
            }
        });
    }
}

fn update_mark_drag(
    response: &egui::Response,
    start: Option<Pos2>,
    point: Option<Pos2>,
    pending: &mut Option<Mark>,
    begin: impl FnOnce(Pos2) -> Mark,
) -> Option<Mark> {
    let primary = egui::PointerButton::Primary;
    let (cancel, pressed, released, held) = response.ctx.input(|i| {
        (
            i.pointer.secondary_pressed(),
            i.pointer.primary_pressed(),
            i.pointer.primary_released(),
            i.pointer.primary_down(),
        )
    });
    if cancel {
        *pending = None;
        return None;
    }
    if pressed
        && response.is_pointer_button_down_on()
        && let Some(start) = start
    {
        *pending = Some(begin(start));
    }
    if (held || released)
        && let (Some(mark), Some(end)) = (pending.as_mut(), point)
    {
        mark.update(end);
    }
    if released {
        return pending.take().filter(|mark| {
            (mark.tool == Tool::Arrow || response.drag_stopped_by(primary)) && mark.is_complete()
        });
    }
    if !held {
        *pending = None;
    }
    None
}

fn take_preview_save_shortcut(ctx: &egui::Context) -> bool {
    ctx.input_mut(|i| {
        i.modifiers.is_none()
            && (i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                | i.consume_key(egui::Modifiers::NONE, egui::Key::Space))
    })
}

#[cfg(test)]
mod preview_tests {
    use super::*;

    #[test]
    fn arrow_drag_uses_press_and_release_positions_and_can_be_cancelled() {
        let ctx = egui::Context::default();
        let start = Pos2::new(40., 40.);
        let middle = Pos2::new(70., 64.);
        let end = Pos2::new(110., 100.);
        let button = |pos, button, pressed| egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let primary = egui::PointerButton::Primary;
        let secondary = egui::PointerButton::Secondary;
        let mut pending: Option<Mark> = None;
        let mut frame = |events| {
            let mut committed = None;
            let _ = ctx.run(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let response = ui.allocate_rect(ui.max_rect(), Sense::click_and_drag());
                        let origin = ctx.input(|i| i.pointer.press_origin());
                        let point = response
                            .interact_pointer_pos()
                            .or_else(|| ctx.pointer_hover_pos());
                        committed =
                            update_mark_drag(&response, origin, point, &mut pending, |start| {
                                Mark::new(Tool::Arrow, start, Color32::RED, 6.)
                            });
                    });
                },
            );
            committed
        };
        frame(vec![egui::Event::PointerMoved(start)]);
        assert!(frame(vec![button(start, primary, true)]).is_none());
        assert!(frame(vec![egui::Event::PointerMoved(middle)]).is_none());
        let arrow = frame(vec![button(end, primary, false)]).expect("arrow on release");
        assert_eq!(arrow.points, [start, end]);

        frame(vec![egui::Event::PointerMoved(start)]);
        frame(vec![button(start, primary, true)]);
        frame(vec![egui::Event::PointerMoved(middle)]);
        assert!(frame(vec![button(middle, secondary, true)]).is_none());
        frame(vec![button(middle, secondary, false)]);
        frame(vec![egui::Event::PointerMoved(end)]);
        assert!(frame(vec![button(end, primary, false)]).is_none());
    }

    #[test]
    fn enter_and_space_are_consumed_even_with_widget_focus() {
        for key in [egui::Key::Enter, egui::Key::Space] {
            let ctx = egui::Context::default();
            let input = egui::RawInput {
                events: vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("tool")));
                assert!(take_preview_save_shortcut(ctx));
                assert!(!ctx.input(|i| i.key_pressed(key)));
            });
        }
    }
}

fn word_rect(word: &ocr::Word) -> Rect {
    Rect::from_min_size(Pos2::new(word.x, word.y), Vec2::new(word.w, word.h))
}

fn copy_to_clipboard(image: &RgbaImage) -> Result<()> {
    arboard::Clipboard::new()?.set_image(arboard::ImageData {
        width: image.width() as usize,
        height: image.height() as usize,
        bytes: std::borrow::Cow::Borrowed(image.as_raw()),
    })?;
    Ok(())
}

fn write_image(image: &RgbaImage, path: &std::path::Path) -> Result<()> {
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let result = (|| -> Result<()> {
        if path.extension().is_some_and(|extension| extension == "jpg") {
            let file = std::fs::File::create(&temporary)?;
            let rgb = image::DynamicImage::ImageRgba8(image.clone()).into_rgb8();
            image::codecs::jpeg::JpegEncoder::new_with_quality(file, 92).encode_image(&rgb)?;
        } else {
            image.save_with_format(&temporary, image::ImageFormat::Png)?;
        }
        std::fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        let interval = if self.page == Page::Hidden { 75 } else { 30 };
        ctx.request_repaint_after(Duration::from_millis(interval));
        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.show(Page::Hidden, ctx);
        }
        let [
            capture_requested,
            settings_requested,
            reload_requested,
            exit_requested,
        ] = self.events.take();
        let mut capture = capture_requested;
        let mut settings = settings_requested;
        while let Ok(event) = tray_icon::menu::MenuEvent::receiver().try_recv() {
            if event.id == self.tray_capture {
                capture = true;
            }
            if event.id == self.tray_settings {
                settings = true;
            }
            if event.id == self.tray_exit {
                self.quitting = true;
            }
        }
        while let Ok(event) = tray_icon::TrayIconEvent::receiver().try_recv() {
            if matches!(
                event,
                tray_icon::TrayIconEvent::Click {
                    button: tray_icon::MouseButton::Left,
                    button_state: tray_icon::MouseButtonState::Up,
                    ..
                }
            ) {
                capture = true;
            }
        }
        if reload_requested {
            if let Ok(s) = AppSettings::load(&self.paths) {
                self.settings = s;
                self.update_print_screen();
                let _ = self.tray.set_visible(self.settings.show_tray_icon);
                self.theme(ctx);
            }
        }
        if exit_requested || self.quitting {
            self.quitting = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        let escape = ctx.input(|i| {
            i.events.iter().any(|event| {
                matches!(
                    event,
                    egui::Event::Key {
                        key: egui::Key::Escape,
                        pressed: true,
                        repeat: false,
                        ..
                    }
                )
            })
        });
        let overlay_blurred = if self.page == Page::Overlay {
            let foreground = unsafe { GetForegroundWindow() } == self.window;
            self.overlay_was_foreground |= foreground;
            self.overlay_was_foreground
                && !foreground
                && self.overlay_opened.elapsed() > Duration::from_millis(700)
        } else {
            false
        };
        if escape && self.settings_open {
            self.settings_open = false;
        } else if (escape && self.page != Page::Hidden) || overlay_blurred {
            self.show(Page::Hidden, ctx);
        }
        let capture_while_overlay = capture && self.page == Page::Overlay;
        if std::mem::take(&mut self.capture_after_hide) {
            self.request_capture(ctx, true);
        }
        if capture && !capture_while_overlay {
            self.start_capture(ctx);
        }
        if settings
            && self.page != Page::Overlay
            && self.capture_rx.is_none()
            && !self.capture_after_hide
        {
            if self.page == Page::Preview {
                self.settings_open = true;
            } else {
                self.show(Page::Settings, ctx);
            }
        }
        if let Some(rx) = &self.capture_rx {
            match rx.try_recv() {
                Ok(Ok(capture)) => {
                    self.capture_rx = None;
                    self.accept_capture(capture, ctx);
                    ctx.request_repaint();
                    return;
                }
                Ok(Err(error)) => {
                    self.capture_rx = None;
                    self.capture_error(ctx, error);
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.capture_rx = None;
                    self.capture_error(ctx, anyhow::anyhow!("Поток захвата остановился"));
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if let Some(rx) = &self.ocr_rx {
            if let Ok(result) = rx.try_recv() {
                self.ocr_rx = None;
                match result {
                    Ok(words) => {
                        self.notice = if words.is_empty() {
                            "Текст не найден".into()
                        } else {
                            "Выделите текст мышью · Ctrl+C — копировать".into()
                        };
                        self.words = words;
                        self.selected.clear();
                        self.ocr_visible = !self.words.is_empty();
                        self.pending = None;
                        self.tool = Tool::Cursor;
                    }
                    Err(e) => self.notice = e.to_string(),
                }
            }
        }
        if self.page == Page::Preview && !self.settings_open {
            if ctx.input(|i| {
                i.modifiers.ctrl
                    && (i.key_pressed(egui::Key::Y)
                        || (i.modifiers.shift && i.key_pressed(egui::Key::Z)))
            }) {
                self.redo_mark();
            } else if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Z)) {
                self.undo_mark();
            }
            if self.ocr_visible && ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::A)) {
                self.selected.extend(0..self.words.len());
            }
            if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::C)) {
                if self.selected.is_empty() {
                    self.copy_image(false, ctx);
                } else {
                    self.copy_text();
                }
            }
            if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::S)) {
                self.save(false, ctx);
            }
            if take_preview_save_shortcut(ctx) {
                self.save(true, ctx);
            }
        }
        match self.page {
            Page::Hidden => {}
            Page::Settings => {
                self.header(ctx, "ThodShot   /   Настройки");
                egui::CentralPanel::default()
                    .frame(egui::Frame::new().fill(ctx.style().visuals.panel_fill))
                    .show(ctx, |ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            self.settings_ui(ui, ctx);
                            if !self.notice.is_empty() {
                                ui.label(&self.notice);
                            }
                        });
                    });
            }
            Page::Overlay => self.overlay(ctx),
            Page::Preview => self.preview(ctx),
        }
        if capture_while_overlay {
            // A key press can arrive in the same frame as a click that cancels selection.
            if self.page == Page::Hidden {
                self.capture_after_hide = true;
                ctx.request_repaint();
            } else if self.page == Page::Overlay {
                let _ = unsafe { SetForegroundWindow(self.window) };
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
        }
    }
}
