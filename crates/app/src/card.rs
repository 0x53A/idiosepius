//! The swipeable card: motion state, rigid rotation, and painting.
//!
//! The card is drawn directly with the painter rather than laid out as
//! widgets, because egui can translate and scale a layer but not rotate one,
//! and the tilt is most of what makes a swipe feel physical.

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Stroke, Vec2};
use eframe::epaint::{Shape, TextShape};
use std::sync::Arc;

/// Horizontal drag needed to commit an answer, in points.
pub const COMMIT_DIST: f32 = 105.0;
/// Tilt at full commit distance, radians (about 7°).
const MAX_TILT: f32 = 0.12;
/// How fast a released card returns to centre (higher = snappier).
const SPRING: f32 = 18.0;
/// Fly-off speed, points per second.
const FLY_SPEED: f32 = 2400.0;
/// Entry animation duration, seconds.
const ENTRY_TIME: f32 = 0.22;

/// Motion state of the top card. One per visible card.
#[derive(Debug, Default, Clone)]
pub struct Motion {
    /// Displacement from the resting position.
    pub offset: Vec2,
    pub dragging: bool,
    /// Set once the card has been committed and is leaving the screen.
    pub fly: Option<Vec2>,
    /// Entry animation, 0 (just dealt) to 1 (settled).
    pub entry: f32,
}

impl Motion {
    /// Start a fresh card: centred, invisible, about to animate in.
    pub fn deal() -> Self {
        Motion {
            offset: Vec2::ZERO,
            dragging: false,
            fly: None,
            entry: 0.0,
        }
    }

    /// Advance the animation. Returns true while a repaint is still needed.
    pub fn update(&mut self, dt: f32) -> bool {
        let mut animating = false;

        if self.entry < 1.0 {
            self.entry = (self.entry + dt / ENTRY_TIME).min(1.0);
            animating = true;
        }

        if let Some(dir) = self.fly {
            self.offset += dir * FLY_SPEED * dt;
            animating = true;
        } else if !self.dragging && self.offset != Vec2::ZERO {
            // Exponential ease back to centre; frame-rate independent.
            let k = (-SPRING * dt).exp();
            self.offset *= k;
            if self.offset.length() < 0.5 {
                self.offset = Vec2::ZERO;
            } else {
                animating = true;
            }
        }

        animating
    }

    /// Send the card off screen in the given horizontal direction (-1 or +1).
    pub fn launch(&mut self, dir: f32) {
        // Keep whatever vertical drift the drag had, so it leaves along the
        // line the hand was actually moving.
        let vertical = (self.offset.y / COMMIT_DIST).clamp(-0.6, 0.6);
        self.fly = Some(Vec2::new(dir, vertical).normalized());
    }

    pub fn is_flying(&self) -> bool {
        self.fly.is_some()
    }

    /// True once the card has cleared the viewport.
    pub fn is_gone(&self, viewport: Rect) -> bool {
        self.fly.is_some() && self.offset.x.abs() > viewport.width() + 200.0
    }

    /// How far the drag has gone toward a commit, -1 ..= 1.
    pub fn commit_progress(&self) -> f32 {
        (self.offset.x / COMMIT_DIST).clamp(-1.0, 1.0)
    }

    /// The direction the card would commit to if released right now.
    pub fn pending_dir(&self) -> Option<f32> {
        (self.offset.x.abs() >= COMMIT_DIST).then(|| self.offset.x.signum())
    }

    pub fn angle(&self) -> f32 {
        self.commit_progress() * MAX_TILT
    }

    /// Scale and opacity from the entry animation.
    pub fn entry_scale(&self) -> f32 {
        0.94 + 0.06 * ease_out(self.entry)
    }

    pub fn opacity(&self) -> f32 {
        let entering = ease_out(self.entry);
        // Fade as it leaves, so the card does not simply vanish at the edge.
        let leaving = if self.fly.is_some() {
            (1.0 - self.offset.x.abs() / 900.0).clamp(0.0, 1.0)
        } else {
            1.0
        };
        entering * leaving
    }
}

fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// Rotate `p` around `pivot` by `angle` radians (clockwise, screen coords).
pub fn rotate(p: Pos2, pivot: Pos2, angle: f32) -> Pos2 {
    let (sin, cos) = angle.sin_cos();
    let d = p - pivot;
    pivot + Vec2::new(d.x * cos - d.y * sin, d.x * sin + d.y * cos)
}

