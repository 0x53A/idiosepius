//! Experimental bitmap identity classifier. Scores are not calibrated probabilities
//! or handwriting-quality grades. The fixed network runs identically on CPU/Wasm.
use crate::kana::{CharacterInk, KanaScript};
use serde::{Deserialize, Serialize};

pub const SIDE: usize = 32;
pub const RASTERIZER_ID: &str = "kana-bitmap-32-v1";
pub const ARCHITECTURE: &str = "conv8x3-pool-conv16x3-pool-linear92-v1";
fn parameter_count(side: usize, depth: usize, width: usize) -> usize {
    let mut count = 0;
    let mut input = 1;
    for output in [8 * width, 16 * width, 32 * width].into_iter().take(depth) {
        count += output * (input * 9 + 1);
        input = output;
    }
    count + 92 * input * (side / (1 << depth)).pow(2) + 92
}
fn residual_parameter_count(depth: usize, width: usize) -> usize {
    [8 * width, 16 * width, 32 * width]
        .into_iter()
        .take(depth)
        .map(|c| 2 * c * (c * 9 + 1))
        .sum()
}
fn rasterizer_id(side: usize) -> &'static str {
    if side == 32 {
        RASTERIZER_ID
    } else {
        "kana-bitmap-64-v1"
    }
}
const LABELS: &str = "あいうえおかきくけこさしすせそたちつてとなにぬねのはひふへほまみむめもやゆよらりるれろわをんアイウエオカキクケコサシスセソタチツテトナニヌネノハヒフヘホマミムメモヤユヨラリルレロワヲン";

/// Normalize white ink on black, preserving aspect ratio and centering its bounds.
/// The threshold determines the crop only; interpolation preserves grayscale.
pub fn normalize_bitmap(pixels: &[u8], width: usize, height: usize) -> Result<Vec<f32>, String> {
    normalize_bitmap_at(pixels, width, height, SIDE)
}

pub fn normalize_bitmap_at(
    pixels: &[u8],
    width: usize,
    height: usize,
    side: usize,
) -> Result<Vec<f32>, String> {
    if ![32, 64].contains(&side) {
        return Err("unsupported raster size".into());
    }

    if width == 0 || height == 0 || width > 2048 || height > 2048 || pixels.len() != width * height
    {
        return Err("invalid bitmap dimensions".into());
    }
    let mut bounds = (width, height, 0, 0);
    for (i, &value) in pixels.iter().enumerate() {
        if value >= 64 {
            bounds.0 = bounds.0.min(i % width);
            bounds.1 = bounds.1.min(i / width);
            bounds.2 = bounds.2.max(i % width);
            bounds.3 = bounds.3.max(i / width);
        }
    }
    let mut out = vec![0.; side * side];
    if bounds.0 == width {
        return Ok(out);
    }
    let span = (bounds.2 - bounds.0 + 1).max(bounds.3 - bounds.1 + 1) as f32;
    let cx = (bounds.0 + bounds.2) as f32 / 2.;
    let cy = (bounds.1 + bounds.3) as f32 / 2.;
    let get = |x: isize, y: isize| {
        if x >= 0 && y >= 0 && x < width as isize && y < height as isize {
            pixels[y as usize * width + x as usize] as f32 / 255.
        } else {
            0.
        }
    };
    for y in 0..side {
        for x in 0..side {
            let sx = (x as f32 - (side as f32 - 1.) / 2.) * span / (side as f32 * 0.75) + cx;
            let sy = (y as f32 - (side as f32 - 1.) / 2.) * span / (side as f32 * 0.75) + cy;
            let ix = sx.floor() as isize;
            let iy = sy.floor() as isize;
            let dx = sx - ix as f32;
            let dy = sy - iy as f32;
            out[y * side + x] = (get(ix, iy) * (1. - dx) + get(ix + 1, iy) * dx) * (1. - dy)
                + (get(ix, iy + 1) * (1. - dx) + get(ix + 1, iy + 1) * dx) * dy;
        }
    }
    Ok(out)
}

