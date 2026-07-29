//! The shipped soundscapes, and a lexical colouriser for editing them.
//!
//! The presets are copies of documents from the Apteronotus `songs/`
//! directory, which is that project's specification corpus rather than a
//! library it exports — there is no crate to depend on for them, so they
//! travel here as assets. `audio::tests::every_preset_is_playable` is what
//! keeps a copy from quietly rotting as the engine moves next door.

use eframe::egui::text::{ByteIndex, LayoutJob, LayoutSection, TextFormat};
use eframe::egui::{
    self, Align2, Color32, FontId, Id, Pos2, Rect, Sense, Shape, Stroke, Vec2, vec2,
};
use std::ops::Range;

use crate::audio::{MAX_DECIBELS, MIN_DECIBELS, Soundscape, Status};
use crate::library::Library;
use crate::theme::{Palette, text, tracked};

/// One shipped soundscape.
pub(crate) struct Preset {
    /// What the picker shows.
    pub(crate) name: &'static str,
    /// One line on what it sounds like — this is a mood picker, not a
    /// feature list.
    pub(crate) mood: &'static str,
    pub(crate) source: &'static str,
}

pub(crate) const PRESETS: &[Preset] = &[
    Preset {
        name: "Waves",
        mood: "slow surf and struck glass — nothing has an onset",
        source: apteronotus_songs::WAVES,
    },
    Preset {
        name: "Drift",
        mood: "synthwave with the arrangement taken out",
        source: apteronotus_songs::DRIFT,
    },
    Preset {
        name: "Neon",
        mood: "a through-composed noir cue that ends rather than loops",
        source: apteronotus_songs::NEON,
    },
];

/// The soundscape a fresh installation opens with.
///
/// Deliberately the quietest one: this plays under revision, and the first
/// impression of a study app should not be a kick drum.
pub(crate) fn default_source() -> &'static str {
    PRESETS[0].source
}

/// Which shipped preset `source` is, verbatim.
pub(crate) fn opened_preset(source: &str) -> Option<usize> {
    PRESETS.iter().position(|preset| preset.source == source)
}

/// What a new document starts as.
///
/// Not empty: an empty score is not playable, so an empty editor is a screen
/// whose only possible next action is an error message. This is the smallest
/// thing that makes a sound, and it says where the vocabulary is documented.
pub(crate) const NEW_SOURCE: &str = r#"-- A new soundscape. The language is Apteronotus's, and every shipped template
-- is a worked example of it — opening one and saving a copy is the short way in.

tempo(48)

hum = voice {
  graph = function(n)
    local osc = sine(n.hz) + sine(n.hz * 2.005) * 0.3
    return osc * 0.25
        >> mul(adsr(secs(4), secs(2), 0.8, secs(6)))
        >> pan(rand(-0.3, 0.3))
  end,
}

play(hum, "<c2 g2 a#1 f2>" >> slow(8) >> velocity(0.3))

master(limiter(0.05, 1.2) >> mul(0.7))
"#;

// ---------------------------------------------------------------------------
// Colouring
//
// Purely lexical, and presentation only: it never parses and never decides
// whether a document is valid. Running it is the only thing that does that.
// Adapted from the editor in the Apteronotus tree — the lexer is the same
// shape, the colours are this app's.
// ---------------------------------------------------------------------------

/// Lua's own reserved words.
const KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in",
    "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

/// The sandbox vocabulary: what a soundscape is here to call. Mirrors the
/// Apteronotus bindings; a name the sandbox does not bind is left uncoloured
/// rather than promised.
#[rustfmt::skip]
const BUILTINS: &[&str] = &[
    // program structure
    "voice", "patch", "control", "control_input", "audio_input", "bus",
    "send", "run", "master", "play", "tempo", "key", "timeline", "at", "span",
    "pattern", "note", "chord", "anchor", "voicing", "root_notes", "octave",
    "hold", "velocity", "curve", "phase",
    // pattern transforms
    "fast", "slow", "shift", "late", "early", "rev", "segment", "range",
    "degrade", "every", "off", "sometimes", "ply", "arp", "duck",
    // sources
    "sine", "cosine", "saw", "soft_saw", "perlin", "pulse", "noise", "pink",
    "impulse", "rand", "dc", "zero", "init_random", "init_rand",
    // processing
    "lowpass", "highpass", "bandpass", "peak", "moog", "pluck", "shape",
    "dcblock", "delay", "diffuse", "fdn", "param", "reverb", "limiter",
    "chorus", "ensemble", "envelope_follower", "pitch_tracker", "feedback",
    "slew", "gate_env", "ring", "ringmod", "tremolo", "pan", "mix", "scale",
    "exp2", "semitones", "note_hz", "window", "to", "add", "sub", "mul",
    "div", "clamp", "neg",
    // envelopes and the curve basis
    "adsr", "step", "ramp", "line", "decay",
    // time
    "secs", "ms", "bars", "beats",
    // the Lua subset the sandbox keeps
    "math", "table", "string", "coroutine", "ipairs", "pairs", "next",
    "tostring", "tonumber", "select", "type", "error", "pcall", "assert",
    "setmetatable", "getmetatable", "rawget", "rawset", "rawequal", "rawlen",
];

// Six roles, drawn from the palette the rest of the app uses, with one
// addition. Deliberately absent: `CORRECT` and `WRONG`. Those two are the
// app's verdict colours — they mean "you were right" and "you were wrong" —
// and a score being edited is never being marked. Borrowing spring green for
// numeric literals would spend the one pair of colours the interface reserves
// for judgement on something that is not one.

/// A mini-notation string: the part of a soundscape most worth seeing while
/// typing, and the closest thing a score has to content.
const CODE_STRING: Color32 = Palette::ACCENT;
/// The sandbox vocabulary — what makes this document a soundscape rather than
/// a Lua file.
const CODE_BUILTIN: Color32 = Palette::VIOLET;
/// Numeric literals. A synthesis score is mostly coefficients, so they earn a
/// colour; a pale blue keeps it inside the blue-green family.
const CODE_NUMBER: Color32 = Color32::from_rgb(0x8f, 0xdc, 0xf0);
/// Lua's own words. Structure recedes, the way chrome does everywhere else.
const CODE_KEYWORD: Color32 = Palette::TEXT_DIM;
const CODE_COMMENT: Color32 = Palette::TEXT_FAINT;
const CODE_PUNCT: Color32 = Palette::TEXT_DIM;

