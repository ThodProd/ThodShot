use eframe::egui::{self, Color32, Pos2, Rect, Stroke, Vec2};
use image::RgbaImage;
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Tool {
    Cursor,
    Arrow,
    Rectangle,
    Highlighter,
    Pen,
}
#[derive(Clone)]
pub struct Mark {
    pub tool: Tool,
    pub points: Vec<Pos2>,
    pub color: Color32,
    pub width: f32,
}
impl Mark {
    pub fn new(tool: Tool, p: Pos2, color: Color32, width: f32) -> Self {
        Self {
            tool,
            points: vec![p, p],
            color,
            width: if tool == Tool::Highlighter {
                (width * 2.4).max(8.)
            } else {
                width
            },
        }
    }
    pub fn update(&mut self, p: Pos2) {
        if matches!(self.tool, Tool::Pen | Tool::Highlighter) {
            self.points.push(p);
        } else {
            self.points[1] = p;
        }
    }
    pub fn is_complete(&self) -> bool {
        self.tool != Tool::Arrow || self.arrow().is_some()
    }

    // Follow the outside edge of the shaft and head as a single silhouette.
    fn arrow(&self) -> Option<[Pos2; 7]> {
        let a = self.points[0];
        let b = self.points[1];
        let d = b - a;
        let len = d.length();
        if len < 2. {
            return None;
        }
        let u = d / len;
        let h = (self.width * 4.).max(14.).min(len * 0.62);
        let base = b - u * h;
        let perpendicular = Vec2::new(-u.y, u.x);
        let head = perpendicular * h * 0.56;
        let shaft = perpendicular * (self.width * 0.5).min(h * 0.42);
        Some([
            a + shaft,
            base + shaft,
            base + head,
            b,
            base - head,
            base - shaft,
            a - shaft,
        ])
    }
    pub fn paint(&self, painter: &egui::Painter, origin: Pos2, scale: f32) {
        let map = |p: Pos2| origin + p.to_vec2() * scale;
        let color = if self.tool == Tool::Highlighter {
            Color32::from_rgba_unmultiplied(self.color.r(), self.color.g(), self.color.b(), 120)
        } else {
            self.color
        };
        let stroke = Stroke::new(self.width * scale, color);
        if self.tool == Tool::Arrow {
            if let Some([tail_l, shaft_l, head_l, tip, head_r, shaft_r, tail_r]) = self.arrow() {
                // Put the shaft slightly inside the head so their antialiased edges cannot split.
                let overlap = (tip - (head_l + (head_r - head_l) / 2.)) * 0.06;
                for part in [
                    vec![
                        map(tail_l),
                        map(shaft_l + overlap),
                        map(shaft_r + overlap),
                        map(tail_r),
                    ],
                    vec![map(head_l), map(tip), map(head_r)],
                ] {
                    painter.add(egui::Shape::convex_polygon(part, color, Stroke::NONE));
                }
            }
        } else if self.tool == Tool::Rectangle {
            painter.rect_stroke(
                Rect::from_two_pos(map(self.points[0]), map(self.points[1])),
                0.,
                stroke,
                egui::StrokeKind::Middle,
            );
        } else {
            painter.add(egui::Shape::line(
                self.points.iter().copied().map(map).collect(),
                stroke,
            ));
        }
    }
    fn path(&self) -> Option<tiny_skia::Path> {
        let mut p = tiny_skia::PathBuilder::new();
        let a = self.points[0];
        p.move_to(a.x, a.y);
        if self.tool == Tool::Rectangle {
            let b = self.points[1];
            p.line_to(b.x, a.y);
            p.line_to(b.x, b.y);
            p.line_to(a.x, b.y);
            p.close();
        } else {
            for b in &self.points[1..] {
                p.line_to(b.x, b.y);
            }
        }
        p.finish()
    }
}
pub fn render(source: &RgbaImage, marks: &[Mark]) -> RgbaImage {
    let mut layer =
        tiny_skia::Pixmap::new(source.width(), source.height()).expect("image dimensions");
    for mark in marks {
        let mut paint = tiny_skia::Paint::default();
        paint.set_color_rgba8(
            mark.color.r(),
            mark.color.g(),
            mark.color.b(),
            if mark.tool == Tool::Highlighter {
                120
            } else {
                255
            },
        );
        paint.anti_alias = true;
        if mark.tool == Tool::Arrow {
            if let Some(points) = mark.arrow() {
                let mut outline = tiny_skia::PathBuilder::new();
                outline.move_to(points[0].x, points[0].y);
                for point in &points[1..] {
                    outline.line_to(point.x, point.y);
                }
                outline.close();
                if let Some(path) = outline.finish() {
                    layer.fill_path(
                        &path,
                        &paint,
                        tiny_skia::FillRule::Winding,
                        tiny_skia::Transform::identity(),
                        None,
                    );
                }
            }
        } else if let Some(path) = mark.path() {
            layer.stroke_path(
                &path,
                &paint,
                &tiny_skia::Stroke {
                    width: mark.width,
                    line_cap: tiny_skia::LineCap::Round,
                    line_join: if mark.tool == Tool::Rectangle {
                        tiny_skia::LineJoin::Miter
                    } else {
                        tiny_skia::LineJoin::Round
                    },
                    ..Default::default()
                },
                tiny_skia::Transform::identity(),
                None,
            );
        }
    }
    let mut output = source.clone();
    for (p, l) in output.pixels_mut().zip(layer.pixels()) {
        let alpha = l.alpha() as u32;
        for (i, c) in [l.red(), l.green(), l.blue()].into_iter().enumerate() {
            p[i] = (c as u32 + (p[i] as u32 * (255 - alpha) + 127) / 255).min(255) as u8;
        }
        p[3] = 255;
    }
    output
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn export_width() {
        let src = RgbaImage::from_pixel(80, 80, image::Rgba([255, 255, 255, 255]));
        let mut mark = Mark::new(Tool::Pen, Pos2::new(10., 40.), Color32::RED, 10.);
        mark.update(Pos2::new(70., 40.));
        let output = render(&src, &[mark]);
        assert_eq!(output.get_pixel(40, 43).0, [255, 0, 0, 255]);
        assert_eq!(output.get_pixel(40, 50).0, [255, 255, 255, 255]);
    }

    #[test]
    fn thick_arrow_ends_at_the_release_point() {
        let source = RgbaImage::from_pixel(64, 64, image::Rgba([255, 255, 255, 255]));
        let start = Pos2::new(16., 32.);
        let end = Pos2::new(32., 32.);
        let mut mark = Mark::new(Tool::Arrow, start, Color32::RED, 20.);
        assert!(!mark.is_complete());
        mark.update(end);
        let shape = mark.arrow().unwrap();
        assert_eq!(shape[3], end);
        assert_eq!((shape[0].x, shape[6].x), (start.x, start.x));
        let saved = render(&source, &[mark]);
        assert_eq!(saved.get_pixel(24, 32).0, [255, 0, 0, 255]);
        assert_eq!(saved.get_pixel(32, 32).0, [255, 255, 255, 255]);
    }
}
