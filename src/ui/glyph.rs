// SPDX-License-Identifier: GPL-3.0-only
//! Vector icons drawn with the painter on a 16×16 grid, so they stay crisp at any DPI, take the
//! theme's colors and add nothing to the binary.

use std::f32::consts::TAU;

use eframe::egui::{Color32, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, Vec2, vec2};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    Library,
    Plus,
    Folder,
    Gear,
    Search,
    Play,
    Stop,
    Trash,
    More,
    Back,
    Forward,
    Close,
    Minimize,
    Maximize,
    Restore,
    ChevronDown,
    Check,
    User,
    Refresh,
    Download,
}

pub fn paint(painter: &Painter, rect: Rect, glyph: Glyph, color: Color32) {
    let side = rect.width().min(rect.height());
    let s = side / 16.0;
    let origin = rect.center() - Vec2::splat(side / 2.0);
    let p = |x: f32, y: f32| origin + vec2(x * s, y * s);
    let stroke = Stroke::new((1.5 * s).max(1.0), color);
    let line = |pts: &[(f32, f32)]| {
        painter.add(Shape::line(pts.iter().map(|&(x, y)| p(x, y)).collect(), stroke));
    };
    let r = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(p(x, y), vec2(w * s, h * s));
    let radius = |v: f32| CornerRadius::same((v * s).round() as u8);

    match glyph {
        Glyph::Library => {
            for (x, y) in [(2.0, 2.0), (9.0, 2.0), (2.0, 9.0), (9.0, 9.0)] {
                painter.rect_stroke(r(x, y, 5.0, 5.0), radius(1.5), stroke, StrokeKind::Middle);
            }
        }
        Glyph::Plus => {
            line(&[(8.0, 3.0), (8.0, 13.0)]);
            line(&[(3.0, 8.0), (13.0, 8.0)]);
        }
        Glyph::Folder => {
            line(&[(2.0, 4.0), (6.0, 4.0), (7.5, 5.5), (14.0, 5.5), (14.0, 12.5), (2.0, 12.5), (2.0, 4.0)]);
        }
        Glyph::Gear => {
            let c = p(8.0, 8.0);
            painter.circle_stroke(c, 2.2 * s, stroke);
            painter.circle_stroke(c, 4.6 * s, stroke);
            for i in 0..8 {
                let a = i as f32 * TAU / 8.0;
                let d = vec2(a.cos(), a.sin());
                painter.line_segment([c + d * 4.6 * s, c + d * 6.6 * s], Stroke::new(stroke.width * 1.4, color));
            }
        }
        Glyph::Search => {
            painter.circle_stroke(p(7.0, 7.0), 4.2 * s, stroke);
            line(&[(10.2, 10.2), (13.5, 13.5)]);
        }
        Glyph::Play => {
            let pts = vec![p(5.0, 3.0), p(13.0, 8.0), p(5.0, 13.0)];
            painter.add(Shape::convex_polygon(pts, color, Stroke::NONE));
        }
        Glyph::Stop => {
            painter.rect_filled(r(4.0, 4.0, 8.0, 8.0), radius(1.5), color);
        }
        Glyph::Trash => {
            line(&[(2.5, 4.5), (13.5, 4.5)]);
            line(&[(6.0, 4.5), (6.5, 2.5), (9.5, 2.5), (10.0, 4.5)]);
            line(&[(4.0, 4.5), (4.8, 13.5), (11.2, 13.5), (12.0, 4.5)]);
        }
        Glyph::More => {
            for x in [4.0, 8.0, 12.0] {
                painter.circle_filled(p(x, 8.0), 1.2 * s, color);
            }
        }
        Glyph::Back => {
            line(&[(10.0, 3.0), (5.0, 8.0), (10.0, 13.0)]);
        }
        Glyph::Forward => {
            line(&[(6.0, 3.0), (11.0, 8.0), (6.0, 13.0)]);
        }
        Glyph::Close => {
            line(&[(4.0, 4.0), (12.0, 12.0)]);
            line(&[(12.0, 4.0), (4.0, 12.0)]);
        }
        Glyph::Minimize => {
            line(&[(3.5, 8.0), (12.5, 8.0)]);
        }
        Glyph::Maximize => {
            painter.rect_stroke(r(3.5, 3.5, 9.0, 9.0), radius(1.0), stroke, StrokeKind::Middle);
        }
        Glyph::Restore => {
            painter.rect_stroke(r(3.0, 5.5, 7.5, 7.5), radius(1.0), stroke, StrokeKind::Middle);
            line(&[(5.5, 5.5), (5.5, 3.0), (13.0, 3.0), (13.0, 10.5), (10.5, 10.5)]);
        }
        Glyph::ChevronDown => {
            line(&[(4.0, 6.0), (8.0, 10.0), (12.0, 6.0)]);
        }
        Glyph::Check => {
            line(&[(3.0, 8.5), (6.5, 12.0), (13.0, 4.5)]);
        }
        Glyph::User => {
            painter.circle_stroke(p(8.0, 5.5), 2.8 * s, stroke);
            line(&[(2.5, 14.0), (3.5, 11.0), (6.0, 9.8), (10.0, 9.8), (12.5, 11.0), (13.5, 14.0)]);
        }
        Glyph::Refresh => {
            let c = p(8.0, 8.0);
            let pts: Vec<Pos2> = (0..=20)
                .map(|i| {
                    let a = TAU * (0.1 + 0.75 * i as f32 / 20.0);
                    c + vec2(a.cos(), a.sin()) * 5.0 * s
                })
                .collect();
            painter.add(Shape::line(pts, stroke));
            line(&[(13.0, 2.5), (13.0, 6.0), (9.5, 6.0)]);
        }
        Glyph::Download => {
            line(&[(8.0, 2.5), (8.0, 10.5)]);
            line(&[(4.5, 7.0), (8.0, 10.5), (11.5, 7.0)]);
            line(&[(3.0, 13.5), (13.0, 13.5)]);
        }
    }
}