/// The four corners of `rect`, rotated about `pivot`.
pub fn corners(rect: Rect, pivot: Pos2, angle: f32) -> Vec<Pos2> {
    [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
    ]
    .into_iter()
    .map(|p| rotate(p, pivot, angle))
    .collect()
}

/// Paint a rectangular card face, rotated about its own centre.
pub fn face(painter: &egui::Painter, rect: Rect, angle: f32, fill: Color32, stroke: Stroke) {
    let pts = corners(rect, rect.center(), angle);
    painter.add(Shape::convex_polygon(pts, fill, stroke));
}

/// A crisp, rectangular hover halo around a card.
///
/// This is several fading hairlines rather than a blurred shadow: the card
/// remains square and instrument-like, while the larger active footprint is
/// immediately visible. The neutral blue-grey deliberately does not suggest
/// either the TRUE or FALSE swipe direction.
pub fn hover_glow(painter: &egui::Painter, rect: Rect, angle: f32, color: Color32, strength: f32) {
    let strength = strength.clamp(0.0, 1.0);
    if strength <= 0.01 {
        return;
    }

    for (expansion, width, fade) in [(2.0, 2.2, 0.72), (5.0, 1.8, 0.38), (9.0, 1.2, 0.17)] {
        face(
            painter,
            rect.expand(expansion * strength),
            angle,
            Color32::TRANSPARENT,
            Stroke::new(width * strength, color.gamma_multiply(fade * strength)),
        );
    }
}

/// Paint text belonging to a rotated card.
///
/// `local` is where the text's top-left corner sits in the card's own
/// unrotated coordinates. Rotating the anchor and then rotating the glyphs
/// about that anchor by the same angle reproduces a rigid rotation exactly.
pub fn text(
    painter: &egui::Painter,
    pivot: Pos2,
    angle: f32,
    local: Pos2,
    galley: Arc<egui::Galley>,
    color: Color32,
    opacity: f32,
) {
    let shape = TextShape::new(rotate(local, pivot, angle), galley, color)
        .with_override_text_color(color)
        .with_angle(angle)
        .with_opacity_factor(opacity);
    painter.add(shape);
}

/// Lay out `s` and paint it centred on `local_center` in card coordinates.
#[allow(clippy::too_many_arguments)]
pub fn text_centered(
    painter: &egui::Painter,
    pivot: Pos2,
    angle: f32,
    local_center: Pos2,
    s: &str,
    font: FontId,
    color: Color32,
    opacity: f32,
) {
    let galley = painter.layout_no_wrap(s.to_owned(), font, color);
    let local = local_center - galley.rect.size() / 2.0;
    text(painter, pivot, angle, local, galley, color, opacity);
}

/// The stack of cards behind the current one, so the deck has visible depth.
pub fn deck_behind(
    painter: &egui::Painter,
    rect: Rect,
    count: usize,
    fill: Color32,
    line: Color32,
) {
    for i in (1..=count).rev() {
        let k = i as f32;
        let ghost = Rect::from_center_size(
            rect.center() + Vec2::new(0.0, k * 9.0),
            rect.size() * (1.0 - k * 0.028),
        );
        let fade = 1.0 / (1.0 + k * 0.9);
        face(
            painter,
            ghost,
            0.0,
            fill.gamma_multiply(fade),
            Stroke::new(1.0, line.gamma_multiply(fade)),
        );
    }
}

