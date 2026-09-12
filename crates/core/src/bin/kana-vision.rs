//! Replay saved raw ink through both classifiers, without opening a database.
use anyhow::{Context, Result};
use idiosepius_core::{
    kana::{CharacterInk, KanaRecognizer, KanaScript},
    kana_vision::{VisionModel, rasterize_at},
};
use std::{
    io::{self, Read},
    time::Instant,
};
fn main() -> Result<()> {
    let geometry = KanaRecognizer::new()?;
    let mut canonical = false;
    let mut rasters = false;
    let mut model_path = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--canonical" => canonical = true,
            "--rasters" => rasters = true,
            "--model" => {
                model_path = Some(args.next().context("--model needs a binary model path")?)
            }
            "--help" | "-h" => {
                println!(
                    "kana-vision [--model model.bin] [--canonical] [--rasters]\nOtherwise reads saved sample documents as a JSON array on stdin."
                );
                return Ok(());
            }
            _ => anyhow::bail!("unknown argument: {arg}"),
        }
    }
    let model = match model_path {
        Some(path) => VisionModel::from_bytes(
            &std::fs::read(&path).with_context(|| format!("reading {path}"))?,
        ),
        None => VisionModel::embedded(),
    }
    .map_err(anyhow::Error::msg)?;
    let documents: Vec<serde_json::Value> = if canonical {
        geometry.characters().map(|(c,s)|serde_json::json!({"expected":c,"script":s,"strokes":geometry.canonical(c)})).collect()
    } else {
        let mut json = String::new();
        io::stdin().read_to_string(&mut json)?;
        serde_json::from_str(&json).context("expected a JSON array of saved sample documents")?
    };
    let mut output = Vec::new();
    for doc in documents {
        let ink: CharacterInk = serde_json::from_value(doc["strokes"].clone())?;
        let scope = doc
            .get("script")
            .or_else(|| doc.get("recognizer")?.get("script_scope"))
            .context("each sample needs an explicit script or recognizer.script_scope")?;
        let script: KanaScript = serde_json::from_value(scope.clone())?;
        let start = Instant::now();
        let vision = model.recognize(&ink, script, 5);
        let elapsed = start.elapsed().as_secs_f64() * 1000.;
        let reference = vision
            .candidates
            .first()
            .and_then(|c| geometry.compare_reference(&ink, c.character));
        output.push(serde_json::json!({"expected":doc.get("expected"),"expected_invalid":doc.get("expected_invalid"),"script":script,"geometry":geometry.recognize_script(&ink,script,10),"vision":vision,"reference":reference,"vision_ms":elapsed,"raster":if rasters{rasterize_at(&ink, model.side()).ok()}else{None}}));
    }
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}
