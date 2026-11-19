use crate::settings::AppSettings;
use eframe::egui::{self, Color32, RichText, Sense, Stroke};
use kit::Icon;
use std::{path::Path, process::Command};

#[derive(Clone, Copy, Default, PartialEq)]
pub enum Section {
    #[default]
    Capture,
    Save,
    App,
    Ocr,
}

impl Section {
    const ALL: [Self; 4] = [Self::Capture, Self::Save, Self::App, Self::Ocr];

    fn title(self) -> &'static str {
        match self {
            Self::Capture => "Съёмка",
            Self::Save => "Сохранение",
            Self::App => "Программа",
            Self::Ocr => "Распознавание текста",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Self::Capture => Icon::Region,
            Self::Save => Icon::Save,
            Self::App => Icon::Gear,
            Self::Ocr => Icon::Scan,
        }
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        settings: &mut AppSettings,
        data_dir: &Path,
        notice: &mut String,
    ) {
        ui.scope(|ui| {
            ui.spacing_mut().button_padding = egui::vec2(12., 7.);
            ui.spacing_mut().item_spacing = egui::vec2(8., 12.);
            let width = ui.available_width();
            if width >= 580. {
                ui.horizontal_top(|ui| {
                    let sidebar_width = 200.;
                    let content_width = width - sidebar_width - ui.spacing().item_spacing.x;
                    let column = egui::Layout::top_down(egui::Align::Min);
                    ui.allocate_ui_with_layout(egui::vec2(sidebar_width, 0.), column, |ui| {
                        ui.set_width(sidebar_width);
                        sidebar(ui, self, data_dir, notice);
                    });
                    ui.allocate_ui_with_layout(egui::vec2(content_width, 0.), column, |ui| {
                        ui.set_width(content_width);
                        egui::Frame::new().inner_margin(24).show(ui, |ui| {
                            self.content(ui, settings, notice);
                        });
                    });
                });
            } else {
                egui::Frame::new().inner_margin(16).show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(6., 6.);
                        for section in Self::ALL {
                            let look = if *self == section {
                                kit::Look::Selected
                            } else {
                                kit::Look::Plain
                            };
                            if kit::icon_btn(ui, section.icon(), section.title(), look, true)
                                .clicked()
                            {
                                *self = section;
                            }
                        }
                    });
                    self.content(ui, settings, notice);
                    footer(ui, data_dir, notice);
                });
            }
        });
    }

    fn content(self, ui: &mut egui::Ui, settings: &mut AppSettings, notice: &mut String) {
        ui.set_min_width(ui.available_width());
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(30., 30.), Sense::hover());
            kit::paint_icon(ui.painter(), self.icon(), rect.shrink(2.), kit::ACCENT, 0.);
            ui.heading(RichText::new(self.title()).strong().size(24.));
        });
        egui::Frame::new()
            .fill(surface(ui))
            .stroke(Stroke::new(
                1_f32,
                ui.visuals().widgets.noninteractive.bg_stroke.color,
            ))
            .corner_radius(16)
            .inner_margin(18)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                match self {
                    Self::Capture => {
                        choice(
                            ui,
                            "Режим по умолчанию",
                            &mut settings.default_mode,
                            &[
                                ("region", "Область", Some(Icon::Region)),
                                ("full", "Весь экран", Some(Icon::Screen)),
                            ],
                        );
                        ui.separator();
                        toggle(
                            ui,
                            &mut settings.intercept_print_screen,
                            "Горячая клавиша PrintScreen",
                            "Открывает выбор области поверх рабочего стола.",
                        );
                    }
                    Self::Save => {
                        folder(ui, settings, notice);
                        ui.separator();
                        choice(
                            ui,
                            "Формат файла",
                            &mut settings.format,
                            &[("png", "PNG", None), ("jpg", "JPG", None)],
                        );
                        ui.separator();
                        toggle(
                            ui,
                            &mut settings.open_folder_after_save,
                            "Открывать Проводник",
                            "Показывает файл после сохранения.",
                        );
                    }
                    Self::App => {
                        choice(
                            ui,
                            "Тема",
                            &mut settings.theme,
                            &[
                                ("dark", "Тёмная", None),
                                ("light", "Светлая", None),
                                ("system", "Системная", None),
                            ],
                        );
                        ui.separator();
                        toggle(
                            ui,
                            &mut settings.autostart,
                            "Запускать вместе с Windows",
                            "PrintScreen доступна сразу после входа.",
                        );
                        ui.separator();
                        toggle(
                            ui,
                            &mut settings.show_tray_icon,
                            "Значок в системном трее",
                            "Даёт быстрый доступ к снимкам и настройкам.",
                        );
                    }
                    Self::Ocr => {
                        choice(
                            ui,
                            "Язык распознавания",
                            &mut settings.ocr_language,
                            &[
                                ("auto", "Авто", None),
                                ("ru", "Русский", None),
                                ("en", "English", None),
                            ],
                        );
                        ui.label(
                            secondary(ui,
                                "Текст распознаётся локально. Для каждого языка требуется пакет распознавания Windows.",
                            ),
                        );
                    }
                }
            });
    }
}

