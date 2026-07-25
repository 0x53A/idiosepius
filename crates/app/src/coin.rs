//! Animated, vector-native version of the Idiosepius coin.
//!
//! `assets/idiosepius-coin-glow.svg` is the portable design source. egui does
//! not animate an SVG's internal groups, so this module mirrors the same
//! geometry with painter paths and applies a horizontal projection for a
//! convincing Y-axis coin spin.

use std::f32::consts::TAU;

use eframe::egui::{self, Color32, Painter, Pos2, Rect, Shape, Stroke, Vec2};

use crate::theme::Palette;

const SPIN_SECONDS: f32 = 0.95;

pub struct CoinAnimation {
    elapsed: f32,
    spinning: bool,
    motion_enabled: bool,
}

impl CoinAnimation {
    pub fn new(animate_on_boot: bool) -> Self {
        Self {
            elapsed: 0.0,
            spinning: animate_on_boot,
            motion_enabled: animate_on_boot,
        }
    }

    /// Start one complete, decelerating revolution.
    pub fn spin(&mut self) {
        if self.motion_enabled {
            self.elapsed = 0.0;
            self.spinning = true;
        }
    }

    /// Paint the coin inside `rect`, requesting frames only while it moves.
    pub fn paint(&mut self, ui: &egui::Ui, rect: Rect) {
        if self.spinning {
            let dt = ui.input(|i| i.stable_dt).min(1.0 / 20.0);
            self.elapsed = (self.elapsed + dt).min(SPIN_SECONDS);
            self.spinning = self.elapsed < SPIN_SECONDS;
            if self.spinning {
                ui.ctx().request_repaint();
            }
        }

        let progress = if self.spinning {
            self.elapsed / SPIN_SECONDS
        } else {
            1.0
        };
        paint_coin(ui.painter(), rect, spin_scale(progress));
    }
}

fn spin_scale(progress: f32) -> f32 {
    if progress >= 1.0 {
        return 1.0;
    }
    // Fast first half, then a long settle. The coin completes exactly one
    // revolution and returns to a pixel-identical resting face.
    let eased = 1.0 - (1.0 - progress).powi(3);
    (TAU * eased).cos()
}

#[derive(Clone, Copy)]
struct Projection {
    center: Pos2,
    radius: f32,
    x_scale: f32,
}

impl Projection {
    fn at(self, x: f32, y: f32) -> Pos2 {
        self.center + Vec2::new(x * self.radius * self.x_scale, y * self.radius)
    }
}

fn paint_coin(painter: &Painter, rect: Rect, x_scale: f32) {
    let radius = rect.width().min(rect.height()) * 0.48;
    let projection = Projection {
        center: rect.center(),
        radius,
        x_scale,
    };
    let edge_on = x_scale.abs() < 0.09;
    let front = x_scale >= 0.0;
    let accent = if front {
        Palette::ACCENT
    } else {
        Palette::VIOLET
    };

    let face = irregular_ring(projection, 1.0, 72);
    painter.add(Shape::convex_polygon(
        face[..face.len() - 1].to_vec(),
        if front {
            Palette::SURFACE.gamma_multiply(0.78)
        } else {
            Palette::BG.gamma_multiply(0.92)
        },
        Stroke::NONE,
    ));
    glow_path(painter, face, accent, 1.15, 0.95);

    let inner = irregular_ring(projection, 0.90, 64);
    glow_path(painter, inner, Palette::ACCENT, 0.65, 0.48);

    if edge_on {
        let top = projection.at(0.0, -0.98);
        let bottom = projection.at(0.0, 0.98);
        glow_path(painter, vec![top, bottom], Palette::ACCENT, 2.0, 1.0);
        return;
    }

    if front {
        paint_squid(painter, projection);
        paint_flourish(painter, projection);
    } else {
        paint_reverse(painter, projection);
    }
}