/// Build a colourised, unwrapped layout job for `source`.
pub(crate) fn layout(source: &str, font: FontId) -> LayoutJob {
    let mut job = LayoutJob {
        text: source.to_owned(),
        ..Default::default()
    };
    // A code editor scrolls sideways; it does not reflow. Wrapping would also
    // desynchronise the line-number gutter, which counts newlines.
    job.wrap.max_width = f32::INFINITY;
    for (range, color) in spans(source) {
        job.sections.push(LayoutSection {
            leading_space: 0.0,
            byte_range: ByteIndex(range.start)..ByteIndex(range.end),
            format: TextFormat::simple(font.clone(), color),
        });
    }
    job
}

fn spans(source: &str) -> Vec<(Range<usize>, Color32)> {
    let bytes = source.as_bytes();
    let mut spans = Vec::new();
    let mut at = 0;

    while at < bytes.len() {
        let start = at;
        let color = match bytes[at] {
            b'-' if bytes.get(at + 1) == Some(&b'-') => {
                at += 2;
                match long_bracket(bytes, at) {
                    Some(end) => at = end,
                    None => at = line_end(bytes, at),
                }
                CODE_COMMENT
            }
            b'"' | b'\'' => {
                at = quoted(bytes, at);
                CODE_STRING
            }
            b'[' if long_bracket(bytes, at).is_some() => {
                at = long_bracket(bytes, at).expect("checked above");
                CODE_STRING
            }
            b'0'..=b'9' => {
                at = number(bytes, at);
                CODE_NUMBER
            }
            b'.' if matches!(bytes.get(at + 1), Some(b'0'..=b'9')) => {
                at = number(bytes, at);
                CODE_NUMBER
            }
            byte if is_word_start(byte) => {
                while at < bytes.len() && is_word(bytes[at]) {
                    at += 1;
                }
                let word = &source[start..at];
                if KEYWORDS.contains(&word) {
                    CODE_KEYWORD
                } else if BUILTINS.contains(&word) {
                    CODE_BUILTIN
                } else {
                    Palette::TEXT
                }
            }
            byte if byte.is_ascii_whitespace() => {
                while at < bytes.len() && bytes[at].is_ascii_whitespace() {
                    at += 1;
                }
                Palette::TEXT
            }
            _ => {
                // Advance by a whole character so a multi-byte glyph is never
                // split across two sections.
                at += char_width(bytes[at]);
                CODE_PUNCT
            }
        };
        spans.push((start..at, color));
    }

    spans
}

fn char_width(lead: u8) -> usize {
    match lead {
        0x00..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        _ => 4,
    }
}

fn is_word_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn line_end(bytes: &[u8], mut at: usize) -> usize {
    while at < bytes.len() && bytes[at] != b'\n' {
        at += 1;
    }
    at
}

fn number(bytes: &[u8], mut at: usize) -> usize {
    while at < bytes.len()
        && (bytes[at].is_ascii_alphanumeric()
            || bytes[at] == b'.'
            || ((bytes[at] == b'+' || bytes[at] == b'-')
                && matches!(bytes[at - 1], b'e' | b'E' | b'p' | b'P')))
    {
        at += 1;
    }
    at
}

fn quoted(bytes: &[u8], mut at: usize) -> usize {
    let quote = bytes[at];
    at += 1;
    while at < bytes.len() {
        match bytes[at] {
            b'\\' => at += 2,
            b'\n' => return at,
            byte if byte == quote => return at + 1,
            _ => at += 1,
        }
    }
    bytes.len()
}

/// If a `[=*[` long bracket opens at `at`, return the index just past its close.
fn long_bracket(bytes: &[u8], at: usize) -> Option<usize> {
    if bytes.get(at) != Some(&b'[') {
        return None;
    }
    let mut level = 0;
    let mut cursor = at + 1;
    while bytes.get(cursor) == Some(&b'=') {
        level += 1;
        cursor += 1;
    }
    if bytes.get(cursor) != Some(&b'[') {
        return None;
    }
    cursor += 1;

    while cursor < bytes.len() {
        if bytes[cursor] == b']' {
            let mut closing = 0;
            let mut probe = cursor + 1;
            while bytes.get(probe) == Some(&b'=') {
                closing += 1;
                probe += 1;
            }
            if closing == level && bytes.get(probe) == Some(&b']') {
                return Some(probe + 1);
            }
        }
        cursor += 1;
    }
    Some(bytes.len())
}

// ---------------------------------------------------------------------------
// The chrome control
//
// One split control rather than two buttons: muting and editing are the same
// subject, and the interface already spends its top-right corner on Back, the
// cog and the formula sheet. It sits immediately left of the cog wherever the
// cog is — top chrome on an inner screen, bottom row on the deck list — so the
// two are always found together.
// ---------------------------------------------------------------------------

/// Both halves plus the hairline between them.
pub(crate) const BUTTON_WIDTH: f32 = 96.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    ToggleTransport,
    Open,
}

/// Where the split control goes, given the settings cog it sits beside.
pub(crate) fn button_rect(cog: Rect) -> Rect {
    Rect::from_min_size(
        cog.min - vec2(BUTTON_WIDTH + 8.0, 0.0),
        vec2(BUTTON_WIDTH, cog.height()),
    )
}