/// The app icon: a feather on an accent tile. Same 24-unit geometry as `assets/make-icon.ps1`.
pub fn app_mark(painter: &Painter, rect: Rect, tile: Color32, ink: Color32) {
    let s = rect.width().min(rect.height()) / 24.0;
    let origin = rect.center() - Vec2::splat(12.0 * s);
    let p = |x: f32, y: f32| origin + vec2(x * s, y * s);
    painter.rect_filled(Rect::from_min_size(origin, Vec2::splat(24.0 * s)), CornerRadius::same((5.5 * s) as u8), tile);
    // Vane: two cubic curves from the base to the tip, sampled into one convex outline.
    let cubic = |a: Pos2, b: Pos2, c: Pos2, d: Pos2, t: f32| {
        let u = 1.0 - t;
        let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
        Pos2::new(w[0] * a.x + w[1] * b.x + w[2] * c.x + w[3] * d.x, w[0] * a.y + w[1] * b.y + w[2] * c.y + w[3] * d.y)
    };
    let (base, tip) = (p(7.0, 17.0), p(19.5, 4.5));
    let mut outline: Vec<Pos2> = (0..12).map(|i| cubic(base, p(6.5, 11.0), p(12.0, 5.5), tip, i as f32 / 12.0)).collect();
    outline.extend((0..12).map(|i| cubic(tip, p(18.5, 11.5), p(13.0, 17.5), base, i as f32 / 12.0)));
    painter.add(Shape::convex_polygon(outline, ink, Stroke::NONE));
    let line = |a: Pos2, b: Pos2, width: f32, color: Color32| {
        painter.line_segment([a, b], Stroke::new((width * s).max(1.0), color));
    };
    line(p(8.6, 15.4), p(16.5, 7.5), 1.3, tile); // shaft
    line(p(13.6, 10.4), p(17.8, 11.6), 1.1, tile); // barb splits
    line(p(11.0, 13.0), p(14.6, 15.4), 1.1, tile);
    line(p(5.0, 19.0), p(8.2, 15.8), 1.6, ink); // stem
}

/// Where a glyph's ink starts inside its 16 px box. Buttons pad from the ink, not the box, so a
/// narrow glyph such as Play doesn't push its label off center.
pub fn ink_left(glyph: Glyph) -> f32 {
    match glyph {
        Glyph::Play => 5.0,
        Glyph::Stop => 4.0,
        _ => 3.0,
    }
}

pub fn centered(rect: Rect, size: f32) -> Rect {
    Rect::from_center_size(rect.center(), Vec2::splat(size))
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::pos2;

    #[test]
    fn centered_box() {
        let r = centered(Rect::from_min_size(pos2(0.0, 0.0), vec2(32.0, 32.0)), 16.0);
        assert_eq!(r.min, pos2(8.0, 8.0));
        assert_eq!(r.size(), vec2(16.0, 16.0));
    }
}
