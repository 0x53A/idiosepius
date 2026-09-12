//! Streaming training adapter: u16 LE width/height followed by white-ink u8 pixels.
//! Emits 32x32 or 64x64 little-endian f32 images. JSON ink and model verification are optional.
use anyhow::Context;
use idiosepius_core::kana_vision::{VisionModel, normalize_bitmap_at, rasterize_at};
use std::io::{self, Read, Write};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Bitmap,
    Ink,
    InkBatch,
    Logits,
    Help,
}

#[derive(Debug)]
struct Options {
    mode: Mode,
    side: usize,
    model: Option<String>,
}

fn options(args: impl IntoIterator<Item = String>) -> anyhow::Result<Options> {
    let mut args = args.into_iter();
    let mut mode = Mode::Bitmap;
    let mut side = None;
    let mut model = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--side" => {
                side = Some(
                    args.next()
                        .context("--side needs 32 or 64")?
                        .parse::<usize>()
                        .context("invalid side")?,
                )
            }
            "--model" => model = Some(args.next().context("--model needs a binary model path")?),
            "--ink" | "--ink-batch" | "--logits" => {
                anyhow::ensure!(mode == Mode::Bitmap, "choose only one input mode");
                mode = match arg.as_str() {
                    "--ink" => Mode::Ink,
                    "--ink-batch" => Mode::InkBatch,
                    _ => Mode::Logits,
                };
            }
            "--help" | "-h" => {
                return Ok(Options {
                    mode: Mode::Help,
                    side: 32,
                    model: None,
                });
            }
            _ => anyhow::bail!("unknown argument: {arg}"),
        }
    }
    anyhow::ensure!(
        side.is_none_or(|n| n == 32 || n == 64),
        "--side needs 32 or 64"
    );
    anyhow::ensure!(
        model.is_none() || mode == Mode::Logits,
        "--model requires --logits"
    );
    anyhow::ensure!(
        side.is_none() || mode != Mode::Logits,
        "--side applies to rasterization; logits use the model's input size"
    );
    Ok(Options {
        mode,
        side: side.unwrap_or(32),
        model,
    })
}

fn main() -> anyhow::Result<()> {
    let Options { mode, side, model } = options(std::env::args().skip(1))?;
    if mode == Mode::Help {
        println!(
            "kana-rasterize [--side 32|64] [--ink|--ink-batch]\nkana-rasterize --logits [--model model.bin]\nReads stdin; default input is u16 LE width/height followed by u8 white-ink pixels.\nRaster output is little-endian f32, except single --ink uses JSON. Logits use JSON batches."
        );
        return Ok(());
    }
    let mut input = Vec::new();
    io::stdin().read_to_end(&mut input)?;
    if mode == Mode::InkBatch {
        let batch: Vec<idiosepius_core::kana::CharacterInk> = serde_json::from_slice(&input)?;
        let mut output = io::BufWriter::new(io::stdout().lock());
        for ink in batch {
            let pixels = match rasterize_at(&ink, side) {
                Ok(pixels) => pixels,
                Err(error) if error == "insufficient ink" => vec![0.; side * side],
                Err(error) => return Err(anyhow::Error::msg(error)),
            };
            for value in pixels {
                output.write_all(&value.to_le_bytes())?;
            }
        }
    } else if mode == Mode::Logits {
        let model = match model {
            Some(path) => VisionModel::from_bytes(
                &std::fs::read(&path).with_context(|| format!("reading {path}"))?,
            ),
            None => VisionModel::embedded(),
        }
        .map_err(anyhow::Error::msg)?;
        let batch: Vec<Vec<f32>> = serde_json::from_slice(&input)?;
        let out: Result<Vec<_>, _> = batch.iter().map(|x| model.logits(x)).collect();
        println!(
            "{}",
            serde_json::to_string(&out.map_err(anyhow::Error::msg)?)?
        );
    } else if mode == Mode::Ink {
        let ink = serde_json::from_slice(&input)?;
        println!(
            "{}",
            serde_json::to_string(&rasterize_at(&ink, side).map_err(anyhow::Error::msg)?)?
        );
    } else {
        let mut output = io::BufWriter::new(io::stdout().lock());
        let mut offset = 0;
        while offset < input.len() {
            anyhow::ensure!(offset + 4 <= input.len(), "truncated dimensions");
            let width = u16::from_le_bytes(input[offset..offset + 2].try_into()?) as usize;
            let height = u16::from_le_bytes(input[offset + 2..offset + 4].try_into()?) as usize;
            offset += 4;
            anyhow::ensure!(offset + width * height <= input.len(), "truncated pixels");
            for value in
                normalize_bitmap_at(&input[offset..offset + width * height], width, height, side)
                    .map_err(anyhow::Error::msg)?
            {
                output.write_all(&value.to_le_bytes())?;
            }
            offset += width * height;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_modes_and_ignored_model_paths_are_rejected_before_reading_stdin() {
        for args in [
            vec!["--model", "candidate.bin"],
            vec!["--ink", "--logits"],
            vec!["--ink-batch", "--ink"],
            vec!["--side", "4096"],
            vec!["--side"],
            vec!["--unknown"],
            vec!["--logits", "--model"],
            vec!["--logits", "--side", "64"],
        ] {
            assert!(options(args.into_iter().map(str::to_owned)).is_err());
        }
        let parsed = options(["--model", "candidate.bin", "--logits"].map(str::to_owned)).unwrap();
        assert_eq!(parsed.mode, Mode::Logits);
        assert_eq!(parsed.model.as_deref(), Some("candidate.bin"));
        let parsed = options(["--ink-batch", "--side", "64"].map(str::to_owned)).unwrap();
        assert_eq!(parsed.side, 64);
        assert_eq!(parsed.mode, Mode::InkBatch);
    }
}