/// Draw the split control. The left half is the transport and carries the
/// state; the right half opens the editor.
pub(crate) fn audio_button(
    ui: &egui::Ui,
    rect: Rect,
    stopped: bool,
    status: &Status,
    id: &'static str,
) -> Option<Action> {
    let (left, right) = rect.split_left_right_at_fraction(0.5);
    let transport = ui
        .interact(left, Id::new(("soundscape-transport", id)), Sense::click())
        .on_hover_text(if stopped {
            "Play the background soundscape (M)"
        } else {
            "Stop the background soundscape (M)"
        });
    let open = ui
        .interact(right, Id::new(("soundscape-open", id)), Sense::click())
        .on_hover_text("Soundscape — choose or edit what plays while you study");

    let transport_hot = transport.hovered() || transport.has_focus();
    let open_hot = open.hovered() || open.has_focus();
    let painter = ui.painter();

    // The frame is one control, so it brightens as a whole; only the fill
    // distinguishes which half the pointer is over.
    let edge = if transport_hot || open_hot {
        Palette::ACCENT
    } else {
        // The same rest edge as the cog and the formula sheet beside it: this
        // is one more control in that row, not a quieter class of thing.
        Palette::TEXT_DIM
    };
    painter.rect_filled(rect, 0, Palette::SURFACE);
    if transport_hot {
        painter.rect_filled(left, 0, Palette::CARD);
    }
    if open_hot {
        painter.rect_filled(right, 0, Palette::CARD);
    }
    painter.rect_stroke(rect, 0, Stroke::new(1.0, edge), egui::StrokeKind::Inside);
    painter.line_segment(
        [
            Pos2::new(left.right(), rect.top() + 8.0),
            Pos2::new(left.right(), rect.bottom() - 8.0),
        ],
        Stroke::new(1.0, Palette::LINE),
    );

    // The glyph shows the *action*, as a transport control does everywhere:
    // a triangle to start, a square to stop. The colour shows the state, which
    // is the one piece of soundscape status worth carrying in the chrome —
    // silent, sounding, or refused. A refusal is magenta for the same reason a
    // wrong answer is: it is this app's colour for "that did not work", and
    // red belongs to a design language this one does not use.
    let colour = match (stopped, status) {
        (true, _) if transport_hot => Palette::TEXT_DIM,
        (true, _) => Palette::TEXT_FAINT,
        (false, Status::Failed(_)) => Palette::WRONG,
        (false, Status::Playing) => Palette::ACCENT,
        (false, _) => Palette::TEXT_DIM,
    };
    paint_transport(painter, left.center(), colour, stopped);
    paint_waveform(
        painter,
        right.center(),
        if open_hot {
            Palette::ACCENT
        } else {
            Palette::TEXT_DIM
        },
    );

    transport.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            if stopped { "Play" } else { "Stop" },
        )
    });
    open.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Soundscape"));

    if transport.clicked() {
        Some(Action::ToggleTransport)
    } else if open.clicked() {
        Some(Action::Open)
    } else {
        None
    }
}

/// The transport glyph: a triangle to start, a square to stop. Both are the
/// universal marks for it, both are straight-edged, and neither could be
/// mistaken for the waveform beside it.
fn paint_transport(painter: &egui::Painter, centre: Pos2, colour: Color32, stopped: bool) {
    if stopped {
        // Nudged right by a third of its width: a triangle's visual centre is
        // not its bounding box's, and centred by the box it reads as leaning.
        let c = centre + vec2(1.5, 0.0);
        painter.add(Shape::convex_polygon(
            vec![
                Pos2::new(c.x - 5.0, c.y - 8.0),
                Pos2::new(c.x + 7.0, c.y),
                Pos2::new(c.x - 5.0, c.y + 8.0),
            ],
            colour,
            Stroke::NONE,
        ));
        return;
    }
    painter.rect_filled(Rect::from_center_size(centre, Vec2::splat(13.0)), 0, colour);
}

/// One cycle of a sine: the score, as opposed to the speaker's playback state.
fn paint_waveform(painter: &egui::Painter, centre: Pos2, colour: Color32) {
    const WIDTH: f32 = 24.0;
    const STEPS: usize = 24;
    let points = (0..=STEPS)
        .map(|step| {
            let t = step as f32 / STEPS as f32;
            Pos2::new(
                centre.x - WIDTH / 2.0 + WIDTH * t,
                centre.y - (t * std::f32::consts::TAU).sin() * 6.5,
            )
        })
        .collect();
    painter.add(Shape::line(points, Stroke::new(1.4, colour)));
}

// ---------------------------------------------------------------------------
// The editor screen
// ---------------------------------------------------------------------------

/// Where the document in the editor came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    /// A shipped template. Compiled into the binary, so it cannot be written
    /// back to — which is the whole reason "save" reads "save as" here.
    Template(usize),
    /// A score in the library, by name.
    File(String),
    /// Never saved anywhere: a new document, or one restored from the
    /// settings file after the score it came from was deleted.
    Unsaved,
}

/// Everything the soundscape screen remembers between frames.
pub(crate) struct Editor {
    pub(crate) library: Library,
    /// What the name field says. It is what "save" saves as, so editing it and
    /// saving is how a score is copied or renamed — the same gesture as Save
    /// As anywhere else.
    name: String,
    origin: Origin,
    cursor_line: usize,
    /// A delete that has been asked for but not confirmed. Two clicks,
    /// because there is no undo for a file and no bin to fish it out of.
    armed_delete: Option<String>,
}

impl Editor {
    /// Open on `source`, which the settings file says came from `file`.
    pub(crate) fn new(source: &str, file: Option<&str>, library: Library) -> Self {
        let origin = match file {
            // A name whose score has since been deleted, or whose contents
            // have drifted from what is loaded, is not that file any more.
            Some(name) if library.source(name) == Some(source) => Origin::File(name.to_owned()),
            _ => match opened_preset(source) {
                Some(index) => Origin::Template(index),
                None => Origin::Unsaved,
            },
        };
        Self {
            name: default_name(&origin),
            origin,
            library,
            cursor_line: 0,
            armed_delete: None,
        }
    }

    /// The library entry the editor is on, for the settings file.
    pub(crate) fn file(&self) -> Option<&str> {
        match &self.origin {
            Origin::File(name) => Some(name.as_str()),
            Origin::Template(_) | Origin::Unsaved => None,
        }
    }

