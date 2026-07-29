//! Quiet, vector-native sea life behind the study surfaces.
//!
//! The animals are deliberately line art rather than sprites: they inherit the
//! app palette, stay crisp at every zoom level, and remain subordinate to the
//! cards and chrome. Screenshot mode leaves the layer out altogether so visual
//! checks stay reproducible.

use std::time::Duration;

use eframe::egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, Vec2};

use crate::theme::Palette;

const FRAME_TIME: Duration = Duration::from_millis(50);

#[derive(Clone, Copy)]
enum Kind {
    Fish,
    Squid,
    Jellyfish,
}

#[derive(Clone, Copy)]
struct Swimmer {
    kind: Kind,
    phase: f32,
    lane: f32,
    speed: f32,
    scale: f32,
    bob: f32,
    direction: f32,
    colour: Color32,
}

const SWIMMERS: [Swimmer; 9] = [
    Swimmer {
        kind: Kind::Fish,
        phase: 0.08,
        lane: 0.18,
        speed: 0.012,
        scale: 0.72,
        bob: 11.0,
        direction: 1.0,
        colour: Palette::ACCENT,
    },
    Swimmer {
        kind: Kind::Fish,
        phase: 0.46,
        lane: 0.30,
        speed: 0.009,
        scale: 0.48,
        bob: 7.0,
        direction: -1.0,
        colour: Palette::VIOLET,
    },
    Swimmer {
        kind: Kind::Squid,
        phase: 0.72,
        lane: 0.42,
        speed: 0.007,
        scale: 0.84,
        bob: 15.0,
        direction: -1.0,
        colour: Palette::ACCENT,
    },
    Swimmer {
        kind: Kind::Fish,
        phase: 0.27,
        lane: 0.57,
        speed: 0.010,
        scale: 0.62,
        bob: 10.0,
        direction: 1.0,
        colour: Palette::ACCENT,
    },
    Swimmer {
        kind: Kind::Jellyfish,
        phase: 0.36,
        lane: 0.49,
        speed: 0.005,
        scale: 0.72,
        bob: 14.0,
        direction: -1.0,
        colour: Palette::ACCENT,
    },
    Swimmer {
        kind: Kind::Fish,
        phase: 0.89,
        lane: 0.68,
        speed: 0.008,
        scale: 0.42,
        bob: 8.0,
        direction: -1.0,
        colour: Palette::VIOLET,
    },
    Swimmer {
        kind: Kind::Squid,
        phase: 0.17,
        lane: 0.79,
        speed: 0.006,
        scale: 0.68,
        bob: 13.0,
        direction: 1.0,
        colour: Palette::VIOLET,
    },
    Swimmer {
        kind: Kind::Jellyfish,
        phase: 0.47,
        lane: 0.74,
        speed: 0.0045,
        scale: 0.55,
        bob: 12.0,
        direction: 1.0,
        colour: Palette::VIOLET,
    },
    Swimmer {
        kind: Kind::Fish,
        phase: 0.30,
        lane: 0.90,
        speed: 0.011,
        scale: 0.54,
        bob: 6.0,
        direction: 1.0,
        colour: Palette::ACCENT,
    },
];

pub struct OceanBackground {
    elapsed: f32,
    motion_enabled: bool,
    visible: bool,
}

impl OceanBackground {
    pub fn new(motion_enabled: bool) -> Self {
        Self {
            elapsed: 0.0,
            motion_enabled,
            visible: true,
        }
    }

    /// No sea at all, for `--shot`.
    ///
    /// Freezing the composition was not enough to make captures diffable:
    /// every swimmer is positioned from the width of the frame, so the layout
    /// stayed coupled to whatever size the window had reached by the capture
    /// frame. Leaving the layer out removes that coupling entirely, and nine
    /// animals drifting behind a card are the part of the picture a visual
    /// check is least interested in.
    pub fn hidden() -> Self {
        Self {
            elapsed: 0.0,
            motion_enabled: false,
            visible: false,
        }
    }

    pub fn paint(&mut self, ui: &eframe::egui::Ui, rect: Rect) {
        if !self.visible {
            return;
        }
        if self.motion_enabled {
            let dt = ui.input(|input| input.stable_dt).min(1.0 / 20.0);
            self.elapsed += dt;
            ui.ctx().request_repaint_after(FRAME_TIME);
        }

        let painter = ui.painter().with_clip_rect(rect);
        for swimmer in SWIMMERS {
            paint_swimmer(&painter, rect, swimmer, self.elapsed);
        }
    }
}