/// Direction stamp ("TRUE" / "FALSE") that fades in as the card is dragged.
#[allow(clippy::too_many_arguments)]
pub fn stamp(
    painter: &egui::Painter,
    rect: Rect,
    angle: f32,
    label: &str,
    color: Color32,
    align: Align2,
    strength: f32,
) {
    if strength <= 0.01 {
        return;
    }
    let galley = painter.layout_no_wrap(label.to_owned(), crate::theme::text::stamp(), color);
    let size = galley.rect.size() + Vec2::new(26.0, 14.0);

    // Inset from the actual stamp width, not a guess: "FALSE" is a whole
    // character wider than "TRUE" and would otherwise hang off the edge.
    let inset = 24.0;
    let local_center = match align {
        // Below the topic label row, so the stamp never buries it.
        Align2::LEFT_TOP => rect.left_top() + Vec2::new(inset + size.x / 2.0, inset + 62.0),
        _ => rect.right_top() + Vec2::new(-inset - size.x / 2.0, inset + 62.0),
    };
    let box_rect = Rect::from_center_size(local_center, size);

    // Tilted a little further than the card, in the same direction: a stamp
    // slapped on askew. Tilting *against* the card would cancel its rotation
    // out and read as a rendering fault rather than a flourish.
    let tilt = if align == Align2::LEFT_TOP {
        -0.09
    } else {
        0.09
    };
    let pivot = rect.center();

    let pts = corners(box_rect, box_rect.center(), tilt)
        .into_iter()
        .map(|p| rotate(p, pivot, angle))
        .collect::<Vec<_>>();
    let mut ring = pts.clone();
    ring.push(pts[0]);
    painter.add(Shape::line(
        ring,
        Stroke::new(2.5, color.gamma_multiply(strength)),
    ));

    let local = local_center - galley.rect.size() / 2.0;
    let shape = TextShape::new(rotate(local, pivot, angle), galley, color)
        .with_override_text_color(color)
        .with_angle(angle + tilt)
        .with_opacity_factor(strength);
    painter.add(shape);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-3, "{a} != {b}");
    }

    #[test]
    fn rotation_by_zero_is_identity() {
        let p = Pos2::new(3.0, 7.0);
        let r = rotate(p, Pos2::new(1.0, 1.0), 0.0);
        approx(r.x, 3.0);
        approx(r.y, 7.0);
    }

    #[test]
    fn rotation_preserves_distance_from_the_pivot() {
        let pivot = Pos2::new(10.0, 10.0);
        let p = Pos2::new(40.0, 10.0);
        let r = rotate(p, pivot, 0.9);
        approx((r - pivot).length(), (p - pivot).length());
    }

    #[test]
    fn quarter_turn_maps_right_to_down() {
        // Screen coordinates: +y is down, so a positive angle is clockwise.
        let r = rotate(Pos2::new(1.0, 0.0), Pos2::ZERO, std::f32::consts::FRAC_PI_2);
        approx(r.x, 0.0);
        approx(r.y, 1.0);
    }

    #[test]
    fn commit_requires_passing_the_threshold() {
        let mut m = Motion::deal();
        m.offset.x = COMMIT_DIST - 1.0;
        assert_eq!(m.pending_dir(), None);
        m.offset.x = COMMIT_DIST;
        assert_eq!(m.pending_dir(), Some(1.0));
        m.offset.x = -COMMIT_DIST - 50.0;
        assert_eq!(m.pending_dir(), Some(-1.0));
    }

    #[test]
    fn a_released_card_springs_back_to_centre() {
        let mut m = Motion::deal();
        m.entry = 1.0;
        m.offset = Vec2::new(80.0, 12.0);
        for _ in 0..120 {
            m.update(1.0 / 60.0);
        }
        assert_eq!(m.offset, Vec2::ZERO, "card must settle exactly, not drift");
    }

    #[test]
    fn a_dragged_card_does_not_spring_back() {
        let mut m = Motion::deal();
        m.entry = 1.0;
        m.dragging = true;
        m.offset = Vec2::new(80.0, 0.0);
        m.update(1.0 / 60.0);
        assert_eq!(m.offset.x, 80.0);
    }

    #[test]
    fn a_launched_card_eventually_leaves_the_viewport() {
        let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 800.0));
        let mut m = Motion::deal();
        m.entry = 1.0;
        m.offset.x = COMMIT_DIST;
        m.launch(1.0);

        let mut frames = 0;
        while !m.is_gone(viewport) {
            m.update(1.0 / 60.0);
            frames += 1;
            assert!(frames < 600, "card never left the screen");
        }
        assert!(m.offset.x > 0.0, "must exit on the side it was launched to");
    }

    #[test]
    fn tilt_is_bounded_however_far_you_drag() {
        let mut m = Motion::deal();
        m.offset.x = 10_000.0;
        assert!(m.angle() <= MAX_TILT + 1e-6);
        m.offset.x = -10_000.0;
        assert!(m.angle() >= -MAX_TILT - 1e-6);
    }

    #[test]
    fn entry_animation_completes_and_stops_requesting_repaints() {
        let mut m = Motion::deal();
        assert!(m.opacity() < 1.0);
        for _ in 0..60 {
            m.update(1.0 / 60.0);
        }
        assert_eq!(m.entry, 1.0);
        approx(m.entry_scale(), 1.0);
        assert!(
            !m.update(1.0 / 60.0),
            "a settled card must not keep animating"
        );
    }
}