fn sidebar(ui: &mut egui::Ui, active: &mut Section, data_dir: &Path, notice: &mut String) {
    let bottom = ui.clip_rect().bottom();
    let panel = egui::Frame::new()
        .fill(surface(ui))
        .inner_margin(14)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.set_min_height((bottom - ui.cursor().min.y - 14.).max(0.));
            ui.spacing_mut().item_spacing.y = 4.;
            for section in Section::ALL {
                let title = if section == Section::Ocr {
                    "Распознавание"
                } else {
                    section.title()
                };
                if kit::nav_item(ui, section.icon(), title, *active == section).clicked() {
                    *active = section;
                }
            }
            ui.add_space((bottom - ui.cursor().min.y - 57.).max(12.));
            footer(ui, data_dir, notice);
        });
    let rect = panel.response.rect;
    ui.painter().line_segment(
        [rect.right_top(), rect.right_bottom()],
        ui.visuals().widgets.noninteractive.bg_stroke,
    );
}

fn footer(ui: &mut egui::Ui, data_dir: &Path, notice: &mut String) {
    ui.vertical(|ui| {
        ui.label(secondary(ui, concat!("ThodShot ", env!("CARGO_PKG_VERSION"))).small());
        if ui
            .link("Папка данных")
            .on_hover_text(data_dir.display().to_string())
            .clicked()
            && let Err(error) = Command::new("explorer.exe").arg(data_dir).spawn()
        {
            *notice = format!("Не удалось открыть папку данных: {error}");
        }
    });
}

fn folder(ui: &mut egui::Ui, settings: &mut AppSettings, notice: &mut String) {
    let folder = settings.effective_folder();
    ui.label(RichText::new("Папка для снимков").strong());
    let path = folder.display().to_string();
    ui.add(egui::Label::new(secondary(ui, path.as_str())).wrap())
        .on_hover_text(&path);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(6., 6.);
        if kit::seg_button(ui, "Выбрать папку…", Some(Icon::Folder), false).clicked()
            && let Some(folder) = rfd::FileDialog::new().set_directory(&folder).pick_folder()
        {
            settings.save_folder = folder.display().to_string();
        }
        if kit::seg_button(ui, "Открыть папку", Some(Icon::Folder), false).clicked()
            && let Err(error) = std::fs::create_dir_all(&folder)
                .and_then(|()| Command::new("explorer.exe").arg(&folder).spawn())
        {
            *notice = format!("Не удалось открыть папку: {error}");
        }
    });
}

fn choice(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut String,
    options: &[(&str, &str, Option<Icon>)],
) {
    ui.label(RichText::new(label).strong());
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(6., 6.);
        for &(option, label, icon) in options {
            let selected = value == option;
            let response = kit::seg_button(ui, label, icon, selected);
            response.widget_info(|| {
                egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, label)
            });
            if response.clicked() {
                *value = option.to_owned();
            }
        }
    });
}