fn paint_swimmer(painter: &Painter, rect: Rect, swimmer: Swimmer, time: f32) {
    let margin = 90.0 * swimmer.scale;
    let progress = (swimmer.phase + time * swimmer.speed).rem_euclid(1.0);
    let travel = rect.width() + margin * 2.0;
    let from_left = rect.left() - margin + progress * travel;
    let x = if swimmer.direction > 0.0 {
        from_left
    } else {
        rect.right() + margin - progress * travel
    };
    let wave = (time * 0.42 + swimmer.phase * std::f32::consts::TAU).sin();
    let y = rect.top() + rect.height() * swimmer.lane + wave * swimmer.bob;
    let center = Pos2::new(x, y);

    match swimmer.kind {
        Kind::Fish => paint_fish(
            painter,
            center,
            swimmer.direction,
            swimmer.scale,
            swimmer.colour,
            time,
            swimmer.phase,
        ),
        Kind::Squid => paint_squid(
            painter,
            center,
            swimmer.direction,
            swimmer.scale,
            swimmer.colour,
            time,
            swimmer.phase,
        ),
        Kind::Jellyfish => paint_jellyfish(
            painter,
            center,
            swimmer.direction,
            swimmer.scale,
            swimmer.colour,
            time,
            swimmer.phase,
        ),
    }
}

fn paint_fish(
    painter: &Painter,
    center: Pos2,
    direction: f32,
    scale: f32,
    colour: Color32,
    time: f32,
    phase: f32,
) {
    let point = |x: f32, y: f32| center + Vec2::new(x * direction, y) * scale;
    let tail_wave = (time * 1.35 + phase * 9.0).sin() * 3.0;
    let ink = colour.gamma_multiply(0.42);
    let faint = colour.gamma_multiply(0.20);

    // Angular silhouettes keep these background marks related to the
    // interface's crisp rectangular geometry.
    let body = vec![
        point(31.0, 0.0),
        point(13.0, -12.0),
        point(-13.0, -10.0),
        point(-25.0, 0.0),
        point(-13.0, 10.0),
        point(13.0, 12.0),
        point(31.0, 0.0),
    ];
    painter.add(Shape::line(body, Stroke::new(1.0, ink)));

    painter.add(Shape::line(
        vec![
            point(-23.0, 0.0),
            point(-40.0, -13.0 + tail_wave),
            point(-37.0, 1.0),
            point(-40.0, 13.0 + tail_wave),
            point(-23.0, 0.0),
        ],
        Stroke::new(1.0, ink),
    ));
    painter.line_segment(
        [point(-8.0, -9.0), point(-2.0, 0.0)],
        Stroke::new(1.0, faint),
    );
    painter.line_segment(
        [point(-2.0, 0.0), point(-8.0, 9.0)],
        Stroke::new(1.0, faint),
    );
    painter.rect_filled(
        Rect::from_center_size(point(20.0, -2.5), Vec2::splat((2.2 * scale).max(1.0))),
        0,
        ink,
    );
}

fn paint_squid(
    painter: &Painter,
    center: Pos2,
    direction: f32,
    scale: f32,
    colour: Color32,
    time: f32,
    phase: f32,
) {
    let point = |x: f32, y: f32| center + Vec2::new(x, y) * scale;
    let pulse = 1.0 + (time * 0.75 + phase * 8.0).sin() * 0.04;
    let ink = colour.gamma_multiply(0.38);
    let faint = colour.gamma_multiply(0.18);
    let glow = colour.gamma_multiply(0.06);
    let eye = if colour == Palette::ACCENT {
        Palette::VIOLET.gamma_multiply(0.46)
    } else {
        Palette::ACCENT.gamma_multiply(0.46)
    };

    let mantle = vec![
        point(0.0, -39.0 * pulse),
        point(-7.0, -35.0),
        point(-13.0, -25.0),
        point(-16.0, -10.0),
        point(-14.0, 2.0),
        point(-9.0, 13.0),
        point(0.0, 17.0),
        point(9.0, 13.0),
        point(14.0, 2.0),
        point(16.0, -10.0),
        point(13.0, -25.0),
        point(7.0, -35.0),
        point(0.0, -39.0 * pulse),
    ];
    painter.add(Shape::line(mantle.clone(), Stroke::new(4.0, glow)));
    painter.add(Shape::line(mantle, Stroke::new(1.15, ink)));

    // The swept fins, contrasting eyes and many curling arms echo the squid
    // engraved on the brand coin.
    painter.add(Shape::line(
        vec![
            point(-11.0, -28.0),
            point(-23.0, -22.0),
            point(-25.0, -9.0),
            point(-15.0, -2.0),
        ],
        Stroke::new(1.0, faint),
    ));
    painter.add(Shape::line(
        vec![
            point(11.0, -28.0),
            point(23.0, -22.0),
            point(25.0, -9.0),
            point(15.0, -2.0),
        ],
        Stroke::new(1.0, faint),
    ));
    for x in [-6.0_f32, 6.0] {
        painter.rect_filled(
            Rect::from_center_size(point(x, 3.0), Vec2::splat((2.6 * scale).max(1.2))),
            0,
            eye,
        );
    }

    let tentacle_wave = time * 1.1 + phase * 11.0;
    for (index, x) in [-12.0_f32, -8.0, -4.0, 0.0, 4.0, 8.0, 12.0]
        .into_iter()
        .enumerate()
    {
        let wave = (tentacle_wave + index as f32 * 0.9).sin() * 4.0;
        let trail = -direction * 5.0;
        let arm = vec![
            point(x * 0.78, 13.0),
            point(x + trail * 0.25 + wave * 0.25, 27.0),
            point(x * 0.72 + trail * 0.65 + wave, 43.0),
            point(x * 0.52 + trail - wave * 0.20, 60.0),
        ];
        painter.add(Shape::line(arm.clone(), Stroke::new(3.0, glow)));
        painter.add(Shape::line(arm, Stroke::new(1.0, ink)));
    }
}