/// Union of antialiased capsules: independent of stroke order and direction.
/// Pressure and timing remain in raw ink and do not affect identity rasterization.
pub fn rasterize(ink: &CharacterInk) -> Result<Vec<f32>, String> {
    rasterize_at(ink, SIDE)
}

pub fn rasterize_at(ink: &CharacterInk, side: usize) -> Result<Vec<f32>, String> {
    if ![32, 64].contains(&side) {
        return Err("unsupported raster size".into());
    }
    let center = (side as f64 - 1.) / 2.;
    let ink_span = side as f64 * 0.75 - 1.;
    let radius = 1.25 * side as f32 / 32.;

    let count: usize = ink.0.iter().map(Vec::len).sum();
    if count > 16384 || ink.0.len() > 16384 {
        return Err("input limit".into());
    }
    let mut bounds = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    for p in ink.0.iter().flatten() {
        if !p.x.is_finite() || !p.y.is_finite() {
            return Err("non-finite coordinate".into());
        }
        bounds[0] = bounds[0].min(p.x);
        bounds[1] = bounds[1].min(p.y);
        bounds[2] = bounds[2].max(p.x);
        bounds[3] = bounds[3].max(p.y);
    }
    let span = (bounds[2] as f64 - bounds[0] as f64).max(bounds[3] as f64 - bounds[1] as f64);
    if count < 2 || span < 1e-6 {
        return Err("insufficient ink".into());
    }
    let cx = (bounds[0] as f64 + bounds[2] as f64) / 2.;
    let cy = (bounds[1] as f64 + bounds[3] as f64) / 2.;
    let point = |p: &crate::kana::InkPoint| {
        [
            ((p.x as f64 - cx) * ink_span / span + center) as f32,
            ((p.y as f64 - cy) * ink_span / span + center) as f32,
        ]
    };
    let mut out = vec![0f32; side * side];
    for stroke in &ink.0 {
        for i in 0..stroke.len() {
            let a = point(&stroke[i]);
            let b = point(&stroke[(i + 1).min(stroke.len() - 1)]);
            let dx = b[0] - a[0];
            let dy = b[1] - a[1];
            let length = dx * dx + dy * dy;
            let x0 = (a[0].min(b[0]) - radius).floor().max(0.) as usize;
            let x1 = (a[0].max(b[0]) + radius).ceil().min(side as f32 - 1.) as usize;
            let y0 = (a[1].min(b[1]) - radius).floor().max(0.) as usize;
            let y1 = (a[1].max(b[1]) + radius).ceil().min(side as f32 - 1.) as usize;
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let t = if length > 0. {
                        (((x as f32 - a[0]) * dx + (y as f32 - a[1]) * dy) / length).clamp(0., 1.)
                    } else {
                        0.
                    };
                    let distance = ((x as f32 - a[0] - t * dx).powi(2)
                        + (y as f32 - a[1] - t * dy).powi(2))
                    .sqrt();
                    out[y * side + x] = out[y * side + x].max((radius - distance).clamp(0., 1.));
                }
            }
        }
    }
    Ok(out)
}

#[derive(Deserialize)]
struct ModelFile {
    architecture: String,
    rasterizer: String,
    labels: String,
    weights: Vec<f32>,
}

