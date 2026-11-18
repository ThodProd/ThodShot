use crate::{
    annotation::{self, Mark, Tool},
    capture::{self, Capture},
    hook::Hook,
    integration::Integration,
    ipc, ocr,
    settings::{AppSettings, Paths},
    settings_view::{self, Section},
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
    Foundation::{HWND, POINT},
    Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint},
    UI::WindowsAndMessaging::{GetCursorPos, HWND_TOPMOST, SWP_SHOWWINDOW, SetWindowPos},
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
        Box::new(move |cc| Ok(Box::new(App::new(cc, paths, settings, command, events)?))),
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
struct App {
    window: HWND,
    paths: Paths,
    settings: AppSettings,
    page: Page,
    hook: Hook,
    events: [ipc::Listener; 4],
    tray: tray_icon::TrayIcon,
    tray_capture: tray_icon::menu::MenuId,
    tray_settings: tray_icon::menu::MenuId,
    tray_exit: tray_icon::menu::MenuId,
    capture_rx: Option<Receiver<Result<Capture>>>,
    capture_due: Option<Instant>,
    capture: Option<Capture>,
    image: Option<RgbaImage>,
    texture: Option<egui::TextureHandle>,
    marks: Vec<Mark>,
    pending: Option<Mark>,
    tool: Tool,
    color: Color32,
    width: f32,
    selection: Option<Rect>,
    drag: Option<Pos2>,
    toolbar_monitor: Rect,
    overlay_opened: Instant,
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
    ) -> Result<Self> {
        let mut fonts = egui::FontDefinitions::default();
        if let Some(bytes) = std::env::var_os("WINDIR")
            .and_then(|dir| std::fs::read(PathBuf::from(dir).join("Fonts/segoeui.ttf")).ok())
        {
            fonts
                .font_data
                .insert("Segoe".into(), egui::FontData::from_owned(bytes).into());
            fonts
                .families
                .get_mut(&egui::FontFamily::Proportional)
                .unwrap()
                .insert(0, "Segoe".into());
        }
        cc.egui_ctx.set_fonts(fonts);
        let hook = Hook::new()?;
        hook.enable(settings.intercept_print_screen);
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
            capture_rx: None,
            capture_due: None,
            capture: None,
            image: None,
            texture: None,
            marks: vec![],
            pending: None,
            tool: Tool::Cursor,
            color: Color32::from_rgb(242, 59, 48),
            width: 6.,
            selection: None,
            drag: None,
            toolbar_monitor: Rect::NOTHING,
            overlay_opened: Instant::now(),
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
        v.window_fill = if dark {
            Color32::from_rgb(30, 35, 45)
        } else {
            Color32::WHITE
        };
        v.window_corner_radius = 12.into();
        v.menu_corner_radius = 10.into();
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
            widget.corner_radius = 7.into();
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
        self.page = page;
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
    fn start_capture(&mut self, ctx: &egui::Context) {
        if self.page == Page::Overlay || self.capture_rx.is_some() || self.capture_due.is_some() {
            return;
        }
        self.show(Page::Hidden, ctx);
        self.capture_due = Some(Instant::now() + Duration::from_millis(160));
        self.notice.clear();
    }
    fn accept_capture(&mut self, capture: Capture, ctx: &egui::Context) {
        self.texture = None;
        self.image = None;
        self.marks.clear();
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
    fn header(&mut self, ctx: &egui::Context, title: &str) {
        egui::TopBottomPanel::top("header")
            .exact_height(44.)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(10.);
                    let r = ui.add_sized(
                        [(ui.available_width() - 86.).max(0.), 34.],
                        egui::Label::new(egui::RichText::new(title).strong())
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
                    if ui
                        .add_sized([32., 28.], egui::Button::new("□"))
                        .on_hover_text("Развернуть или восстановить")
                        .clicked()
                    {
                        let max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
                    }
                    if ui
                        .add_sized([32., 28.], egui::Button::new("×"))
                        .on_hover_text("Закрыть")
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
            self.hook.enable(self.settings.intercept_print_screen);
            if let Err(error) = self.tray.set_visible(self.settings.show_tray_icon) {
                self.notice = format!("Не удалось обновить значок в трее: {error}");
            }
            self.theme(ctx);
        }
    }
    fn overlay(&mut self, ctx: &egui::Context) {
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
                        egui::Frame::popup(ui.style())
                            .inner_margin(8)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    if ui
                                        .add(settings_view::option_button(
                                            ui,
                                            "Область",
                                            !self.full,
                                        ))
                                        .clicked()
                                    {
                                        self.full = false;
                                        self.selection = None;
                                    }
                                    if ui.button("Весь экран").clicked() {
                                        finish = Some(toolbar_monitor);
                                    }
                                    if ui.button("×").on_hover_text("Отмена (Esc)").clicked()
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
        egui::TopBottomPanel::top("tools").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (t, label) in [
                    (Tool::Cursor, "Курсор"),
                    (Tool::Arrow, "Стрелка"),
                    (Tool::Rectangle, "Прямоугольник"),
                    (Tool::Highlighter, "Выделитель"),
                    (Tool::Pen, "Маркер"),
                ] {
                    let selected = self.tool == t;
                    let response = ui.add(settings_view::option_button(ui, label, selected));
                    response.widget_info(|| {
                        egui::WidgetInfo::selected(
                            egui::WidgetType::SelectableLabel,
                            true,
                            selected,
                            label,
                        )
                    });
                    if response.clicked() {
                        self.tool = t;
                        self.pending = None;
                    }
                }
                ui.separator();
                ui.add(
                    egui::Slider::new(&mut self.width, 2.0..=20.0)
                        .integer()
                        .text("Толщина"),
                );
                let (swatch, _) = ui.allocate_exact_size(Vec2::splat(24.), Sense::hover());
                let width = if self.tool == Tool::Highlighter {
                    (self.width * 2.4).max(8.)
                } else {
                    self.width
                };
                ui.painter()
                    .circle_filled(swatch.center(), (width / 2.).min(12.), self.color);
                egui::color_picker::color_edit_button_srgba(
                    ui,
                    &mut self.color,
                    egui::color_picker::Alpha::Opaque,
                );
                if let Some(mark) = &mut self.pending {
                    mark.color = self.color;
                    mark.width = width;
                }
                if ui.button("↶").on_hover_text("Отменить (Ctrl+Z)").clicked() {
                    if self.pending.take().is_none() {
                        self.marks.pop();
                    }
                }
            });
        });
        egui::TopBottomPanel::bottom("actions").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        self.ocr_rx.is_none(),
                        egui::Button::new(
                            match (
                                self.ocr_rx.is_some(),
                                self.words.is_empty(),
                                self.ocr_visible,
                            ) {
                                (true, _, _) => "Распознавание…",
                                (_, true, _) => "Распознать",
                                (_, _, true) => "Скрыть текст",
                                _ => "Показать текст",
                            },
                        ),
                    )
                    .clicked()
                {
                    if !self.words.is_empty() {
                        self.ocr_visible = !self.ocr_visible;
                        self.selected.clear();
                        self.notice = if self.ocr_visible {
                            "Выделите текст мышью · Ctrl+C — копировать".into()
                        } else {
                            String::new()
                        };
                        if self.ocr_visible {
                            self.tool = Tool::Cursor;
                            self.pending = None;
                        }
                    } else if let Some(img) = self.image.clone() {
                        let language = self.settings.ocr_language.clone();
                        let (tx, rx) = mpsc::channel();
                        self.ocr_rx = Some(rx);
                        std::thread::spawn(move || {
                            let _ = tx.send(ocr::recognize(img, language));
                        });
                    }
                }
                ui.add_space((ui.available_width() - 388.).max(4.));
                if ui.button("Копировать").clicked() {
                    self.copy_image(true, ctx);
                }
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("Сохранить")
                                .strong()
                                .color(Color32::WHITE),
                        )
                        .fill(Color32::from_rgb(42, 106, 210)),
                    )
                    .on_hover_text("Enter, Space или Ctrl+S")
                    .clicked()
                {
                    self.save(true, ctx);
                }
                if ui
                    .add(
                        egui::Button::new(egui::RichText::new("Отменить снимок").color(
                            if ui.visuals().dark_mode {
                                Color32::from_rgb(255, 167, 171)
                            } else {
                                Color32::from_rgb(158, 35, 50)
                            },
                        ))
                        .frame(false),
                    )
                    .on_hover_text("Закрыть предпросмотр (Esc)")
                    .clicked()
                {
                    self.show(Page::Hidden, ctx);
                }
                if ui.button("Настройки").clicked() {
                    self.settings_open = true;
                }
            });
            if !self.notice.is_empty() {
                ui.label(egui::RichText::new(&self.notice).weak());
            }
        });
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
                let viewport = ui.max_rect();
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
                if response.secondary_clicked() {
                    self.pending = None;
                    self.drag = None;
                }
                if self.tool == Tool::Arrow {
                    if let (Some(mark), Some(p)) = (&mut self.pending, point) {
                        mark.update(p);
                    }
                    if response.clicked_by(primary)
                        && response
                            .interact_pointer_pos()
                            .is_some_and(|p| rect.contains(p))
                    {
                        if self.pending.is_some() {
                            self.marks.push(self.pending.take().unwrap());
                        } else if let Some(p) = point {
                            self.pending = Some(Mark::new(self.tool, p, self.color, self.width));
                        }
                    }
                } else if matches!(self.tool, Tool::Rectangle | Tool::Pen | Tool::Highlighter) {
                    if response.drag_started_by(primary) {
                        if let Some(p) = press {
                            self.pending = Some(Mark::new(self.tool, p, self.color, self.width));
                        }
                    }
                    if response.dragged_by(primary) || response.drag_stopped_by(primary) {
                        if let (Some(mark), Some(p)) = (&mut self.pending, point) {
                            mark.update(p);
                        }
                    }
                    if response.drag_stopped_by(primary) {
                        if let Some(mark) = self.pending.take() {
                            self.marks.push(mark);
                        }
                    }
                } else if self.ocr_visible {
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
                .default_size([640., 462.])
                .vscroll(true)
                .show(ctx, |ui| self.settings_ui(ui, ctx));
            self.settings_open = open;
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
        let interval =
            if self.page == Page::Hidden && self.capture_due.is_none() && self.capture_rx.is_none()
            {
                75
            } else {
                30
            };
        ctx.request_repaint_after(Duration::from_millis(interval));
        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.show(Page::Hidden, ctx);
        }
        let mut capture = self.hook.take() | self.events[0].is_signaled();
        let mut settings = self.events[1].is_signaled();
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
        if self.events[2].is_signaled() {
            if let Ok(s) = AppSettings::load(&self.paths) {
                self.settings = s;
                self.hook.enable(self.settings.intercept_print_screen);
                let _ = self.tray.set_visible(self.settings.show_tray_icon);
                self.theme(ctx);
            }
        }
        if self.events[3].is_signaled() || self.quitting {
            self.quitting = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if capture {
            self.start_capture(ctx);
        }
        if settings
            && self.page != Page::Overlay
            && self.capture_rx.is_none()
            && self.capture_due.is_none()
        {
            if self.page == Page::Preview {
                self.settings_open = true;
            } else {
                self.show(Page::Settings, ctx);
            }
        }
        if self.capture_due.is_some_and(|at| Instant::now() >= at) {
            self.capture_due = None;
            let (tx, rx) = mpsc::channel();
            self.capture_rx = Some(rx);
            std::thread::spawn(move || {
                let _ = tx.send(capture::capture());
            });
        }
        if let Some(rx) = &self.capture_rx {
            if let Ok(result) = rx.try_recv() {
                self.capture_rx = None;
                match result {
                    Ok(c) => self.accept_capture(c, ctx),
                    Err(e) => {
                        self.paths.log(&format!("Захват экрана: {e:#}"));
                        self.notice = format!("Не удалось сделать снимок: {e}");
                        self.show(Page::Settings, ctx);
                    }
                }
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
        let escape = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let overlay_blurred = self.page == Page::Overlay
            && self.overlay_opened.elapsed() > Duration::from_millis(700)
            && !ctx.input(|i| i.focused);
        if escape && self.settings_open {
            self.settings_open = false;
        } else if escape || overlay_blurred {
            self.show(Page::Hidden, ctx);
        }
        if self.page == Page::Preview && !self.settings_open {
            if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Z)) {
                if self.pending.take().is_none() {
                    self.marks.pop();
                }
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
            if ctx.input(|i| {
                i.modifiers.is_none()
                    && (i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Space))
            }) && ctx.memory(|memory| memory.focused().is_none())
            {
                self.save(true, ctx);
            }
        }
        match self.page {
            Page::Hidden => {}
            Page::Settings => {
                self.header(ctx, "ThodShot   /   Настройки");
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
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
    }
}