fn paint_squid(painter: &Painter, p: Projection) {
    let mut mantle = cubic(
        p,
        (0.0, -0.63),
        (-0.16, -0.57),
        (-0.18, -0.18),
        (-0.09, 0.08),
        12,
    );
    append_cubic(
        &mut mantle,
        p,
        (-0.09, 0.08),
        (-0.06, 0.15),
        (-0.03, 0.16),
        (0.0, 0.17),
        6,
    );
    append_cubic(
        &mut mantle,
        p,
        (0.0, 0.17),
        (0.03, 0.16),
        (0.06, 0.15),
        (0.09, 0.08),
        6,
    );
    append_cubic(
        &mut mantle,
        p,
        (0.09, 0.08),
        (0.18, -0.18),
        (0.16, -0.57),
        (0.0, -0.63),
        12,
    );
    glow_path(painter, mantle, Palette::ACCENT, 1.15, 1.0);

    for side in [-1.0_f32, 1.0] {
        glow_path(
            painter,
            cubic(
                p,
                (0.12 * side, -0.42),
                (0.25 * side, -0.34),
                (0.21 * side, -0.13),
                (0.13 * side, -0.08),
                10,
            ),
            Palette::ACCENT,
            0.8,
            0.62,
        );

        ellipse(
            painter,
            p,
            (0.13 * side, 0.02),
            (0.047, 0.068),
            Palette::VIOLET,
            0.95,
        );
        dot(
            painter,
            p.at(0.13 * side, 0.02),
            p.radius * 0.012 * p.x_scale.abs().max(0.35),
            Palette::TEXT,
        );

        let arms = [
            (
                (-0.02 * side, 0.15),
                (-0.08 * side, 0.27),
                (-0.26 * side, 0.30),
                (-0.40 * side, 0.18),
            ),
            (
                (-0.03 * side, 0.17),
                (-0.12 * side, 0.35),
                (-0.31 * side, 0.43),
                (-0.46 * side, 0.32),
            ),
            (
                (-0.02 * side, 0.18),
                (-0.11 * side, 0.45),
                (-0.26 * side, 0.53),
                (-0.38 * side, 0.49),
            ),
            (
                (-0.01 * side, 0.18),
                (-0.08 * side, 0.50),
                (-0.16 * side, 0.66),
                (-0.29 * side, 0.62),
            ),
            (
                (0.0, 0.18),
                (-0.02 * side, 0.52),
                (-0.08 * side, 0.75),
                (-0.18 * side, 0.80),
            ),
        ];
        for (a, b, c, d) in arms {
            glow_path(
                painter,
                cubic(p, a, b, c, d, 12),
                Palette::ACCENT,
                1.0,
                0.92,
            );
        }

        glow_path(
            painter,
            cubic(
                p,
                (-0.07 * side, -0.31),
                (-0.01 * side, -0.27),
                (-0.03 * side, -0.16),
                (-0.10 * side, -0.12),
                8,
            ),
            Palette::VIOLET,
            0.75,
            0.78,
        );
    }

    glow_path(
        painter,
        cubic(
            p,
            (-0.06, -0.04),
            (-0.02, 0.00),
            (-0.02, 0.07),
            (0.0, 0.12),
            8,
        ),
        Palette::VIOLET,
        0.75,
        0.72,
    );
    glow_path(
        painter,
        cubic(p, (0.06, -0.04), (0.02, 0.00), (0.02, 0.07), (0.0, 0.12), 8),
        Palette::VIOLET,
        0.75,
        0.72,
    );
}

fn paint_flourish(painter: &Painter, p: Projection) {
    for side in [-1.0_f32, 1.0] {
        glow_path(
            painter,
            cubic(
                p,
                (0.36 * side, -0.43),
                (0.58 * side, -0.48),
                (0.63 * side, -0.27),
                (0.49 * side, -0.23),
                11,
            ),
            Palette::ACCENT,
            0.55,
            0.42,
        );
        glow_path(
            painter,
            cubic(
                p,
                (0.48 * side, 0.54),
                (0.65 * side, 0.48),
                (0.65 * side, 0.69),
                (0.53 * side, 0.66),
                10,
            ),
            Palette::ACCENT,
            0.55,
            0.35,
        );
    }
}

