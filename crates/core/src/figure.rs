//! Pure figure specifications and transfer-function plot geometry.
//!
//! This module deliberately knows nothing about egui. Content turns into
//! polylines, axes and ticks here; the app only maps that data into card-local
//! coordinates and paints it.

use serde::{Deserialize, Serialize};

const SAMPLES: usize = 401;
const EPS: f64 = 1.0e-12;

/// A figure authored inline in a question pack.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Figure {
    Bode {
        num: Vec<f64>,
        den: Vec<f64>,
        #[serde(default)]
        phase: bool,
    },
    Nyquist {
        num: Vec<f64>,
        den: Vec<f64>,
    },
    Step {
        num: Vec<f64>,
        den: Vec<f64>,
        t: [f64; 2],
    },
    Svg {
        src: String,
    },
}

impl Figure {
    pub fn kind_name(&self) -> &'static str {
        match self {
            Figure::Bode { .. } => "Bode plot",
            Figure::Nyquist { .. } => "Nyquist plot",
            Figure::Step { .. } => "step response",
            Figure::Svg { .. } => "diagram",
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        match self {
            Figure::Bode { num, den, .. } | Figure::Nyquist { num, den } => {
                validate_transfer_function(num, den)
            }
            Figure::Step { num, den, t } => {
                validate_transfer_function(num, den)?;
                if degree(num) > degree(den) {
                    return Err("step response requires a proper transfer function".into());
                }
                if !t.iter().all(|v| v.is_finite()) || t[0] < 0.0 || t[1] <= t[0] {
                    return Err(
                        "step time range must be finite and satisfy 0 <= t[0] < t[1]".into(),
                    );
                }
                Ok(())
            }
            Figure::Svg { src } => {
                if src.trim().is_empty() {
                    Err("SVG source is empty".into())
                } else {
                    Ok(())
                }
            }
        }
    }

    /// Produce vector plot data. SVG stays in the app's raster path.
    pub fn plot(&self) -> Result<Option<Plot>, String> {
        self.validate()?;
        match self {
            Figure::Bode { num, den, phase } => Ok(Some(bode(num, den, *phase))),
            Figure::Nyquist { num, den } => Ok(Some(nyquist(num, den))),
            Figure::Step { num, den, t } => Ok(Some(step(num, den, *t))),
            Figure::Svg { .. } => Ok(None),
        }
    }
}

