//! Palette, typography and egui styling.
//!
//! Deep water: near-black blue-greens with a bioluminescent cyan accent.
//! Everything is rectangular — no corner radius anywhere in the app.

use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Stroke};

pub struct Palette;

#[allow(dead_code)]
impl Palette {
    /// Page background, the darkest surface.
    pub const BG: Color32 = Color32::from_rgb(6, 10, 13);
    /// Panels and bars sitting on the background.
    pub const SURFACE: Color32 = Color32::from_rgb(11, 18, 22);
    /// The card face itself.
    pub const CARD: Color32 = Color32::from_rgb(17, 27, 32);
    /// Cards further down the deck, drawn dimmer.
    pub const CARD_DEEP: Color32 = Color32::from_rgb(12, 20, 24);
    /// 1 px borders.
    pub const LINE: Color32 = Color32::from_rgb(30, 48, 55);
    pub const LINE_BRIGHT: Color32 = Color32::from_rgb(52, 82, 92);

    pub const TEXT: Color32 = Color32::from_rgb(216, 230, 234);
    pub const TEXT_DIM: Color32 = Color32::from_rgb(116, 145, 154);
    pub const TEXT_FAINT: Color32 = Color32::from_rgb(70, 92, 100);

    /// Cyan. Used for the accent, and for the "true" swipe direction.
    pub const ACCENT: Color32 = Color32::from_rgb(47, 224, 200);
    /// Violet. The "false" swipe direction.
    pub const VIOLET: Color32 = Color32::from_rgb(140, 112, 236);

    /// Answer feedback. Deliberately not red/green: spring green versus
    /// magenta stays legible for red-green colour blindness, and keeps the
    /// palette in the blue-green family.
    pub const CORRECT: Color32 = Color32::from_rgb(53, 224, 160);
    pub const WRONG: Color32 = Color32::from_rgb(236, 74, 160);
    pub const SKIP: Color32 = Color32::from_rgb(150, 168, 176);
}

/// Named text styles, so sizes are decided in one place.
pub mod text {
    use super::*;

    pub fn prompt(size: f32) -> FontId {
        FontId::new(size, FontFamily::Proportional)
    }
    pub fn body() -> FontId {
        FontId::new(15.0, FontFamily::Proportional)
    }
    pub fn small() -> FontId {
        FontId::new(12.5, FontFamily::Proportional)
    }
    /// All-caps labels, tracking-heavy chrome.
    pub fn label() -> FontId {
        FontId::new(11.0, FontFamily::Monospace)
    }
    pub fn stamp() -> FontId {
        FontId::new(46.0, FontFamily::Monospace)
    }
    pub fn title() -> FontId {
        FontId::new(21.0, FontFamily::Monospace)
    }
    pub fn number() -> FontId {
        FontId::new(30.0, FontFamily::Monospace)
    }
}

/// Widen a short label into tracked-out capitals: `S T A B I L I T Y`.
///
/// egui has no letter-spacing, and these labels are the main thing keeping
/// the chrome from looking like a default egui window.
pub fn tracked(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for (i, c) in s.chars().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.extend(c.to_uppercase());
    }
    out
}

pub fn install(ctx: &egui::Context) {
    install_fonts(ctx);

    ctx.all_styles_mut(style_of);
}

fn style_of(style: &mut egui::Style) {
    let v = &mut style.visuals;

    v.dark_mode = true;
    v.panel_fill = Palette::BG;
    v.window_fill = Palette::SURFACE;
    v.extreme_bg_color = Palette::BG;
    v.faint_bg_color = Palette::SURFACE;
    v.override_text_color = Some(Palette::TEXT);
    v.window_stroke = Stroke::new(1.0, Palette::LINE);
    v.selection.bg_fill = Palette::ACCENT.gamma_multiply(0.25);
    v.selection.stroke = Stroke::new(1.0, Palette::ACCENT);

    // Rectangular everywhere. This is the one rule the whole UI obeys.
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = CornerRadius::ZERO;
        w.expansion = 0.0;
    }
    v.window_corner_radius = CornerRadius::ZERO;
    v.menu_corner_radius = CornerRadius::ZERO;

    v.widgets.noninteractive.bg_fill = Palette::SURFACE;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Palette::LINE);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, Palette::TEXT_DIM);

    v.widgets.inactive.bg_fill = Palette::CARD;
    v.widgets.inactive.weak_bg_fill = Palette::CARD;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, Palette::LINE);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, Palette::TEXT);

    v.widgets.hovered.bg_fill = Palette::CARD;
    v.widgets.hovered.weak_bg_fill = Palette::CARD;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, Palette::ACCENT);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, Palette::TEXT);

    v.widgets.active.bg_fill = Palette::ACCENT.gamma_multiply(0.18);
    v.widgets.active.weak_bg_fill = Palette::ACCENT.gamma_multiply(0.18);
    v.widgets.active.bg_stroke = Stroke::new(1.0, Palette::ACCENT);
    v.widgets.active.fg_stroke = Stroke::new(1.0, Palette::TEXT);

    // No drop shadows: they read as rounded even on square corners.
    v.window_shadow = Default::default();
    v.popup_shadow = Default::default();

    style.spacing.item_spacing = egui::vec2(10.0, 10.0);
    style.spacing.button_padding = egui::vec2(14.0, 9.0);
}

/// Load a monospace face for the UI chrome.
///
/// Berkeley Mono if the system has it, JetBrains Mono otherwise, and egui's
/// built-in font if neither is installed. Nothing is vendored into the repo,
/// so no font licence travels with the source.
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let mut installed = None;

    for family in [
        "Berkeley Mono",
        "JetBrains Mono",
        "Iosevka",
        "DejaVu Sans Mono",
    ] {
        let Some(path) = find_font(family) else {
            continue;
        };
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };

        fonts.font_data.insert(
            "ui-mono".to_owned(),
            std::sync::Arc::new(egui::FontData::from_owned(bytes)),
        );
        installed = Some(family);
        break;
    }

    if installed.is_some() {
        // Monospace throughout, including prompts: it is a deliberate look,
        // and it keeps formulas like w0²/(s²+2ζw0s+w0²) aligned.
        for family in [FontFamily::Monospace, FontFamily::Proportional] {
            fonts
                .families
                .entry(family)
                .or_default()
                .insert(0, "ui-mono".to_owned());
        }
    }

    ctx.set_fonts(fonts);
}

/// Ask fontconfig where a family lives, and verify it actually returned that
/// family — `fc-match` always answers with *something*.
fn find_font(family: &str) -> Option<String> {
    let out = std::process::Command::new("fc-match")
        .args(["-f", "%{family}\t%{file}", family])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let (got, path) = text.split_once('\t')?;

    let wanted = family.to_lowercase();
    got.to_lowercase()
        .split(',')
        .any(|f| f.trim() == wanted)
        .then(|| path.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracked_spaces_out_capitals() {
        assert_eq!(tracked("bode"), "B O D E");
        assert_eq!(tracked(""), "");
        assert_eq!(tracked("a"), "A");
    }

    #[test]
    fn a_missing_family_is_not_silently_substituted() {
        // fontconfig would happily return some default face here.
        assert_eq!(find_font("No Such Font Family 12345"), None);
    }
}