fn toggle(ui: &mut egui::Ui, value: &mut bool, label: &str, detail: &str) {
    ui.horizontal_top(|ui| {
        let text_width = (ui.available_width() - 54.).max(56.);
        ui.allocate_ui_with_layout(
            egui::vec2(text_width, 0.),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(text_width);
                ui.spacing_mut().item_spacing.y = 4.;
                ui.label(RichText::new(label).strong());
                ui.label(secondary(ui, detail));
            },
        );
        let (rect, mut response) = ui.allocate_exact_size(egui::vec2(42., 24.), Sense::click());
        if response.clicked() {
            *value = !*value;
            response.mark_changed();
        }
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, *value, label)
        });
        let t = ui.ctx().animate_bool_responsive(response.id, *value);
        let visuals = ui.style().interact(&response);
        let fill = visuals.bg_fill.lerp_to_gamma(kit::ACCENT, t);
        let border = if response.has_focus() {
            ui.visuals().selection.stroke
        } else {
            visuals.bg_stroke
        };
        ui.painter()
            .rect(rect, 12, fill, border, egui::StrokeKind::Inside);
        let thumb_x = egui::lerp((rect.left() + 12.)..=(rect.right() - 12.), t);
        ui.painter()
            .circle_filled(egui::pos2(thumb_x, rect.center().y), 7., Color32::WHITE);
    });
}

pub fn surface(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(30, 35, 45)
    } else {
        Color32::from_rgb(252, 253, 255)
    }
}

fn secondary(ui: &egui::Ui, text: &str) -> RichText {
    RichText::new(text).color(if ui.visuals().dark_mode {
        Color32::from_rgb(164, 175, 193)
    } else {
        Color32::from_rgb(83, 98, 121)
    })
}

/// Небольшой набор виджетов: нарисованные иконки и полупрозрачные «стеклянные» кнопки.
pub mod kit {
    use eframe::egui::{
        self, Align2, Color32, FontId, Frame, Painter, Pos2, Rect, Response, Sense, Shape, Stroke,
        StrokeKind, Ui, Vec2,
    };

    pub const ACCENT: Color32 = Color32::from_rgb(0, 120, 212);

    pub const PALETTE: [(Color32, &str); 7] = [
        (Color32::from_rgb(242, 59, 48), "Красный"),
        (Color32::from_rgb(255, 149, 0), "Оранжевый"),
        (Color32::from_rgb(255, 204, 0), "Жёлтый"),
        (Color32::from_rgb(52, 199, 89), "Зелёный"),
        (Color32::from_rgb(10, 132, 255), "Синий"),
        (Color32::from_rgb(255, 255, 255), "Белый"),
        (Color32::from_rgb(28, 28, 30), "Чёрный"),
    ];

    #[derive(Clone, Copy, PartialEq)]
    pub enum Icon {
        Cursor,
        Arrow,
        Rectangle,
        Highlighter,
        Pen,
        Undo,
        Redo,
        Copy,
        Save,
        Delete,
        Close,
        Maximize,
        Gear,
        Scan,
        Region,
        Screen,
        Folder,
        Spinner,
    }

    #[derive(Clone, Copy, PartialEq)]
    pub enum Look {
        Plain,
        Selected,
        Accent,
        Danger,
    }

    const BRACKETS: [[(f32, f32); 3]; 4] = [
        [(4., 9.), (4., 4.), (9., 4.)],
        [(15., 4.), (20., 4.), (20., 9.)],
        [(4., 15.), (4., 20.), (9., 20.)],
        [(15., 20.), (20., 20.), (20., 15.)],
    ];