fn validate_transfer_function(num: &[f64], den: &[f64]) -> Result<(), String> {
    if num.is_empty() || den.is_empty() {
        return Err("coefficient arrays must not be empty".into());
    }
    if !num.iter().chain(den).all(|v| v.is_finite()) {
        return Err("coefficients must be finite".into());
    }
    if den.iter().all(|v| v.abs() <= EPS) {
        return Err("denominator must not be identically zero".into());
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
pub struct Plot {
    pub panels: Vec<Panel>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Panel {
    pub x: Axis,
    pub y: Axis,
    pub lines: Vec<Polyline>,
    /// Bare LaTeX, passed to the app's math renderer.
    pub x_label: &'static str,
    /// Bare LaTeX, passed to the app's math renderer.
    pub y_label: &'static str,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Axis {
    pub min: f64,
    pub max: f64,
    pub ticks: Vec<Tick>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tick {
    pub value: f64,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Polyline {
    pub points: Vec<[f64; 2]>,
}

#[derive(Debug, Clone, Copy, Default)]
struct Complex {
    re: f64,
    im: f64,
}

impl Complex {
    fn new(re: f64, im: f64) -> Self {
        Self { re, im }
    }

    fn abs(self) -> f64 {
        self.re.hypot(self.im)
    }

    fn arg(self) -> f64 {
        self.im.atan2(self.re)
    }

    fn div(self, rhs: Self) -> Self {
        let d = rhs.re * rhs.re + rhs.im * rhs.im;
        Self::new(
            (self.re * rhs.re + self.im * rhs.im) / d,
            (self.im * rhs.re - self.re * rhs.im) / d,
        )
    }

    fn mul(self, rhs: Self) -> Self {
        Self::new(
            self.re * rhs.re - self.im * rhs.im,
            self.re * rhs.im + self.im * rhs.re,
        )
    }

    fn add_real(self, rhs: f64) -> Self {
        Self::new(self.re + rhs, self.im)
    }
}

/// Evaluate coefficients in descending-power order, as in MATLAB's `tf`.
fn horner(coeffs: &[f64], s: Complex) -> Complex {
    coeffs
        .iter()
        .fold(Complex::default(), |acc, &c| acc.mul(s).add_real(c))
}

fn response(num: &[f64], den: &[f64], omega: f64) -> Complex {
    horner(num, Complex::new(0.0, omega)).div(horner(den, Complex::new(0.0, omega)))
}

fn bode(num: &[f64], den: &[f64], phase: bool) -> Plot {
    let (lo, hi) = frequency_range(num, den);
    let mut magnitude = Vec::with_capacity(SAMPLES);
    let mut angles = Vec::with_capacity(SAMPLES);
    let mut previous = None;

    for i in 0..SAMPLES {
        let x = lerp(lo, hi, i as f64 / (SAMPLES - 1) as f64);
        let h = response(num, den, 10.0_f64.powf(x));
        let db = 20.0 * h.abs().max(1.0e-15).log10();
        magnitude.push([x, db.clamp(-300.0, 300.0)]);

        let mut deg = h.arg().to_degrees();
        if let Some(prev) = previous {
            while deg - prev > 180.0 {
                deg -= 360.0;
            }
            while deg - prev < -180.0 {
                deg += 360.0;
            }
        }
        previous = Some(deg);
        angles.push([x, deg]);
    }

    let x_axis = log_frequency_axis(lo, hi);
    let (mag_min, mag_max) = bounds(magnitude.iter().map(|p| p[1]), true, false);
    let mut panels = vec![Panel {
        x: x_axis.clone(),
        y: linear_axis(mag_min, mag_max),
        lines: vec![Polyline { points: magnitude }],
        x_label: r"\omega",
        y_label: r"|H|_{\mathrm{dB}}",
    }];

    if phase {
        let (phase_min, phase_max) = bounds(angles.iter().map(|p| p[1]), true, false);
        panels.push(Panel {
            x: x_axis,
            y: linear_axis(phase_min, phase_max),
            lines: vec![Polyline { points: angles }],
            x_label: r"\omega",
            y_label: r"\varphi\ [^\circ]",
        });
    }
    Plot { panels }
}

fn nyquist(num: &[f64], den: &[f64]) -> Plot {
    let (lo, hi) = frequency_range(num, den);
    let mut positive = Vec::with_capacity(SAMPLES);
    for i in 0..SAMPLES {
        let x = lerp(lo, hi, i as f64 / (SAMPLES - 1) as f64);
        let h = response(num, den, 10.0_f64.powf(x));
        if h.re.is_finite() && h.im.is_finite() && h.abs() < 1.0e8 {
            positive.push([h.re, h.im]);
        }
    }
    let mut points: Vec<[f64; 2]> = positive.iter().rev().map(|p| [p[0], -p[1]]).collect();
    points.extend(positive);

    let (x_min, x_max) = bounds(
        points.iter().map(|p| p[0]).chain(std::iter::once(0.0)),
        false,
        true,
    );
    let (y_min, y_max) = bounds(
        points.iter().map(|p| p[1]).chain(std::iter::once(0.0)),
        false,
        true,
    );
    Plot {
        panels: vec![Panel {
            x: linear_axis(x_min, x_max),
            y: linear_axis(y_min, y_max),
            lines: vec![Polyline { points }],
            x_label: r"\operatorname{Re}",
            y_label: r"\operatorname{Im}",
        }],
    }
}

fn step(num: &[f64], den: &[f64], t: [f64; 2]) -> Plot {
    let den = trim_leading_zeros(den);
    let num = trim_leading_zeros(num);
    let order = den.len().saturating_sub(1);
    let mut points = Vec::with_capacity(SAMPLES);

    if order == 0 {
        let gain = num.last().copied().unwrap_or(0.0) / den[0];
        points.extend((0..SAMPLES).map(|i| {
            let x = lerp(t[0], t[1], i as f64 / (SAMPLES - 1) as f64);
            [x, gain]
        }));
    } else {
        let lead = den[0];
        let a: Vec<f64> = den[1..].iter().map(|v| v / lead).collect();
        let mut b = vec![0.0; order + 1];
        let offset = b.len() - num.len();
        for (dst, src) in b[offset..].iter_mut().zip(num) {
            *dst = *src / lead;
        }
        let direct = b[0];
        let c: Vec<f64> = (0..order)
            .map(|i| b[order - i] - direct * a[order - 1 - i])
            .collect();

        // RK4 step size is independent of display sampling. This keeps
        // lightly damped and moderately stiff authored examples stable.
        let display_dt = (t[1] - t[0]) / (SAMPLES - 1) as f64;
        let dt = (display_dt / 8.0).min(t[1] / 4000.0).max(1.0e-6);
        let mut state = vec![0.0; order];
        let mut time = 0.0;
        for i in 0..SAMPLES {
            let target = lerp(t[0], t[1], i as f64 / (SAMPLES - 1) as f64);
            while time + EPS < target {
                let h = dt.min(target - time);
                rk4(&mut state, h, &a);
                time += h;
            }
            let y = dot(&c, &state) + direct;
            points.push([target, y]);
        }
    }

    let (y_min, y_max) = bounds(
        points.iter().map(|p| p[1]).chain(std::iter::once(0.0)),
        false,
        true,
    );
    Plot {
        panels: vec![Panel {
            x: linear_axis(t[0], t[1]),
            y: linear_axis(y_min, y_max),
            lines: vec![Polyline { points }],
            x_label: "t",
            y_label: "y(t)",
        }],
    }
}

fn derivative(state: &[f64], a: &[f64]) -> Vec<f64> {
    let n = state.len();
    let mut out = vec![0.0; n];
    if n > 1 {
        out[..n - 1].copy_from_slice(&state[1..]);
    }
    out[n - 1] = 1.0
        - state
            .iter()
            .zip(a.iter().rev())
            .map(|(x, coefficient)| x * coefficient)
            .sum::<f64>();
    out
}

fn rk4(state: &mut [f64], dt: f64, a: &[f64]) {
    let k1 = derivative(state, a);
    let at = |k: &[f64], scale: f64| {
        state
            .iter()
            .zip(k)
            .map(|(x, dx)| x + scale * dx)
            .collect::<Vec<_>>()
    };
    let k2 = derivative(&at(&k1, dt / 2.0), a);
    let k3 = derivative(&at(&k2, dt / 2.0), a);
    let k4 = derivative(&at(&k3, dt), a);
    for i in 0..state.len() {
        state[i] += dt * (k1[i] + 2.0 * k2[i] + 2.0 * k3[i] + k4[i]) / 6.0;
    }
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn trim_leading_zeros(values: &[f64]) -> &[f64] {
    let first = values
        .iter()
        .position(|v| v.abs() > EPS)
        .unwrap_or(values.len().saturating_sub(1));
    &values[first..]
}

fn degree(values: &[f64]) -> usize {
    trim_leading_zeros(values).len().saturating_sub(1)
}

/// Estimate interesting corner frequencies from coefficient ratios. It needs
/// no root solver, but still finds the useful decades for ordinary authored
/// polynomials, including integrators.
fn frequency_range(num: &[f64], den: &[f64]) -> (f64, f64) {
    let mut scales = Vec::new();
    for coeffs in [num, den] {
        for i in 0..coeffs.len() {
            for j in i + 1..coeffs.len() {
                let a = coeffs[i].abs();
                let b = coeffs[j].abs();
                if a > EPS && b > EPS {
                    let scale = (b / a).powf(1.0 / (j - i) as f64);
                    if scale.is_finite() && scale > EPS {
                        scales.push(scale);
                    }
                }
            }
        }
    }
    if scales.is_empty() {
        return (-2.0, 2.0);
    }
    let smallest = scales.iter().copied().fold(f64::INFINITY, f64::min);
    let largest = scales.iter().copied().fold(0.0, f64::max);
    let lo = (smallest.log10().floor() - 2.0).clamp(-8.0, 6.0);
    let hi = (largest.log10().ceil() + 2.0).clamp(-6.0, 8.0);
    if hi > lo { (lo, hi) } else { (-2.0, 2.0) }
}

fn log_frequency_axis(lo: f64, hi: f64) -> Axis {
    let ticks = (lo.ceil() as i32..=hi.floor() as i32)
        .map(|power| Tick {
            value: power as f64,
            label: if power == 0 {
                "1".into()
            } else {
                format!("10^{power}")
            },
        })
        .collect();
    Axis {
        min: lo,
        max: hi,
        ticks,
    }
}

fn linear_axis(min: f64, max: f64) -> Axis {
    let span = (max - min).max(EPS);
    let raw = span / 4.0;
    let power = 10.0_f64.powf(raw.log10().floor());
    let fraction = raw / power;
    let step = if fraction <= 1.0 {
        power
    } else if fraction <= 2.0 {
        2.0 * power
    } else if fraction <= 5.0 {
        5.0 * power
    } else {
        10.0 * power
    };
    let start = (min / step).ceil() as i64;
    let end = (max / step).floor() as i64;
    let ticks = (start..=end)
        .map(|i| {
            let value = i as f64 * step;
            Tick {
                value,
                label: format_tick(value, step),
            }
        })
        .collect();
    Axis { min, max, ticks }
}

fn format_tick(value: f64, step: f64) -> String {
    if value.abs() < step * 1.0e-8 {
        return "0".into();
    }
    let decimals = (-step.abs().log10().floor() as i32).clamp(0, 4) as usize;
    format!("{value:.decimals$}")
}

fn bounds(values: impl Iterator<Item = f64>, snap: bool, include_padding: bool) -> (f64, f64) {
    let (mut min, mut max) = values
        .filter(|v| v.is_finite())
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
            (lo.min(v), hi.max(v))
        });
    if !min.is_finite() || !max.is_finite() {
        return (-1.0, 1.0);
    }
    if (max - min).abs() < EPS {
        let pad = max.abs().max(1.0) * 0.2;
        min -= pad;
        max += pad;
    } else {
        let pad = (max - min) * if include_padding { 0.08 } else { 0.05 };
        min -= pad;
        max += pad;
    }
    if snap {
        min = (min / 10.0).floor() * 10.0;
        max = (max / 10.0).ceil() * 10.0;
    }
    (min, max)
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integrator_has_minus_twenty_db_per_decade() {
        let figure = Figure::Bode {
            num: vec![1.0],
            den: vec![1.0, 0.0],
            phase: false,
        };
        let plot = figure.plot().unwrap().unwrap();
        let points = &plot.panels[0].lines[0].points;
        let at = |x: f64| {
            points
                .iter()
                .min_by(|a, b| (a[0] - x).abs().total_cmp(&(b[0] - x).abs()))
                .unwrap()[1]
        };
        assert!(((at(1.0) - at(0.0)) + 20.0).abs() < 0.2);
    }

    #[test]
    fn first_order_step_reaches_its_dc_gain() {
        let figure = Figure::Step {
            num: vec![1.0],
            den: vec![1.0, 1.0],
            t: [0.0, 8.0],
        };
        let plot = figure.plot().unwrap().unwrap();
        let points = &plot.panels[0].lines[0].points;
        assert!(points[0][1].abs() < 1.0e-9);
        assert!((points.last().unwrap()[1] - 1.0).abs() < 0.001);
    }

    #[test]
    fn bad_coefficients_are_rejected() {
        let bad = Figure::Nyquist {
            num: vec![1.0],
            den: vec![0.0, 0.0],
        };
        assert!(bad.validate().unwrap_err().contains("identically zero"));
    }

    #[test]
    fn authoring_examples_deserialize_exactly() {
        let bode: Figure =
            serde_json::from_str(r#"{"kind":"bode","num":[1],"den":[1,10,0],"phase":true}"#)
                .unwrap();
        assert!(matches!(bode, Figure::Bode { phase: true, .. }));

        let svg: Figure =
            serde_json::from_str(r#"{"kind":"svg","src":"<svg viewBox='0 0 1 1'/>"}"#).unwrap();
        assert!(matches!(svg, Figure::Svg { .. }));
    }
}