    /// Adopt a document, from wherever it came.
    fn open(&mut self, soundscape: &mut Soundscape, source: String, origin: Origin) {
        soundscape.set_source(source);
        self.name = default_name(&origin);
        self.origin = origin;
        self.cursor_line = 0;
        self.armed_delete = None;
    }

    /// Record a completed save: the editor is now on that file, under that
    /// name, whatever it was on before.
    pub(crate) fn saved_as(&mut self, name: String, source: &str) {
        self.library.insert(&name, source);
        self.name = name.clone();
        self.origin = Origin::File(name);
        self.armed_delete = None;
    }

    /// Record a completed delete. A deleted score stays *open* — it is still
    /// the document being edited and still what is playing — it simply has
    /// nowhere to be saved back to.
    pub(crate) fn deleted(&mut self, name: &str) {
        self.library.remove(name);
        if self.origin == Origin::File(name.to_owned()) {
            self.origin = Origin::Unsaved;
        }
        self.armed_delete = None;
    }

    /// Whether "save" would replace the file the editor is on, or make a new
    /// one. A template is never the former, and neither is a retyped name.
    fn overwrites(&self) -> bool {
        matches!(&self.origin, Origin::File(name) if *name == crate::library::clean_name(&self.name))
    }
}

fn default_name(origin: &Origin) -> String {
    match origin {
        Origin::Template(index) => crate::library::clean_name(PRESETS[*index].name),
        Origin::File(name) => name.clone(),
        Origin::Unsaved => "untitled".into(),
    }
}

/// What the screen asks the shell to do. Everything that touches storage
/// leaves here as one of these, the same way importing a deck does.
pub(crate) enum Command {
    /// The persisted state — score, transport or level — changed.
    Persist,
    Save {
        name: String,
        source: String,
    },
    Delete {
        name: String,
    },
    /// Browser-only: natively the library is a directory the caption names.
    #[cfg(target_arch = "wasm32")]
    Download {
        name: String,
        source: String,
    },
}

/// Draw the soundscape screen.
pub(crate) fn screen(
    ui: &mut egui::Ui,
    soundscape: &mut Soundscape,
    editor: &mut Editor,
) -> Option<Command> {
    let full = ui.available_rect_before_wrap();
    let panel = Rect::from_center_size(
        full.center(),
        Vec2::new(full.width().min(920.0), full.height().min(700.0)),
    );
    ui.painter().text(
        panel.left_top() + vec2(0.0, 10.0),
        Align2::LEFT_TOP,
        "SOUNDSCAPE",
        text::title(),
        Palette::TEXT,
    );
    ui.painter().line_segment(
        [
            panel.left_top() + vec2(0.0, 54.0),
            panel.right_top() + vec2(0.0, 54.0),
        ],
        Stroke::new(1.0, Palette::LINE),
    );

    let content = Rect::from_min_max(panel.left_top() + vec2(0.0, 72.0), panel.right_bottom());
    let mut command = None;
    ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| {
        ui.set_width(content.width());

        library_section(ui, soundscape, editor, &mut command);

        ui.add_space(14.0);
        let save = score_header(ui, editor);
        ui.add_space(6.0);

        // Reserve the status strip before the editor, so the editor takes
        // whatever is left rather than pushing the strip off the panel.
        let editor_height = (ui.available_height() - 74.0).max(140.0);
        score_editor(ui, soundscape, &mut editor.cursor_line, editor_height);
        if save {
            command = Some(Command::Save {
                name: editor
                    .library
                    .free_name_unless(&editor.name, editor.overwrites()),
                source: soundscape.source().to_owned(),
            });
        }

        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let (play, _) = ui.allocate_exact_size(Vec2::new(126.0, 40.0), Sense::hover());
            if verb_button(ui, play, "restart", !soundscape.stopped()) {
                soundscape.restart();
                command.get_or_insert(Command::Persist);
            }
            let (stop, _) = ui.allocate_exact_size(Vec2::new(126.0, 40.0), Sense::hover());
            let stopped = soundscape.stopped();
            if verb_button(ui, stop, if stopped { "start" } else { "stop" }, false) {
                soundscape.set_stopped(!stopped);
                command.get_or_insert(Command::Persist);
            }
            ui.add_space(10.0);
            // Measured, not guessed: `tracked` interleaves spaces, so the
            // rendered width of a five-letter word is not something to
            // estimate — and the chosen typeface is a user setting anyway.
            caption(ui, "level", 40.0);
            let readout = format_decibels(soundscape.decibels());
            let readout_width = ui
                .painter()
                .layout_no_wrap(readout.clone(), text::small(), Palette::TEXT_DIM)
                .rect
                .width();
            let (track, _) = ui.allocate_exact_size(
                Vec2::new(
                    (ui.available_width() - readout_width - 90.0).clamp(80.0, 190.0),
                    40.0,
                ),
                Sense::hover(),
            );
            let mut decibels = soundscape.decibels();
            if fader(ui, track, &mut decibels) {
                soundscape.set_decibels(decibels);
                command.get_or_insert(Command::Persist);
            }
            let (label, _) =
                ui.allocate_exact_size(Vec2::new(readout_width + 8.0, 40.0), Sense::hover());
            ui.painter().text(
                Pos2::new(label.right(), label.center().y),
                Align2::RIGHT_CENTER,
                readout,
                text::small(),
                Palette::TEXT_DIM,
            );

            ui.add_space(6.0);
            let (colour, message) = match soundscape.status() {
                Status::Silent => (Palette::TEXT_FAINT, "silent".to_owned()),
                Status::Starting => (Palette::TEXT_DIM, "starting".to_owned()),
                Status::Playing => (Palette::ACCENT, "playing".to_owned()),
                Status::Failed(error) => (Palette::WRONG, error.clone()),
            };
            ui.add(
                egui::Label::new(
                    egui::RichText::new(message)
                        .font(text::small())
                        .color(colour),
                )
                .truncate(),
            );
        });
    });
    command
}

