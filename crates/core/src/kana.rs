//! Order-independent online handwriting recognition for basic Japanese kana.
//!
//! Raw pointer samples remain in [`CharacterInk`]. Recognition derives a
//! normalized line representation without changing that input, and compares
//! the geometry independently of stroke order, stroke direction, or where a
//! stroke was split.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

const TEMPLATE_DATA: &str = include_str!("../assets/kana_templates.json");
pub const ALGORITHM_ID: &str = "multi-prototype-symmetric-directional-chamfer-v3";
const TEMPLATE_FINGERPRINT: u64 = fnv1a64(TEMPLATE_DATA.as_bytes());
const FEATURE_SPACING: f32 = 0.025;
const DIRECTION_WEIGHT: f32 = 0.012;
// Bitmap-derived variants improve ranking across handwriting styles, but are
// less faithful evidence than the canonical online trajectories. Require a
// distinctly close and unambiguous hit before one can produce an acceptance.
pub const VARIANT_MAXIMUM_DISTANCE: f32 = 0.05;
pub const VARIANT_MINIMUM_MARGIN: f32 = 0.02;

/// Optional on older saved samples; shared by native and browser replay.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
pub struct VariantThresholds {
    pub variant_maximum_distance: Option<f32>,
    pub variant_minimum_margin: Option<f32>,
}

impl VariantThresholds {
    pub fn is_complete(self) -> bool {
        self.variant_maximum_distance.is_some() && self.variant_minimum_margin.is_some()
    }

    pub fn changed(self) -> bool {
        self.variant_maximum_distance
            .is_some_and(|v| v != VARIANT_MAXIMUM_DISTANCE)
            || self
                .variant_minimum_margin
                .is_some_and(|v| v != VARIANT_MINIMUM_MARGIN)
    }
}
const PATH_SIMPLIFY_TOLERANCE: f32 = 0.015;
const CONNECTOR_MIN_POINTS: usize = 6;
const CONNECTOR_MIN_LENGTH: f32 = 0.14;
const CONNECTOR_MEDIAN_FACTOR: f32 = 8.0;
const EPSILON: f32 = 1.0e-6;

const fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325;
    let mut index = 0;
    while index < bytes.len() {
        hash ^= bytes[index] as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        index += 1;
    }
    hash
}

/// One original pointer sample.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InkPoint {
    pub x: f32,
    pub y: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pressure: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pointer: Option<PointerKind>,
}