fn paint_jellyfish(
    painter: &Painter,
    center: Pos2,
    direction: f32,
    scale: f32,
    colour: Color32,
    time: f32,
    phase: f32,
) {
    let point = |x: f32, y: f32| center + Vec2::new(x, y) * scale;
    let pulse = 1.0 + (time * 0.9 + phase * 10.0).sin() * 0.07;
    let ink = colour.gamma_multiply(0.34);
    let faint = colour.gamma_multiply(0.16);

    // A faceted bell keeps the animal legible while retaining the crisp,
    // instrument-like line language of the rest of the interface.
    let bell = vec![
        point(-24.0 * pulse, 4.0),
        point(-22.0 * pulse, -7.0),
        point(-16.0 * pulse, -18.0),
        point(-7.0 * pulse, -25.0),
        point(0.0, -28.0),
        point(7.0 * pulse, -25.0),
        point(16.0 * pulse, -18.0),
        point(22.0 * pulse, -7.0),
        point(24.0 * pulse, 4.0),
        point(16.0, 0.0),
        point(10.0, 7.0),
        point(4.0, 0.0),
        point(0.0, 7.0),
        point(-4.0, 0.0),
        point(-10.0, 7.0),
        point(-16.0, 0.0),
        point(-24.0 * pulse, 4.0),
    ];
    painter.add(Shape::line(bell, Stroke::new(1.0, ink)));

    for x in [-10.0_f32, 0.0, 10.0] {
        painter.line_segment(
            [point(x * 0.65, -21.0), point(x, 1.5)],
            Stroke::new(1.0, faint),
        );
    }

    let tentacle_wave = time * 0.95 + phase * 12.0;
    for (index, x) in [-14.0_f32, -7.0, 0.0, 7.0, 14.0].into_iter().enumerate() {
        let wave = (tentacle_wave + index as f32 * 0.75).sin() * 4.5;
        let trail = -direction * 4.0;
        painter.add(Shape::line(
            vec![
                point(x, 4.0),
                point(x * 0.92 + wave * 0.25, 20.0),
                point(x * 0.76 + trail * 0.55 - wave * 0.45, 39.0),
                point(x * 0.58 + trail + wave, 59.0),
            ],
            Stroke::new(1.0, ink),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_swimmer_stays_in_a_normalised_lane() {
        assert!(
            SWIMMERS
                .iter()
                .all(|swimmer| (0.0..=1.0).contains(&swimmer.lane))
        );
    }

    #[test]
    fn a_hidden_ocean_paints_nothing_and_asks_for_no_frames() {
        let ocean = OceanBackground::hidden();
        assert!(!ocean.visible);
        assert!(
            !ocean.motion_enabled,
            "a hidden sea must not drive repaints"
        );
    }

    #[test]
    fn the_background_contains_each_kind_of_swimmer() {
        assert!(
            SWIMMERS
                .iter()
                .any(|swimmer| matches!(swimmer.kind, Kind::Fish))
        );
        assert!(
            SWIMMERS
                .iter()
                .any(|swimmer| matches!(swimmer.kind, Kind::Squid))
        );
        assert!(
            SWIMMERS
                .iter()
                .any(|swimmer| matches!(swimmer.kind, Kind::Jellyfish))
        );
    }
}