/// The templates, then whatever has been saved.
///
/// Two lists rather than one, because they are not the same kind of thing: a
/// template is shipped, identical in every installation and impossible to
/// damage, and a file is the user's. Merging them would mean explaining, per
/// row, which one you were about to overwrite.
fn library_section(
    ui: &mut egui::Ui,
    soundscape: &mut Soundscape,
    editor: &mut Editor,
    command: &mut Option<Command>,
) {
    ui.label(
        egui::RichText::new(tracked("templates"))
            .font(text::label())
            .color(Palette::TEXT_FAINT),
    );
    ui.add_space(8.0);

    let open_template = match editor.origin {
        Origin::Template(index) => Some(index),
        _ => None,
    };
    let mut picked = None;
    ui.horizontal_wrapped(|ui| {
        for (index, preset) in PRESETS.iter().enumerate() {
            let width = ((ui.available_width() - 24.0) / 2.0).max(190.0);
            let (row, _) = ui.allocate_exact_size(Vec2::new(width, 46.0), Sense::hover());
            if preset_row(ui, row, preset, open_template == Some(index)) {
                picked = Some(index);
            }
        }
    });
    if let Some(index) = picked {
        editor.open(
            soundscape,
            PRESETS[index].source.to_owned(),
            Origin::Template(index),
        );
        *command = Some(Command::Persist);
    }

    ui.add_space(14.0);
    ui.horizontal(|ui| {
        caption(ui, "saved", 26.0);
        ui.add_space(8.0);
        ui.add(
            egui::Label::new(
                egui::RichText::new(STORAGE_NOTE)
                    .font(text::small())
                    .color(Palette::TEXT_FAINT),
            )
            .truncate(),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let (new, _) = ui.allocate_exact_size(Vec2::new(80.0, 26.0), Sense::hover());
            if small_button(ui, new, "new", "library-new", false) {
                editor.open(soundscape, NEW_SOURCE.to_owned(), Origin::Unsaved);
                *command = Some(Command::Persist);
            }
        });
    });
    ui.add_space(6.0);
    saved_list(ui, soundscape, editor, command);
}

/// The saved scores, in a well of their own so an empty library is visibly a
/// place things go rather than a gap in the screen.
fn saved_list(
    ui: &mut egui::Ui,
    soundscape: &mut Soundscape,
    editor: &mut Editor,
    command: &mut Option<Command>,
) {
    const ROW: f32 = 28.0;
    let rows = editor.library.scores().len().clamp(1, 3) as f32;
    let well = Rect::from_min_size(
        ui.cursor().min,
        Vec2::new(ui.available_width(), rows * ROW + 8.0),
    );
    ui.painter().rect_filled(well, 0, Palette::BG);
    ui.painter().rect_stroke(
        well,
        0,
        Stroke::new(1.0, Palette::LINE),
        egui::StrokeKind::Inside,
    );
    ui.advance_cursor_after_rect(well);

    if editor.library.scores().is_empty() {
        ui.painter().text(
            well.left_center() + vec2(12.0, 0.0),
            Align2::LEFT_CENTER,
            "nothing saved yet — edit a template and save it under a name",
            text::small(),
            Palette::TEXT_FAINT,
        );
        return;
    }

    let inner = well.shrink(4.0);
    let open = editor.file().map(str::to_owned);
    let mut action = None;
    ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
        ui.set_clip_rect(inner);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for score in editor.library.scores() {
                    let (row, _) = ui
                        .allocate_exact_size(Vec2::new(ui.available_width(), ROW), Sense::hover());
                    if let Some(picked) = saved_row(
                        ui,
                        row,
                        &score.name,
                        open.as_deref() == Some(score.name.as_str()),
                        editor.armed_delete.as_deref() == Some(score.name.as_str()),
                    ) {
                        action = Some((picked, score.name.clone()));
                    }
                }
            });
    });

    let Some((action, name)) = action else { return };
    match action {
        RowAction::Open => {
            if let Some(source) = editor.library.source(&name).map(str::to_owned) {
                editor.open(soundscape, source, Origin::File(name));
                *command = Some(Command::Persist);
            }
        }
        #[cfg(target_arch = "wasm32")]
        RowAction::Download => {
            let source = editor.library.source(&name).unwrap_or_default().to_owned();
            *command = Some(Command::Download { name, source });
        }
        // The first click arms, the second one asks for it. Arming a different
        // row disarms this one, so a mis-click is undone by moving on.
        RowAction::Delete if editor.armed_delete.as_deref() != Some(name.as_str()) => {
            editor.armed_delete = Some(name);
        }
        RowAction::Delete => *command = Some(Command::Delete { name }),
    }
}

/// The name of the document, and the one button that writes it anywhere.
///
/// Returns whether a save was asked for. The label is the whole feature: over
/// a template — or over a name that has been retyped — it says "save as",
/// because that is what it will do.
fn score_header(ui: &mut egui::Ui, editor: &mut Editor) -> bool {
    let mut save = false;
    ui.horizontal(|ui| {
        caption(ui, "score", 30.0);
        ui.add_space(8.0);
        ui.add(
            egui::TextEdit::singleline(&mut editor.name)
                .id_salt("soundscape-name")
                .font(text::small())
                .hint_text("name")
                .desired_width(200.0)
                .min_size(Vec2::new(0.0, 30.0)),
        );
        if let Origin::Template(index) = editor.origin {
            ui.add_space(8.0);
            ui.add(
                egui::Label::new(
                    egui::RichText::new(format!(
                        "{} is a template — saving makes a copy",
                        PRESETS[index].name
                    ))
                    .font(text::small())
                    .color(Palette::TEXT_FAINT),
                )
                .truncate(),
            );
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let (button, _) = ui.allocate_exact_size(Vec2::new(110.0, 30.0), Sense::hover());
            let label = if editor.overwrites() {
                "save"
            } else {
                "save as"
            };
            let named = !crate::library::clean_name(&editor.name).is_empty();
            save = small_button(ui, button, label, "soundscape-save", named) && named;
        });
    });
    save
}