    /// Рисует иконку на сетке 24×24, масштабируя её под `rect`.
    pub fn paint_icon(p: &Painter, icon: Icon, rect: Rect, color: Color32, time: f64) {
        let u = rect.width().min(rect.height()) / 24.;
        let origin = rect.center() - Vec2::splat(12. * u);
        let pt = |x: f32, y: f32| origin + Vec2::new(x, y) * u;
        let w = 1.8 * u;
        let st = Stroke::new(w, color);
        let seg = |a: Pos2, b: Pos2| {
            p.line_segment([a, b], st);
            p.circle_filled(a, w / 2., color);
            p.circle_filled(b, w / 2., color);
        };
        let path = |pts: &[(f32, f32)], closed: bool| {
            let v: Vec<Pos2> = pts.iter().map(|&(x, y)| pt(x, y)).collect();
            for &q in &v {
                p.circle_filled(q, w / 2., color);
            }
            p.add(if closed {
                Shape::closed_line(v, st)
            } else {
                Shape::line(v, st)
            });
        };
        match icon {
            Icon::Cursor => path(
                &[
                    (6., 3.),
                    (6., 19.),
                    (10., 15.),
                    (13., 21.),
                    (15.8, 19.8),
                    (12.8, 14.),
                    (18.5, 14.),
                ],
                true,
            ),
            Icon::Arrow => {
                seg(pt(5., 19.), pt(19., 5.));
                path(&[(10., 5.), (19., 5.), (19., 14.)], false);
            }
            Icon::Rectangle => {
                p.rect_stroke(
                    Rect::from_min_max(pt(4.5, 6.5), pt(19.5, 17.5)),
                    2. * u,
                    st,
                    StrokeKind::Middle,
                );
            }
            Icon::Highlighter => {
                path(&[(14.5, 3.5), (20.5, 9.5), (11.5, 18.5), (5.5, 12.5)], true);
                p.line_segment(
                    [pt(4., 21.5), pt(14., 21.5)],
                    Stroke::new(3. * u, color.gamma_multiply(0.55)),
                );
            }
            Icon::Pen => {
                path(
                    &[
                        (15.5, 4.5),
                        (19.5, 8.5),
                        (9.5, 18.5),
                        (4., 20.),
                        (5.5, 14.5),
                    ],
                    true,
                );
                seg(pt(13., 7.), pt(17., 11.));
            }
            Icon::Undo | Icon::Redo => {
                let mirrored = |x, y| pt(if icon == Icon::Redo { 24. - x } else { x }, y);
                let (cx, cy, r) = (12_f32, 13_f32, 7.5_f32);
                let n = 18;
                let v: Vec<(f32, f32)> = (0..=n)
                    .map(|i| {
                        let a = i as f32 / n as f32 * 3.665;
                        (cx + r * a.cos(), cy - r * a.sin())
                    })
                    .collect();
                p.add(Shape::line(
                    v.iter().map(|&(x, y)| mirrored(x, y)).collect(),
                    st,
                ));
                let (ex, ey) = *v.last().unwrap();
                let (dx, dy) = (0.5_f32, 0.866_f32);
                let (bx, by) = (ex - dx * 1.2, ey - dy * 1.2);
                let (nx, ny) = (-dy * 4.2, dx * 4.2);
                p.add(Shape::convex_polygon(
                    vec![
                        mirrored(ex + dx * 3.5, ey + dy * 3.5),
                        mirrored(bx + nx, by + ny),
                        mirrored(bx - nx, by - ny),
                    ],
                    color,
                    Stroke::NONE,
                ));
            }
            Icon::Copy => {
                path(
                    &[(9., 9.), (9., 3.5), (19.5, 3.5), (19.5, 14.), (15., 14.)],
                    false,
                );
                p.rect_stroke(
                    Rect::from_min_max(pt(4.5, 9.), pt(15., 20.5)),
                    2. * u,
                    st,
                    StrokeKind::Middle,
                );
            }
            Icon::Save => {
                path(
                    &[(5., 4.5), (16., 4.5), (19.5, 8.), (19.5, 19.5), (5., 19.5)],
                    true,
                );
                path(&[(8.5, 4.5), (8.5, 9.), (14.5, 9.), (14.5, 4.5)], false);
                path(&[(8., 19.5), (8., 13.5), (16.5, 13.5), (16.5, 19.5)], false);
            }
            Icon::Close => {
                seg(pt(6., 6.), pt(18., 18.));
                seg(pt(18., 6.), pt(6., 18.));
            }
            Icon::Delete => {
                seg(pt(4., 7.), pt(20., 7.));
                path(&[(9., 7.), (9., 4.), (15., 4.), (15., 7.)], false);
                path(&[(6., 7.), (7.5, 20.), (16.5, 20.), (18., 7.)], false);
                seg(pt(10., 10.), pt(10.5, 17.));
                seg(pt(14., 10.), pt(13.5, 17.));
            }
            Icon::Maximize => {
                p.rect_stroke(
                    Rect::from_min_max(pt(5.5, 5.5), pt(18.5, 18.5)),
                    2. * u,
                    st,
                    StrokeKind::Middle,
                );
            }
            Icon::Gear => {
                let c = pt(12., 12.);
                p.circle_stroke(c, 6.2 * u, st);
                p.circle_stroke(c, 2.6 * u, st);
                for i in 0..8 {
                    let d = Vec2::angled(i as f32 * std::f32::consts::FRAC_PI_4);
                    p.line_segment(
                        [c + d * 7.4 * u, c + d * 10. * u],
                        Stroke::new(2.6 * u, color),
                    );
                }
            }
            Icon::Scan => {
                for b in BRACKETS {
                    path(&b, false);
                }
                seg(pt(8.5, 9.5), pt(15.5, 9.5));
                seg(pt(12., 9.5), pt(12., 15.5));
            }
            Icon::Region => {
                for b in BRACKETS {
                    path(&b, false);
                }
            }
            Icon::Screen => {
                p.rect_stroke(
                    Rect::from_min_max(pt(3.5, 5.), pt(20.5, 16.5)),
                    2. * u,
                    st,
                    StrokeKind::Middle,
                );
                seg(pt(9., 20.), pt(15., 20.));
                seg(pt(12., 16.5), pt(12., 20.));
            }
            Icon::Folder => path(
                &[
                    (3.5, 6.5),
                    (9., 6.5),
                    (11., 9.),
                    (20.5, 9.),
                    (20.5, 19.),
                    (3.5, 19.),
                ],
                true,
            ),
            Icon::Spinner => {
                let c = pt(12., 12.);
                let t = time as f32 * 4.;
                let n = 20;
                let pts: Vec<Pos2> = (0..=n)
                    .map(|i| c + Vec2::angled(t + i as f32 / n as f32 * 4.4) * 8. * u)
                    .collect();
                p.add(Shape::line(pts, st));
            }
        }
    }