fn paint_reverse(painter: &Painter, p: Projection) {
    for radius in [0.52, 0.36] {
        glow_path(
            painter,
            regular_ring(p, radius, 52),
            Palette::VIOLET,
            0.7,
            0.48,
        );
    }
    for side in [-1.0_f32, 1.0] {
        glow_path(
            painter,
            cubic(
                p,
                (0.0, -0.25),
                (0.32 * side, -0.16),
                (0.32 * side, 0.16),
                (0.0, 0.25),
                12,
            ),
            Palette::ACCENT,
            0.9,
            0.72,
        );
    }
}

fn irregular_ring(p: Projection, radius: f32, steps: usize) -> Vec<Pos2> {
    (0..=steps)
        .map(|i| {
            let t = TAU * i as f32 / steps as f32;
            let wobble = 1.0 + 0.012 * (3.0 * t + 0.6).sin() + 0.007 * (7.0 * t).sin();
            p.at(t.cos() * radius * wobble, t.sin() * radius * wobble)
        })
        .collect()
}

fn regular_ring(p: Projection, radius: f32, steps: usize) -> Vec<Pos2> {
    (0..=steps)
        .map(|i| {
            let t = TAU * i as f32 / steps as f32;
            p.at(t.cos() * radius, t.sin() * radius)
        })
        .collect()
}

fn cubic(
    p: Projection,
    a: (f32, f32),
    b: (f32, f32),
    c: (f32, f32),
    d: (f32, f32),
    steps: usize,
) -> Vec<Pos2> {
    (0..=steps)
        .map(|i| {
            let t = i as f32 / steps as f32;
            let u = 1.0 - t;
            let x = u.powi(3) * a.0
                + 3.0 * u.powi(2) * t * b.0
                + 3.0 * u * t.powi(2) * c.0
                + t.powi(3) * d.0;
            let y = u.powi(3) * a.1
                + 3.0 * u.powi(2) * t * b.1
                + 3.0 * u * t.powi(2) * c.1
                + t.powi(3) * d.1;
            p.at(x, y)
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn append_cubic(
    points: &mut Vec<Pos2>,
    p: Projection,
    a: (f32, f32),
    b: (f32, f32),
    c: (f32, f32),
    d: (f32, f32),
    steps: usize,
) {
    points.extend(cubic(p, a, b, c, d, steps).into_iter().skip(1));
}

fn ellipse(
    painter: &Painter,
    p: Projection,
    center: (f32, f32),
    radii: (f32, f32),
    color: Color32,
    width: f32,
) {
    let points = (0..=24)
        .map(|i| {
            let t = TAU * i as f32 / 24.0;
            p.at(center.0 + radii.0 * t.cos(), center.1 + radii.1 * t.sin())
        })
        .collect();
    glow_path(painter, points, color, width, 0.9);
}

fn dot(painter: &Painter, center: Pos2, radius: f32, color: Color32) {
    painter.circle_filled(center, radius.max(0.7), color);
}

fn glow_path(painter: &Painter, points: Vec<Pos2>, color: Color32, width: f32, alpha: f32) {
    if points.len() < 2 {
        return;
    }
    painter.add(Shape::line(
        points.clone(),
        Stroke::new(width * 6.0, color.gamma_multiply(0.055 * alpha)),
    ));
    painter.add(Shape::line(
        points.clone(),
        Stroke::new(width * 2.8, color.gamma_multiply(0.16 * alpha)),
    ));
    painter.add(Shape::line(
        points,
        Stroke::new(width, color.gamma_multiply(alpha)),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spin_finishes_on_the_front_face() {
        assert_eq!(spin_scale(1.0), 1.0);
    }

    #[test]
    fn half_turn_shows_the_reverse() {
        assert!(spin_scale(0.2) < 0.0);
    }
}