/// The fader's readout. The bottom of the travel is silence, so it says so
/// rather than claiming −60 dB of something.
fn format_decibels(decibels: f64) -> String {
    if decibels <= MIN_DECIBELS {
        "−∞ dB".into()
    } else {
        // The typographic minus, so a level does not read as a list item and
        // matches the −∞ at the bottom of the same travel.
        format!("{decibels:.0} dB").replace('-', "−")
    }
}

/// Where saved scores are, said once, next to the list of them.
///
/// Natively that is a path, because it is a path you can open in an editor and
/// copy onto a stick. In the browser there is no path to give, so it says what
/// is true there instead, and every row offers the download that gets a score
/// out.
#[cfg(not(target_arch = "wasm32"))]
const STORAGE_NOTE: &str = "~/idiosepius/soundscapes";
#[cfg(target_arch = "wasm32")]
const STORAGE_NOTE: &str = "kept in this browser — download to take one with you";

/// A tracked-out chrome caption, measured rather than estimated: `tracked`
/// interleaves spaces, and the typeface is a user setting.
fn caption(ui: &mut egui::Ui, label: &str, height: f32) {
    let caption = tracked(label);
    let width = ui
        .painter()
        .layout_no_wrap(caption.clone(), text::label(), Palette::TEXT_FAINT)
        .rect
        .width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width + 2.0, height), Sense::hover());
    ui.painter().text(
        Pos2::new(rect.left(), rect.center().y),
        Align2::LEFT_CENTER,
        caption,
        text::label(),
        Palette::TEXT_FAINT,
    );
}

/// What a row in the saved list was asked to do.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RowAction {
    Open,
    #[cfg(target_arch = "wasm32")]
    Download,
    Delete,
}

/// One saved score: its name opens it, and the actions sit at the right.
fn saved_row(ui: &egui::Ui, rect: Rect, name: &str, open: bool, armed: bool) -> Option<RowAction> {
    let mut action = None;
    let mut right = rect.right() - 6.0;
    let mut button = |label: &str, id: &str, live: bool| -> bool {
        let width = ui
            .painter()
            .layout_no_wrap(tracked(label), text::small(), Palette::TEXT_DIM)
            .rect
            .width()
            + 16.0;
        let button = Rect::from_min_max(
            Pos2::new(right - width, rect.top() + 3.0),
            Pos2::new(right, rect.bottom() - 3.0),
        );
        right -= width + 6.0;
        small_button(ui, button, label, id, live)
    };
    if armed && button("really?", &format!("{name}-confirm"), true) {
        action = Some(RowAction::Delete);
    }
    if !armed && button("delete", &format!("{name}-delete"), false) {
        action = Some(RowAction::Delete);
    }
    // Downloading is how a score leaves the browser sandbox. Natively the
    // directory is right there in the caption above, so there is nothing to
    // offer.
    #[cfg(target_arch = "wasm32")]
    if button("download", &format!("{name}-download"), false) {
        action = Some(RowAction::Download);
    }

    let label = Rect::from_min_max(rect.left_top(), Pos2::new(right, rect.bottom()));
    let response = ui.interact(label, Id::new(("soundscape-file", name)), Sense::click());
    let hot = response.hovered() || response.has_focus();
    if hot {
        ui.painter().rect_filled(label, 0, Palette::CARD);
    }
    if open {
        ui.painter().rect_filled(
            Rect::from_min_size(rect.left_top(), Vec2::new(2.0, rect.height())),
            0,
            Palette::ACCENT,
        );
    }
    ui.painter().text(
        Pos2::new(rect.left() + 12.0, rect.center().y),
        Align2::LEFT_CENTER,
        name,
        text::small(),
        match (open, hot) {
            (true, _) => Palette::ACCENT,
            (false, true) => Palette::TEXT,
            (false, false) => Palette::TEXT_DIM,
        },
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name));
    if response.clicked() {
        action = Some(RowAction::Open);
    }
    action
}

/// A short chrome button: the same rectangle as `verb_button`, at the size the
/// list rows and the header have room for.
fn small_button(ui: &egui::Ui, rect: Rect, label: &str, id: &str, live: bool) -> bool {
    let response = ui.interact(rect, Id::new(("soundscape-small", id)), Sense::click());
    let hot = response.hovered() || response.has_focus();
    let colour = if hot || live {
        Palette::ACCENT
    } else {
        Palette::TEXT_DIM
    };
    let painter = ui.painter();
    painter.rect_filled(rect, 0, if hot { Palette::CARD } else { Palette::SURFACE });
    painter.rect_stroke(rect, 0, Stroke::new(1.0, colour), egui::StrokeKind::Inside);
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        tracked(label),
        text::small(),
        colour,
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    response.clicked()
}

/// The editable document, in a well with a line-number gutter.
fn score_editor(
    ui: &mut egui::Ui,
    soundscape: &mut Soundscape,
    cursor_line: &mut usize,
    height: f32,
) {
    let well = Rect::from_min_size(ui.cursor().min, Vec2::new(ui.available_width(), height));
    ui.painter().rect_filled(well, 0, Palette::BG);
    ui.painter().rect_stroke(
        well,
        0,
        Stroke::new(1.0, Palette::LINE),
        egui::StrokeKind::Inside,
    );

    let font = text::code();
    let row_height = ui.fonts_mut(|fonts| fonts.row_height(&font));
    let lines = soundscape.source().lines().count().max(1);
    let gutter_width = ui
        .painter()
        .layout_no_wrap(format!("{lines}"), font.clone(), Palette::TEXT_FAINT)
        .rect
        .width()
        + 16.0;

    let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, _width: f32| {
        let job = layout(buffer.as_str(), font.clone());
        ui.fonts_mut(|fonts| fonts.layout_job(job))
    };

    let inner = well.shrink(8.0);
    ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
        ui.set_clip_rect(inner);
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let numbers = ui
                        .allocate_exact_size(
                            Vec2::new(gutter_width, row_height * lines as f32),
                            Sense::hover(),
                        )
                        .0;
                    for line in 0..lines {
                        ui.painter().text(
                            Pos2::new(
                                numbers.right() - 8.0,
                                numbers.top() + row_height * line as f32,
                            ),
                            Align2::RIGHT_TOP,
                            format!("{}", line + 1),
                            font.clone(),
                            if line == *cursor_line {
                                Palette::TEXT_DIM
                            } else {
                                Palette::TEXT_FAINT
                            },
                        );
                    }

                    let rows = (inner.height() / row_height).floor() as usize;
                    let output = egui::TextEdit::multiline(soundscape.source_mut())
                        .font(font.clone())
                        .layouter(&mut layouter)
                        .lock_focus(true)
                        .frame(egui::Frame::NONE)
                        .margin(egui::Margin::ZERO)
                        .desired_rows(rows.max(lines))
                        .desired_width(f32::INFINITY)
                        .show(ui);

                    // The layouter never wraps, so a source line is exactly a
                    // gutter row: count the newlines the cursor has passed.
                    if let Some(cursor) = output.cursor_range {
                        *cursor_line = soundscape
                            .source()
                            .chars()
                            .take(cursor.primary.index.0)
                            .filter(|character| *character == '\n')
                            .count();
                    }
                });
            });
    });
    ui.advance_cursor_after_rect(well);
}