    fn text_color(dark: bool) -> Color32 {
        if dark {
            Color32::from_gray(236)
        } else {
            Color32::from_gray(36)
        }
    }

    fn base_fill(dark: bool) -> Color32 {
        if dark {
            Color32::from_white_alpha(16)
        } else {
            Color32::from_black_alpha(10)
        }
    }

    fn hover_fill(dark: bool) -> Color32 {
        if dark {
            Color32::from_white_alpha(52)
        } else {
            Color32::from_black_alpha(30)
        }
    }

    fn selected_colors(dark: bool, k: f32) -> (Color32, Color32, Color32) {
        let (a, b) = if dark { (0.40, 0.58) } else { (0.20, 0.34) };
        let fill = ACCENT
            .gamma_multiply(a)
            .lerp_to_gamma(ACCENT.gamma_multiply(b), k);
        let fg = if dark {
            Color32::from_rgb(175, 210, 255)
        } else {
            Color32::from_rgb(0, 84, 168)
        };
        (fill, ACCENT.gamma_multiply(0.85), fg)
    }

    pub fn icon_btn(ui: &mut Ui, icon: Icon, tip: &str, look: Look, enabled: bool) -> Response {
        let size = if look == Look::Accent {
            Vec2::new(48., 38.)
        } else {
            Vec2::splat(38.)
        };
        icon_btn_sized(ui, icon, tip, look, enabled, size)
    }