impl InkPoint {
    pub fn at(x: f32, y: f32) -> Self {
        Self {
            x,
            y,
            time_ms: None,
            pressure: None,
            pointer: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointerKind {
    Mouse,
    Pen,
    Touch,
    Unknown,
}

pub type Stroke = Vec<InkPoint>;

/// Ordered raw strokes. Recognition never mutates or replaces these samples.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CharacterInk(pub Vec<Stroke>);

impl CharacterInk {
    pub fn from_xy(strokes: impl IntoIterator<Item = Vec<[f32; 2]>>) -> Self {
        Self(
            strokes
                .into_iter()
                .map(|stroke| {
                    stroke
                        .into_iter()
                        .map(|[x, y]| InkPoint::at(x, y))
                        .collect()
                })
                .collect(),
        )
    }

    pub fn strokes(&self) -> &[Stroke] {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.iter().all(Vec::is_empty)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KanaScript {
    Hiragana,
    Katakana,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub character: char,
    pub script: KanaScript,
    /// Raw geometric distance. Lower is better; it is not a probability.
    pub distance: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectionReason {
    InputLimit,
    InsufficientInk,
    PoorMatch,
    ExcessiveInk,
    Ambiguous,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecognitionState {
    Recognized,
    Unrecognized { reason: RejectionReason },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Recognition {
    pub state: RecognitionState,
    /// Ranked diagnostics are retained even when the sample is rejected.
    pub candidates: Vec<Candidate>,
    /// Simplified path length relative to the top template (always >= 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_length_ratio: Option<f32>,
}

impl Recognition {
    pub fn top(&self) -> Option<&Candidate> {
        self.candidates.first()
    }

    pub fn recognized_character(&self) -> Option<char> {
        match self.state {
            RecognitionState::Recognized => self.top().map(|candidate| candidate.character),
            RecognitionState::Unrecognized { .. } => None,
        }
    }

    /// Whether a current, possibly longer candidate list reproduces this result.
    ///
    /// Diagnostic result limits may grow without changing the stored verdict
    /// or any candidate the older run actually returned.
    pub fn reproduced_by(&self, current: &Self) -> bool {
        self.state == current.state
            && self
                .path_length_ratio
                .is_none_or(|stored| current.path_length_ratio == Some(stored))
            && self.candidates.len() <= current.candidates.len()
            && self.candidates == current.candidates[..self.candidates.len()]
    }
}

/// Rejection thresholds calibrated by the deterministic synthetic evaluation.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct RecognitionConfig {
    pub maximum_distance: f32,
    pub maximum_path_length_ratio: f32,
    pub minimum_margin: f32,
}

impl Default for RecognitionConfig {
    fn default() -> Self {
        Self {
            maximum_distance: 0.065,
            maximum_path_length_ratio: 1.7,
            minimum_margin: 0.012,
        }
    }
}

#[derive(Debug, Error)]
pub enum TemplateError {
    #[error("kana recognizer configuration is invalid: {0}")]
    Config(&'static str),
    #[error("kana template data is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("kana template label {0:?} is not exactly one character")]
    Label(String),
    #[error("kana template {0} contains no usable ink")]
    Empty(char),
    #[error("kana template {0} occurs more than once")]
    Duplicate(char),
}

#[derive(Clone, Debug)]
struct PreparedTemplate {
    character: char,
    script: KanaScript,
    ink: CharacterInk,
    shapes: Vec<PreparedInk>,
}

/// Geometric diagnostics for one independently selected identity.
#[derive(Clone, Debug, Serialize)]
pub struct ReferenceComparison {
    pub character: char,
    pub input_to_reference: f32,
    pub reference_to_input: f32,
    pub symmetric_distance: f32,
    /// Signed ratio: below one means less path length than the reference.
    pub path_length_ratio: f32,
}

/// Reusable recognizer containing the 46 basic hiragana and 46 basic katakana.
#[derive(Clone, Debug)]
pub struct KanaRecognizer {
    templates: Vec<PreparedTemplate>,
    config: RecognitionConfig,
}

impl KanaRecognizer {
    pub fn new() -> Result<Self, TemplateError> {
        Self::with_config(RecognitionConfig::default())
    }

    pub fn with_config(config: RecognitionConfig) -> Result<Self, TemplateError> {
        if !config.maximum_distance.is_finite() || config.maximum_distance < 0.0 {
            return Err(TemplateError::Config(
                "maximum_distance must be finite and non-negative",
            ));
        }
        if !config.maximum_path_length_ratio.is_finite() || config.maximum_path_length_ratio < 1.0 {
            return Err(TemplateError::Config(
                "maximum_path_length_ratio must be finite and at least 1",
            ));
        }
        if !config.minimum_margin.is_finite() || config.minimum_margin < 0.0 {
            return Err(TemplateError::Config(
                "minimum_margin must be finite and non-negative",
            ));
        }
        let raw: RawLibrary = serde_json::from_str(TEMPLATE_DATA)?;
        let mut templates = Vec::with_capacity(raw.templates.len());
        let mut seen = BTreeMap::new();

        for template in raw.templates {
            let mut label_characters = template.label.chars();
            let Some(character) = label_characters.next() else {
                return Err(TemplateError::Label(template.label));
            };
            if label_characters.next().is_some() {
                return Err(TemplateError::Label(template.label));
            }
            if seen.insert(character, ()).is_some() {
                return Err(TemplateError::Duplicate(character));
            }
            let ink = CharacterInk::from_xy(template.strokes);
            let Some(shape) = PreparedInk::new(&ink) else {
                return Err(TemplateError::Empty(character));
            };
            let mut shapes = vec![shape];
            for variant in template.variants {
                let variant = CharacterInk::from_xy(variant);
                let Some(shape) = PreparedInk::new(&variant) else {
                    return Err(TemplateError::Empty(character));
                };
                shapes.push(shape);
            }
            templates.push(PreparedTemplate {
                character,
                script: template.script,
                ink,
                shapes,
            });
        }

        Ok(Self { templates, config })
    }

    pub fn config(&self) -> RecognitionConfig {
        self.config
    }

    pub fn template_count(&self) -> usize {
        self.templates.len()
    }

    /// Stable identity of the exact embedded template asset.
    pub fn template_fingerprint(&self) -> u64 {
        TEMPLATE_FINGERPRINT
    }

    pub fn canonical(&self, character: char) -> Option<CharacterInk> {
        self.templates
            .iter()
            .find(|template| template.character == character)
            .map(|template| template.ink.clone())
    }

    pub fn characters(&self) -> impl ExactSizeIterator<Item = (char, KanaScript)> + '_ {
        self.templates
            .iter()
            .map(|template| (template.character, template.script))
    }

    /// Rank the nearest kana and apply absolute-distance and ambiguity rejection.
    pub fn recognize(&self, ink: &CharacterInk, limit: usize) -> Recognition {
        self.recognize_filtered(ink, limit, |_| true)
    }

    /// Rank only templates from one script.
    ///
    /// A lesson normally knows whether it is teaching hiragana or katakana.
    /// Keeping that context out of the geometric score prevents equivalent
    /// glyphs in the other script from manufacturing ambiguity.
    pub fn recognize_script(
        &self,
        ink: &CharacterInk,
        script: KanaScript,
        limit: usize,
    ) -> Recognition {
        self.recognize_filtered(ink, limit, |template| template.script == script)
    }

    /// Compare against the canonical reference for an independently selected identity.
    /// These distances describe similarity, not pedagogical handwriting quality.
    pub fn compare_reference(
        &self,
        ink: &CharacterInk,
        character: char,
    ) -> Option<ReferenceComparison> {
        let shape = PreparedInk::prepare(ink).ok()?;
        let reference = &self
            .templates
            .iter()
            .find(|t| t.character == character)?
            .shapes[0];
        Some(ReferenceComparison {
            character,
            input_to_reference: directed_distance(&shape, reference).sqrt(),
            reference_to_input: directed_distance(reference, &shape).sqrt(),
            symmetric_distance: shape_distance(&shape, reference),
            path_length_ratio: shape.simplified_length / reference.simplified_length,
        })
    }

    fn recognize_filtered(
        &self,
        ink: &CharacterInk,
        limit: usize,
        include: impl Fn(&PreparedTemplate) -> bool,
    ) -> Recognition {
        let shape = match PreparedInk::prepare(ink) {
            Ok(shape) => shape,
            Err(reason) => {
                return Recognition {
                    state: RecognitionState::Unrecognized { reason },
                    candidates: Vec::new(),
                    path_length_ratio: None,
                };
            }
        };

        let mut scored: Vec<_> = self
            .templates
            .iter()
            .filter(|template| include(template))
            .map(|template| {
                let (variant_index, distance, template_length) = template
                    .shapes
                    .iter()
                    .enumerate()
                    .map(|(index, variant)| {
                        (
                            index,
                            shape_distance(&shape, variant),
                            variant.simplified_length,
                        )
                    })
                    .min_by(|left, right| left.1.total_cmp(&right.1))
                    .unwrap();
                let length_ratio = shape.simplified_length / template_length;
                (
                    Candidate {
                        character: template.character,
                        script: template.script,
                        distance,
                    },
                    length_ratio.max(length_ratio.recip()),
                    variant_index != 0,
                )
            })
            .collect();
        debug_assert!(!scored.is_empty());
        scored.sort_by(|left, right| left.0.distance.total_cmp(&right.0.distance));
        let path_length_ratio = scored[0].1;
        let top_uses_variant = scored[0].2;
        let mut candidates: Vec<_> = scored
            .into_iter()
            .map(|(candidate, _, _)| candidate)
            .collect();

        let maximum_distance = if top_uses_variant {
            self.config.maximum_distance.min(VARIANT_MAXIMUM_DISTANCE)
        } else {
            self.config.maximum_distance
        };
        let minimum_margin = if top_uses_variant {
            self.config.minimum_margin.max(VARIANT_MINIMUM_MARGIN)
        } else {
            self.config.minimum_margin
        };
        let state = if candidates[0].distance > maximum_distance {
            RecognitionState::Unrecognized {
                reason: RejectionReason::PoorMatch,
            }
        } else if path_length_ratio > self.config.maximum_path_length_ratio {
            RecognitionState::Unrecognized {
                reason: RejectionReason::ExcessiveInk,
            }
        } else if candidates.len() > 1
            && candidates[1].distance - candidates[0].distance < minimum_margin
        {
            RecognitionState::Unrecognized {
                reason: RejectionReason::Ambiguous,
            }
        } else {
            RecognitionState::Recognized
        };

        candidates.truncate(limit.max(1).min(candidates.len()));
        Recognition {
            state,
            candidates,
            path_length_ratio: Some(path_length_ratio),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Feature {
    x: f32,
    y: f32,
    dx: f32,
    dy: f32,
    weight: f32,
}

#[derive(Clone, Debug)]
struct PreparedInk {
    features: Vec<Feature>,
    total_weight: f32,
    simplified_length: f32,
}

impl PreparedInk {
    fn new(ink: &CharacterInk) -> Option<Self> {
        Self::prepare(ink).ok()
    }

    fn prepare(ink: &CharacterInk) -> Result<Self, RejectionReason> {
        // Keep synchronous native/browser recognition bounded. The raw sample
        // stays intact for saving and diagnosis even when it exceeds the budget.
        if ink.0.len() > 16_384 || ink.0.iter().map(Vec::len).sum::<usize>() > 16_384 {
            return Err(RejectionReason::InputLimit);
        }
        let mut min_x = f32::INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut max_y = f32::NEG_INFINITY;

        for point in ink.0.iter().flatten() {
            if !point.x.is_finite() || !point.y.is_finite() {
                continue;
            }
            min_x = min_x.min(point.x);
            min_y = min_y.min(point.y);
            max_x = max_x.max(point.x);
            max_y = max_y.max(point.y);
        }
        let extent = (max_x - min_x).max(max_y - min_y);
        if !extent.is_finite() || extent <= EPSILON {
            return Err(RejectionReason::InsufficientInk);
        }
        // This form cannot overflow when both finite bounds have the same
        // sign. Opposite extremes already produce a non-finite `extent` and
        // are rejected above.
        let center_x = min_x + (max_x - min_x) * 0.5;
        let center_y = min_y + (max_y - min_y) * 0.5;

        let mut features = Vec::new();
        let mut total_weight = 0.0;
        let mut simplified_length = 0.0;
        for stroke in &ink.0 {
            let points: Vec<_> = stroke
                .iter()
                .filter(|point| point.x.is_finite() && point.y.is_finite())
                .map(|point| ((point.x - center_x) / extent, (point.y - center_y) / extent))
                .collect();
            let connector_threshold = connector_threshold(&points);
            for pair in points.windows(2) {
                let (x0, y0) = pair[0];
                let (x1, y1) = pair[1];
                let vx = x1 - x0;
                let vy = y1 - y0;
                let length = vx.hypot(vy);
                if length <= EPSILON {
                    continue;
                }
                // Cursive input often keeps the pointer down between two
                // otherwise separate strokes. With a normally sampled line,
                // that movement appears as one conspicuously long segment.
                // Treat it as an in-air connector rather than glyph geometry.
                if connector_threshold.is_some_and(|threshold| length > threshold) {
                    continue;
                }
                let count = (length / FEATURE_SPACING).ceil().max(1.0) as usize;
                if features.len() + count > 4_096 {
                    return Err(RejectionReason::InputLimit);
                }
                let weight = length / count as f32;
                for index in 0..count {
                    let t = (index as f32 + 0.5) / count as f32;
                    features.push(Feature {
                        x: x0 + vx * t,
                        y: y0 + vy * t,
                        dx: vx / length,
                        dy: vy / length,
                        weight,
                    });
                    total_weight += weight;
                }
            }
            simplified_length +=
                simplified_stroke_length(&points, connector_threshold, PATH_SIMPLIFY_TOLERANCE);
        }

        if features.is_empty() || total_weight <= EPSILON {
            Err(RejectionReason::InsufficientInk)
        } else {
            Ok(Self {
                features,
                total_weight,
                simplified_length,
            })
        }
    }
}

fn simplified_stroke_length(
    points: &[(f32, f32)],
    connector_threshold: Option<f32>,
    tolerance: f32,
) -> f32 {
    let Some(first) = points.first().copied() else {
        return 0.0;
    };
    let mut total = 0.0;
    let mut run = vec![first];
    for pair in points.windows(2) {
        let length = (pair[1].0 - pair[0].0).hypot(pair[1].1 - pair[0].1);
        if connector_threshold.is_some_and(|threshold| length > threshold) {
            total += simplified_polyline_length(&run, tolerance);
            run.clear();
            run.push(pair[1]);
        } else {
            run.push(pair[1]);
        }
    }
    total + simplified_polyline_length(&run, tolerance)
}

fn simplified_polyline_length(points: &[(f32, f32)], tolerance: f32) -> f32 {
    // An explicit stack also handles adversarial, deeply nested subdivisions.
    let mut pending = vec![points];
    let mut total = 0.0;
    while let Some(points) = pending.pop() {
        if points.len() < 2 {
            continue;
        }
        let first = points[0];
        let last = points[points.len() - 1];
        let mut furthest = (0.0, 0);
        for (index, point) in points[1..points.len() - 1].iter().enumerate() {
            let distance = point_segment_distance(*point, first, last);
            if distance > furthest.0 {
                furthest = (distance, index + 1);
            }
        }
        if furthest.0 <= tolerance {
            total += (last.0 - first.0).hypot(last.1 - first.1);
        } else {
            pending.push(&points[furthest.1..]);
            pending.push(&points[..=furthest.1]);
        }
    }
    total
}

fn point_segment_distance(point: (f32, f32), start: (f32, f32), end: (f32, f32)) -> f32 {
    let vx = end.0 - start.0;
    let vy = end.1 - start.1;
    let length_squared = vx * vx + vy * vy;
    if length_squared <= EPSILON {
        return (point.0 - start.0).hypot(point.1 - start.1);
    }
    let t =
        (((point.0 - start.0) * vx + (point.1 - start.1) * vy) / length_squared).clamp(0.0, 1.0);
    (point.0 - (start.0 + t * vx)).hypot(point.1 - (start.1 + t * vy))
}

fn connector_threshold(points: &[(f32, f32)]) -> Option<f32> {
    if points.len() < CONNECTOR_MIN_POINTS {
        return None;
    }
    let mut lengths: Vec<_> = points
        .windows(2)
        .map(|pair| (pair[1].0 - pair[0].0).hypot(pair[1].1 - pair[0].1))
        .filter(|length| *length > EPSILON)
        .collect();
    if lengths.len() < CONNECTOR_MIN_POINTS - 1 {
        return None;
    }
    lengths.sort_by(f32::total_cmp);
    let median = lengths[lengths.len() / 2];
    Some((median * CONNECTOR_MEDIAN_FACTOR).max(CONNECTOR_MIN_LENGTH))
}

fn shape_distance(left: &PreparedInk, right: &PreparedInk) -> f32 {
    let left_to_right = directed_distance(left, right);
    let right_to_left = directed_distance(right, left);
    ((left_to_right + right_to_left) * 0.5).sqrt()
}

fn directed_distance(from: &PreparedInk, to: &PreparedInk) -> f32 {
    let mut total = 0.0;
    for source in &from.features {
        let nearest = to
            .features
            .iter()
            .map(|target| {
                let x = source.x - target.x;
                let y = source.y - target.y;
                let direction = 1.0 - (source.dx * target.dx + source.dy * target.dy).abs();
                x * x + y * y + DIRECTION_WEIGHT * direction * direction
            })
            .fold(f32::INFINITY, f32::min);
        total += source.weight * nearest;
    }
    total / from.total_weight
}

#[derive(Deserialize)]
struct RawLibrary {
    #[allow(dead_code)]
    source: String,
    #[allow(dead_code)]
    source_commit: String,
    templates: Vec<RawTemplate>,
}

#[derive(Deserialize)]
struct RawTemplate {
    label: String,
    script: KanaScript,
    strokes: Vec<Vec<[f32; 2]>>,
    #[serde(default)]
    variants: Vec<Vec<Vec<[f32; 2]>>>,
}

/// Deterministic synthetic evaluation used while calibrating the recognizer.
pub mod synthetic {
    use std::time::Instant;

    use super::*;

    /// Changes whenever the same seed and configuration would generate a
    /// different corpus. Reports keep this alongside the recognizer metadata.
    pub const GENERATOR_ID: &str = "kana-synthetic-v3";
    const INVALID_STREAM_SALT: u64 = 0x8f4d_29b7_c61a_e503;
    const VALID_SAMPLE_SALT: u64 = 0x9e37_79b9_7f4a_7c15;

    #[derive(Clone, Copy, Debug)]
    pub struct SyntheticConfig {
        pub seed: u64,
        pub samples_per_character: usize,
        pub invalid_samples: usize,
        pub perturbations: SyntheticPerturbations,
    }

    /// Optional hard perturbations layered on the whole-character transform.
    /// Keeping these explicit makes evaluator regressions reproducible and
    /// lets the CLI attribute failures instead of averaging them together.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
    pub struct SyntheticPerturbations {
        pub independent_strokes: bool,
        pub shortened_terminals: bool,
        pub joined_strokes: bool,
    }

    impl Default for SyntheticConfig {
        fn default() -> Self {
            Self {
                seed: 0x1d10_5e91_05ca_11ab,
                samples_per_character: 24,
                invalid_samples: 256,
                perturbations: SyntheticPerturbations {
                    independent_strokes: true,
                    shortened_terminals: true,
                    joined_strokes: true,
                },
            }
        }
    }

    #[derive(Clone, Debug, Default, Serialize)]
    pub struct SyntheticReport {
        pub generator: &'static str,
        pub recognizer: SyntheticRecognizer,
        pub seed: u64,
        pub scripted: bool,
        pub samples_per_character: usize,
        pub perturbations: SyntheticPerturbations,
        pub character_count: usize,
        pub samples: usize,
        pub top1: usize,
        pub top5: usize,
        pub accepted_correct: usize,
        pub accepted_wrong: usize,
        pub rejected: usize,
        pub poor_match: usize,
        pub excessive_ink: usize,
        pub ambiguous: usize,
        pub invalid_samples: usize,
        pub invalid_rejected: usize,
        pub valid_distance_p95: f32,
        pub valid_distance_max: f32,
        pub valid_margin_p05: f32,
        pub valid_path_length_ratio_p95: f32,
        pub valid_path_length_ratio_max: f32,
        pub invalid_distance_p05: f32,
        pub invalid_distance_min: f32,
        pub invalid_margin_p95: f32,
        pub recognition_elapsed_ms: f64,
        pub invalid_breakdown: Vec<InvalidReport>,
        pub invalid_acceptances: Vec<InvalidAcceptance>,
        pub valid_failure_count: usize,
        pub valid_failures: Vec<ValidFailure>,
        pub characters: Vec<CharacterReport>,
        pub confusions: Vec<Confusion>,
    }

    #[derive(Clone, Debug, Default, PartialEq, Serialize)]
    pub struct SyntheticRecognizer {
        pub algorithm: &'static str,
        pub template_fingerprint: String,
        pub template_count: usize,
        pub config: RecognitionConfig,
        pub variant_maximum_distance: f32,
        pub variant_minimum_margin: f32,
    }

    #[derive(Clone, Debug, PartialEq, Serialize)]
    pub struct CharacterReport {
        pub character: char,
        pub script: KanaScript,
        pub samples: usize,
        pub top1: usize,
        pub top5: usize,
        pub accepted_correct: usize,
        pub accepted_wrong: usize,
        pub rejected: usize,
        pub poor_match: usize,
        pub excessive_ink: usize,
        pub ambiguous: usize,
        pub path_length_ratio_p95: f32,
        pub path_length_ratio_max: f32,
    }

    #[derive(Clone, Debug, PartialEq, Serialize)]
    pub struct Confusion {
        pub expected: char,
        pub recognized: char,
        pub count: usize,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub enum InvalidKind {
        Spiral,
        RandomWalk,
        FigureEight,
        Crosshatch,
    }

    impl InvalidKind {
        const ALL: [Self; 4] = [
            Self::Spiral,
            Self::RandomWalk,
            Self::FigureEight,
            Self::Crosshatch,
        ];

        fn index(self) -> usize {
            match self {
                Self::Spiral => 0,
                Self::RandomWalk => 1,
                Self::FigureEight => 2,
                Self::Crosshatch => 3,
            }
        }
    }

    #[derive(Clone, Debug, PartialEq, Serialize)]
    pub struct InvalidReport {
        pub kind: InvalidKind,
        pub samples: usize,
        pub rejected: usize,
    }

    #[derive(Clone, Debug, PartialEq, Serialize)]
    pub struct InvalidAcceptance {
        pub provenance: SyntheticCaseProvenance,
        pub probe_index: usize,
        pub kind: InvalidKind,
        pub recognized: char,
        pub script: KanaScript,
        pub distance: f32,
        pub path_length_ratio: f32,
        pub result: Recognition,
        pub ink: CharacterInk,
    }

    #[derive(Clone, Debug, PartialEq, Serialize)]
    pub struct ValidFailure {
        pub provenance: SyntheticCaseProvenance,
        pub variant_index: usize,
        pub expected: char,
        pub script: KanaScript,
        pub variation: VariationTrace,
        pub result: Recognition,
        pub ink: CharacterInk,
    }

    #[derive(Clone, Debug, PartialEq, Serialize)]
    pub struct SyntheticCaseProvenance {
        pub generator: &'static str,
        pub seed: u64,
        pub scripted: bool,
        pub perturbations: SyntheticPerturbations,
        pub recognizer: SyntheticRecognizer,
    }

    #[derive(Clone, Debug, PartialEq, Serialize)]
    pub struct VariationTrace {
        pub rotation_radians: f32,
        pub scale: [f32; 2],
        pub shear: f32,
        pub bend: [f32; 2],
        pub translation: [f32; 2],
        pub noise_amplitude: f32,
        pub maximum_stroke_scale_delta: f32,
        pub maximum_stroke_shift: f32,
        pub maximum_terminal_trim_fraction: f32,
        pub source_strokes: usize,
        pub shortened_strokes: usize,
        pub reversed_strokes: usize,
        pub joined_strokes: bool,
        pub split_strokes: usize,
        pub output_strokes: usize,
    }

    impl SyntheticReport {
        pub fn top1_rate(&self) -> f32 {
            ratio(self.top1, self.samples)
        }

        pub fn top5_rate(&self) -> f32 {
            ratio(self.top5, self.samples)
        }

        pub fn rejection_rate(&self) -> f32 {
            ratio(self.rejected, self.samples)
        }

        pub fn accepted_wrong_rate(&self) -> f32 {
            ratio(self.accepted_wrong, self.samples)
        }

        pub fn invalid_rejection_rate(&self) -> f32 {
            ratio(self.invalid_rejected, self.invalid_samples)
        }
    }

    pub fn evaluate(recognizer: &KanaRecognizer, config: SyntheticConfig) -> SyntheticReport {
        evaluate_with_scope(recognizer, config, false)
    }

    /// Evaluate with the script context a real hiragana or katakana lesson has.
    pub fn evaluate_scripted(
        recognizer: &KanaRecognizer,
        config: SyntheticConfig,
    ) -> SyntheticReport {
        evaluate_with_scope(recognizer, config, true)
    }

    fn evaluate_with_scope(
        recognizer: &KanaRecognizer,
        config: SyntheticConfig,
        scripted: bool,
    ) -> SyntheticReport {
        let mut report = SyntheticReport {
            generator: GENERATOR_ID,
            recognizer: SyntheticRecognizer {
                algorithm: ALGORITHM_ID,
                template_fingerprint: format!("{:016x}", recognizer.template_fingerprint()),
                template_count: recognizer.template_count(),
                config: recognizer.config(),
                variant_maximum_distance: VARIANT_MAXIMUM_DISTANCE,
                variant_minimum_margin: VARIANT_MINIMUM_MARGIN,
            },
            seed: config.seed,
            scripted,
            samples_per_character: config.samples_per_character,
            perturbations: config.perturbations,
            character_count: recognizer.template_count(),
            invalid_samples: config.invalid_samples,
            invalid_breakdown: InvalidKind::ALL
                .into_iter()
                .map(|kind| InvalidReport {
                    kind,
                    samples: 0,
                    rejected: 0,
                })
                .collect(),
            ..SyntheticReport::default()
        };
        let case_provenance = case_provenance(&report);
        let mut confusions = BTreeMap::new();
        let mut valid_distances = Vec::new();
        let mut valid_margins = Vec::new();
        let mut valid_path_length_ratios = Vec::new();
        let mut invalid_distances = Vec::new();
        let mut invalid_margins = Vec::new();

        for template in &recognizer.templates {
            let mut character_report = CharacterReport {
                character: template.character,
                script: template.script,
                samples: 0,
                top1: 0,
                top5: 0,
                accepted_correct: 0,
                accepted_wrong: 0,
                rejected: 0,
                poor_match: 0,
                excessive_ink: 0,
                ambiguous: 0,
                path_length_ratio_p95: 0.0,
                path_length_ratio_max: 0.0,
            };
            let mut character_path_length_ratios = Vec::new();
            let mut failure_example = None;
            for sample_index in 0..config.samples_per_character {
                let mut rng = Rng::new(valid_sample_seed(
                    config.seed,
                    template.character,
                    template.script,
                    sample_index,
                ));
                let (sample, variation) = vary(&template.ink, &mut rng, config.perturbations);
                if let Some(sample_shape) = PreparedInk::new(&sample) {
                    let ratio =
                        sample_shape.simplified_length / template.shapes[0].simplified_length;
                    let ratio = ratio.max(ratio.recip());
                    valid_path_length_ratios.push(ratio);
                    character_path_length_ratios.push(ratio);
                }
                let started = Instant::now();
                let result = if scripted {
                    recognizer.recognize_script(&sample, template.script, 5)
                } else {
                    recognizer.recognize(&sample, 5)
                };
                report.recognition_elapsed_ms += started.elapsed().as_secs_f64() * 1_000.0;
                record_score(&result, &mut valid_distances, &mut valid_margins);
                report.samples += 1;
                character_report.samples += 1;
                let rejected = !matches!(result.state, RecognitionState::Recognized);
                if rejected {
                    report.rejected += 1;
                    character_report.rejected += 1;
                    match &result.state {
                        RecognitionState::Unrecognized {
                            reason: RejectionReason::PoorMatch,
                        } => {
                            report.poor_match += 1;
                            character_report.poor_match += 1;
                        }
                        RecognitionState::Unrecognized {
                            reason: RejectionReason::Ambiguous,
                        } => {
                            report.ambiguous += 1;
                            character_report.ambiguous += 1;
                        }
                        RecognitionState::Unrecognized {
                            reason: RejectionReason::ExcessiveInk,
                        } => {
                            report.excessive_ink += 1;
                            character_report.excessive_ink += 1;
                        }
                        RecognitionState::Unrecognized {
                            reason: RejectionReason::InsufficientInk | RejectionReason::InputLimit,
                        }
                        | RecognitionState::Recognized => {}
                    }
                }
                let top_matches = result
                    .top()
                    .is_some_and(|top| top.character == template.character);
                if top_matches {
                    report.top1 += 1;
                    character_report.top1 += 1;
                } else if let Some(top) = result.top() {
                    *confusions
                        .entry((template.character, top.character))
                        .or_insert(0) += 1;
                }
                if !rejected && top_matches {
                    report.accepted_correct += 1;
                    character_report.accepted_correct += 1;
                } else if !rejected {
                    report.accepted_wrong += 1;
                    character_report.accepted_wrong += 1;
                }
                if result
                    .candidates
                    .iter()
                    .any(|candidate| candidate.character == template.character)
                {
                    report.top5 += 1;
                    character_report.top5 += 1;
                }
                if rejected || !top_matches {
                    report.valid_failure_count += 1;
                    let candidate = ValidFailure {
                        provenance: case_provenance.clone(),
                        variant_index: sample_index,
                        expected: template.character,
                        script: template.script,
                        variation,
                        result,
                        ink: sample,
                    };
                    let replace = failure_example
                        .as_ref()
                        .is_none_or(|current: &ValidFailure| {
                            current
                                .result
                                .top()
                                .is_some_and(|top| top.character == template.character)
                                && !top_matches
                        });
                    if replace {
                        failure_example = Some(candidate);
                    }
                }
            }
            character_path_length_ratios.sort_by(f32::total_cmp);
            character_report.path_length_ratio_p95 =
                percentile(&character_path_length_ratios, 0.95);
            character_report.path_length_ratio_max = character_path_length_ratios
                .last()
                .copied()
                .unwrap_or_default();
            if let Some(failure) = failure_example {
                report.valid_failures.push(failure);
            }
            report.characters.push(character_report);
        }

        // Invalid probes are an independent corpus. Changing the number or
        // shape of valid variants must not move the rejection benchmark.
        let mut invalid_rng = Rng::new(config.seed ^ INVALID_STREAM_SALT);
        for index in 0..config.invalid_samples {
            let (kind, sample) = invalid_scribble(&mut invalid_rng);
            let started = Instant::now();
            let result = if scripted {
                let script = if index % 2 == 0 {
                    KanaScript::Hiragana
                } else {
                    KanaScript::Katakana
                };
                recognizer.recognize_script(&sample, script, 5)
            } else {
                recognizer.recognize(&sample, 5)
            };
            report.recognition_elapsed_ms += started.elapsed().as_secs_f64() * 1_000.0;
            let invalid_report = &mut report.invalid_breakdown[kind.index()];
            invalid_report.samples += 1;
            record_score(&result, &mut invalid_distances, &mut invalid_margins);
            if !matches!(result.state, RecognitionState::Recognized) {
                report.invalid_rejected += 1;
                invalid_report.rejected += 1;
            } else if let Some(top) = result.top() {
                let path_length_ratio = result.path_length_ratio.unwrap_or(1.0);
                report.invalid_acceptances.push(InvalidAcceptance {
                    provenance: case_provenance.clone(),
                    probe_index: index,
                    kind,
                    recognized: top.character,
                    script: top.script,
                    distance: top.distance,
                    path_length_ratio,
                    result,
                    ink: sample,
                });
            }
        }

        let mut confusions: Vec<_> = confusions
            .into_iter()
            .map(|((expected, recognized), count)| Confusion {
                expected,
                recognized,
                count,
            })
            .collect();
        confusions.sort_by(|left, right| {
            right
                .count
                .cmp(&left.count)
                .then_with(|| left.expected.cmp(&right.expected))
                .then_with(|| left.recognized.cmp(&right.recognized))
        });
        report.confusions = confusions;
        valid_distances.sort_by(f32::total_cmp);
        valid_margins.sort_by(f32::total_cmp);
        valid_path_length_ratios.sort_by(f32::total_cmp);
        invalid_distances.sort_by(f32::total_cmp);
        invalid_margins.sort_by(f32::total_cmp);
        report.valid_distance_p95 = percentile(&valid_distances, 0.95);
        report.valid_distance_max = valid_distances.last().copied().unwrap_or_default();
        report.valid_margin_p05 = percentile(&valid_margins, 0.05);
        report.valid_path_length_ratio_p95 = percentile(&valid_path_length_ratios, 0.95);
        report.valid_path_length_ratio_max =
            valid_path_length_ratios.last().copied().unwrap_or_default();
        report.invalid_distance_p05 = percentile(&invalid_distances, 0.05);
        report.invalid_distance_min = invalid_distances.first().copied().unwrap_or_default();
        report.invalid_margin_p95 = percentile(&invalid_margins, 0.95);
        report
    }

    fn case_provenance(report: &SyntheticReport) -> SyntheticCaseProvenance {
        SyntheticCaseProvenance {
            generator: report.generator,
            seed: report.seed,
            scripted: report.scripted,
            perturbations: report.perturbations,
            recognizer: report.recognizer.clone(),
        }
    }

    fn valid_sample_seed(
        seed: u64,
        character: char,
        script: KanaScript,
        sample_index: usize,
    ) -> u64 {
        let script_key = match script {
            KanaScript::Hiragana => 0x4849_5241_u64,
            KanaScript::Katakana => 0x4b41_5441_u64,
        };
        let value = seed
            ^ (character as u64).rotate_left(17)
            ^ script_key.rotate_left(33)
            ^ (sample_index as u64).wrapping_mul(VALID_SAMPLE_SALT);
        splitmix64(value)
    }

    fn splitmix64(mut value: u64) -> u64 {
        value = value.wrapping_add(VALID_SAMPLE_SALT);
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }

    fn record_score(result: &Recognition, distances: &mut Vec<f32>, margins: &mut Vec<f32>) {
        if let Some(top) = result.candidates.first() {
            distances.push(top.distance);
        }
        if result.candidates.len() >= 2 {
            margins.push(result.candidates[1].distance - result.candidates[0].distance);
        }
    }

    fn percentile(sorted: &[f32], quantile: f32) -> f32 {
        let index = ((sorted.len().saturating_sub(1)) as f32 * quantile).round() as usize;
        sorted.get(index).copied().unwrap_or_default()
    }

    fn ratio(numerator: usize, denominator: usize) -> f32 {
        if denominator == 0 {
            0.0
        } else {
            numerator as f32 / denominator as f32
        }
    }

    fn vary(
        source: &CharacterInk,
        rng: &mut Rng,
        perturbations: SyntheticPerturbations,
    ) -> (CharacterInk, VariationTrace) {
        let (center_x, center_y, extent) = bounds(source);
        let angle = rng.range(-0.09, 0.09);
        let (sin, cos) = angle.sin_cos();
        let scale_x = rng.range(0.88, 1.12);
        let scale_y = rng.range(0.88, 1.12);
        let shear = rng.range(-0.07, 0.07);
        let bend_x = rng.range(-0.025, 0.025);
        let bend_y = rng.range(-0.025, 0.025);
        let translate_x = rng.range(-80.0, 80.0);
        let translate_y = rng.range(-80.0, 80.0);
        let noise = rng.range(1.0, 7.0);

        let mut strokes = Vec::new();
        let mut maximum_stroke_scale_delta: f32 = 0.0;
        let mut maximum_stroke_shift: f32 = 0.0;
        let mut maximum_terminal_trim_fraction: f32 = 0.0;
        let mut shortened_strokes = 0;
        let mut reversed_strokes = 0;
        for source_stroke in &source.0 {
            let dense = densify(source_stroke, 7);
            let stroke_center_x = source_stroke
                .iter()
                .map(|point| (point.x - center_x) / extent)
                .sum::<f32>()
                / source_stroke.len().max(1) as f32;
            let stroke_center_y = source_stroke
                .iter()
                .map(|point| (point.y - center_y) / extent)
                .sum::<f32>()
                / source_stroke.len().max(1) as f32;
            let varied_scale_x = rng.range(0.94, 1.06);
            let varied_scale_y = rng.range(0.94, 1.06);
            let varied_shift_x = rng.range(-0.025, 0.025);
            let varied_shift_y = rng.range(-0.025, 0.025);
            let (stroke_scale_x, stroke_scale_y, stroke_shift_x, stroke_shift_y) =
                if perturbations.independent_strokes {
                    (
                        varied_scale_x,
                        varied_scale_y,
                        varied_shift_x,
                        varied_shift_y,
                    )
                } else {
                    (1.0, 1.0, 0.0, 0.0)
                };
            maximum_stroke_scale_delta = maximum_stroke_scale_delta
                .max((stroke_scale_x - 1.0).abs())
                .max((stroke_scale_y - 1.0).abs());
            maximum_stroke_shift = maximum_stroke_shift.max(stroke_shift_x.hypot(stroke_shift_y));
            let mut stroke = Vec::with_capacity(dense.len());
            for point in dense {
                let nx = (point.x - center_x) / extent;
                let ny = (point.y - center_y) / extent;
                let warped_x = stroke_center_x
                    + stroke_scale_x * (nx - stroke_center_x)
                    + stroke_shift_x
                    + bend_x * (ny * std::f32::consts::PI).sin();
                let warped_y = stroke_center_y
                    + stroke_scale_y * (ny - stroke_center_y)
                    + stroke_shift_y
                    + bend_y * (nx * std::f32::consts::PI).sin();
                let x = scale_x * warped_x + shear * warped_y;
                let y = scale_y * warped_y;
                stroke.push(InkPoint::at(
                    512.0 + 1024.0 * (cos * x - sin * y) + translate_x + rng.range(-noise, noise),
                    512.0 + 1024.0 * (sin * x + cos * y) + translate_y + rng.range(-noise, noise),
                ));
            }
            if stroke.len() >= 16 {
                let trim_start = rng.range(0.0, 0.05);
                let trim_end = rng.range(0.0, 0.05);
                if perturbations.shortened_terminals {
                    shortened_strokes += 1;
                    maximum_terminal_trim_fraction =
                        maximum_terminal_trim_fraction.max(trim_start).max(trim_end);
                    trim_stroke(&mut stroke, trim_start, trim_end);
                }
            }
            if rng.chance(1, 3) {
                stroke.reverse();
                reversed_strokes += 1;
            }
            strokes.push(stroke);
        }

        let join_strokes = strokes.len() >= 2 && rng.chance(1, 3);
        let joined_strokes = perturbations.joined_strokes && join_strokes;
        if joined_strokes {
            join_nearest_strokes(&mut strokes);
        }

        let mut split_strokes = Vec::new();
        let mut split_count = 0;
        for mut stroke in strokes {
            if stroke.len() >= 8 && rng.chance(1, 3) {
                let split = rng.usize(3, stroke.len() - 3);
                let second = stroke.split_off(split);
                split_strokes.push(stroke);
                split_strokes.push(second);
                split_count += 1;
            } else {
                split_strokes.push(stroke);
            }
        }
        rng.shuffle(&mut split_strokes);
        let trace = VariationTrace {
            rotation_radians: angle,
            scale: [scale_x, scale_y],
            shear,
            bend: [bend_x, bend_y],
            translation: [translate_x, translate_y],
            noise_amplitude: noise,
            maximum_stroke_scale_delta,
            maximum_stroke_shift,
            maximum_terminal_trim_fraction,
            source_strokes: source.0.len(),
            shortened_strokes,
            reversed_strokes,
            joined_strokes,
            split_strokes: split_count,
            output_strokes: split_strokes.len(),
        };
        (CharacterInk(split_strokes), trace)
    }

    fn trim_stroke(stroke: &mut Stroke, start_fraction: f32, end_fraction: f32) {
        let mut cumulative = Vec::with_capacity(stroke.len());
        cumulative.push(0.0);
        for pair in stroke.windows(2) {
            let length = (pair[1].x - pair[0].x).hypot(pair[1].y - pair[0].y);
            cumulative.push(cumulative.last().copied().unwrap() + length);
        }
        let total = cumulative.last().copied().unwrap_or_default();
        if total <= EPSILON {
            return;
        }
        let start = total * start_fraction;
        let end = total * (1.0 - end_fraction);
        if end - start <= EPSILON {
            return;
        }

        let mut trimmed = Vec::new();
        trimmed.push(point_at_distance(stroke, &cumulative, start));
        for (point, distance) in stroke.iter().zip(&cumulative) {
            if *distance > start && *distance < end {
                trimmed.push(point.clone());
            }
        }
        trimmed.push(point_at_distance(stroke, &cumulative, end));
        *stroke = trimmed;
    }

    fn point_at_distance(stroke: &[InkPoint], cumulative: &[f32], distance: f32) -> InkPoint {
        let upper = cumulative.partition_point(|value| *value < distance);
        if upper == 0 {
            return stroke[0].clone();
        }
        if upper >= stroke.len() {
            return stroke.last().unwrap().clone();
        }
        let lower = upper - 1;
        let segment = cumulative[upper] - cumulative[lower];
        let t = if segment <= EPSILON {
            0.0
        } else {
            (distance - cumulative[lower]) / segment
        };
        InkPoint::at(
            stroke[lower].x + (stroke[upper].x - stroke[lower].x) * t,
            stroke[lower].y + (stroke[upper].y - stroke[lower].y) * t,
        )
    }

    fn join_nearest_strokes(strokes: &mut Vec<Stroke>) {
        let mut best = (f32::INFINITY, 0, 1, false, false);
        for left in 0..strokes.len() {
            for right in (left + 1)..strokes.len() {
                for reverse_left in [false, true] {
                    for reverse_right in [false, true] {
                        let left_end = if reverse_left {
                            &strokes[left][0]
                        } else {
                            strokes[left].last().unwrap()
                        };
                        let right_start = if reverse_right {
                            strokes[right].last().unwrap()
                        } else {
                            &strokes[right][0]
                        };
                        let distance =
                            (left_end.x - right_start.x).hypot(left_end.y - right_start.y);
                        if distance < best.0 {
                            best = (distance, left, right, reverse_left, reverse_right);
                        }
                    }
                }
            }
        }
        let (_, left, right, reverse_left, reverse_right) = best;
        let mut right_stroke = strokes.remove(right);
        let mut left_stroke = strokes.remove(left);
        if reverse_left {
            left_stroke.reverse();
        }
        if reverse_right {
            right_stroke.reverse();
        }
        left_stroke.extend(right_stroke);
        strokes.push(left_stroke);
    }

    fn bounds(ink: &CharacterInk) -> (f32, f32, f32) {
        let mut min_x = f32::INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        for point in ink.0.iter().flatten() {
            min_x = min_x.min(point.x);
            min_y = min_y.min(point.y);
            max_x = max_x.max(point.x);
            max_y = max_y.max(point.y);
        }
        (
            (min_x + max_x) * 0.5,
            (min_y + max_y) * 0.5,
            (max_x - min_x).max(max_y - min_y).max(1.0),
        )
    }

    fn densify(stroke: &[InkPoint], subdivisions: usize) -> Stroke {
        if stroke.len() < 2 {
            return stroke.to_vec();
        }
        let mut dense = Vec::new();
        for pair in stroke.windows(2) {
            for index in 0..subdivisions {
                let t = index as f32 / subdivisions as f32;
                dense.push(InkPoint::at(
                    pair[0].x + (pair[1].x - pair[0].x) * t,
                    pair[0].y + (pair[1].y - pair[0].y) * t,
                ));
            }
        }
        dense.push(stroke.last().unwrap().clone());
        dense
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn reference_comparison_uses_the_requested_identity_without_changing_ranking() {
            let recognizer = KanaRecognizer::new().unwrap();
            let ink = recognizer.canonical('フ').unwrap();
            let matched = recognizer.compare_reference(&ink, 'フ').unwrap();
            assert!(matched.symmetric_distance < 0.000001);
            assert!((matched.path_length_ratio - 1.).abs() < 0.000001);
            let other = recognizer.compare_reference(&ink, 'ア').unwrap();
            assert_eq!(other.character, 'ア');
            assert!(other.symmetric_distance > 0.01);
            assert!(recognizer.compare_reference(&ink, '漢').is_none());
            assert_eq!(
                recognizer
                    .recognize_script(&ink, KanaScript::Katakana, 1)
                    .top()
                    .unwrap()
                    .character,
                'フ'
            );
        }

        #[test]
        fn terminal_shortening_uses_length_not_point_density() {
            let mut sparse_then_dense = vec![
                InkPoint::at(0.0, 0.0),
                InkPoint::at(90.0, 0.0),
                InkPoint::at(99.0, 0.0),
                InkPoint::at(100.0, 0.0),
            ];
            trim_stroke(&mut sparse_then_dense, 0.1, 0.1);

            assert!((sparse_then_dense.first().unwrap().x - 10.0).abs() < 1.0e-5);
            assert!((sparse_then_dense.last().unwrap().x - 90.0).abs() < 1.0e-5);
        }

        #[test]
        fn valid_variant_seeds_are_stable_and_partitioned() {
            let seed = 0x1234_5678;
            let first = valid_sample_seed(seed, 'あ', KanaScript::Hiragana, 0);
            assert_eq!(
                first,
                valid_sample_seed(seed, 'あ', KanaScript::Hiragana, 0)
            );
            assert_ne!(
                first,
                valid_sample_seed(seed, 'あ', KanaScript::Hiragana, 1)
            );
            assert_ne!(
                first,
                valid_sample_seed(seed, 'い', KanaScript::Hiragana, 0)
            );
            assert_ne!(
                first,
                valid_sample_seed(seed, 'ア', KanaScript::Katakana, 0)
            );
        }

        #[test]
        fn variation_trace_describes_applied_structural_changes() {
            let source = CharacterInk::from_xy([
                vec![[0.0, 0.0], [0.3, 0.2], [0.6, 0.1], [1.0, 0.0]],
                vec![[0.0, 1.0], [0.3, 0.8], [0.6, 0.9], [1.0, 1.0]],
            ]);
            let disabled = SyntheticPerturbations::default();
            let (ink, trace) = vary(&source, &mut Rng::new(7), disabled);
            assert_eq!(trace.source_strokes, 2);
            assert_eq!(trace.output_strokes, ink.0.len());
            assert_eq!(trace.shortened_strokes, 0);
            assert!(!trace.joined_strokes);
            assert_eq!(trace.maximum_stroke_scale_delta, 0.0);
            assert_eq!(trace.maximum_stroke_shift, 0.0);
            assert_eq!(trace.maximum_terminal_trim_fraction, 0.0);

            let enabled = SyntheticPerturbations {
                independent_strokes: true,
                shortened_terminals: true,
                joined_strokes: true,
            };
            let mut observed_join = false;
            for seed in 1..=32 {
                let (ink, trace) = vary(&source, &mut Rng::new(seed), enabled);
                assert_eq!(trace.output_strokes, ink.0.len());
                assert_eq!(trace.shortened_strokes, 2);
                assert!(trace.maximum_stroke_scale_delta > 0.0);
                assert!(trace.maximum_stroke_shift > 0.0);
                assert!(trace.maximum_terminal_trim_fraction > 0.0);
                observed_join |= trace.joined_strokes;
            }
            assert!(observed_join);
        }
    }

    fn invalid_scribble(rng: &mut Rng) -> (InvalidKind, CharacterInk) {
        match rng.next() % 4 {
            0 => (InvalidKind::Spiral, spiral(rng)),
            1 => (InvalidKind::RandomWalk, random_walk(rng)),
            2 => (InvalidKind::FigureEight, figure_eight(rng)),
            _ => (InvalidKind::Crosshatch, crosshatch(rng)),
        }
    }

    fn spiral(rng: &mut Rng) -> CharacterInk {
        let turns = rng.range(2.5, 6.5);
        let points = rng.usize(20, 55);
        let phase = rng.range(0.0, std::f32::consts::TAU);
        let mut stroke = Vec::with_capacity(points);
        for index in 0..points {
            let t = index as f32 / (points - 1) as f32;
            let angle = phase + turns * std::f32::consts::TAU * t;
            let radius = 70.0 + 360.0 * t;
            stroke.push(InkPoint::at(
                512.0 + radius * angle.cos() + rng.range(-16.0, 16.0),
                512.0 + radius * angle.sin() + rng.range(-16.0, 16.0),
            ));
        }
        CharacterInk(vec![stroke])
    }

    fn random_walk(rng: &mut Rng) -> CharacterInk {
        let points = rng.usize(12, 30);
        let mut x = 512.0;
        let mut y = 512.0;
        let mut stroke = vec![InkPoint::at(x, y)];
        for _ in 1..points {
            x = (x + rng.range(-260.0, 260.0)).clamp(40.0, 984.0);
            y = (y + rng.range(-260.0, 260.0)).clamp(40.0, 984.0);
            stroke.push(InkPoint::at(x, y));
        }
        CharacterInk(vec![stroke])
    }

    fn figure_eight(rng: &mut Rng) -> CharacterInk {
        let points = rng.usize(30, 60);
        let phase = rng.range(0.0, std::f32::consts::TAU);
        let mut stroke = Vec::with_capacity(points);
        for index in 0..points {
            let t = phase + std::f32::consts::TAU * index as f32 / (points - 1) as f32;
            stroke.push(InkPoint::at(
                512.0 + 410.0 * t.sin() + rng.range(-12.0, 12.0),
                512.0 + 360.0 * (2.0 * t).sin() + rng.range(-12.0, 12.0),
            ));
        }
        CharacterInk(vec![stroke])
    }

    fn crosshatch(rng: &mut Rng) -> CharacterInk {
        let mut strokes = Vec::new();
        for index in 0..rng.usize(5, 10) {
            let offset = 100.0 + index as f32 * 100.0;
            let wobble = rng.range(-30.0, 30.0);
            strokes.push(vec![
                InkPoint::at(80.0, offset + wobble),
                InkPoint::at(944.0, 1024.0 - offset + wobble),
            ]);
        }
        CharacterInk(strokes)
    }

    #[derive(Clone, Copy, Debug)]
    struct Rng(u64);

    impl Rng {
        fn new(seed: u64) -> Self {
            Self(seed.max(1))
        }

        fn next(&mut self) -> u64 {
            let mut value = self.0;
            value ^= value << 13;
            value ^= value >> 7;
            value ^= value << 17;
            self.0 = value;
            value
        }

        fn unit(&mut self) -> f32 {
            (self.next() >> 40) as f32 / (1_u32 << 24) as f32
        }

        fn range(&mut self, low: f32, high: f32) -> f32 {
            low + (high - low) * self.unit()
        }

        fn usize(&mut self, low: usize, high: usize) -> usize {
            low + self.next() as usize % (high - low)
        }

        fn chance(&mut self, numerator: u64, denominator: u64) -> bool {
            self.next() % denominator < numerator
        }

        fn shuffle<T>(&mut self, values: &mut [T]) {
            for index in (1..values.len()).rev() {
                values.swap(index, self.next() as usize % (index + 1));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_all_basic_kana() {
        let recognizer = KanaRecognizer::new().unwrap();
        assert_eq!(recognizer.template_count(), 92);
        assert!(
            recognizer
                .templates
                .iter()
                .filter(|template| template.script == KanaScript::Hiragana)
                .all(|template| template.shapes.len() == 2)
        );
        assert!(
            recognizer
                .templates
                .iter()
                .filter(|template| template.script == KanaScript::Katakana)
                .all(|template| template.shapes.len() == 1)
        );
        assert_eq!(
            recognizer
                .characters()
                .filter(|(_, script)| *script == KanaScript::Hiragana)
                .count(),
            46
        );
        assert_eq!(
            recognizer
                .characters()
                .filter(|(_, script)| *script == KanaScript::Katakana)
                .count(),
            46
        );
    }

    #[test]
    fn screen_oriented_katakana_matches_independent_svg_coordinates() {
        // Pinned AnimCJK SVG centerline, not read from our generated JSON asset.
        let ink = CharacterInk::from_xy([vec![
            [163., 306.],
            [241., 318.],
            [691., 218.],
            [774., 252.],
            [566., 568.],
            [246., 847.],
        ]]);
        let recognizer = KanaRecognizer::new().unwrap();
        assert_eq!(
            recognizer
                .recognize_script(&ink, KanaScript::Katakana, 10)
                .recognized_character(),
            Some('フ')
        );
    }

    #[test]
    fn huge_input_is_rejected_without_discarding_original_samples() {
        let recognizer = KanaRecognizer::new().unwrap();
        for ink in [
            CharacterInk(vec![vec![InkPoint::at(0., 0.); 16_385]]),
            CharacterInk::from_xy([vec![[0., 0.], [100., 100.]].repeat(4_000)]),
        ] {
            let original = ink.clone();
            let result = recognizer.recognize(&ink, 10);
            assert_eq!(
                result.state,
                RecognitionState::Unrecognized {
                    reason: RejectionReason::InputLimit
                }
            );
            assert!(result.candidates.is_empty());
            assert_eq!(ink, original);
        }
    }

    #[test]
    fn variant_threshold_provenance_distinguishes_changes_from_legacy_absence() {
        let missing = VariantThresholds::default();
        assert!(!missing.is_complete());
        assert!(!missing.changed());
        let current = VariantThresholds {
            variant_maximum_distance: Some(VARIANT_MAXIMUM_DISTANCE),
            variant_minimum_margin: Some(VARIANT_MINIMUM_MARGIN),
        };
        assert!(current.is_complete());
        assert!(!current.changed());
        for changed in [
            VariantThresholds {
                variant_maximum_distance: Some(0.9),
                ..current
            },
            VariantThresholds {
                variant_minimum_margin: Some(0.9),
                ..current
            },
        ] {
            assert!(changed.changed());
        }
    }

    #[test]
    fn hiragana_variants_are_aggregated_by_character() {
        let recognizer = KanaRecognizer::new().unwrap();
        let raw: RawLibrary = serde_json::from_str(TEMPLATE_DATA).unwrap();
        for template in raw
            .templates
            .into_iter()
            .filter(|template| template.script == KanaScript::Hiragana)
        {
            let character = template.label.chars().next().unwrap();
            assert_eq!(template.variants.len(), 1, "{character}");
            let ink = CharacterInk::from_xy(template.variants.into_iter().next().unwrap());
            let result = recognizer.recognize_script(&ink, KanaScript::Hiragana, 100);
            assert_eq!(
                result.top().map(|candidate| candidate.character),
                Some(character)
            );
            assert_eq!(result.candidates.len(), 46);
            assert_eq!(
                result
                    .candidates
                    .iter()
                    .map(|candidate| candidate.character)
                    .collect::<std::collections::BTreeSet<_>>()
                    .len(),
                46
            );
        }
    }

    #[test]
    fn canonical_templates_recognize_themselves() {
        let recognizer = KanaRecognizer::new().unwrap();
        for (character, _) in recognizer.characters() {
            let ink = recognizer.canonical(character).unwrap();
            let result = recognizer.recognize(&ink, 10);
            assert_eq!(
                result.recognized_character(),
                Some(character),
                "{character}"
            );
            assert!(result.top().unwrap().distance < 1.0e-7, "{character}");
        }
    }

    #[test]
    fn result_reproduction_allows_only_additional_trailing_candidates() {
        let recognizer = KanaRecognizer::new().unwrap();
        let ink = recognizer.canonical('あ').unwrap();
        let current = recognizer.recognize_script(&ink, KanaScript::Hiragana, 10);
        let mut stored = current.clone();
        stored.candidates.truncate(5);
        assert!(stored.reproduced_by(&current));

        let mut legacy = stored.clone();
        legacy.path_length_ratio = None;
        assert!(legacy.reproduced_by(&current));

        let mut changed = current.clone();
        changed.candidates[0].distance += 0.001;
        assert!(!stored.reproduced_by(&changed));

        let mut changed_path = current.clone();
        changed_path.path_length_ratio = Some(current.path_length_ratio.unwrap() + 0.01);
        assert!(!stored.reproduced_by(&changed_path));

        let mut too_long = current.clone();
        too_long.candidates.push(current.candidates[0].clone());
        assert!(!too_long.reproduced_by(&current));
    }

    #[test]
    fn script_scope_never_returns_candidates_from_the_other_script() {
        let recognizer = KanaRecognizer::new().unwrap();
        let ink = recognizer.canonical('へ').unwrap();

        let hiragana = recognizer.recognize_script(&ink, KanaScript::Hiragana, 10);
        assert!(
            hiragana
                .candidates
                .iter()
                .all(|candidate| candidate.script == KanaScript::Hiragana)
        );
        assert_eq!(hiragana.recognized_character(), Some('へ'));

        let katakana = recognizer.recognize_script(&ink, KanaScript::Katakana, 10);
        assert!(
            katakana
                .candidates
                .iter()
                .all(|candidate| candidate.script == KanaScript::Katakana)
        );
    }

    #[test]
    fn stroke_order_and_direction_do_not_change_identity() {
        let recognizer = KanaRecognizer::new().unwrap();
        for character in ['あ', 'き', 'ふ', 'ア', 'キ', 'ホ'] {
            let mut ink = recognizer.canonical(character).unwrap();
            ink.0.reverse();
            for stroke in &mut ink.0 {
                stroke.reverse();
            }
            assert_eq!(
                recognizer.recognize(&ink, 10).recognized_character(),
                Some(character),
                "{character}"
            );
        }
    }

    #[test]
    fn translation_and_uniform_scale_do_not_change_identity() {
        let recognizer = KanaRecognizer::new().unwrap();
        for character in ['あ', 'ち', 'ぬ', 'ア', 'ソ', 'フ'] {
            let mut ink = recognizer.canonical(character).unwrap();
            for point in ink.0.iter_mut().flatten() {
                point.x = point.x * 3.25 + 12_345.0;
                point.y = point.y * 3.25 - 9_876.0;
            }
            let result = recognizer.recognize_script(
                &ink,
                recognizer
                    .characters()
                    .find_map(|(candidate, script)| (candidate == character).then_some(script))
                    .unwrap(),
                10,
            );
            assert_eq!(
                result.recognized_character(),
                Some(character),
                "{character}"
            );
            assert!(result.top().unwrap().distance < 1.0e-6, "{character}");
        }
    }

    #[test]
    fn duplicate_samples_and_stroke_splitting_do_not_change_identity() {
        let recognizer = KanaRecognizer::new().unwrap();
        for character in ['き', 'む', 'キ', 'ホ'] {
            let mut ink = recognizer.canonical(character).unwrap();
            for stroke in &mut ink.0 {
                *stroke = stroke
                    .iter()
                    .flat_map(|point| [point.clone(), point.clone()])
                    .collect();
            }
            let duplicated = recognizer.recognize(&ink, 10);
            assert_eq!(
                duplicated.recognized_character(),
                Some(character),
                "{character}"
            );
            assert!(duplicated.top().unwrap().distance < 1.0e-6, "{character}");
            let split = ink.0[0].len() / 2;
            let second = ink.0[0].split_off(split);
            ink.0.push(second);

            let result = recognizer.recognize(&ink, 10);
            assert_eq!(
                result.recognized_character(),
                Some(character),
                "{character}"
            );
        }
    }

    #[test]
    fn a_long_pen_down_connector_between_strokes_is_ignored() {
        let recognizer = KanaRecognizer::new().unwrap();
        let mut ink = recognizer.canonical('い').unwrap();
        assert_eq!(ink.0.len(), 2);
        let second = ink.0.pop().unwrap();
        ink.0[0].extend(second);

        let result = recognizer.recognize_script(&ink, KanaScript::Hiragana, 10);
        assert_eq!(result.recognized_character(), Some('い'), "{result:#?}");
    }

    #[test]
    fn simplified_path_length_removes_small_sampling_jitter() {
        let jittered: Vec<_> = (0..=100)
            .map(|index| {
                (
                    index as f32 / 100.0,
                    if index % 2 == 0 { -0.005 } else { 0.005 },
                )
            })
            .collect();
        let simplified = simplified_stroke_length(&jittered, None, PATH_SIMPLIFY_TOLERANCE);
        assert!((simplified - 1.0).abs() < 0.01, "{simplified}");

        let zigzag = [(0.0, 0.0), (0.5, 0.5), (0.0, 1.0), (1.0, 1.0)];
        assert!(simplified_stroke_length(&zigzag, None, PATH_SIMPLIFY_TOLERANCE) > 2.0);
    }

    #[test]
    fn excessive_zigzag_is_rejected_even_when_its_nearest_shape_is_close() {
        let recognizer = KanaRecognizer::new().unwrap();
        let ink = CharacterInk::from_xy([vec![
            [512.0, 512.0],
            [446.0184, 327.41467],
            [340.16702, 561.70203],
            [538.2016, 445.56055],
            [741.9529, 471.47467],
            [867.5983, 689.8933],
            [984.0, 689.3019],
            [787.7697, 595.33716],
            [739.6944, 551.3172],
            [910.54974, 751.6754],
            [984.0, 684.8216],
            [984.0, 754.11145],
            [984.0, 676.9887],
        ]]);

        let result = recognizer.recognize_script(&ink, KanaScript::Hiragana, 10);
        assert_eq!(result.top().unwrap().character, 'へ');
        assert_eq!(
            result.state,
            RecognitionState::Unrecognized {
                reason: RejectionReason::ExcessiveInk,
            }
        );
        assert!(result.path_length_ratio.unwrap() > recognizer.config().maximum_path_length_ratio);
    }

    #[test]
    fn synthetic_variation_is_credible_and_scribbles_are_rejected() {
        let recognizer = KanaRecognizer::new().unwrap();
        let report = synthetic::evaluate(
            &recognizer,
            synthetic::SyntheticConfig {
                samples_per_character: 3,
                invalid_samples: 48,
                ..synthetic::SyntheticConfig::default()
            },
        );
        assert_eq!(report.generator, synthetic::GENERATOR_ID);
        assert_eq!(report.recognizer.algorithm, ALGORITHM_ID);
        assert_eq!(
            report.recognizer.template_count,
            recognizer.template_count()
        );
        assert_eq!(report.recognizer.config, recognizer.config());
        assert_eq!(report.recognizer.template_fingerprint.len(), 16);
        assert_eq!(report.seed, synthetic::SyntheticConfig::default().seed);
        assert!(!report.scripted);
        assert_eq!(report.samples_per_character, 3);
        assert_eq!(
            report.perturbations,
            synthetic::SyntheticConfig::default().perturbations
        );
        assert!(report.top1_rate() >= 0.95, "{report:#?}");
        assert!(report.top5_rate() >= 0.99, "{report:#?}");
        assert!(report.rejection_rate() <= 0.10, "{report:#?}");
        assert!(report.accepted_wrong_rate() <= 0.05, "{report:#?}");
        assert!(report.invalid_rejection_rate() >= 0.95, "{report:#?}");
        assert!(report.recognition_elapsed_ms > 0.0);
        assert!(
            report.valid_path_length_ratio_max < recognizer.config().maximum_path_length_ratio,
            "{report:#?}"
        );
        assert_eq!(
            report
                .invalid_breakdown
                .iter()
                .map(|invalid| invalid.samples)
                .sum::<usize>(),
            report.invalid_samples
        );
        assert_eq!(
            report
                .invalid_breakdown
                .iter()
                .map(|invalid| invalid.rejected)
                .sum::<usize>(),
            report.invalid_rejected
        );
        assert_eq!(
            report.invalid_acceptances.len(),
            report.invalid_samples - report.invalid_rejected
        );
        assert_eq!(report.characters.len(), 92);
        assert!(report.valid_failures.iter().all(|failure| {
            failure.provenance.generator == report.generator
                && failure.provenance.seed == report.seed
                && failure.provenance.scripted == report.scripted
                && failure.provenance.perturbations == report.perturbations
                && failure.provenance.recognizer == report.recognizer
                && failure.variant_index < report.samples_per_character
                && failure.variation.output_strokes == failure.ink.0.len()
                && (!matches!(failure.result.state, RecognitionState::Recognized)
                    || !failure
                        .result
                        .top()
                        .is_some_and(|top| top.character == failure.expected))
        }));
        assert!(report.valid_failure_count >= report.rejected);
        assert!(report.valid_failure_count >= report.samples - report.top1);
        assert!(report.valid_failures.len() <= report.characters.len());
        assert_eq!(
            report
                .characters
                .iter()
                .map(|character| character.samples)
                .sum::<usize>(),
            report.samples
        );
        assert_eq!(
            report
                .characters
                .iter()
                .map(|character| character.rejected)
                .sum::<usize>(),
            report.rejected
        );
        assert_eq!(report.poor_match + report.ambiguous, report.rejected);
        assert_eq!(
            report.accepted_correct + report.accepted_wrong + report.rejected,
            report.samples
        );
        assert!(report.characters.iter().all(|character| {
            character.poor_match + character.ambiguous == character.rejected
                && character.accepted_correct + character.accepted_wrong + character.rejected
                    == character.samples
        }));
    }

    #[test]
    fn synthetic_invalid_corpus_is_independent_of_valid_variants() {
        let recognizer = KanaRecognizer::new().unwrap();
        let seed = 0xabc0_1234_5678_def0;
        let first = synthetic::evaluate_scripted(
            &recognizer,
            synthetic::SyntheticConfig {
                seed,
                samples_per_character: 0,
                invalid_samples: 8,
                perturbations: synthetic::SyntheticPerturbations::default(),
            },
        );
        let second = synthetic::evaluate_scripted(
            &recognizer,
            synthetic::SyntheticConfig {
                seed,
                samples_per_character: 1,
                invalid_samples: 8,
                ..synthetic::SyntheticConfig::default()
            },
        );

        assert_eq!(first.invalid_breakdown, second.invalid_breakdown);
        assert_eq!(first.invalid_acceptances, second.invalid_acceptances);
        assert_eq!(first.invalid_rejected, second.invalid_rejected);
        assert_eq!(first.invalid_distance_p05, second.invalid_distance_p05);
        assert_eq!(first.invalid_distance_min, second.invalid_distance_min);
        assert_eq!(first.invalid_margin_p95, second.invalid_margin_p95);
    }

    #[test]
    fn synthetic_invalid_acceptance_retains_the_complete_result() {
        let recognizer = KanaRecognizer::with_config(RecognitionConfig {
            maximum_distance: 10.0,
            maximum_path_length_ratio: 100.0,
            minimum_margin: 0.0,
        })
        .unwrap();
        let report = synthetic::evaluate_scripted(
            &recognizer,
            synthetic::SyntheticConfig {
                samples_per_character: 0,
                invalid_samples: 4,
                ..synthetic::SyntheticConfig::default()
            },
        );

        assert_eq!(report.invalid_acceptances.len(), 4);
        assert!(
            report
                .invalid_acceptances
                .iter()
                .enumerate()
                .all(|(index, acceptance)| {
                    acceptance.provenance.generator == report.generator
                        && acceptance.provenance.seed == report.seed
                        && acceptance.provenance.scripted == report.scripted
                        && acceptance.provenance.perturbations == report.perturbations
                        && acceptance.provenance.recognizer == report.recognizer
                        && acceptance.probe_index == index
                        && matches!(acceptance.result.state, RecognitionState::Recognized)
                        && acceptance
                            .result
                            .top()
                            .is_some_and(|top| top.character == acceptance.recognized)
                })
        );
    }

    #[test]
    fn empty_and_non_finite_ink_is_insufficient() {
        let recognizer = KanaRecognizer::new().unwrap();
        for ink in [
            CharacterInk::default(),
            CharacterInk(vec![vec![InkPoint::at(f32::NAN, 1.0)]]),
            CharacterInk(vec![vec![InkPoint::at(3.0, 3.0), InkPoint::at(3.0, 3.0)]]),
        ] {
            assert_eq!(
                recognizer.recognize(&ink, 10).state,
                RecognitionState::Unrecognized {
                    reason: RejectionReason::InsufficientInk
                }
            );
        }
    }

    #[test]
    fn invalid_recognition_thresholds_are_rejected_at_construction() {
        for (config, message) in [
            (
                RecognitionConfig {
                    maximum_distance: f32::NAN,
                    ..RecognitionConfig::default()
                },
                "maximum_distance",
            ),
            (
                RecognitionConfig {
                    maximum_path_length_ratio: 0.99,
                    ..RecognitionConfig::default()
                },
                "maximum_path_length_ratio",
            ),
            (
                RecognitionConfig {
                    minimum_margin: -0.01,
                    ..RecognitionConfig::default()
                },
                "minimum_margin",
            ),
        ] {
            let error = KanaRecognizer::with_config(config).unwrap_err();
            assert!(error.to_string().contains(message), "{error}");
        }
    }

    #[test]
    fn extreme_finite_coordinates_do_not_overflow_diagnostics() {
        let recognizer = KanaRecognizer::new().unwrap();
        let high = f32::MAX;
        let ink = CharacterInk::from_xy([vec![
            [high * 0.50, high * 0.50],
            [high * 0.75, high * 0.60],
            [high, high],
        ]]);

        let result = recognizer.recognize(&ink, 10);
        assert!(!result.candidates.is_empty());
        assert!(
            result
                .candidates
                .iter()
                .all(|candidate| candidate.distance.is_finite())
        );
        assert!(result.path_length_ratio.is_some_and(f32::is_finite));
    }
}