fn preset_row(ui: &egui::Ui, rect: Rect, preset: &Preset, open: bool) -> bool {
    let response = ui.interact(
        rect,
        Id::new(("soundscape-preset", preset.name)),
        Sense::click(),
    );
    let hot = response.hovered() || response.has_focus();
    let colour = if hot || open {
        Palette::ACCENT
    } else {
        Palette::LINE
    };
    let painter = ui.painter();
    painter.rect_filled(rect, 0, if hot { Palette::CARD } else { Palette::SURFACE });
    painter.rect_stroke(rect, 0, Stroke::new(1.0, colour), egui::StrokeKind::Inside);
    // A 2 px rail marks the open one, the same way a cited fact is railed.
    if open {
        painter.rect_filled(
            Rect::from_min_size(rect.left_top(), Vec2::new(2.0, rect.height())),
            0,
            Palette::ACCENT,
        );
    }
    painter.text(
        rect.left_top() + vec2(14.0, 8.0),
        Align2::LEFT_TOP,
        tracked(preset.name),
        text::label(),
        if open { Palette::ACCENT } else { Palette::TEXT },
    );
    painter.text(
        rect.left_bottom() + vec2(14.0, -8.0),
        Align2::LEFT_BOTTOM,
        preset.mood,
        text::small(),
        Palette::TEXT_FAINT,
    );
    response.clicked()
}

/// A rectangular fader: a track, a filled portion, and a square handle.
///
/// Hand-drawn rather than `egui::Slider`, whose grabber is a circle and whose
/// spinner is a text field — neither belongs in an interface with no corner
/// radius anywhere in it.
///
/// It has no disabled state, because the gain it drives sits between the
/// engine and the device rather than inside the score. Whatever is loaded, and
/// whether or not anything is playing at all, this always means something.
///
/// The travel is linear in decibels, which is what gives it a usable taper —
/// a linear-amplitude fader does its whole audible job in the bottom fifth.
fn fader(ui: &egui::Ui, rect: Rect, decibels: &mut f64) -> bool {
    let response = ui.interact(rect, Id::new("soundscape-volume"), Sense::click_and_drag());
    let hot = response.hovered() || response.has_focus();

    let span = MAX_DECIBELS - MIN_DECIBELS;
    let mut changed = false;
    if (response.dragged() || response.clicked())
        && let Some(pointer) = response.interact_pointer_pos()
    {
        let fraction = f64::from(((pointer.x - rect.left()) / rect.width()).clamp(0.0, 1.0));
        let picked = MIN_DECIBELS + fraction * span;
        if (picked - *decibels).abs() > f64::EPSILON {
            *decibels = picked;
            changed = true;
        }
    }
    let value = (*decibels - MIN_DECIBELS) / span;

    let colour = if hot {
        Palette::ACCENT
    } else {
        Palette::TEXT_DIM
    };
    let track = Rect::from_center_size(rect.center(), Vec2::new(rect.width(), 6.0));
    let painter = ui.painter();
    painter.rect_filled(track, 0, Palette::BG);
    painter.rect_stroke(
        track,
        0,
        Stroke::new(1.0, Palette::LINE),
        egui::StrokeKind::Inside,
    );
    painter.rect_filled(
        Rect::from_min_size(
            track.left_top(),
            Vec2::new(track.width() * value as f32, track.height()),
        ),
        0,
        colour,
    );
    let handle = Rect::from_center_size(
        Pos2::new(rect.left() + rect.width() * value as f32, rect.center().y),
        Vec2::new(6.0, 20.0),
    );
    painter.rect_filled(handle, 0, if hot { Palette::ACCENT } else { Palette::TEXT });
    painter.rect_stroke(
        handle,
        0,
        Stroke::new(1.0, Palette::BG),
        egui::StrokeKind::Inside,
    );

    response.widget_info(|| egui::WidgetInfo::slider(true, *decibels, "Volume"));
    changed
}