pub struct VisionModel {
    side: usize,
    depth: usize,
    width: usize,
    residual: bool,
    labels: Vec<char>,
    weights: Vec<f32>,
    fingerprint: String,
}
#[derive(Debug, Serialize)]
pub struct VisionCandidate {
    pub character: char,
    pub score: f32,
}
#[derive(Debug, Serialize)]
pub struct VisionResult {
    pub model_fingerprint: String,
    pub rasterizer: &'static str,
    pub experimental: bool,
    pub candidates: Vec<VisionCandidate>,
    pub error: Option<String>,
}
impl VisionModel {
    /// Debug/training export reader. The deployed artifact uses fixed-width f32 bytes.
    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > 64_000_000 {
            return Err("model too large".into());
        }
        let file: ModelFile = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        let residual = file.architecture.starts_with("kana-residual-");
        let (side, depth, width) = match file.architecture.as_str() {
            ARCHITECTURE => (32, 2, 1),
            "conv8x3-pool-conv16x3-pool-linear92-64-v1" => (64, 2, 1),
            "conv8x3-pool-conv16x3-pool-conv32x3-pool-linear92-32-v1" => (32, 3, 1),
            "conv8x3-pool-conv16x3-pool-conv32x3-pool-linear92-64-v1" => (64, 3, 1),
            name => {
                let mut found = None;
                for side in [32, 64] {
                    for depth in [2, 3] {
                        for width in [1, 2, 4] {
                            let family = if residual { "residual" } else { "conv" };
                            if name == format!("kana-{family}-depth{depth}-width{width}-{side}-v1")
                            {
                                found = Some((side, depth, width));
                            }
                        }
                    }
                }
                found.ok_or("unsupported model architecture")?
            }
        };
        if file.rasterizer != rasterizer_id(side) {
            return Err("unsupported model rasterizer".into());
        }
        Self::from_parts(
            file.labels,
            file.weights,
            side,
            depth,
            width,
            residual,
            bytes,
        )
    }
    /// IDKANA01 (two convolutions) or IDKANA02 (three), u32 LE side,
    /// 92 u32 LE Unicode labels, then f32 LE parameters in layer order.
    /// IDKANA03 adds u32 LE depth and width after side. Only bounded, supported
    /// IDKANA04 uses the same descriptor, with two residual convolutions per
    /// pooled stage stored after the plain network parameters (including linear).
    /// Only bounded shapes are accepted; dimensions never come from weights.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < 12 {
            return Err("unsupported model header".into());
        }
        let residual = &bytes[..8] == b"IDKANA04";
        let (depth, width, header) = match &bytes[..8] {
            b"IDKANA01" => (2, 1, 12),
            b"IDKANA02" => (3, 1, 12),
            b"IDKANA03" | b"IDKANA04" if bytes.len() >= 20 => (
                u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize,
                u32::from_le_bytes(bytes[16..20].try_into().unwrap()) as usize,
                20,
            ),
            _ => return Err("unsupported model header".into()),
        };
        let side = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        if ![32, 64].contains(&side)
            || ![2, 3].contains(&depth)
            || ![1, 2, 4].contains(&width)
            || bytes.len()
                != header
                    + 92 * 4
                    + (parameter_count(side, depth, width)
                        + if residual {
                            residual_parameter_count(depth, width)
                        } else {
                            0
                        })
                        * 4
        {
            return Err("invalid model size".into());
        }
        let mut labels = String::new();
        for word in bytes[header..header + 368].chunks_exact(4) {
            labels.push(
                char::from_u32(u32::from_le_bytes(word.try_into().unwrap()))
                    .ok_or("invalid model label")?,
            );
        }
        let weights = bytes[header + 368..]
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        Self::from_parts(labels, weights, side, depth, width, residual, bytes)
    }
    fn from_parts(
        labels: String,
        weights: Vec<f32>,
        side: usize,
        depth: usize,
        width: usize,
        residual: bool,
        bytes: &[u8],
    ) -> Result<Self, String> {
        if labels != LABELS
            || weights.len()
                != parameter_count(side, depth, width)
                    + if residual {
                        residual_parameter_count(depth, width)
                    } else {
                        0
                    }
            || weights.iter().any(|v| !v.is_finite() || v.abs() > 100.)
        {
            return Err("unsupported or invalid vision model".into());
        }
        let hash = bytes.iter().fold(0xcbf29ce484222325u64, |h, b| {
            (h ^ *b as u64).wrapping_mul(0x100000001b3)
        });
        Ok(Self {
            side,
            depth,
            width,
            residual,
            labels: labels.chars().collect(),
            weights,
            fingerprint: format!("{hash:016x}"),
        })
    }
    pub fn embedded() -> Result<Self, String> {
        Self::from_bytes(include_bytes!("../assets/kana_vision.bin"))
    }
    pub fn side(&self) -> usize {
        self.side
    }
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
    pub fn logits(&self, input: &[f32]) -> Result<Vec<f32>, String> {
        if input.len() != self.side * self.side
            || input
                .iter()
                .any(|v| !v.is_finite() || !(0. ..=1.).contains(v))
        {
            return Err("invalid normalized bitmap".into());
        }
        let mut features = input.to_vec();
        let mut offset = 0;
        let mut channels = 1;
        let mut residual_offset = parameter_count(self.side, self.depth, self.width);
        for (layer, output) in [8 * self.width, 16 * self.width, 32 * self.width]
            .into_iter()
            .take(self.depth)
            .enumerate()
        {
            let end = offset + output * (channels * 9 + 1);
            features = conv_pool(
                &features,
                channels,
                output,
                self.side >> layer,
                &self.weights[offset..end],
            );
            if self.residual {
                let n = self.side >> (layer + 1);
                let count = output * (output * 9 + 1);
                let mut hidden = conv_same(
                    &features,
                    output,
                    n,
                    &self.weights[residual_offset..residual_offset + count],
                );
                hidden.iter_mut().for_each(|v| *v = v.max(0.));
                residual_offset += count;
                let branch = conv_same(
                    &hidden,
                    output,
                    n,
                    &self.weights[residual_offset..residual_offset + count],
                );
                for (value, residual) in features.iter_mut().zip(branch) {
                    *value = (*value + residual).max(0.);
                }
                residual_offset += count;
            }
            channels = output;
            offset = end;
        }
        let weights = &self.weights[offset..];
        let n = features.len();
        Ok((0..92)
            .map(|c| {
                features
                    .iter()
                    .zip(&weights[c * n..(c + 1) * n])
                    .fold(weights[92 * n + c], |a, (x, w)| a + x * w)
            })
            .collect())
    }
    pub fn recognize(&self, ink: &CharacterInk, script: KanaScript, limit: usize) -> VisionResult {
        let mut result = VisionResult {
            model_fingerprint: self.fingerprint.clone(),
            rasterizer: rasterizer_id(self.side),
            experimental: true,
            candidates: Vec::new(),
            error: None,
        };
        match rasterize_at(ink, self.side).and_then(|x| self.logits(&x)) {
            Err(e) => result.error = Some(e),
            Ok(logits) => {
                let offset = if script == KanaScript::Hiragana {
                    0
                } else {
                    46
                };
                let max = logits[offset..offset + 46]
                    .iter()
                    .copied()
                    .fold(f32::NEG_INFINITY, f32::max);
                let scores: Vec<_> = logits[offset..offset + 46]
                    .iter()
                    .map(|x| (x - max).exp())
                    .collect();
                let total: f32 = scores.iter().sum();
                result.candidates = (0..46)
                    .map(|i| VisionCandidate {
                        character: self.labels[offset + i],
                        score: scores[i] / total,
                    })
                    .collect();
                result
                    .candidates
                    .sort_by(|a, b| b.score.total_cmp(&a.score));
                result.candidates.truncate(limit.min(46));
            }
        }
        result
    }
}
fn conv_pool(input: &[f32], ci: usize, co: usize, side: usize, weights: &[f32]) -> Vec<f32> {
    let n = side / 2;
    let mut out = vec![0f32; co * n * n];
    for oc in 0..co {
        for y in 0..side {
            for x in 0..side {
                let mut sum = weights[co * ci * 9 + oc];
                for ic in 0..ci {
                    for ky in 0..3 {
                        for kx in 0..3 {
                            let sy = y as isize + ky as isize - 1;
                            let sx = x as isize + kx as isize - 1;
                            if sy >= 0 && sx >= 0 && sy < side as isize && sx < side as isize {
                                sum += input[ic * side * side + sy as usize * side + sx as usize]
                                    * weights[((oc * ci + ic) * 3 + ky) * 3 + kx];
                            }
                        }
                    }
                }
                let target = &mut out[oc * n * n + (y / 2) * n + x / 2];
                *target = target.max(sum);
            }
        }
    }
    out
}

