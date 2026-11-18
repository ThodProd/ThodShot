use crate::settings::AppSettings;
use eframe::egui::{self, Color32, RichText, Sense, Stroke};
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
                        for section in Self::ALL {
                            let button = option_button(ui, section.title(), *self == section);
                            if ui.add(button).clicked() {
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
        ui.heading(RichText::new(self.title()).strong().size(25.));
        egui::Frame::new()
            .fill(surface(ui))
            .stroke(Stroke::new(1_f32, ui.visuals().widgets.noninteractive.bg_stroke.color))
            .corner_radius(14)
            .inner_margin(18)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                match self {
                    Self::Capture => {
                        choice(
                            ui,
                            "Режим по умолчанию",
                            &mut settings.default_mode,
                            &[("region", "Область"), ("full", "Весь экран")],
                        );
                        ui.separator();
                        toggle(ui, &mut settings.intercept_print_screen, "Горячая клавиша PrintScreen",
                            "Открывает выбор области поверх рабочего стола.");
                    }
                    Self::Save => {
                        folder(ui, settings, notice);
                        ui.separator();
                        choice(ui, "Формат файла", &mut settings.format, &[("png", "PNG"), ("jpg", "JPG")]);
                        ui.separator();
                        toggle(ui, &mut settings.open_folder_after_save, "Открывать Проводник",
                            "Показывает файл после сохранения.");
                    }
                    Self::App => {
                        choice(
                            ui,
                            "Тема",
                            &mut settings.theme,
                            &[("dark", "Тёмная"), ("light", "Светлая"), ("system", "Системная")],
                        );
                        ui.separator();
                        toggle(ui, &mut settings.autostart, "Запускать вместе с Windows",
                            "PrintScreen доступна сразу после входа.");
                        ui.separator();
                        toggle(ui, &mut settings.show_tray_icon, "Значок в системном трее",
                            "Даёт быстрый доступ к снимкам и настройкам.");
                    }
                    Self::Ocr => {
                        choice(ui, "Язык распознавания", &mut settings.ocr_language,
                            &[("auto", "Авто"), ("ru", "Русский"), ("en", "English")]);
                        ui.label(RichText::new("Текст распознаётся локально. Для каждого языка требуется пакет распознавания Windows.").weak());
                    }
                }
            });
    }
}

fn sidebar(ui: &mut egui::Ui, active: &mut Section, data_dir: &Path, notice: &mut String) {
    let height = (ui.clip_rect().bottom() - ui.cursor().min.y - 29.).max(290.);
    egui::Frame::new()
        .fill(surface(ui))
        .inner_margin(14)
        .show(ui, |ui| {
            ui.set_width(172.);
            for section in Section::ALL {
                let button = option_button(ui, section.title(), *active == section)
                    .frame(*active == section)
                    .min_size(egui::vec2(172., 38.));
                if ui.add(button).clicked() {
                    *active = section;
                }
            }
            ui.add_space((height - 246.).max(12.));
            footer(ui, data_dir, notice);
        });
}

fn footer(ui: &mut egui::Ui, data_dir: &Path, notice: &mut String) {
    ui.vertical(|ui| {
        ui.label(
            RichText::new(concat!("ThodShot ", env!("CARGO_PKG_VERSION")))
                .small()
                .weak(),
        );
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
    ui.add(egui::Label::new(RichText::new(&path).weak()).wrap())
        .on_hover_text(&path);
    ui.horizontal_wrapped(|ui| {
        if ui.button("Выбрать папку…").clicked()
            && let Some(folder) = rfd::FileDialog::new().set_directory(&folder).pick_folder()
        {
            settings.save_folder = folder.display().to_string();
        }
        if ui.button("Открыть папку").clicked()
            && let Err(error) = std::fs::create_dir_all(&folder)
                .and_then(|()| Command::new("explorer.exe").arg(&folder).spawn())
        {
            *notice = format!("Не удалось открыть папку: {error}");
        }
    });
}

fn choice(ui: &mut egui::Ui, label: &str, value: &mut String, options: &[(&str, &str)]) {
    ui.label(RichText::new(label).strong());
    ui.horizontal_wrapped(|ui| {
        for &(option, label) in options {
            let selected = value == option;
            let response = ui.add(option_button(ui, label, selected));
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
                ui.spacing_mut().item_spacing.y = 4.;
                ui.label(RichText::new(label).strong());
                ui.label(RichText::new(detail).weak());
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
        let visuals = ui.style().interact(&response);
        let fill = if *value {
            Color32::from_rgb(43, 104, 210)
        } else {
            visuals.bg_fill
        };
        let border = if response.has_focus() {
            ui.visuals().selection.stroke
        } else {
            visuals.bg_stroke
        };
        ui.painter()
            .rect(rect, 12, fill, border, egui::StrokeKind::Inside);
        let thumb_x = if *value {
            rect.right() - 12.
        } else {
            rect.left() + 12.
        };
        ui.painter()
            .circle_filled(egui::pos2(thumb_x, rect.center().y), 7., Color32::WHITE);
    });
}

pub fn option_button(ui: &egui::Ui, label: &str, selected: bool) -> egui::Button<'static> {
    let dark = ui.visuals().dark_mode;
    let mut text = RichText::new(label.to_owned());
    if selected {
        text = text.strong().color(if dark {
            Color32::from_rgb(188, 214, 255)
        } else {
            Color32::from_rgb(25, 68, 139)
        });
    }
    let button = egui::Button::new(text).corner_radius(8);
    if selected {
        button.fill(if dark {
            Color32::from_rgb(37, 58, 91)
        } else {
            Color32::from_rgb(219, 232, 255)
        })
    } else {
        button
    }
}

pub fn surface(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(30, 35, 45)
    } else {
        Color32::WHITE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_sections_fit_narrow_and_regular_windows() {
        for width in [430., 800.] {
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