    pub fn icon_btn_sized(
        ui: &mut Ui,
        icon: Icon,
        tip: &str,
        look: Look,
        enabled: bool,
        size: Vec2,
    ) -> Response {
        let sense = if enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (rect, response) = ui.allocate_exact_size(size, sense);
        let dark = ui.visuals().dark_mode;
        let hot = enabled && response.hovered();
        let down = enabled && response.is_pointer_button_down_on();
        let k = ui.ctx().animate_bool_responsive(response.id, hot);
        let text = text_color(dark);
        let (mut fill, mut stroke, mut fg) = match look {
            Look::Plain => (
                base_fill(dark).lerp_to_gamma(hover_fill(dark), k),
                Color32::TRANSPARENT,
                text,
            ),
            Look::Selected => {
                let (fill, stroke, fg) = selected_colors(dark, k);
                (fill, stroke, fg)
            }
            Look::Accent => (
                ACCENT.lerp_to_gamma(Color32::from_rgb(40, 150, 240), k),
                Color32::TRANSPARENT,
                Color32::WHITE,
            ),
            Look::Danger => (
                base_fill(dark).lerp_to_gamma(Color32::from_rgb(232, 17, 35), k),
                Color32::TRANSPARENT,
                text.lerp_to_gamma(Color32::WHITE, k),
            ),
        };
        if response.has_focus() {
            stroke = ACCENT;
        }
        if down {
            fill = fill.gamma_multiply(0.8);
        }
        if !enabled {
            fg = fg.gamma_multiply(0.35);
            if look == Look::Accent {
                fill = fill.gamma_multiply(0.4);
            }
        }
        let time = ui.input(|i| i.time);
        let painter = ui.painter();
        painter.rect(
            rect,
            11,
            fill,
            Stroke::new(1_f32, stroke),
            StrokeKind::Inside,
        );
        paint_icon(
            painter,
            icon,
            Rect::from_center_size(rect.center(), Vec2::splat(22.)),
            fg,
            time,
        );
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, tip));
        response.on_hover_text(tip.to_owned())
    }

    /// Полупрозрачная панель в стиле «стекла» для плавающих тулбаров.
    pub fn glass(ui: &Ui) -> Frame {
        let dark = ui.visuals().dark_mode;
        Frame::new()
            .fill(if dark {
                Color32::from_rgba_unmultiplied(22, 26, 36, 214)
            } else {
                Color32::from_rgba_unmultiplied(255, 255, 255, 222)
            })
            .stroke(Stroke::new(
                1_f32,
                if dark {
                    Color32::from_white_alpha(30)
                } else {
                    Color32::from_black_alpha(24)
                },
            ))
            .corner_radius(18)
            .inner_margin(egui::Margin::symmetric(10, 8))
            .shadow(egui::Shadow {
                offset: [0, 6],
                blur: 24,
                spread: 0,
                color: Color32::from_black_alpha(90),
            })
    }

    /// Небольшое уведомление над нижней панелью.
    pub fn toast(ui: &mut Ui, text: &str) {
        let dark = ui.visuals().dark_mode;
        Frame::new()
            .fill(if dark {
                Color32::from_rgba_unmultiplied(22, 26, 36, 205)
            } else {
                Color32::from_rgba_unmultiplied(255, 255, 255, 225)
            })
            .stroke(Stroke::new(
                1_f32,
                if dark {
                    Color32::from_white_alpha(26)
                } else {
                    Color32::from_black_alpha(20)
                },
            ))
            .corner_radius(12)
            .inner_margin(egui::Margin::symmetric(12, 6))
            .show(ui, |ui| {
                ui.add(egui::Label::new(egui::RichText::new(text).size(13.)));
            });
    }

    pub fn vsep(ui: &mut Ui) {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(9., 24.), Sense::hover());
        ui.painter().line_segment(
            [rect.center_top(), rect.center_bottom()],
            Stroke::new(1_f32, ui.visuals().widgets.noninteractive.bg_stroke.color),
        );
    }

    /// Круглая цветная точка выбора цвета.
    pub fn swatch(ui: &mut Ui, color: Color32, name: &str, selected: bool) -> Response {
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(26.), Sense::click());
        let c = rect.center();
        let r = if response.hovered() { 10. } else { 9. };
        let painter = ui.painter();
        painter.circle_filled(c, r, color);
        painter.circle_stroke(
            c,
            r,
            Stroke::new(1_f32, Color32::from_gray(128).gamma_multiply(0.6)),
        );
        if selected || response.has_focus() {
            painter.circle_stroke(
                c,
                12.2,
                Stroke::new(2_f32, ui.visuals().strong_text_color()),
            );
        }
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, name)
        });
        response.on_hover_text(name)
    }

    /// Пункт бокового меню: иконка и название.
    pub fn nav_item(ui: &mut Ui, icon: Icon, title: &str, selected: bool) -> Response {
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.), Sense::click());
        let dark = ui.visuals().dark_mode;
        let k = ui
            .ctx()
            .animate_bool_responsive(response.id, response.hovered());
        let (fill, stroke, fg) = if selected {
            selected_colors(dark, k)
        } else {
            (
                Color32::TRANSPARENT.lerp_to_gamma(hover_fill(dark), k),
                Color32::TRANSPARENT,
                text_color(dark),
            )
        };
        let painter = ui.painter();
        painter.rect(
            rect,
            11,
            fill,
            Stroke::new(1_f32, stroke),
            StrokeKind::Inside,
        );
        paint_icon(
            painter,
            icon,
            Rect::from_center_size(rect.left_center() + Vec2::new(22., 0.), Vec2::splat(20.)),
            fg,
            0.,
        );
        painter.text(
            Pos2::new(rect.left() + 42., rect.center().y),
            Align2::LEFT_CENTER,
            title,
            FontId::proportional(14.5),
            fg,
        );
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, title)
        });
        response
    }

    /// Кнопка выбора в настройках: полупрозрачная «таблетка», опционально с иконкой.
    pub fn seg_button(ui: &mut Ui, label: &str, icon: Option<Icon>, selected: bool) -> Response {
        let dark = ui.visuals().dark_mode;
        let (fill_sel, stroke_sel, fg_sel) = selected_colors(dark, 0.);
        let fg = if selected { fg_sel } else { text_color(dark) };
        let galley = ui
            .painter()
            .layout_no_wrap(label.to_owned(), FontId::proportional(14.), fg);
        let icon_width = if icon.is_some() { 26. } else { 0. };
        let size = Vec2::new(galley.size().x + icon_width + 28., 34.);
        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
        let k = ui
            .ctx()
            .animate_bool_responsive(response.id, response.hovered());
        let (fill, stroke) = if selected {
            (fill_sel, stroke_sel)
        } else {
            (
                base_fill(dark).lerp_to_gamma(hover_fill(dark), k),
                Color32::TRANSPARENT,
            )
        };
        let painter = ui.painter();
        painter.rect(
            rect,
            10,
            fill,
            Stroke::new(1_f32, stroke),
            StrokeKind::Inside,
        );
        let mut x = rect.left() + 14.;
        if let Some(icon) = icon {
            paint_icon(
                painter,
                icon,
                Rect::from_center_size(Pos2::new(x + 9., rect.center().y), Vec2::splat(18.)),
                fg,
                0.,
            );
            x += icon_width;
        }
        painter.galley(
            Pos2::new(x, rect.center().y - galley.size().y / 2.),
            galley,
            fg,
        );
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_sections_fit_narrow_and_regular_windows() {
        for width in [430., 580., 800.] {
            for mut section in Section::ALL {
                let context = egui::Context::default();
                crate::app::configure_fonts(&context);
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 504.),
                    )),
                    ..Default::default()
                };
                let _ = context.run(input, |ctx| {
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE)
                        .show(ctx, |ui| {
                            let scroll = egui::ScrollArea::vertical().show(ui, |ui| {
                                section.show(
                                    ui,
                                    &mut AppSettings::default(),
                                    Path::new("data"),
                                    &mut String::new(),
                                );
                            });
                            assert!(
                                scroll.content_size.x <= width + 1.,
                                "{} overlaps the window edge at {width} points",
                                section.title()
                            );
                        });
                });
            }
        }
    }
}