// Same zero-padded 3x3 cross-correlation as PyTorch, with no activation or pooling.
fn conv_same(input: &[f32], channels: usize, side: usize, weights: &[f32]) -> Vec<f32> {
    let mut out = vec![0.; input.len()];
    for oc in 0..channels {
        for y in 0..side {
            for x in 0..side {
                let mut sum = weights[channels * channels * 9 + oc];
                for ic in 0..channels {
                    for ky in 0..3 {
                        for kx in 0..3 {
                            let sy = y as isize + ky as isize - 1;
                            let sx = x as isize + kx as isize - 1;
                            if sy >= 0 && sx >= 0 && sy < side as isize && sx < side as isize {
                                sum += input[ic * side * side + sy as usize * side + sx as usize]
                                    * weights[((oc * channels + ic) * 3 + ky) * 3 + kx];
                            }
                        }
                    }
                }
                out[oc * side * side + y * side + x] = sum;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn embedded_model_matches_pytorch_on_a_synthetic_bitmap() {
        let model = VisionModel::embedded().unwrap();
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../assets/kana_vision_golden.json")).unwrap();
        let input: Vec<f32> = (0..model.side() * model.side())
            .map(|i| ((i * 37 + 11) % 101) as f32 / 100.)
            .collect();
        let actual = model.logits(&input).unwrap();
        assert_eq!(fixture["logits"].as_array().unwrap().len(), 92);
        assert_eq!(actual.len(), 92);
        for (value, expected) in actual.iter().zip(fixture["logits"].as_array().unwrap()) {
            assert!((*value as f64 - expected.as_f64().unwrap()).abs() < 0.0002);
        }
        assert!(model.logits(&vec![f32::NAN; 1024]).is_err());
    }

    #[test]
    fn every_supported_network_shape_matches_independent_pytorch_arithmetic() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../assets/kana_vision_arithmetic.json")).unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let side = case["side"].as_u64().unwrap() as usize;
            let depth = case["depth"].as_u64().unwrap() as usize;
            let width = case["width"].as_u64().unwrap_or(1) as usize;
            let residual = case["residual"].as_bool().unwrap_or(false);
            let mut bytes = if residual {
                b"IDKANA04".to_vec()
            } else if width != 1 {
                b"IDKANA03".to_vec()
            } else if depth == 3 {
                b"IDKANA02".to_vec()
            } else {
                b"IDKANA01".to_vec()
            };
            bytes.extend_from_slice(&(side as u32).to_le_bytes());
            if width != 1 || residual {
                bytes.extend_from_slice(&(depth as u32).to_le_bytes());
                bytes.extend_from_slice(&(width as u32).to_le_bytes());
            }
            for c in LABELS.chars() {
                bytes.extend_from_slice(&(c as u32).to_le_bytes());
            }
            let count = parameter_count(side, depth, width)
                + if residual {
                    residual_parameter_count(depth, width)
                } else {
                    0
                };
            assert_eq!(count, case["parameters"].as_u64().unwrap() as usize);
            for i in 0..count {
                let value = (((i * 17 + 13) % 37) as f32 - 18.) / 80.;
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            let model = VisionModel::from_bytes(&bytes).unwrap();
            let input: Vec<f32> = (0..side * side)
                .map(|i| ((i * 37 + 11) % 101) as f32 / 100.)
                .collect();
            let actual = model.logits(&input).unwrap();
            for (i, (value, expected)) in actual
                .iter()
                .zip(case["logits"].as_array().unwrap())
                .enumerate()
            {
                // Residual synthetic activations can exceed 250; scalar accumulation
                // and PyTorch's vectorized kernels require an infinity-norm relative
                // bound, including cancellation in individual output logits.
                // Real candidate exports retain the stricter 2e-4 absolute check.
                let scale = case["logits"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_f64().unwrap().abs())
                    .fold(0f64, f64::max);
                let tolerance = 0.0002 + if residual { scale * 0.00005 } else { 0. };
                assert!(
                    (*value as f64 - expected.as_f64().unwrap()).abs() < tolerance,
                    "{side}px depth {depth} width {width} residual {residual} logit {i}: {value} vs {expected}"
                );
            }
            if side == 32 && depth == 3 && width == 2 {
                let json = serde_json::to_vec(&serde_json::json!({
                    "architecture": if residual { "kana-residual-depth3-width2-32-v1" } else { "kana-conv-depth3-width2-32-v1" },
                    "rasterizer": RASTERIZER_ID,
                    "labels": LABELS,
                    "weights": model.weights,
                }))
                .unwrap();
                let parsed = VisionModel::from_json(&json).unwrap();
                assert_eq!(parsed.logits(&input).unwrap(), actual);
                for (offset, value) in [(12, 0u32), (12, u32::MAX), (16, 3), (16, u32::MAX)] {
                    let mut invalid = bytes.clone();
                    invalid[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
                    assert!(VisionModel::from_bytes(&invalid).is_err());
                }
            }
            bytes[7] = if depth == 3 { b'1' } else { b'2' };
            assert!(
                VisionModel::from_bytes(&bytes).is_err(),
                "header must agree with tensor shapes"
            );
        }
    }

    #[test]
    fn model_shape_and_label_order_are_checked_before_inference() {
        let mut bytes = include_bytes!("../assets/kana_vision.bin").to_vec();
        let labels_offset = if matches!(&bytes[..8], b"IDKANA03" | b"IDKANA04") {
            20
        } else {
            12
        };
        bytes[labels_offset..labels_offset + 4].copy_from_slice(&0xffffffffu32.to_le_bytes());
        assert!(VisionModel::from_bytes(&bytes).is_err());
        assert!(VisionModel::from_bytes(&bytes[..20]).is_err());
    }

    #[test]
    fn raster_is_order_and_direction_independent() {
        let a =
            CharacterInk::from_xy([vec![[0., 0.], [2., 1.], [4., 5.]], vec![[1., 4.], [5., 4.]]]);
        let mut b = a.clone();
        b.0.reverse();
        for s in &mut b.0 {
            s.reverse();
        }
        let x = rasterize(&a).unwrap();
        let y = rasterize(&b).unwrap();
        assert!(x.iter().zip(y).all(|(x, y)| (x - y).abs() < 1e-5));
        for p in b.0.iter_mut().flatten() {
            p.x = p.x * 3. + 12.;
            p.y = p.y * 3. - 42.;
        }
        assert!(
            x.iter()
                .zip(rasterize(&b).unwrap())
                .all(|(x, y)| (x - y).abs() < 1e-5)
        );
    }
    #[test]
    fn both_raster_sizes_preserve_aspect_and_ignore_pointer_metadata() {
        let a =
            CharacterInk::from_xy([vec![[0., 0.], [2., 1.], [4., 5.]], vec![[1., 4.], [5., 4.]]]);
        let mut b = a.clone();
        b.0.reverse();
        for stroke in &mut b.0 {
            stroke.reverse();
            for p in stroke {
                p.pressure = Some(0.99);
                p.time_ms = Some(100.);
            }
        }
        for side in [32, 64] {
            let x = rasterize_at(&a, side).unwrap();
            let y = rasterize_at(&b, side).unwrap();
            assert!(x.iter().zip(y).all(|(x, y)| (x - y).abs() < 0.00001));
            let bitmap =
                normalize_bitmap_at(&[255, 255, 255, 255, 255, 255, 255, 255], 8, 1, side).unwrap();
            let occupied: Vec<_> = bitmap
                .iter()
                .enumerate()
                .filter(|(_, v)| **v > 0.5)
                .map(|(i, _)| (i % side, i / side))
                .collect();
            let width = occupied.iter().map(|p| p.0).max().unwrap()
                - occupied.iter().map(|p| p.0).min().unwrap();
            let height = occupied.iter().map(|p| p.1).max().unwrap()
                - occupied.iter().map(|p| p.1).min().unwrap();
            assert!(
                width > height * 4,
                "normalization must not stretch a line into a square"
            );
        }
        assert!(rasterize_at(&a, usize::MAX).is_err());
        assert!(normalize_bitmap_at(&[255], 1, 1, usize::MAX).is_err());
    }

    #[test]
    fn malformed_and_empty_inputs_are_bounded() {
        assert!(rasterize(&CharacterInk::default()).is_err());
        assert!(normalize_bitmap(&[0], 2048, 2048).is_err());
        assert!(VisionModel::from_json(b"{}").is_err());
        assert!(rasterize(&CharacterInk::from_xy([vec![[f32::NAN, 0.], [1., 1.]]])).is_err());
    }
}