fn verb_button(ui: &egui::Ui, rect: Rect, label: &'static str, live: bool) -> bool {
    let response = ui.interact(rect, Id::new(("soundscape-verb", label)), Sense::click());
    let hot = response.hovered() || response.has_focus();
    let colour = if hot || live {
        Palette::ACCENT
    } else {
        Palette::TEXT_DIM
    };
    let painter = ui.painter();
    painter.rect_filled(rect, 0, if hot { Palette::CARD } else { Palette::SURFACE });
    painter.rect_stroke(rect, 0, Stroke::new(1.0, colour), egui::StrokeKind::Inside);
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        tracked(label),
        text::label(),
        colour,
    );
    response.clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colored(source: &str, needle: &str) -> Vec<Color32> {
        let start = source.find(needle).expect("needle is in the source");
        let range = start..start + needle.len();
        spans(source)
            .into_iter()
            .filter(|(span, _)| span.start < range.end && range.start < span.end)
            .map(|(_, color)| color)
            .collect()
    }

    #[test]
    fn spans_cover_the_source_exactly_once() {
        let source = "local v = voice { graph = function(n) return sine(n.hz) end }\n-- x\n";
        let mut at = 0;
        for (span, _) in spans(source) {
            assert_eq!(span.start, at, "spans must be contiguous");
            assert!(span.end > span.start, "spans must advance");
            at = span.end;
        }
        assert_eq!(at, source.len());
    }

    #[test]
    fn mini_notation_strings_are_one_span() {
        assert_eq!(
            colored("play(v, \"c4 e4 g4\")", "\"c4 e4 g4\""),
            [CODE_STRING]
        );
    }

    #[test]
    fn keywords_builtins_and_names_are_distinguished() {
        let source = "local tone = sine(220)";
        assert_eq!(colored(source, "local"), [CODE_KEYWORD]);
        assert_eq!(colored(source, "sine"), [CODE_BUILTIN]);
        assert_eq!(colored(source, "tone"), [Palette::TEXT]);
        assert_eq!(colored(source, "220"), [CODE_NUMBER]);
    }

    #[test]
    fn comments_run_to_the_end_of_their_line_and_no_further() {
        let source = "-- sine(1)\nsine(1)";
        assert_eq!(colored(source, "-- sine(1)"), [CODE_COMMENT]);
        assert_eq!(colored(&source[11..], "sine"), [CODE_BUILTIN]);
    }

    #[test]
    fn non_ascii_text_is_never_split_mid_character() {
        // Span boundaries must stay on character boundaries, or slicing the
        // source for a layout section would panic.
        let source = "local x = 1 -- ♪\n§ y";
        for (span, _) in spans(source) {
            assert!(source.is_char_boundary(span.start));
            assert!(source.is_char_boundary(span.end));
        }
    }

    fn library_of(entries: &[(&str, &str)]) -> Library {
        Library::new(
            entries
                .iter()
                .map(|(name, source)| (format!("{name}.eod"), (*source).to_owned()))
                .collect(),
        )
    }

    /// What the editor opens on decides which button it shows, so getting the
    /// origin wrong is getting "save" versus "save as" wrong.
    #[test]
    fn a_reopened_document_knows_where_it_came_from() {
        let library = library_of(&[("night-drive", "tempo(90)")]);

        let editor = Editor::new(
            "tempo(90)",
            Some("night-drive"),
            library_of(&[("night-drive", "tempo(90)")]),
        );
        assert_eq!(editor.origin, Origin::File("night-drive".into()));
        assert_eq!(editor.name, "night-drive");
        assert!(
            editor.overwrites(),
            "its own file is saved over, not copied"
        );

        // Edited since it was saved: it is not that file any more, so saving
        // must not quietly replace it.
        let editor = Editor::new("tempo(91)", Some("night-drive"), library);
        assert_eq!(editor.origin, Origin::Unsaved);
        assert!(!editor.overwrites());

        let editor = Editor::new(default_source(), None, Library::default());
        assert_eq!(editor.origin, Origin::Template(0));
        assert_eq!(editor.name, "waves");
        assert!(!editor.overwrites(), "a template is never written back to");
        assert_eq!(editor.file(), None, "and is not a library entry");
    }

    /// Saving a template makes a copy; saving that copy again replaces it.
    #[test]
    fn saving_a_template_makes_a_file_the_editor_then_owns() {
        let mut editor = Editor::new(default_source(), None, Library::default());
        let name = editor
            .library
            .free_name_unless(&editor.name, editor.overwrites());
        assert_eq!(name, "waves");
        editor.saved_as(name, default_source());

        assert_eq!(editor.origin, Origin::File("waves".into()));
        assert!(editor.overwrites());
        assert_eq!(editor.file(), Some("waves"));
        assert_eq!(
            editor
                .library
                .free_name_unless(&editor.name, editor.overwrites()),
            "waves",
            "saving again replaces the file rather than making waves-2"
        );

        // Retyping the name is Save As: the same document, a second file.
        editor.name = "waves night".into();
        assert!(!editor.overwrites());
        assert_eq!(
            editor
                .library
                .free_name_unless(&editor.name, editor.overwrites()),
            "waves-night"
        );
    }

    /// A deleted score stays open — it is still what is playing — it simply
    /// has nowhere to be saved back to.
    #[test]
    fn deleting_the_open_score_leaves_it_open_and_unsaved() {
        let mut editor = Editor::new(
            "tempo(90)",
            Some("acid"),
            library_of(&[("acid", "tempo(90)")]),
        );
        assert_eq!(editor.origin, Origin::File("acid".into()));

        editor.deleted("acid");
        assert_eq!(editor.origin, Origin::Unsaved);
        assert!(!editor.library.contains("acid"));
        assert_eq!(editor.file(), None);
    }

    #[test]
    fn the_bottom_of_the_fader_reads_as_silence_rather_than_a_number() {
        assert_eq!(format_decibels(MIN_DECIBELS), "−∞ dB");
        assert_eq!(format_decibels(MIN_DECIBELS - 10.0), "−∞ dB");
        assert_eq!(format_decibels(-12.0), "−12 dB");
        assert_eq!(format_decibels(MAX_DECIBELS), "0 dB");
    }

    #[test]
    fn the_default_soundscape_is_a_shipped_preset() {
        assert_eq!(opened_preset(default_source()), Some(0));
        assert_eq!(opened_preset("tempo(90)"), None);
    }

    #[test]
    fn presets_are_distinct_and_described() {
        for preset in PRESETS {
            assert!(!preset.name.is_empty());
            assert!(!preset.mood.is_empty());
            assert!(
                preset.source.trim_start().starts_with("--"),
                "{} should open with a comment explaining itself",
                preset.name
            );
        }
        for (index, preset) in PRESETS.iter().enumerate() {
            assert!(
                !PRESETS[..index]
                    .iter()
                    .any(|earlier| earlier.name == preset.name),
                "duplicate preset name {}",
                preset.name
            );
        }
    }
}
