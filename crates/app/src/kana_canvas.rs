//! Native handwriting-recognizer playground.

use idiosepius_core::kana::VariantThresholds;

use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use eframe::egui::{
    self, Align, Color32, Event, FontFamily, FontId, Layout, PointerButton, Pos2, Rect, RichText,
    Sense, Shape, Stroke, TouchDeviceId, TouchId, TouchPhase, Vec2,
};
use idiosepius_core::kana::{
    ALGORITHM_ID, CharacterInk, InkPoint, KanaRecognizer, KanaScript, PointerKind, Recognition,
    RecognitionConfig, RecognitionState, RejectionReason, VARIANT_MAXIMUM_DISTANCE,
    VARIANT_MINIMUM_MARGIN,
};
use serde::{Deserialize, Serialize};

use crate::theme::{self, Palette, text};

const CANVAS_COORDS: f32 = 1024.0;
const INK_WIDTH: f32 = 5.0;
const RESULT_LIMIT: usize = 10;
const SAMPLE_FORMAT: &str = "idiosepius-kana-sample";
const SAMPLE_FORMAT_VERSION: u32 = 1;

type SampleSaver =
    Box<dyn Fn(&CharacterInk, &Recognition, SampleMetadata) -> Result<PathBuf> + 'static>;

#[derive(Clone)]
struct SampleMetadata {
    config: RecognitionConfig,
    template_count: usize,
    template_fingerprint: u64,
    script_scope: Option<KanaScript>,
    expected: Option<char>,
    expected_invalid: bool,
    writing_validity: WritingValidity,
    expected_source: Option<ExpectedSource>,
    prompt: Option<PromptMetadata>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum WritingValidity {
    #[default]
    Unknown,
    Valid,
    Invalid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ExpectedSource {
    Manual,
    Prompted,
}

#[derive(Clone, Serialize)]
struct PromptMetadata {
    run_started_at_unix_ms: u64,
    position: usize,
    total: usize,
    saved_before: usize,
    sequence: String,
}

pub fn run(
    icon: Option<egui::IconData>,
    shot: Option<PathBuf>,
    sample_dir: PathBuf,
    initial_sample: Option<PathBuf>,
) -> Result<()> {
    let initial_sample = initial_sample
        .map(|path| {
            let json = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            parse_initial_sample(&json).with_context(|| format!("loading {}", path.display()))
        })
        .transpose()?;
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([940.0, 700.0])
        .with_min_inner_size([720.0, 560.0])
        .with_title("Idiosepius — kana canvas");
    if let Some(icon) = icon {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    let shot_completed = shot.as_ref().map(|_| Arc::new(AtomicBool::new(false)));
    let app_shot_completed = shot_completed.clone();

    eframe::run_native(
        "idiosepius-kana-canvas",
        options,
        Box::new(move |creation| {
            theme::install(&creation.egui_ctx);
            let recognizer = KanaRecognizer::new()?;
            let save_sample: SampleSaver = Box::new(move |ink, result, metadata| {
                write_sample(&sample_dir, ink, result, metadata)
            });
            let mut app =
                CanvasApp::new_with_shot_status(recognizer, shot, app_shot_completed, save_sample);
            if let Some(sample) = initial_sample {
                app.load_sample(sample)?;
            }
            Ok(Box::new(app))
        }),
    )
    .map_err(|error| anyhow::anyhow!("{error}"))?;
    if shot_completed.is_some_and(|completed| !completed.load(Ordering::Acquire)) {
        bail!("canvas screenshot was not written");
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ActivePointer {
    Mouse,
    Touch(TouchDeviceId, TouchId),
}

struct CanvasApp {
    recognizer: KanaRecognizer,
    ink: CharacterInk,
    ink_view: InkView,
    expected: String,
    loaded_validity: Option<(CharacterInk, String, WritingValidity)>,
    prompt: Option<PromptRun>,
    active: Option<ActivePointer>,
    script_scope: Option<KanaScript>,
    recognition: Option<Recognition>,
    vision_model: Option<idiosepius_core::kana_vision::VisionModel>,
    vision: Option<idiosepius_core::kana_vision::VisionResult>,
    notice: Option<Notice>,
    save_sample: SampleSaver,
    shot: Option<CanvasShot>,
}

struct Notice {
    text: String,
    error: bool,
}

struct PromptRun {
    scope: Option<KanaScript>,
    run_started_at_unix_ms: u64,
    sequence: Vec<char>,
    position: usize,
    total: usize,
    saved: usize,
}

struct InitialSample {
    ink: CharacterInk,
    expected: Option<char>,
    expected_invalid: bool,
    writing_validity: WritingValidity,
    origin: InitialOrigin,
    script_scope: Option<KanaScript>,
    stored_result: Option<Recognition>,
    stored_algorithm: Option<String>,
    stored_template_fingerprint: Option<String>,
    stored_maximum_distance: Option<f32>,
    stored_maximum_path_length_ratio: Option<f32>,
    stored_minimum_margin: Option<f32>,
    stored_variant_thresholds: VariantThresholds,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InitialOrigin {
    SavedSample,
    SyntheticFailure,
    SyntheticInvalidProbe,
    RawInk,
}

struct CanvasShot {
    path: PathBuf,
    frames: u32,
    started: Instant,
    completed: Option<Arc<AtomicBool>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct InkView {
    min_x: f32,
    min_y: f32,
    side: f32,
}

impl InkView {
    const CANVAS: Self = Self {
        min_x: 0.0,
        min_y: 0.0,
        side: CANVAS_COORDS,
    };

    fn fitted(ink: &CharacterInk) -> Self {
        let mut min_x = f32::INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        for point in ink.0.iter().flatten() {
            if point.x.is_finite() && point.y.is_finite() {
                min_x = min_x.min(point.x);
                min_y = min_y.min(point.y);
                max_x = max_x.max(point.x);
                max_y = max_y.max(point.y);
            }
        }
        let extent = (max_x - min_x).max(max_y - min_y);
        if !extent.is_finite() {
            return Self::CANVAS;
        }
        if extent <= f32::EPSILON {
            return Self {
                min_x: min_x - CANVAS_COORDS * 0.5,
                min_y: min_y - CANVAS_COORDS * 0.5,
                side: CANVAS_COORDS,
            };
        }
        let padded_side = extent * 1.2;
        let side = if padded_side.is_finite() {
            padded_side
        } else {
            extent
        };
        let center_x = min_x + (max_x - min_x) * 0.5;
        let center_y = min_y + (max_y - min_y) * 0.5;
        Self {
            min_x: center_x - side * 0.5,
            min_y: center_y - side * 0.5,
            side,
        }
    }
}

impl CanvasApp {
    #[cfg(test)]
    fn new(recognizer: KanaRecognizer, shot: Option<PathBuf>, save_sample: SampleSaver) -> Self {
        Self::new_with_shot_status(recognizer, shot, None, save_sample)
    }

    fn new_with_shot_status(
        recognizer: KanaRecognizer,
        shot: Option<PathBuf>,
        shot_completed: Option<Arc<AtomicBool>>,
        save_sample: SampleSaver,
    ) -> Self {
        let ink = shot
            .as_ref()
            .and_then(|_| recognizer.canonical('あ'))
            .unwrap_or_default();
        let ink_view = if ink.is_empty() {
            InkView::CANVAS
        } else {
            InkView::fitted(&ink)
        };
        let script_scope = shot.as_ref().map(|_| KanaScript::Hiragana);
        let expected = shot.as_ref().map(|_| "あ").unwrap_or_default().to_owned();
        let shot_sequence: Vec<_> = recognizer
            .characters()
            .filter_map(|(character, script)| (script == KanaScript::Hiragana).then_some(character))
            .collect();
        let prompt = shot.as_ref().map(|_| PromptRun {
            scope: Some(KanaScript::Hiragana),
            run_started_at_unix_ms: 0,
            total: shot_sequence.len(),
            sequence: shot_sequence,
            position: 1,
            saved: 0,
        });
        let recognition = (!ink.is_empty())
            .then(|| recognize_with_scope(&recognizer, &ink, script_scope, RESULT_LIMIT));
        let vision_model = idiosepius_core::kana_vision::VisionModel::embedded().ok();
        let vision = script_scope.and_then(|scope| {
            vision_model
                .as_ref()
                .map(|model| model.recognize(&ink, scope, 5))
        });
        Self {
            vision_model,
            loaded_validity: None,
            vision,
            recognizer,
            ink,
            ink_view,
            expected,
            prompt,
            active: None,
            script_scope,
            recognition,
            notice: None,
            save_sample,
            shot: shot.map(|path| CanvasShot {
                path,
                frames: 0,
                started: Instant::now(),
                completed: shot_completed,
            }),
        }
    }

    fn clear(&mut self) {
        self.loaded_validity = None;
        self.ink.0.clear();
        self.ink_view = InkView::CANVAS;
        self.active = None;
        self.recognition = None;
        self.vision = None;
        self.notice = None;
    }

    fn load_sample(&mut self, sample: InitialSample) -> Result<()> {
        if let Some(expected) = sample.expected {
            let Some(expected_script) = self
                .recognizer
                .characters()
                .find_map(|(character, script)| (character == expected).then_some(script))
            else {
                bail!("saved sample has an expected character outside the basic kana set");
            };
            if sample
                .script_scope
                .is_some_and(|scope| scope != expected_script)
            {
                bail!("saved sample expected kana is outside its recognizer script scope");
            }
        }
        self.ink_view = InkView::fitted(&sample.ink);
        self.ink = sample.ink;
        self.expected = sample
            .expected
            .map(|character| character.to_string())
            .unwrap_or_else(|| {
                if sample.expected_invalid {
                    "-".to_owned()
                } else {
                    String::new()
                }
            });
        self.script_scope = sample.script_scope;
        self.loaded_validity = Some((
            self.ink.clone(),
            self.expected.clone(),
            sample.writing_validity,
        ));
        self.prompt = None;
        self.active = None;
        self.refresh();
        let current_fingerprint = format!("{:016x}", self.recognizer.template_fingerprint());
        let current_config = self.recognizer.config();
        let metadata_changed = sample
            .stored_algorithm
            .as_deref()
            .is_some_and(|stored| stored != ALGORITHM_ID)
            || sample
                .stored_template_fingerprint
                .as_deref()
                .is_some_and(|stored| stored != current_fingerprint.as_str())
            || sample
                .stored_maximum_distance
                .is_some_and(|stored| stored != current_config.maximum_distance)
            || sample
                .stored_maximum_path_length_ratio
                .is_some_and(|stored| stored != current_config.maximum_path_length_ratio)
            || sample
                .stored_minimum_margin
                .is_some_and(|stored| stored != current_config.minimum_margin)
            || sample.stored_variant_thresholds.changed();
        let result_changed = sample
            .stored_result
            .as_ref()
            .zip(self.recognition.as_ref())
            .is_some_and(|(stored, current)| !stored.reproduced_by(current));
        let metadata_missing = sample.origin == InitialOrigin::SavedSample
            && (sample.stored_algorithm.is_none()
                || sample.stored_template_fingerprint.is_none()
                || sample.stored_maximum_distance.is_none()
                || sample.stored_maximum_path_length_ratio.is_none()
                || sample.stored_minimum_margin.is_none()
                || !sample.stored_variant_thresholds.is_complete());
        let message = if sample.origin == InitialOrigin::SyntheticFailure && result_changed {
            "Loaded synthetic failure; current result differs."
        } else if sample.origin == InitialOrigin::SyntheticFailure {
            "Loaded synthetic failure; result reproduced."
        } else if sample.origin == InitialOrigin::SyntheticInvalidProbe && result_changed {
            "Loaded synthetic invalid probe; current result differs."
        } else if sample.origin == InitialOrigin::SyntheticInvalidProbe
            && sample.stored_result.is_some()
        {
            "Loaded synthetic invalid probe; result reproduced."
        } else if sample.origin == InitialOrigin::SyntheticInvalidProbe {
            "Loaded synthetic invalid probe; result recomputed."
        } else if sample.origin == InitialOrigin::RawInk {
            "Loaded raw ink; result recomputed."
        } else if metadata_changed {
            "Loaded sample; recognizer metadata changed, result recomputed."
        } else if metadata_missing {
            "Loaded sample; recognizer metadata incomplete, result recomputed."
        } else if result_changed {
            "Loaded sample; result changed since save."
        } else if sample.stored_result.is_some() {
            "Loaded sample; saved result reproduced."
        } else {
            "Loaded sample; result recomputed."
        };
        self.notice = Some(Notice::success(message));
        Ok(())
    }

    fn undo(&mut self) {
        self.active = None;
        self.ink.0.pop();
        if self.ink.is_empty() {
            self.ink_view = InkView::CANVAS;
        }
        self.refresh();
        self.notice = None;
    }

    fn refresh(&mut self) {
        // Scope shortcuts and edited labels may request a refresh mid-stroke.
        // A partial drawing must not acquire a saveable prediction through them.
        if self.active.is_some() {
            self.recognition = None;
            self.vision = None;
            return;
        }
        self.vision = self.script_scope.and_then(|scope| {
            self.vision_model
                .as_ref()
                .map(|model| model.recognize(&self.ink, scope, 5))
        });
        self.recognition = (!self.ink.is_empty()).then(|| {
            recognize_with_scope(&self.recognizer, &self.ink, self.script_scope, RESULT_LIMIT)
        });
    }

    fn set_script_scope(&mut self, script_scope: Option<KanaScript>) {
        if self.script_scope != script_scope {
            self.script_scope = script_scope;
            self.prompt = None;
            self.refresh();
            self.notice = None;
        }
    }

    fn next_prompt(&mut self) -> Option<char> {
        let Some(scope) = self.script_scope else {
            self.notice = Some(Notice::error(
                "Choose hiragana or katakana before starting prompts.",
            ));
            return None;
        };
        let mut characters: Vec<_> = self
            .recognizer
            .characters()
            .filter_map(|(character, script)| (scope == script).then_some(character))
            .collect();
        if characters.is_empty() {
            return None;
        }
        let needs_run = self
            .prompt
            .as_ref()
            .is_none_or(|run| run.scope != self.script_scope || run.position >= run.total);
        if needs_run {
            let now = SystemTime::now().duration_since(UNIX_EPOCH).ok();
            let shuffle_seed = now.as_ref().map_or(0, prompt_shuffle_seed);
            shuffle_prompt(&mut characters, shuffle_seed);
            let run_started_at_unix_ms = now
                .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
                .unwrap_or(0);
            self.prompt = Some(PromptRun {
                scope: self.script_scope,
                run_started_at_unix_ms,
                sequence: characters,
                position: 0,
                total: self
                    .recognizer
                    .characters()
                    .filter(|(_, script)| *script == scope)
                    .count(),
                saved: 0,
            });
        }
        let Some(run) = self.prompt.as_mut() else {
            self.notice = Some(Notice::error("Could not initialize the prompt sequence."));
            return None;
        };
        let Some(character) = run.sequence.get(run.position).copied() else {
            self.notice = Some(Notice::error("Prompt sequence metadata is inconsistent."));
            return None;
        };
        run.position += 1;
        self.expected = character.to_string();
        self.clear();
        Some(character)
    }

    fn copy_ink(&mut self, ctx: &egui::Context) {
        match serde_json::to_string_pretty(&self.ink) {
            Ok(json) => {
                ctx.copy_text(json);
                self.notice = Some(Notice::success("Ink JSON copied."));
            }
            Err(error) => {
                self.notice = Some(Notice::error(format!("Copy failed: {error}")));
            }
        }
    }

    fn writing_validity(&self) -> WritingValidity {
        if self.expected_is_invalid() {
            return WritingValidity::Invalid;
        }
        self.loaded_validity
            .as_ref()
            .filter(|(ink, expected, _)| ink == &self.ink && expected == &self.expected)
            .map_or(WritingValidity::Unknown, |(_, _, validity)| *validity)
    }

    fn save_sample(&mut self) {
        if self.active.is_some() {
            self.notice = Some(Notice::error("Finish the active stroke before saving."));
            return;
        }
        let Some(recognition) = self.recognition.as_ref() else {
            return;
        };
        let Ok(expected) = self.expected_character() else {
            self.notice = Some(Notice::error(
                "Expected must be one basic kana, '-' for invalid ink, or blank.",
            ));
            return;
        };
        let expected_invalid = self.expected_is_invalid();
        if self.prompt.is_some() && expected != self.active_prompt_character() {
            self.notice = Some(Notice::error(
                "Expected must stay on the prompted kana while collecting a run.",
            ));
            return;
        }
        if expected.is_some()
            && self.script_scope.is_some()
            && self.expected_script() != self.script_scope
        {
            self.notice = Some(Notice::error(
                "Expected kana is outside the selected script scope.",
            ));
            return;
        }
        let prompt_metadata = self.prompt.as_ref().map(|prompt| PromptMetadata {
            run_started_at_unix_ms: prompt.run_started_at_unix_ms,
            position: prompt.position,
            total: prompt.total,
            saved_before: prompt.saved,
            sequence: prompt.sequence.iter().collect(),
        });
        let metadata = SampleMetadata {
            config: self.recognizer.config(),
            template_count: self.recognizer.template_count(),
            template_fingerprint: self.recognizer.template_fingerprint(),
            script_scope: self.script_scope,
            expected,
            expected_invalid,
            writing_validity: self.writing_validity(),
            expected_source: (expected.is_some() || expected_invalid).then(|| {
                if self.prompt.is_some() {
                    ExpectedSource::Prompted
                } else {
                    ExpectedSource::Manual
                }
            }),
            prompt: prompt_metadata,
        };
        match (self.save_sample)(&self.ink, recognition, metadata) {
            Ok(path) => {
                if self.prompt.is_some() {
                    if let Some(prompt) = self.prompt.as_mut() {
                        prompt.saved += 1;
                    }
                    let completed = self
                        .prompt
                        .as_ref()
                        .is_some_and(|run| run.position >= run.total);
                    if completed {
                        let Some(prompt) = self.prompt.take() else {
                            self.notice = Some(Notice::error(format!(
                                "Saved {}, but prompt completion state was lost.",
                                path.display()
                            )));
                            return;
                        };
                        self.expected.clear();
                        self.clear();
                        let completion = format!(
                            "prompt sequence complete · {} saved · {} skipped",
                            prompt.saved,
                            prompt.total - prompt.saved
                        );
                        self.notice = Some(Notice::success(format!(
                            "Saved {} · {completion}.",
                            path.display()
                        )));
                    } else {
                        if let Some(next) = self.next_prompt() {
                            self.notice = Some(Notice::success(format!(
                                "Saved {} · next {next}",
                                path.display()
                            )));
                        } else {
                            self.notice = Some(Notice::error(format!(
                                "Saved {}, but the next prompt is unavailable.",
                                path.display()
                            )));
                        }
                    }
                } else {
                    self.notice = Some(Notice::success(format!("Saved {}", path.display())));
                }
            }
            Err(error) => {
                self.notice = Some(Notice::error(format!("Save failed: {error:#}")));
            }
        }
    }

    fn expected_character(&self) -> std::result::Result<Option<char>, ()> {
        let trimmed = self.expected.trim();
        if trimmed.is_empty() || trimmed == "-" {
            return Ok(None);
        }
        let mut characters = trimmed.chars();
        let Some(character) = characters.next() else {
            return Ok(None);
        };
        if characters.next().is_some()
            || !self
                .recognizer
                .characters()
                .any(|(candidate, _)| candidate == character)
        {
            return Err(());
        }
        Ok(Some(character))
    }

    fn expected_is_invalid(&self) -> bool {
        self.expected.trim() == "-"
    }

    fn active_prompt_character(&self) -> Option<char> {
        let prompt = self.prompt.as_ref()?;
        let position = prompt.position.checked_sub(1)?;
        if prompt.sequence.len() != prompt.total {
            return None;
        }
        prompt.sequence.get(position).copied()
    }

    fn expected_script(&self) -> Option<KanaScript> {
        self.expected_character()
            .ok()
            .flatten()
            .and_then(|expected| {
                self.recognizer
                    .characters()
                    .find_map(|(character, script)| (character == expected).then_some(script))
            })
    }

    fn start_stroke(
        &mut self,
        owner: ActivePointer,
        rect: Rect,
        pos: Pos2,
        time_ms: f64,
        pressure: Option<f32>,
        pointer: PointerKind,
    ) -> bool {
        let Some(point) = ink_point(rect, self.ink_view, pos, time_ms, pressure, pointer) else {
            return false;
        };
        if self.active.is_some() {
            return false;
        }
        self.active = Some(owner);
        self.ink.0.push(vec![point]);
        true
    }

    fn append(
        &mut self,
        rect: Rect,
        pos: Pos2,
        time_ms: f64,
        pressure: Option<f32>,
        pointer: PointerKind,
    ) -> bool {
        // Pointer capture may carry a stroke outside the square. Preserve those
        // coordinates instead of connecting two separated in-bounds fragments.
        let point = raw_ink_point(rect, self.ink_view, pos, time_ms, pressure, pointer);
        let Some(stroke) = self.ink.0.last_mut() else {
            return false;
        };
        // Time and pressure can change without motion. Keep every raw event;
        // recognition already ignores zero-length geometric segments.
        stroke.push(point);
        true
    }

    fn capture_events(&mut self, ui: &egui::Ui, rect: Rect) {
        let (events, time) = ui.input(|input| (input.raw.events.clone(), input.time * 1_000.0));
        self.capture_batch(events, time, rect);
    }

    fn capture_batch(&mut self, events: Vec<Event>, time: f64, rect: Rect) {
        let has_touch = events
            .iter()
            .any(|event| matches!(event, Event::Touch { .. }));
        let mut changed = false;
        let mut completed = false;

        for event in events {
            match event {
                Event::Touch {
                    device_id,
                    id,
                    phase,
                    pos,
                    force,
                } => {
                    let owner = ActivePointer::Touch(device_id, id);
                    match phase {
                        TouchPhase::Start => {
                            changed |= self.start_stroke(
                                owner,
                                rect,
                                pos,
                                time,
                                force,
                                PointerKind::Touch,
                            );
                        }
                        TouchPhase::Move if self.active == Some(owner) => {
                            changed |= self.append(rect, pos, time, force, PointerKind::Touch);
                        }
                        TouchPhase::End | TouchPhase::Cancel if self.active == Some(owner) => {
                            changed |= self.append(rect, pos, time, force, PointerKind::Touch);
                            self.active = None;
                            completed = true;
                        }
                        _ => {}
                    }
                }
                Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed,
                    ..
                } if accepts_mouse_channel(has_touch, self.active) => {
                    if pressed {
                        changed |= self.start_stroke(
                            ActivePointer::Mouse,
                            rect,
                            pos,
                            time,
                            None,
                            PointerKind::Mouse,
                        );
                    } else if self.active == Some(ActivePointer::Mouse) {
                        changed |= self.append(rect, pos, time, None, PointerKind::Mouse);
                        self.active = None;
                        completed = true;
                    }
                }
                Event::PointerMoved(pos) if self.active == Some(ActivePointer::Mouse) => {
                    changed |= self.append(rect, pos, time, None, PointerKind::Mouse);
                }
                Event::PointerGone if self.active == Some(ActivePointer::Mouse) => {
                    self.active = None;
                    completed = true;
                }
                _ => {}
            }
        }

        if changed {
            self.notice = None;
            self.recognition = None;
            self.vision = None;
        }
        if completed && self.active.is_none() {
            self.refresh();
        }
    }

    fn canvas(&mut self, ui: &mut egui::Ui, side: f32) {
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(side), Sense::drag());
        let response = response.on_hover_cursor(egui::CursorIcon::Crosshair);
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0, Palette::CARD);
        painter.rect_stroke(
            rect,
            0,
            Stroke::new(
                1.0,
                if response.hovered() || response.dragged() {
                    Palette::ACCENT
                } else {
                    Palette::LINE_BRIGHT
                },
            ),
            egui::StrokeKind::Inside,
        );

        let guide = Stroke::new(1.0, Palette::LINE);
        painter.line_segment(
            [
                Pos2::new(rect.center().x, rect.top()),
                Pos2::new(rect.center().x, rect.bottom()),
            ],
            guide,
        );
        painter.line_segment(
            [
                Pos2::new(rect.left(), rect.center().y),
                Pos2::new(rect.right(), rect.center().y),
            ],
            guide,
        );

        for stroke in &self.ink.0 {
            let points: Vec<_> = stroke
                .iter()
                .map(|point| screen_point(rect, self.ink_view, point))
                .collect();
            match points.as_slice() {
                [] => {}
                [point] => {
                    painter.rect_filled(
                        Rect::from_center_size(*point, Vec2::splat(INK_WIDTH)),
                        0,
                        Palette::ACCENT,
                    );
                }
                _ => {
                    painter.add(Shape::line(points, Stroke::new(INK_WIDTH, Palette::ACCENT)));
                }
            }
        }

        self.capture_events(ui, rect);
    }

    fn diagnostics(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(270.0);
        ui.set_height(ui.available_height());
        let sample_count: usize = self.ink.0.iter().map(Vec::len).sum();
        ui.label(
            RichText::new(format!(
                "{} · {}",
                theme::tracked("strokes"),
                self.ink.0.len()
            ))
            .font(text::label())
            .color(Palette::TEXT_DIM),
        );
        ui.label(
            RichText::new(format!("{} pointer samples", sample_count))
                .font(text::small())
                .color(Palette::TEXT_FAINT),
        );
        ui.add_space(4.0);
        ui.label(
            RichText::new(theme::tracked("expected · optional"))
                .font(text::label())
                .color(Palette::TEXT_FAINT),
        );
        let expected_response = ui
            .add(
                egui::TextEdit::singleline(&mut self.expected)
                    .font(FontId::new(24.0, FontFamily::Proportional))
                    .hint_text("あ")
                    .desired_width(ui.available_width()),
            )
            .on_hover_text("Enter one basic kana, or '-' to label deliberately invalid ink");
        if expected_response.changed() {
            let left_prompt = self.prompt.take().is_some();
            if let Some(script) = self.expected_script() {
                self.set_script_scope(Some(script));
            }
            self.notice = left_prompt
                .then(|| Notice::error("Prompt run ended · edited labels save as manual samples."));
        }
        let expected_valid = self.expected_character().is_ok();
        let expected = self.expected_character().ok().flatten();
        if !expected_valid {
            ui.label(
                RichText::new("Enter one basic kana, '-' for invalid, or leave blank.")
                    .font(text::small())
                    .color(Palette::WRONG),
            );
        }
        if let Some(prompt) = &self.prompt {
            ui.label(
                RichText::new(format!(
                    "prompt {} / {} · save advances",
                    prompt.position, prompt.total
                ))
                .font(text::small())
                .color(Palette::ACCENT),
            );
        }
        ui.add_space(4.0);
        ui.label(
            RichText::new(theme::tracked("script scope"))
                .font(text::label())
                .color(Palette::TEXT_FAINT),
        );
        let mut selected_scope = self.script_scope;
        ui.horizontal(|ui| {
            for (scope, shortcut, label) in [
                (None, "B", "both"),
                (Some(KanaScript::Hiragana), "H", "hira"),
                (Some(KanaScript::Katakana), "K", "kata"),
            ] {
                if ui
                    .add(
                        egui::Button::new(format!("{shortcut} {}", theme::tracked(label)))
                            .selected(selected_scope == scope),
                    )
                    .clicked()
                {
                    selected_scope = scope;
                }
            }
        });
        self.set_script_scope(selected_scope);
        ui.add_space(4.0);
        if ui
            .button(format!("N  {}", theme::tracked("next prompt")))
            .clicked()
        {
            self.next_prompt();
        }
        ui.horizontal(|ui| {
            let clear = ui
                .button(format!("C  {}", theme::tracked("clear")))
                .clicked();
            let undo = ui
                .add_enabled(
                    !self.ink.is_empty(),
                    egui::Button::new(format!("U  {}", theme::tracked("undo"))),
                )
                .clicked();
            if clear {
                self.clear();
            } else if undo {
                self.undo();
            }
        });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !self.ink.is_empty(),
                    egui::Button::new(theme::tracked("copy")),
                )
                .on_hover_text("Copy raw ink JSON")
                .clicked()
            {
                self.copy_ink(ui.ctx());
            }
            if ui
                .add_enabled(
                    self.recognition.is_some() && expected_valid && self.active.is_none(),
                    egui::Button::new(format!("S  {}", theme::tracked("save"))),
                )
                .on_hover_text("Save strokes, metadata and recognition result")
                .clicked()
            {
                self.save_sample();
            }
        });
        if let Some(notice) = &self.notice {
            ui.label(
                RichText::new(&notice.text)
                    .font(text::small())
                    .color(if notice.error {
                        Palette::WRONG
                    } else {
                        Palette::ACCENT
                    }),
            );
        }
        ui.add_space(8.0);

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let Some(recognition) = &self.recognition else {
                    ui.label(
                        RichText::new("Draw one kana in the square.")
                            .font(text::body())
                            .color(Palette::TEXT_DIM),
                    );
                    ui.label(
                        RichText::new(
                            "Recognition updates after each stroke from the raw event stream.",
                        )
                        .font(text::small())
                        .color(Palette::TEXT_FAINT),
                    );
                    return;
                };
                let recognized = matches!(recognition.state, RecognitionState::Recognized);
                let status = if recognized {
                    "R E C O G N I Z E D"
                } else {
                    "U N R E C O G N I Z E D"
                };
                ui.label(
                    RichText::new(status)
                        .font(text::label())
                        .color(if recognized {
                            Palette::ACCENT
                        } else {
                            Palette::TEXT_DIM
                        }),
                );
                ui.label(
                    RichText::new(recognition_diagnostic(
                        recognition,
                        self.recognizer.config(),
                    ))
                    .font(text::small())
                    .color(Palette::TEXT_FAINT),
                );
                ui.columns(2, |columns| {
                    columns[0].label(
                        RichText::new("G E O M E T R Y")
                            .font(text::label())
                            .color(Palette::TEXT_DIM),
                    );
                    if let Some(top) = recognition.top() {
                        columns[0].label(
                            RichText::new(top.character.to_string())
                                .font(FontId::new(40.0, FontFamily::Proportional))
                                .color(Palette::TEXT),
                        );
                    }
                    columns[1].label(
                        RichText::new("V I S I O N")
                            .font(text::label())
                            .color(Palette::TEXT_DIM),
                    );
                    if let Some(top) = self.vision.as_ref().and_then(|v| v.candidates.first()) {
                        columns[1].label(
                            RichText::new(top.character.to_string())
                                .font(FontId::new(40.0, FontFamily::Proportional))
                                .color(Palette::TEXT),
                        );
                    } else {
                        columns[1].label(
                            RichText::new("Choose H or K")
                                .font(text::small())
                                .color(Palette::TEXT_DIM),
                        );
                    }
                });
                if self.expected_is_invalid() {
                    let rejected = !matches!(recognition.state, RecognitionState::Recognized);
                    ui.label(
                        RichText::new(if rejected {
                            "expected invalid · rejected"
                        } else {
                            "expected invalid · false acceptance"
                        })
                        .font(text::small())
                        .color(if rejected {
                            Palette::CORRECT
                        } else {
                            Palette::WRONG
                        }),
                    );
                } else if let Some(expected) = expected {
                    let (diagnostic, verdict) = expected_diagnostic(recognition, expected);
                    ui.label(
                        RichText::new(diagnostic)
                            .font(text::small())
                            .color(match verdict {
                                ExpectedVerdict::AcceptedCorrect => Palette::CORRECT,
                                ExpectedVerdict::AcceptedWrong => Palette::WRONG,
                                ExpectedVerdict::Rejected => Palette::TEXT_DIM,
                            }),
                    );
                }
                ui.add_space(2.0);
                ui.spacing_mut().item_spacing.y = 3.0;
                if let Some(vision) = &self.vision {
                    ui.label(
                        RichText::new("V I S I O N · experimental")
                            .font(text::label())
                            .color(Palette::TEXT_DIM),
                    );
                    if let Some(error) = &vision.error {
                        ui.label(error);
                    }
                    for candidate in &vision.candidates {
                        ui.label(
                            RichText::new(format!(
                                "{}  {:.3}",
                                candidate.character, candidate.score
                            ))
                            .font(text::small()),
                        );
                    }
                    ui.label(
                        RichText::new("Uncalibrated identity scores; not a quality grade.")
                            .font(text::small())
                            .color(Palette::TEXT_DIM),
                    );
                    if let Some(comparison) = vision
                        .candidates
                        .first()
                        .and_then(|c| self.recognizer.compare_reference(&self.ink, c.character))
                    {
                        ui.label(
                            RichText::new(format!(
                                "{} reference: {:.4} · path {:.2}×",
                                comparison.character,
                                comparison.symmetric_distance,
                                comparison.path_length_ratio
                            ))
                            .font(text::small())
                            .color(Palette::TEXT_DIM),
                        );
                    }
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("G E O M E T R Y")
                            .font(text::label())
                            .color(Palette::TEXT_DIM),
                    );
                }
                for (index, candidate) in recognition.candidates.iter().enumerate() {
                    let fill = if index == 0 {
                        Palette::SURFACE
                    } else {
                        Color32::TRANSPARENT
                    };
                    egui::Frame::NONE
                        .fill(fill)
                        .stroke(Stroke::new(1.0, Palette::LINE))
                        .inner_margin(egui::Margin::symmetric(8, 2))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(format!("{:>2}", index + 1))
                                        .font(text::label())
                                        .color(Palette::TEXT_FAINT),
                                );
                                ui.label(
                                    RichText::new(candidate.character.to_string())
                                        .font(FontId::new(19.0, FontFamily::Proportional))
                                        .color(Palette::TEXT),
                                );
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    ui.label(
                                        RichText::new(format!("{:.4}", candidate.distance))
                                            .font(text::small())
                                            .color(Palette::TEXT_DIM),
                                    );
                                    ui.label(
                                        RichText::new(match candidate.script {
                                            idiosepius_core::kana::KanaScript::Hiragana => "H",
                                            idiosepius_core::kana::KanaScript::Katakana => "K",
                                        })
                                        .font(text::label())
                                        .color(Palette::TEXT_FAINT),
                                    );
                                });
                            });
                        });
                }
            });
    }

    fn drive_shot(&mut self, ctx: &egui::Context) {
        let Some(shot) = &mut self.shot else {
            return;
        };
        ctx.request_repaint();
        shot.frames += 1;
        if shot.frames == 8 || (shot.frames > 8 && shot.frames % 120 == 0) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        let image = ctx.input(|input| {
            input.events.iter().find_map(|event| match event {
                Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = image {
            let pixels: Vec<u8> = image
                .pixels
                .iter()
                .flat_map(|pixel| [pixel.r(), pixel.g(), pixel.b(), pixel.a()])
                .collect();
            let header = format!(
                "P7\nWIDTH {}\nHEIGHT {}\nDEPTH 4\nMAXVAL 255\nTUPLTYPE RGB_ALPHA\nENDHDR\n",
                image.width(),
                image.height()
            );
            let mut output = header.into_bytes();
            output.extend_from_slice(&pixels);
            if let Err(error) = std::fs::write(&shot.path, output) {
                eprintln!("could not write {}: {error}", shot.path.display());
            } else {
                eprintln!(
                    "wrote {} ({}x{})",
                    shot.path.display(),
                    image.width(),
                    image.height()
                );
                if let Some(completed) = &shot.completed {
                    completed.store(true, Ordering::Release);
                }
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if shot.started.elapsed() >= Duration::from_secs(10) {
            eprintln!(
                "screenshot backend did not return an image for {}",
                shot.path.display()
            );
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

impl eframe::App for CanvasApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.painter().rect_filled(ui.max_rect(), 0, Palette::BG);

        let ctx = ui.ctx().clone();
        let accept_shortcuts = !ctx.egui_wants_keyboard_input();
        let consume = |modifiers, key| {
            accept_shortcuts && ctx.input_mut(|input| input.consume_key(modifiers, key))
        };
        let clear_key = consume(egui::Modifiers::NONE, egui::Key::C);
        let undo_key = consume(egui::Modifiers::NONE, egui::Key::U);
        let copy_key = consume(egui::Modifiers::COMMAND, egui::Key::C);
        let save_key = consume(egui::Modifiers::NONE, egui::Key::S);
        let next_key = consume(egui::Modifiers::NONE, egui::Key::N);
        let both_key = consume(egui::Modifiers::NONE, egui::Key::B);
        let hiragana_key = consume(egui::Modifiers::NONE, egui::Key::H);
        let katakana_key = consume(egui::Modifiers::NONE, egui::Key::K);
        let invalid_key = consume(egui::Modifiers::NONE, egui::Key::I);
        if clear_key {
            self.clear();
        } else if undo_key {
            self.undo();
        }
        if copy_key && !self.ink.is_empty() {
            self.copy_ink(ui.ctx());
        }
        if save_key && self.recognition.is_some() {
            self.save_sample();
        }
        if next_key {
            self.next_prompt();
        }
        if both_key {
            self.set_script_scope(None);
        } else if hiragana_key {
            self.set_script_scope(Some(KanaScript::Hiragana));
        } else if katakana_key {
            self.set_script_scope(Some(KanaScript::Katakana));
        }
        if invalid_key {
            self.expected = "-".to_owned();
            self.notice = self
                .prompt
                .take()
                .map(|_| Notice::error("Prompt run ended · invalid ink saves as a manual sample."));
        }

        egui::Frame::NONE
            .inner_margin(egui::Margin::same(24))
            .show(ui, |ui| {
                ui.label(
                    RichText::new(theme::tracked("kana recognizer"))
                        .font(text::title())
                        .color(Palette::TEXT),
                );
                ui.label(
                    RichText::new(
                        "Draw with mouse, pen or touch. H/K + N prompts; I labels invalid ink.",
                    )
                    .font(text::body())
                    .color(Palette::TEXT_DIM),
                );
                ui.add_space(12.0);

                let side = (ui.available_width() - 320.0)
                    .min(ui.available_height())
                    .clamp(330.0, 570.0);
                ui.horizontal_top(|ui| {
                    self.canvas(ui, side);
                    ui.add_space(12.0);
                    ui.vertical(|ui| self.diagnostics(ui));
                });
            });
        self.drive_shot(ui.ctx());
    }
}

impl Notice {
    fn success(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            error: false,
        }
    }

    fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            error: true,
        }
    }
}

#[derive(Deserialize)]
struct InitialSampleDocument {
    format: String,
    format_version: u32,
    #[serde(default)]
    expected: Option<char>,
    #[serde(default)]
    expected_invalid: bool,
    #[serde(default)]
    writing_validity: Option<WritingValidity>,
    strokes: CharacterInk,
    #[serde(default)]
    recognizer: Option<InitialRecognizerMetadata>,
    #[serde(default)]
    result: Option<Recognition>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum InitialDocument {
    Saved(InitialSampleDocument),
    SyntheticFailure(InitialSyntheticFailure),
    SyntheticInvalidProbe(InitialSyntheticInvalidProbe),
    Ink(CharacterInk),
}

#[derive(Deserialize)]
struct InitialSyntheticFailure {
    expected: char,
    script: KanaScript,
    result: Recognition,
    ink: CharacterInk,
}

#[derive(Deserialize)]
struct InitialSyntheticInvalidProbe {
    script: KanaScript,
    #[serde(default)]
    result: Option<Recognition>,
    ink: CharacterInk,
}

#[derive(Deserialize)]
struct InitialRecognizerMetadata {
    #[serde(default)]
    script_scope: Option<String>,
    #[serde(default)]
    algorithm: Option<String>,
    #[serde(default)]
    template_fingerprint: Option<String>,
    #[serde(default)]
    maximum_distance: Option<f32>,
    #[serde(default)]
    maximum_path_length_ratio: Option<f32>,
    #[serde(default)]
    minimum_margin: Option<f32>,
    #[serde(flatten)]
    variant_thresholds: VariantThresholds,
}

fn parse_initial_sample(json: &str) -> Result<InitialSample> {
    let input: InitialDocument = serde_json::from_str(json).context("sample is not valid JSON")?;
    let document = match input {
        InitialDocument::Saved(document) => document,
        InitialDocument::SyntheticFailure(failure) => {
            return Ok(InitialSample {
                ink: failure.ink,
                expected: Some(failure.expected),
                expected_invalid: false,
                writing_validity: WritingValidity::Unknown,
                origin: InitialOrigin::SyntheticFailure,
                script_scope: Some(failure.script),
                stored_result: Some(failure.result),
                stored_algorithm: None,
                stored_template_fingerprint: None,
                stored_maximum_distance: None,
                stored_maximum_path_length_ratio: None,
                stored_minimum_margin: None,
                stored_variant_thresholds: VariantThresholds::default(),
            });
        }
        InitialDocument::SyntheticInvalidProbe(probe) => {
            return Ok(InitialSample {
                ink: probe.ink,
                expected: None,
                expected_invalid: true,
                writing_validity: WritingValidity::Invalid,
                origin: InitialOrigin::SyntheticInvalidProbe,
                script_scope: Some(probe.script),
                stored_result: probe.result,
                stored_algorithm: None,
                stored_template_fingerprint: None,
                stored_maximum_distance: None,
                stored_maximum_path_length_ratio: None,
                stored_minimum_margin: None,
                stored_variant_thresholds: VariantThresholds::default(),
            });
        }
        InitialDocument::Ink(ink) => {
            return Ok(InitialSample {
                ink,
                expected: None,
                expected_invalid: false,
                writing_validity: WritingValidity::Unknown,
                origin: InitialOrigin::RawInk,
                script_scope: None,
                stored_result: None,
                stored_algorithm: None,
                stored_template_fingerprint: None,
                stored_maximum_distance: None,
                stored_maximum_path_length_ratio: None,
                stored_minimum_margin: None,
                stored_variant_thresholds: VariantThresholds::default(),
            });
        }
    };
    if document.format != SAMPLE_FORMAT {
        bail!("unsupported kana sample format {:?}", document.format);
    }
    if document.format_version != SAMPLE_FORMAT_VERSION {
        bail!(
            "unsupported kana sample format version {}",
            document.format_version
        );
    }
    if document.expected.is_some() && document.expected_invalid {
        bail!("saved sample cannot be labeled as both kana and invalid ink");
    }
    if document.expected_invalid
        && document
            .writing_validity
            .is_some_and(|v| v != WritingValidity::Invalid)
    {
        bail!("non-kana annotation conflicts with writing validity");
    }
    let script_scope = match document
        .recognizer
        .as_ref()
        .and_then(|metadata| metadata.script_scope.as_deref())
    {
        None | Some("both") => None,
        Some("hiragana") => Some(KanaScript::Hiragana),
        Some("katakana") => Some(KanaScript::Katakana),
        Some(other) => bail!("unsupported script scope {other:?}"),
    };
    let (
        stored_algorithm,
        stored_template_fingerprint,
        stored_maximum_distance,
        stored_maximum_path_length_ratio,
        stored_minimum_margin,
        stored_variant_thresholds,
    ) = document
        .recognizer
        .map(|metadata| {
            (
                metadata.algorithm,
                metadata.template_fingerprint,
                metadata.maximum_distance,
                metadata.maximum_path_length_ratio,
                metadata.minimum_margin,
                metadata.variant_thresholds,
            )
        })
        .unwrap_or((None, None, None, None, None, VariantThresholds::default()));
    Ok(InitialSample {
        ink: document.strokes,
        expected: document.expected,
        expected_invalid: document.expected_invalid,
        writing_validity: document
            .writing_validity
            .unwrap_or(if document.expected_invalid {
                WritingValidity::Invalid
            } else {
                WritingValidity::Unknown
            }),
        origin: InitialOrigin::SavedSample,
        script_scope,
        stored_result: document.result,
        stored_algorithm,
        stored_template_fingerprint,
        stored_maximum_distance,
        stored_maximum_path_length_ratio,
        stored_minimum_margin,
        stored_variant_thresholds,
    })
}

#[derive(Serialize)]
struct SavedSample<'a> {
    format: &'static str,
    format_version: u32,
    saved_at_unix_ms: u64,
    app_version: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected: Option<char>,
    #[serde(skip_serializing_if = "is_false")]
    expected_invalid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected_source: Option<ExpectedSource>,
    writing_validity: WritingValidity,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt: Option<PromptMetadata>,
    capture: CaptureMetadata,
    recognizer: RecognizerMetadata,
    strokes: &'a CharacterInk,
    result: &'a Recognition,
    #[serde(skip_serializing_if = "Option::is_none")]
    vision: Option<idiosepius_core::kana_vision::VisionResult>,
}

#[derive(Serialize)]
struct CaptureMetadata {
    stroke_count: usize,
    point_count: usize,
    coordinate_extent: [f32; 2],
    coordinate_origin: &'static str,
    point_time_ms_clock: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_ms: Option<f64>,
}

#[derive(Serialize)]
struct RecognizerMetadata {
    algorithm: &'static str,
    template_count: usize,
    template_fingerprint: String,
    result_limit: usize,
    script_scope: &'static str,
    maximum_distance: f32,
    maximum_path_length_ratio: f32,
    minimum_margin: f32,
    variant_maximum_distance: f32,
    variant_minimum_margin: f32,
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn write_sample(
    directory: &Path,
    ink: &CharacterInk,
    recognition: &Recognition,
    metadata: SampleMetadata,
) -> Result<PathBuf> {
    let saved_at_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?
        .as_millis()
        .try_into()
        .context("current time does not fit in a 64-bit millisecond timestamp")?;
    write_sample_at(directory, ink, recognition, metadata, saved_at_unix_ms)
}

fn write_sample_at(
    directory: &Path,
    ink: &CharacterInk,
    recognition: &Recognition,
    metadata: SampleMetadata,
    saved_at_unix_ms: u64,
) -> Result<PathBuf> {
    std::fs::create_dir_all(directory)
        .with_context(|| format!("creating {}", directory.display()))?;

    let label = if metadata.expected_invalid {
        "invalid".to_owned()
    } else {
        metadata
            .expected
            .or_else(|| recognition.recognized_character())
            .map(|character| character.to_string())
            .unwrap_or_else(|| "unrecognized".to_owned())
    };
    let json = sample_json(ink, recognition, metadata, saved_at_unix_ms)?;
    for collision in 0..1_000 {
        let suffix = if collision == 0 {
            String::new()
        } else {
            format!("-{collision}")
        };
        let path = directory.join(format!("kana-{saved_at_unix_ms}-{label}{suffix}.json"));
        let mut pending = tempfile::Builder::new()
            .prefix(".kana-sample-")
            .suffix(".json.part")
            .tempfile_in(directory)
            .with_context(|| format!("creating a temporary sample in {}", directory.display()))?;
        pending
            .write_all(&json)
            .and_then(|()| pending.as_file().sync_all())
            .with_context(|| format!("preparing {}", path.display()))?;
        match pending.persist_noclobber(&path) {
            Ok(_) => return Ok(path),
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error.error).with_context(|| format!("publishing {}", path.display()));
            }
        }
    }
    bail!(
        "could not find a free sample filename in {}",
        directory.display()
    )
}

fn sample_json(
    ink: &CharacterInk,
    recognition: &Recognition,
    metadata: SampleMetadata,
    saved_at_unix_ms: u64,
) -> Result<Vec<u8>> {
    let record = SavedSample {
        format: SAMPLE_FORMAT,
        format_version: SAMPLE_FORMAT_VERSION,
        saved_at_unix_ms,
        app_version: env!("CARGO_PKG_VERSION"),
        expected: metadata.expected,
        expected_invalid: metadata.expected_invalid,
        expected_source: metadata.expected_source,
        writing_validity: if metadata.expected_invalid {
            WritingValidity::Invalid
        } else {
            metadata.writing_validity
        },
        prompt: metadata.prompt,
        capture: CaptureMetadata {
            stroke_count: ink.0.len(),
            point_count: ink.0.iter().map(Vec::len).sum(),
            coordinate_extent: [CANVAS_COORDS, CANVAS_COORDS],
            coordinate_origin: "top_left",
            point_time_ms_clock: "egui_monotonic_frame_clock",
            duration_ms: capture_duration_ms(ink),
        },
        recognizer: RecognizerMetadata {
            algorithm: ALGORITHM_ID,
            template_count: metadata.template_count,
            template_fingerprint: format!("{:016x}", metadata.template_fingerprint),
            result_limit: RESULT_LIMIT,
            script_scope: script_name(metadata.script_scope),
            maximum_distance: metadata.config.maximum_distance,
            maximum_path_length_ratio: metadata.config.maximum_path_length_ratio,
            minimum_margin: metadata.config.minimum_margin,
            variant_maximum_distance: VARIANT_MAXIMUM_DISTANCE,
            variant_minimum_margin: VARIANT_MINIMUM_MARGIN,
        },
        strokes: ink,
        result: recognition,
        vision: metadata.script_scope.and_then(|scope| {
            idiosepius_core::kana_vision::VisionModel::embedded()
                .ok()
                .map(|model| model.recognize(ink, scope, 5))
        }),
    };
    let mut json = serde_json::to_vec_pretty(&record).context("serializing kana sample")?;
    json.push(b'\n');
    Ok(json)
}

fn recognize_with_scope(
    recognizer: &KanaRecognizer,
    ink: &CharacterInk,
    script_scope: Option<KanaScript>,
    limit: usize,
) -> Recognition {
    match script_scope {
        Some(script) => recognizer.recognize_script(ink, script, limit),
        None => recognizer.recognize(ink, limit),
    }
}

fn script_name(script_scope: Option<KanaScript>) -> &'static str {
    match script_scope {
        None => "both",
        Some(KanaScript::Hiragana) => "hiragana",
        Some(KanaScript::Katakana) => "katakana",
    }
}

fn recognition_diagnostic(recognition: &Recognition, config: RecognitionConfig) -> String {
    let distance = recognition.top().map(|candidate| candidate.distance);
    let margin = recognition
        .candidates
        .get(1)
        .zip(recognition.top())
        .map(|(second, first)| second.distance - first.distance);
    let path_length_ratio = recognition.path_length_ratio.unwrap_or_default();
    match recognition.state {
        RecognitionState::Unrecognized {
            reason: RejectionReason::InputLimit,
        } => "too much input for recognition; clear or undo a stroke".to_owned(),
        RecognitionState::Recognized => format!(
            "match {:.4} · gap {:.4} · ink {:.2}×",
            distance.unwrap_or_default(),
            margin.unwrap_or_default(),
            path_length_ratio,
        ),
        RecognitionState::Unrecognized {
            reason: RejectionReason::InsufficientInk,
        } => "insufficient ink".to_owned(),
        RecognitionState::Unrecognized {
            reason: RejectionReason::PoorMatch,
        } => format!(
            "poor match · {:.4} > {:.4}",
            distance.unwrap_or_default(),
            config.maximum_distance
        ),
        RecognitionState::Unrecognized {
            reason: RejectionReason::ExcessiveInk,
        } => format!(
            "excessive ink · path {:.2}× > {:.2}×",
            path_length_ratio, config.maximum_path_length_ratio
        ),
        RecognitionState::Unrecognized {
            reason: RejectionReason::Ambiguous,
        } => format!(
            "ambiguous · margin {:.4} < {:.4}",
            margin.unwrap_or_default(),
            config.minimum_margin
        ),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExpectedVerdict {
    AcceptedCorrect,
    AcceptedWrong,
    Rejected,
}

fn expected_diagnostic(recognition: &Recognition, expected: char) -> (String, ExpectedVerdict) {
    let actual = recognition
        .top()
        .map(|candidate| candidate.character.to_string())
        .unwrap_or_else(|| "—".to_owned());
    let rank = if recognition.candidates.is_empty() {
        "—".to_owned()
    } else {
        recognition
            .candidates
            .iter()
            .position(|candidate| candidate.character == expected)
            .map(|index| (index + 1).to_string())
            .unwrap_or_else(|| format!(">{}", recognition.candidates.len()))
    };
    let verdict = match recognition.recognized_character() {
        Some(actual) if actual == expected => ExpectedVerdict::AcceptedCorrect,
        Some(_) => ExpectedVerdict::AcceptedWrong,
        None => ExpectedVerdict::Rejected,
    };
    let outcome = match verdict {
        ExpectedVerdict::AcceptedCorrect => "accepted correct",
        ExpectedVerdict::AcceptedWrong => "accepted wrong",
        ExpectedVerdict::Rejected => "rejected",
    };
    (
        format!("expected {expected} · top {actual} · rank {rank} · {outcome}"),
        verdict,
    )
}

fn capture_duration_ms(ink: &CharacterInk) -> Option<f64> {
    let mut earliest = f64::INFINITY;
    let mut latest = f64::NEG_INFINITY;
    for time in ink
        .0
        .iter()
        .flatten()
        .filter_map(|point| point.time_ms)
        .filter(|time| time.is_finite())
    {
        earliest = earliest.min(time);
        latest = latest.max(time);
    }
    earliest.is_finite().then_some((latest - earliest).max(0.0))
}

fn prompt_shuffle_seed(duration: &Duration) -> u64 {
    let nanos = duration.as_nanos();
    nanos as u64 ^ (nanos >> 64) as u64
}

fn shuffle_prompt(characters: &mut [char], seed: u64) {
    let mut state = seed;
    for upper in (1..characters.len()).rev() {
        state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut random = state;
        random = (random ^ (random >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        random = (random ^ (random >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        random ^= random >> 31;
        characters.swap(upper, random as usize % (upper + 1));
    }
}

fn ink_point(
    rect: Rect,
    view: InkView,
    pos: Pos2,
    time_ms: f64,
    pressure: Option<f32>,
    pointer: PointerKind,
) -> Option<InkPoint> {
    rect.contains(pos)
        .then(|| raw_ink_point(rect, view, pos, time_ms, pressure, pointer))
}

fn raw_ink_point(
    rect: Rect,
    view: InkView,
    pos: Pos2,
    time_ms: f64,
    pressure: Option<f32>,
    pointer: PointerKind,
) -> InkPoint {
    InkPoint {
        x: view.min_x + (pos.x - rect.left()) / rect.width() * view.side,
        y: view.min_y + (pos.y - rect.top()) / rect.height() * view.side,
        time_ms: Some(time_ms),
        pressure,
        pointer: Some(pointer),
    }
}

fn accepts_mouse_channel(has_touch: bool, active: Option<ActivePointer>) -> bool {
    !matches!(active, Some(ActivePointer::Touch(..)))
        && (!has_touch || active == Some(ActivePointer::Mouse))
}

fn screen_point(rect: Rect, view: InkView, point: &InkPoint) -> Pos2 {
    Pos2::new(
        rect.left() + (point.x - view.min_x) / view.side * rect.width(),
        rect.top() + (point.y - view.min_y) / view.side * rect.height(),
    )
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::BTreeSet, rc::Rc};

    use super::*;

    #[test]
    fn starting_another_stroke_hides_both_stale_predictions() {
        let mut app = CanvasApp::new(
            KanaRecognizer::new().unwrap(),
            None,
            Box::new(|_, _, _| panic!("unexpected save")),
        );
        app.script_scope = Some(KanaScript::Katakana);
        app.ink = app.recognizer.canonical('フ').unwrap();
        app.refresh();
        assert!(app.recognition.is_some() && app.vision.is_some());
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::splat(100.));
        let event = |phase| Event::Touch {
            device_id: TouchDeviceId(1),
            id: TouchId(1),
            phase,
            pos: Pos2::new(20., 20.),
            force: None,
        };
        app.capture_batch(vec![event(TouchPhase::Start)], 10., rect);
        assert!(app.recognition.is_none() && app.vision.is_none());
        app.set_script_scope(Some(KanaScript::Hiragana));
        assert!(app.recognition.is_none() && app.vision.is_none());
        app.set_script_scope(Some(KanaScript::Katakana));
        assert!(app.recognition.is_none() && app.vision.is_none());
        // A new stroke can begin in the same frame as the preceding release.
        app.capture_batch(
            vec![event(TouchPhase::End), event(TouchPhase::Start)],
            20.,
            rect,
        );
        assert!(app.recognition.is_none() && app.vision.is_none());
        app.capture_batch(vec![event(TouchPhase::End)], 30., rect);
        assert!(app.recognition.is_some() && app.vision.is_some());
    }

    #[test]
    fn replay_reports_variant_threshold_changes_and_incomplete_legacy_metadata() {
        let recognizer = KanaRecognizer::new().unwrap();
        let config = recognizer.config();
        let current = serde_json::json!({
            "format": SAMPLE_FORMAT, "format_version": 1, "strokes": [],
            "recognizer": {
                "algorithm": ALGORITHM_ID,
                "template_fingerprint": format!("{:016x}", recognizer.template_fingerprint()),
                "maximum_distance": config.maximum_distance,
                "maximum_path_length_ratio": config.maximum_path_length_ratio,
                "minimum_margin": config.minimum_margin,
                "variant_maximum_distance": VARIANT_MAXIMUM_DISTANCE,
                "variant_minimum_margin": VARIANT_MINIMUM_MARGIN,
            },
        });
        let mut app = CanvasApp::new(
            recognizer,
            None,
            Box::new(|_, _, _| panic!("unexpected save")),
        );
        for field in ["variant_maximum_distance", "variant_minimum_margin"] {
            let mut changed = current.clone();
            changed["recognizer"][field] = 0.9.into();
            app.load_sample(parse_initial_sample(&changed.to_string()).unwrap())
                .unwrap();
            assert!(
                app.notice
                    .as_ref()
                    .unwrap()
                    .text
                    .contains("metadata changed")
            );
            changed["recognizer"].as_object_mut().unwrap().remove(field);
            app.load_sample(parse_initial_sample(&changed.to_string()).unwrap())
                .unwrap();
            assert!(
                app.notice
                    .as_ref()
                    .unwrap()
                    .text
                    .contains("metadata incomplete")
            );
        }
    }

    #[test]
    fn captured_touch_keeps_stationary_pressure_and_owns_the_whole_stroke() {
        let mut app = CanvasApp::new(
            KanaRecognizer::new().unwrap(),
            None,
            Box::new(|_, _, _| panic!("unexpected save")),
        );
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::splat(100.));
        let touch = |id, phase, x, force| Event::Touch {
            device_id: TouchDeviceId(1),
            id: TouchId(id),
            phase,
            pos: Pos2::new(x, 20.),
            force: Some(force),
        };
        app.capture_batch(vec![touch(1, TouchPhase::Start, 20., 0.2)], 10., rect);
        app.capture_batch(
            vec![
                touch(2, TouchPhase::Start, 80., 0.9),
                touch(2, TouchPhase::Move, 90., 0.9),
                Event::PointerMoved(Pos2::new(90., 90.)),
                touch(1, TouchPhase::Move, 20., 0.7),
            ],
            20.,
            rect,
        );
        app.capture_batch(vec![touch(1, TouchPhase::Move, 120., 0.5)], 30., rect);
        app.capture_batch(vec![touch(1, TouchPhase::End, 120., 0.)], 40., rect);
        assert_eq!(app.ink.0.len(), 1);
        let points = &app.ink.0[0];
        assert_eq!(points.len(), 4);
        assert_eq!(points[1].pressure, Some(0.7));
        assert_eq!(points[1].time_ms, Some(20.));
        assert!(points[2].x > CANVAS_COORDS);
        assert_eq!(points[3].time_ms, Some(40.));
        assert_eq!(capture_duration_ms(&app.ink), Some(30.));
        assert!(app.active.is_none());
        assert!(app.recognition.is_some());
    }

    #[test]
    fn cancelled_touch_releases_ownership_for_the_next_stroke() {
        let mut app = CanvasApp::new(
            KanaRecognizer::new().unwrap(),
            None,
            Box::new(|_, _, _| panic!("unexpected save")),
        );
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::splat(100.));
        let touch = |phase| Event::Touch {
            device_id: TouchDeviceId(1),
            id: TouchId(1),
            phase,
            pos: Pos2::new(20., 20.),
            force: None,
        };
        app.capture_batch(
            vec![touch(TouchPhase::Start), touch(TouchPhase::Cancel)],
            10.,
            rect,
        );
        assert!(app.active.is_none());
        app.capture_batch(vec![touch(TouchPhase::Start)], 20., rect);
        assert_eq!(app.ink.0.len(), 2);
    }
    use idiosepius_core::kana::{Candidate, KanaScript};

    #[test]
    fn canvas_coordinates_round_trip() {
        let rect = Rect::from_min_size(Pos2::new(20.0, 40.0), Vec2::new(512.0, 512.0));
        let point = ink_point(
            rect,
            InkView::CANVAS,
            Pos2::new(276.0, 296.0),
            123.0,
            Some(0.5),
            PointerKind::Pen,
        )
        .unwrap();
        assert_eq!([point.x, point.y], [512.0, 512.0]);
        assert_eq!(
            screen_point(rect, InkView::CANVAS, &point),
            Pos2::new(276.0, 296.0)
        );
        assert_eq!(point.time_ms, Some(123.0));
        assert_eq!(point.pressure, Some(0.5));
    }

    #[test]
    fn canvas_rejects_points_outside_its_rect() {
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::splat(100.0));
        assert!(
            ink_point(
                rect,
                InkView::CANVAS,
                Pos2::new(101.0, 50.0),
                0.0,
                None,
                PointerKind::Mouse,
            )
            .is_none()
        );
    }

    #[test]
    fn fitted_replay_view_is_reversible_without_mutating_raw_ink() {
        let rect = Rect::from_min_size(Pos2::new(20.0, 40.0), Vec2::new(500.0, 500.0));
        let ink = CharacterInk::from_xy([vec![[-400.0, 200.0], [1_600.0, 1_200.0]]]);
        let original = ink.clone();
        let view = InkView::fitted(&ink);

        for point in ink.0.iter().flatten() {
            let screen = screen_point(rect, view, point);
            assert!(rect.contains(screen));
            let restored = ink_point(rect, view, screen, 0.0, None, PointerKind::Mouse).unwrap();
            assert!((restored.x - point.x).abs() < 0.001);
            assert!((restored.y - point.y).abs() < 0.001);
        }
        assert_eq!(ink, original);
    }

    #[test]
    fn fitted_replay_view_handles_a_remote_dot_and_extreme_finite_extent() {
        let rect = Rect::from_min_size(Pos2::new(20.0, 40.0), Vec2::new(500.0, 500.0));
        let dot = InkPoint::at(50_000.0, -20_000.0);
        let dot_view = InkView::fitted(&CharacterInk(vec![vec![dot.clone()]]));
        assert_eq!(screen_point(rect, dot_view, &dot), rect.center());

        let extreme = CharacterInk::from_xy([vec![[0.0, 0.0], [3.0e38, 3.0e38]]]);
        let extreme_view = InkView::fitted(&extreme);
        assert!(extreme_view.side.is_finite());
        for point in extreme.0.iter().flatten() {
            let screen = screen_point(rect, extreme_view, point);
            assert!(screen.x.is_finite() && screen.y.is_finite());
            assert!(screen.x >= rect.left() && screen.x <= rect.right());
            assert!(screen.y >= rect.top() && screen.y <= rect.bottom());
        }
    }

    #[test]
    fn touch_suppresses_emulated_mouse_but_not_an_existing_mouse_stroke() {
        assert!(!accepts_mouse_channel(true, None));
        assert!(accepts_mouse_channel(false, None));
        assert!(accepts_mouse_channel(true, Some(ActivePointer::Mouse)));
    }

    #[test]
    fn capture_duration_uses_the_full_monotonic_sample_range() {
        let ink = CharacterInk(vec![
            vec![InkPoint {
                time_ms: Some(250.0),
                ..InkPoint::at(0.0, 0.0)
            }],
            vec![
                InkPoint {
                    time_ms: Some(100.0),
                    ..InkPoint::at(1.0, 1.0)
                },
                InkPoint {
                    time_ms: Some(f64::NAN),
                    ..InkPoint::at(2.0, 2.0)
                },
            ],
        ]);
        assert_eq!(capture_duration_ms(&ink), Some(150.0));
        assert_eq!(capture_duration_ms(&CharacterInk::default()), None);
    }

    #[test]
    fn prompted_set_visits_every_character_in_the_selected_script_once() {
        let recognizer = KanaRecognizer::new().unwrap();
        let saver: SampleSaver = Box::new(|_, _, _| unreachable!());
        let mut app = CanvasApp::new(recognizer, None, saver);
        app.set_script_scope(Some(KanaScript::Hiragana));

        let prompted: BTreeSet<_> = (0..46).filter_map(|_| app.next_prompt()).collect();
        assert_eq!(prompted.len(), 46);
        assert!(prompted.iter().all(|expected| {
            app.recognizer
                .characters()
                .any(|(character, script)| character == *expected && script == KanaScript::Hiragana)
        }));
        assert_eq!(app.prompt.as_ref().unwrap().position, 46);

        app.set_script_scope(Some(KanaScript::Katakana));
        let katakana: BTreeSet<_> = (0..46).filter_map(|_| app.next_prompt()).collect();
        assert_eq!(katakana.len(), 46);
        assert!(katakana.iter().all(|expected| {
            app.recognizer
                .characters()
                .any(|(character, script)| character == *expected && script == KanaScript::Katakana)
        }));
        assert_eq!(app.prompt.as_ref().unwrap().position, 46);
    }

    #[test]
    fn prompt_shuffle_is_reproducible_and_not_just_a_rotation() {
        let recognizer = KanaRecognizer::new().unwrap();
        let original: Vec<_> = recognizer
            .characters()
            .filter_map(|(character, script)| (script == KanaScript::Hiragana).then_some(character))
            .collect();
        let mut first = original.clone();
        let mut repeated = original.clone();
        let mut second = original.clone();
        shuffle_prompt(&mut first, 0x0123_4567_89ab_cdef);
        shuffle_prompt(&mut repeated, 0x0123_4567_89ab_cdef);
        shuffle_prompt(&mut second, 0xfedc_ba98_7654_3210);

        assert_eq!(first, repeated);
        assert_ne!(first, second);
        assert_eq!(
            first.iter().copied().collect::<BTreeSet<_>>(),
            original.iter().copied().collect()
        );
        assert_eq!(
            second.iter().copied().collect::<BTreeSet<_>>(),
            original.iter().copied().collect()
        );
        assert!(!(0..first.len()).any(|offset| {
            first
                .iter()
                .enumerate()
                .all(|(index, character)| second[(index + offset) % second.len()] == *character)
        }));
    }

    #[test]
    fn prompted_collection_requires_script_context() {
        let saver: SampleSaver = Box::new(|_, _, _| unreachable!());
        let mut app = CanvasApp::new(KanaRecognizer::new().unwrap(), None, saver);
        assert_eq!(app.next_prompt(), None);
        assert!(app.prompt.is_none());
        assert!(app.notice.as_ref().unwrap().error);
    }

    #[test]
    fn inconsistent_prompt_state_reports_an_error_instead_of_panicking() {
        let saver: SampleSaver = Box::new(|_, _, _| unreachable!());
        let mut app = CanvasApp::new(KanaRecognizer::new().unwrap(), None, saver);
        app.script_scope = Some(KanaScript::Hiragana);
        app.prompt = Some(PromptRun {
            scope: Some(KanaScript::Hiragana),
            run_started_at_unix_ms: 123,
            sequence: Vec::new(),
            position: 0,
            total: 46,
            saved: 0,
        });

        assert_eq!(app.next_prompt(), None);
        assert!(app.notice.as_ref().unwrap().error);
        assert!(app.notice.as_ref().unwrap().text.contains("inconsistent"));
    }

    #[test]
    fn successful_prompted_save_records_provenance_and_advances() {
        let saved = Rc::new(RefCell::new(Vec::new()));
        let saved_from_callback = Rc::clone(&saved);
        let saver: SampleSaver = Box::new(move |ink, _, metadata| {
            assert!(!ink.is_empty());
            let prompt = metadata.prompt.unwrap();
            saved_from_callback.borrow_mut().push((
                metadata.expected,
                metadata.expected_source,
                prompt.run_started_at_unix_ms,
                prompt.position,
                prompt.total,
                prompt.saved_before,
                prompt.sequence,
            ));
            Ok(PathBuf::from("prompted-sample.json"))
        });
        let mut app = CanvasApp::new(KanaRecognizer::new().unwrap(), None, saver);
        app.set_script_scope(Some(KanaScript::Hiragana));
        let first = app.next_prompt().unwrap();
        app.ink = app.recognizer.canonical(first).unwrap();
        app.refresh();

        app.save_sample();

        let second = app.expected_character().unwrap().unwrap();
        app.ink = app.recognizer.canonical(second).unwrap();
        app.refresh();
        app.save_sample();

        let saved = saved.borrow();
        assert_eq!(saved.len(), 2);
        assert_eq!(saved[0].0, Some(first));
        assert_eq!(saved[1].0, Some(second));
        assert_eq!(saved[0].1, Some(ExpectedSource::Prompted));
        assert_eq!(saved[1].1, Some(ExpectedSource::Prompted));
        assert_eq!(saved[0].2, saved[1].2);
        assert_ne!(saved[0].2, 0);
        assert_eq!((saved[0].3, saved[0].4, saved[0].5), (1, 46, 0));
        assert_eq!((saved[1].3, saved[1].4, saved[1].5), (2, 46, 1));
        assert_eq!(saved[0].6, saved[1].6);
        assert_eq!(saved[0].6.chars().count(), 46);
        assert_eq!(saved[0].6.chars().next(), Some(first));
        assert_eq!(saved[1].6.chars().nth(1), Some(second));
        assert!(app.ink.is_empty());
        assert_ne!(app.expected, second.to_string());
        assert_eq!(app.prompt.as_ref().unwrap().position, 3);
        assert_eq!(app.prompt.as_ref().unwrap().saved, 2);
    }

    #[test]
    fn prompted_save_rejects_an_edited_or_invalid_label() {
        let saver: SampleSaver = Box::new(|_, _, _| panic!("mislabeled prompt was saved"));
        let mut app = CanvasApp::new(KanaRecognizer::new().unwrap(), None, saver);
        app.set_script_scope(Some(KanaScript::Hiragana));
        let first = app.next_prompt().unwrap();
        app.ink = app.recognizer.canonical(first).unwrap();
        app.refresh();

        app.expected = "-".into();
        app.save_sample();
        assert!(app.notice.as_ref().unwrap().error);
        assert!(app.notice.as_ref().unwrap().text.contains("prompted kana"));

        app.expected = if first == 'あ' { "い" } else { "あ" }.into();
        app.save_sample();
        assert!(app.notice.as_ref().unwrap().error);
        assert!(app.notice.as_ref().unwrap().text.contains("prompted kana"));
    }

    #[test]
    fn prompt_completion_reports_kana_skipped_with_next() {
        let saver: SampleSaver = Box::new(|_, _, _| Ok(PathBuf::from("last-sample.json")));
        let mut app = CanvasApp::new(KanaRecognizer::new().unwrap(), None, saver);
        app.set_script_scope(Some(KanaScript::Hiragana));
        for _ in 0..46 {
            app.next_prompt().unwrap();
        }
        let last = app.expected_character().unwrap().unwrap();
        app.ink = app.recognizer.canonical(last).unwrap();
        app.refresh();

        app.save_sample();

        assert!(app.prompt.is_none());
        let notice = &app.notice.as_ref().unwrap().text;
        assert!(notice.contains("1 saved"), "{notice}");
        assert!(notice.contains("45 skipped"), "{notice}");
    }

    #[test]
    fn complete_prompted_collection_emits_one_consistent_run() {
        let saved = Rc::new(RefCell::new(Vec::new()));
        let saved_from_callback = Rc::clone(&saved);
        let saver: SampleSaver = Box::new(move |_, _, metadata| {
            let prompt = metadata.prompt.unwrap();
            saved_from_callback.borrow_mut().push((
                metadata.expected.unwrap(),
                prompt.run_started_at_unix_ms,
                prompt.position,
                prompt.total,
                prompt.saved_before,
                prompt.sequence,
            ));
            Ok(PathBuf::from("prompted-sample.json"))
        });
        let mut app = CanvasApp::new(KanaRecognizer::new().unwrap(), None, saver);
        app.set_script_scope(Some(KanaScript::Hiragana));
        app.next_prompt().unwrap();

        for _ in 0..46 {
            let expected = app.expected_character().unwrap().unwrap();
            app.ink = app.recognizer.canonical(expected).unwrap();
            app.refresh();
            app.save_sample();
        }

        let saved = saved.borrow();
        assert_eq!(saved.len(), 46);
        let run_started_at_unix_ms = saved[0].1;
        assert_ne!(run_started_at_unix_ms, 0);
        let sequence = &saved[0].5;
        assert_eq!(sequence.chars().count(), 46);
        for (index, (expected, run, position, total, saved_before, saved_sequence)) in
            saved.iter().enumerate()
        {
            assert_eq!(*run, run_started_at_unix_ms);
            assert_eq!(*position, index + 1);
            assert_eq!(*total, 46);
            assert_eq!(*saved_before, index);
            assert_eq!(saved_sequence, sequence);
            assert_eq!(saved_sequence.chars().nth(index), Some(*expected));
        }
        assert_eq!(
            saved
                .iter()
                .map(|sample| sample.0)
                .collect::<BTreeSet<_>>()
                .len(),
            46
        );
        assert!(app.prompt.is_none());
        let notice = &app.notice.as_ref().unwrap().text;
        assert!(notice.contains("46 saved"), "{notice}");
        assert!(notice.contains("0 skipped"), "{notice}");
    }

    #[test]
    fn invalid_label_is_saved_and_restored_without_becoming_a_kana() {
        let saved = Rc::new(RefCell::new(None));
        let saved_from_callback = Rc::clone(&saved);
        let saver: SampleSaver = Box::new(move |_, _, metadata| {
            *saved_from_callback.borrow_mut() = Some((
                metadata.expected,
                metadata.expected_invalid,
                metadata.expected_source,
            ));
            Ok(PathBuf::from("invalid-sample.json"))
        });
        let recognizer = KanaRecognizer::new().unwrap();
        let mut app = CanvasApp::new(recognizer, None, saver);
        app.expected = "-".to_owned();
        app.ink = app.recognizer.canonical('あ').unwrap();
        app.refresh();

        app.save_sample();

        assert_eq!(
            *saved.borrow(),
            Some((None, true, Some(ExpectedSource::Manual)))
        );

        let json = sample_json(
            &app.ink,
            app.recognition.as_ref().unwrap(),
            SampleMetadata {
                config: app.recognizer.config(),
                template_count: app.recognizer.template_count(),
                template_fingerprint: app.recognizer.template_fingerprint(),
                script_scope: None,
                expected: None,
                expected_invalid: true,
                writing_validity: WritingValidity::Invalid,
                expected_source: Some(ExpectedSource::Manual),
                prompt: None,
            },
            123,
        )
        .unwrap();
        let document: serde_json::Value = serde_json::from_slice(&json).unwrap();
        assert_eq!(document["expected_invalid"], true);
        assert_eq!(document["writing_validity"], "invalid");
        assert!(document.get("expected").is_none());
        let initial = parse_initial_sample(std::str::from_utf8(&json).unwrap()).unwrap();
        assert!(initial.expected_invalid);
        app.load_sample(initial).unwrap();
        assert_eq!(app.expected, "-");
    }

    #[test]
    fn sample_cannot_save_a_label_outside_the_selected_script() {
        let saver: SampleSaver = Box::new(|_, _, _| panic!("inconsistent sample was saved"));
        let mut app = CanvasApp::new(KanaRecognizer::new().unwrap(), None, saver);
        app.expected = "あ".to_owned();
        app.script_scope = Some(KanaScript::Katakana);
        app.ink = app.recognizer.canonical('あ').unwrap();
        app.refresh();

        app.save_sample();

        assert!(app.notice.as_ref().unwrap().error);
        assert!(app.notice.as_ref().unwrap().text.contains("script scope"));
    }

    #[test]
    fn replay_rejects_a_label_outside_the_saved_script_scope() {
        let saver: SampleSaver = Box::new(|_, _, _| unreachable!());
        let mut app = CanvasApp::new(KanaRecognizer::new().unwrap(), None, saver);
        let sample = InitialSample {
            ink: app.recognizer.canonical('あ').unwrap(),
            expected: Some('あ'),
            expected_invalid: false,
            writing_validity: WritingValidity::Unknown,
            origin: InitialOrigin::SavedSample,
            script_scope: Some(KanaScript::Katakana),
            stored_result: None,
            stored_algorithm: None,
            stored_template_fingerprint: None,
            stored_maximum_distance: None,
            stored_maximum_path_length_ratio: None,
            stored_minimum_margin: None,
            stored_variant_thresholds: VariantThresholds::default(),
        };

        let error = app.load_sample(sample).unwrap_err();

        assert!(error.to_string().contains("script scope"));
        assert!(app.ink.is_empty());
    }

    #[test]
    fn active_stroke_cannot_be_saved_with_a_stale_result() {
        let saver: SampleSaver = Box::new(|_, _, _| panic!("active stroke was saved"));
        let mut app = CanvasApp::new(KanaRecognizer::new().unwrap(), None, saver);
        app.expected = "あ".to_owned();
        app.ink = app.recognizer.canonical('あ').unwrap();
        app.refresh();
        app.active = Some(ActivePointer::Mouse);
        app.ink.0.push(vec![InkPoint::at(10.0, 10.0)]);

        app.save_sample();

        assert!(app.notice.as_ref().unwrap().error);
        assert!(app.notice.as_ref().unwrap().text.contains("active stroke"));
    }

    #[test]
    fn saved_sample_can_be_reopened_and_recomputed() {
        let recognizer = KanaRecognizer::new().unwrap();
        let ink = recognizer.canonical('あ').unwrap();
        let recognition = recognizer.recognize_script(&ink, KanaScript::Hiragana, RESULT_LIMIT);
        let json = sample_json(
            &ink,
            &recognition,
            SampleMetadata {
                config: recognizer.config(),
                template_count: recognizer.template_count(),
                template_fingerprint: recognizer.template_fingerprint(),
                script_scope: Some(KanaScript::Hiragana),
                expected: Some('あ'),
                expected_invalid: false,
                writing_validity: WritingValidity::Unknown,
                expected_source: Some(ExpectedSource::Prompted),
                prompt: Some(PromptMetadata {
                    run_started_at_unix_ms: 123,
                    position: 1,
                    total: 46,
                    saved_before: 0,
                    sequence: recognizer
                        .characters()
                        .filter_map(|(character, script)| {
                            (script == KanaScript::Hiragana).then_some(character)
                        })
                        .collect(),
                }),
            },
            123,
        )
        .unwrap();
        let initial = parse_initial_sample(std::str::from_utf8(&json).unwrap()).unwrap();
        let raw = parse_initial_sample(&serde_json::to_string(&ink).unwrap()).unwrap();
        assert_eq!(raw.ink, ink);
        assert_eq!(raw.expected, None);
        assert_eq!(raw.script_scope, None);
        let mut synthetic_result = recognition.clone();
        synthetic_result.candidates.truncate(5);
        let synthetic = parse_initial_sample(
            &serde_json::json!({
                "expected": "あ",
                "script": "hiragana",
                "result": synthetic_result,
                "ink": ink.clone(),
            })
            .to_string(),
        )
        .unwrap();
        assert_eq!(synthetic.expected, Some('あ'));
        assert_eq!(synthetic.script_scope, Some(KanaScript::Hiragana));
        assert!(synthetic.stored_result.is_some());
        let invalid_probe = parse_initial_sample(
            &serde_json::json!({
                "kind": "spiral",
                "recognized": "へ",
                "script": "hiragana",
                "distance": 0.04,
                "path_length_ratio": 1.2,
                "result": {
                    "state": "recognized",
                    "candidates": [{
                        "character": "へ",
                        "script": "hiragana",
                        "distance": 0.04
                    }],
                    "path_length_ratio": 1.2
                },
                "ink": CharacterInk::from_xy([vec![[0.0, 0.0], [1.0, 1.0]]]),
            })
            .to_string(),
        )
        .unwrap();
        assert!(invalid_probe.expected_invalid);
        assert_eq!(invalid_probe.origin, InitialOrigin::SyntheticInvalidProbe);
        assert!(invalid_probe.stored_result.is_some());
        let saver: SampleSaver = Box::new(|_, _, _| unreachable!());
        let mut app = CanvasApp::new(recognizer, None, saver);

        app.load_sample(invalid_probe).unwrap();
        assert!(app.notice.as_ref().unwrap().text.contains("invalid probe"));
        app.load_sample(synthetic).unwrap();
        assert!(
            app.notice
                .as_ref()
                .unwrap()
                .text
                .contains("synthetic failure")
        );
        assert!(app.notice.as_ref().unwrap().text.contains("reproduced"));

        app.load_sample(initial).unwrap();

        assert_eq!(app.ink, ink);
        assert_eq!(app.expected, "あ");
        assert_eq!(app.script_scope, Some(KanaScript::Hiragana));
        assert_eq!(
            app.recognition
                .as_ref()
                .and_then(Recognition::recognized_character),
            Some('あ')
        );
        assert!(app.notice.as_ref().unwrap().text.contains("reproduced"));

        let mut changed_metadata: serde_json::Value = serde_json::from_slice(&json).unwrap();
        let mut without_scope = changed_metadata.clone();
        without_scope["recognizer"]
            .as_object_mut()
            .unwrap()
            .remove("script_scope");
        assert_eq!(
            parse_initial_sample(&without_scope.to_string())
                .unwrap()
                .script_scope,
            None
        );
        changed_metadata["recognizer"]["template_fingerprint"] = "0000000000000000".into();
        let changed = parse_initial_sample(&changed_metadata.to_string()).unwrap();
        app.load_sample(changed).unwrap();
        assert!(
            app.notice
                .as_ref()
                .unwrap()
                .text
                .contains("metadata changed")
        );

        let mut changed_threshold: serde_json::Value = serde_json::from_slice(&json).unwrap();
        changed_threshold["recognizer"]["maximum_distance"] = 0.5.into();
        let changed = parse_initial_sample(&changed_threshold.to_string()).unwrap();
        app.load_sample(changed).unwrap();
        assert!(
            app.notice
                .as_ref()
                .unwrap()
                .text
                .contains("metadata changed")
        );
    }

    #[test]
    fn reviewed_validity_survives_reexport_but_not_changed_ink_or_intent() {
        for (label, validity) in [
            ("valid", WritingValidity::Valid),
            ("invalid", WritingValidity::Invalid),
        ] {
            let recognizer = KanaRecognizer::new().unwrap();
            let ink = recognizer.canonical('あ').unwrap();
            let document = serde_json::json!({
                "format": SAMPLE_FORMAT, "format_version": 1,
                "expected": "あ", "writing_validity": label, "strokes": ink,
                "recognizer": { "script_scope": "hiragana" }
            })
            .to_string();
            let saved = Rc::new(RefCell::new(None));
            let captured = saved.clone();
            let saver: SampleSaver = Box::new(move |ink, result, metadata| {
                *captured.borrow_mut() = Some(sample_json(ink, result, metadata, 123)?);
                Ok(PathBuf::from("test-only-no-file.json"))
            });
            let mut app = CanvasApp::new(recognizer, None, saver);
            app.load_sample(parse_initial_sample(&document).unwrap())
                .unwrap();
            assert_eq!(app.writing_validity(), validity);
            app.save_sample();
            let exported: serde_json::Value =
                serde_json::from_slice(saved.borrow().as_ref().unwrap()).unwrap();
            assert_eq!(exported["expected"], "あ");
            assert_eq!(exported["writing_validity"], label);
            assert!(exported.get("expected_invalid").is_none());

            app.load_sample(parse_initial_sample(&document).unwrap())
                .unwrap();
            app.ink.0[0][0].x += 1.0;
            assert_eq!(app.writing_validity(), WritingValidity::Unknown);
            app.load_sample(parse_initial_sample(&document).unwrap())
                .unwrap();
            app.expected = "い".to_owned();
            assert_eq!(app.writing_validity(), WritingValidity::Unknown);
            app.load_sample(parse_initial_sample(&document).unwrap())
                .unwrap();
            app.clear();
            assert_eq!(app.writing_validity(), WritingValidity::Unknown);
        }
    }

    #[test]
    fn validity_import_defaults_and_conflicts_are_explicit() {
        let base = serde_json::json!({
            "format": SAMPLE_FORMAT, "format_version": 1,
            "expected": "あ", "strokes": []
        });
        assert_eq!(
            parse_initial_sample(&base.to_string())
                .unwrap()
                .writing_validity,
            WritingValidity::Unknown
        );
        let mut conflict = base.clone();
        conflict.as_object_mut().unwrap().remove("expected");
        conflict["expected_invalid"] = serde_json::json!(true);
        conflict["writing_validity"] = serde_json::json!("valid");
        assert!(parse_initial_sample(&conflict.to_string()).is_err());
        let mut malformed = base;
        malformed["writing_validity"] = serde_json::json!("perfect");
        assert!(parse_initial_sample(&malformed.to_string()).is_err());
    }

    #[test]
    fn rejection_diagnostic_names_the_failed_threshold() {
        let recognition = Recognition {
            state: RecognitionState::Unrecognized {
                reason: RejectionReason::PoorMatch,
            },
            candidates: vec![Candidate {
                character: 'あ',
                script: KanaScript::Hiragana,
                distance: 0.08,
            }],
            path_length_ratio: Some(1.0),
        };
        let diagnostic = recognition_diagnostic(&recognition, RecognitionConfig::default());
        assert!(diagnostic.contains("poor match"));
        assert!(diagnostic.contains("0.0800 > 0.0650"));

        let excessive = Recognition {
            state: RecognitionState::Unrecognized {
                reason: RejectionReason::ExcessiveInk,
            },
            candidates: recognition.candidates,
            path_length_ratio: Some(2.4),
        };
        let diagnostic = recognition_diagnostic(&excessive, RecognitionConfig::default());
        assert!(diagnostic.contains("excessive ink"));
        assert!(diagnostic.contains("2.40× > 1.70×"));
    }

    #[test]
    fn expected_diagnostic_distinguishes_near_and_complete_misses() {
        let recognition = Recognition {
            state: RecognitionState::Recognized,
            candidates: vec![
                Candidate {
                    character: 'あ',
                    script: KanaScript::Hiragana,
                    distance: 0.02,
                },
                Candidate {
                    character: 'お',
                    script: KanaScript::Hiragana,
                    distance: 0.04,
                },
            ],
            path_length_ratio: Some(1.0),
        };

        assert_eq!(
            expected_diagnostic(&recognition, 'お'),
            (
                "expected お · top あ · rank 2 · accepted wrong".to_owned(),
                ExpectedVerdict::AcceptedWrong,
            )
        );
        assert_eq!(
            expected_diagnostic(&recognition, 'ん'),
            (
                "expected ん · top あ · rank >2 · accepted wrong".to_owned(),
                ExpectedVerdict::AcceptedWrong,
            )
        );
        assert_eq!(
            expected_diagnostic(&recognition, 'あ').1,
            ExpectedVerdict::AcceptedCorrect
        );
        let empty = Recognition {
            state: RecognitionState::Unrecognized {
                reason: RejectionReason::InsufficientInk,
            },
            candidates: Vec::new(),
            path_length_ratio: None,
        };
        assert_eq!(
            expected_diagnostic(&empty, 'あ'),
            (
                "expected あ · top — · rank — · rejected".to_owned(),
                ExpectedVerdict::Rejected,
            )
        );

        let nearest_correct_but_rejected = Recognition {
            state: RecognitionState::Unrecognized {
                reason: RejectionReason::Ambiguous,
            },
            candidates: recognition.candidates,
            path_length_ratio: Some(1.0),
        };
        assert_eq!(
            expected_diagnostic(&nearest_correct_but_rejected, 'あ'),
            (
                "expected あ · top あ · rank 1 · rejected".to_owned(),
                ExpectedVerdict::Rejected,
            )
        );
    }

    #[test]
    fn saved_sample_contains_lossless_ink_metadata_and_result() {
        let ink = CharacterInk(vec![vec![InkPoint {
            x: 12.5,
            y: 34.75,
            time_ms: Some(456.25),
            pressure: Some(0.625),
            pointer: Some(PointerKind::Pen),
        }]]);
        let recognition = Recognition {
            state: RecognitionState::Recognized,
            candidates: vec![Candidate {
                character: 'あ',
                script: KanaScript::Hiragana,
                distance: 0.0125,
            }],
            path_length_ratio: Some(1.0),
        };
        let mut prompt_sequence: Vec<_> = KanaRecognizer::new()
            .unwrap()
            .characters()
            .filter_map(|(character, script)| (script == KanaScript::Hiragana).then_some(character))
            .collect();
        let a_position = prompt_sequence
            .iter()
            .position(|character| *character == 'あ')
            .unwrap();
        let prompt_sequence_len = prompt_sequence.len();
        prompt_sequence.rotate_left((a_position + prompt_sequence_len - 6) % prompt_sequence_len);
        let json = sample_json(
            &ink,
            &recognition,
            SampleMetadata {
                config: RecognitionConfig::default(),
                template_count: 92,
                template_fingerprint: 0x0123_4567_89ab_cdef,
                script_scope: Some(KanaScript::Hiragana),
                expected: Some('あ'),
                expected_invalid: false,
                writing_validity: WritingValidity::Unknown,
                expected_source: Some(ExpectedSource::Prompted),
                prompt: Some(PromptMetadata {
                    run_started_at_unix_ms: 1_724_999_000_000,
                    position: 7,
                    total: 46,
                    saved_before: 6,
                    sequence: prompt_sequence.iter().collect(),
                }),
            },
            1_725_000_000_123,
        )
        .unwrap();
        let record: serde_json::Value = serde_json::from_slice(&json).unwrap();

        assert_eq!(record["format"], SAMPLE_FORMAT);
        assert_eq!(record["format_version"], SAMPLE_FORMAT_VERSION);
        assert_eq!(record["saved_at_unix_ms"], 1_725_000_000_123_u64);
        assert_eq!(record["expected"], "あ");
        assert_eq!(record["expected_source"], "prompted");
        assert_eq!(record["writing_validity"], "unknown");
        assert_eq!(
            record["prompt"]["run_started_at_unix_ms"],
            1_724_999_000_000_u64
        );
        assert_eq!(record["prompt"]["position"], 7);
        assert_eq!(record["prompt"]["total"], 46);
        assert_eq!(record["prompt"]["saved_before"], 6);
        assert_eq!(
            record["prompt"]["sequence"]
                .as_str()
                .unwrap()
                .chars()
                .nth(6),
            Some('あ')
        );
        assert_eq!(record["capture"]["duration_ms"], 0.0);
        assert_eq!(record["capture"]["stroke_count"], 1);
        assert_eq!(record["capture"]["point_count"], 1);
        assert_eq!(
            record["capture"]["point_time_ms_clock"],
            "egui_monotonic_frame_clock"
        );
        assert_eq!(record["recognizer"]["algorithm"], ALGORITHM_ID);
        assert_eq!(
            record["recognizer"]["template_fingerprint"],
            "0123456789abcdef"
        );
        assert_eq!(record["strokes"][0][0]["time_ms"], 456.25);
        assert_eq!(record["strokes"][0][0]["pressure"], 0.625);
        assert_eq!(record["strokes"][0][0]["pointer"], "pen");
        assert_eq!(record["result"]["state"], "recognized");
        assert_eq!(record["result"]["path_length_ratio"], 1.0);
        assert_eq!(record["result"]["candidates"][0]["character"], "あ");
        assert_eq!(record["result"]["candidates"][0]["distance"], 0.0125);
        assert_eq!(record["recognizer"]["template_count"], 92);
        assert_eq!(record["recognizer"]["script_scope"], "hiragana");
        assert_eq!(record["recognizer"]["maximum_path_length_ratio"], 1.7);
    }

    #[test]
    fn sample_writer_creates_its_directory_and_a_readable_record() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "idiosepius-kana-sample-test-{}-{unique}",
            std::process::id()
        ));
        let ink = CharacterInk::from_xy([vec![[10.0, 20.0], [30.0, 40.0]]]);
        let recognition = Recognition {
            state: RecognitionState::Unrecognized {
                reason: idiosepius_core::kana::RejectionReason::Ambiguous,
            },
            candidates: vec![Candidate {
                character: 'あ',
                script: KanaScript::Hiragana,
                distance: 0.04,
            }],
            path_length_ratio: Some(1.0),
        };

        let path = write_sample(
            &directory,
            &ink,
            &recognition,
            SampleMetadata {
                config: RecognitionConfig::default(),
                template_count: 92,
                template_fingerprint: 0x0123_4567_89ab_cdef,
                script_scope: None,
                expected: None,
                expected_invalid: false,
                writing_validity: WritingValidity::Unknown,
                expected_source: None,
                prompt: None,
            },
        )
        .unwrap();
        assert!(path.starts_with(&directory));
        assert!(
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .contains("unrecognized")
        );
        let record: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(record["capture"]["point_count"], 2);
        assert_eq!(
            record["result"]["state"]["unrecognized"]["reason"],
            "ambiguous"
        );

        let invalid_recognition = Recognition {
            state: RecognitionState::Recognized,
            ..recognition
        };
        let invalid_path = write_sample(
            &directory,
            &ink,
            &invalid_recognition,
            SampleMetadata {
                config: RecognitionConfig::default(),
                template_count: 92,
                template_fingerprint: 0x0123_4567_89ab_cdef,
                script_scope: None,
                expected: None,
                expected_invalid: true,
                writing_validity: WritingValidity::Invalid,
                expected_source: Some(ExpectedSource::Manual),
                prompt: None,
            },
        )
        .unwrap();
        assert!(
            invalid_path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .contains("-invalid")
        );
        let invalid_record: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&invalid_path).unwrap()).unwrap();
        assert_eq!(invalid_record["expected_invalid"], true);

        let metadata = || SampleMetadata {
            config: RecognitionConfig::default(),
            template_count: 92,
            template_fingerprint: 0x0123_4567_89ab_cdef,
            script_scope: None,
            expected: None,
            expected_invalid: true,
            writing_validity: WritingValidity::Invalid,
            expected_source: Some(ExpectedSource::Manual),
            prompt: None,
        };
        let first_collision =
            write_sample_at(&directory, &ink, &invalid_recognition, metadata(), 42).unwrap();
        let first_bytes = std::fs::read(&first_collision).unwrap();
        let second_collision =
            write_sample_at(&directory, &ink, &invalid_recognition, metadata(), 42).unwrap();
        assert_ne!(first_collision, second_collision);
        assert_eq!(first_collision.file_name().unwrap(), "kana-42-invalid.json");
        assert_eq!(
            second_collision.file_name().unwrap(),
            "kana-42-invalid-1.json"
        );
        assert_eq!(std::fs::read(&first_collision).unwrap(), first_bytes);
        let entries: Vec<_> = std::fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(entries.len(), 4);
        assert!(
            entries
                .iter()
                .all(|path| path.extension().unwrap() == "json")
        );
        assert!(entries.iter().all(|path| {
            !path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with(".part")
        }));

        std::fs::remove_dir_all(directory).unwrap();
    }
}
