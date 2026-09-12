//! Small worker-facing API; the study database is never opened by this route.

use idiosepius_core::kana::{
    ALGORITHM_ID, CharacterInk, KanaRecognizer, KanaScript, VARIANT_MAXIMUM_DISTANCE,
    VARIANT_MINIMUM_MARGIN,
};
use std::io::{Cursor, Write};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct KanaEngine(KanaRecognizer, idiosepius_core::kana_vision::VisionModel);

fn js_error(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}

#[wasm_bindgen]
impl KanaEngine {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Result<KanaEngine, JsValue> {
        Ok(Self(
            KanaRecognizer::new().map_err(js_error)?,
            idiosepius_core::kana_vision::VisionModel::embedded().map_err(js_error)?,
        ))
    }

    /// Candidate evaluation without replacing the app's embedded model.
    pub fn with_model(bytes: &[u8]) -> Result<KanaEngine, JsValue> {
        Ok(Self(
            KanaRecognizer::new().map_err(js_error)?,
            idiosepius_core::kana_vision::VisionModel::from_bytes(bytes).map_err(js_error)?,
        ))
    }

    pub fn inventory(&self, script: &str) -> Result<String, JsValue> {
        let scope = scope(script)?;
        Ok(self
            .0
            .characters()
            .filter_map(|(c, s)| (s == scope).then_some(c))
            .collect())
    }

    pub fn metadata(&self, script: &str) -> Result<String, JsValue> {
        scope(script)?;
        let config = self.0.config();
        Ok(serde_json::json!({
            "algorithm": ALGORITHM_ID,
            "template_count": self.0.template_count(),
            "template_fingerprint": format!("{:016x}", self.0.template_fingerprint()),
            "result_limit": 10,
            "script_scope": script,
            "maximum_distance": config.maximum_distance,
            "maximum_path_length_ratio": config.maximum_path_length_ratio,
            "minimum_margin": config.minimum_margin,
            "variant_maximum_distance": VARIANT_MAXIMUM_DISTANCE,
            "variant_minimum_margin": VARIANT_MINIMUM_MARGIN,
        })
        .to_string())
    }

    pub fn recognize_pair(&self, json: &str, script: &str) -> Result<String, JsValue> {
        let ink: CharacterInk = serde_json::from_str(json).map_err(js_error)?;
        let script = scope(script)?;
        let vision = self.1.recognize(&ink, script, 5);
        let reference = vision
            .candidates
            .first()
            .and_then(|candidate| self.0.compare_reference(&ink, candidate.character));
        Ok(serde_json::json!({
            "geometry": self.0.recognize_script(&ink, script, 10),
            "vision": vision, "reference": reference,
        })
        .to_string())
    }

    pub fn recognize(&self, json: &str, script: &str) -> Result<String, JsValue> {
        let ink: CharacterInk = serde_json::from_str(json).map_err(js_error)?;
        serde_json::to_string(&self.0.recognize_script(&ink, scope(script)?, 10)).map_err(js_error)
    }
}

fn scope(script: &str) -> Result<KanaScript, JsValue> {
    match script {
        "hiragana" => Ok(KanaScript::Hiragana),
        "katakana" => Ok(KanaScript::Katakana),
        _ => Err(js_error("select hiragana or katakana")),
    }
}

/// Pack the browser's saved sample documents into the CLI's ordinary corpus layout.
#[wasm_bindgen]
pub fn kana_sample_zip(json: &str) -> Result<Vec<u8>, JsValue> {
    // Keep the shell's serialized documents byte-for-byte. Parsing floating
    // coordinates into Value and serializing again can change their last bits.
    let samples: Vec<String> = serde_json::from_str(json).map_err(js_error)?;
    let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (index, sample) in samples.iter().enumerate() {
        archive
            .start_file(
                format!("browser/samples/{index:06}.json"),
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated),
            )
            .map_err(js_error)?;
        archive.write_all(sample.as_bytes()).map_err(js_error)?;
    }
    archive
        .finish()
        .map(|buffer| buffer.into_inner())
        .map_err(js_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn archive_preserves_document_bytes_including_coordinate_precision() {
        let document = "{\"strokes\":[[{\"x\":123.45678901234567,\"y\":0.000000000001}]],\"expected\":\"フ\"}\n";
        let bytes = kana_sample_zip(&serde_json::to_string(&vec![document]).unwrap()).unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut restored = String::new();
        archive
            .by_name("browser/samples/000000.json")
            .unwrap()
            .read_to_string(&mut restored)
            .unwrap();
        assert_eq!(restored, document);
    }
}
