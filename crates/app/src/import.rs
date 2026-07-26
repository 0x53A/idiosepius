//! Browser deck-file decoding.
//!
//! A picker may return loose JSON packs, ZIP archives, or a mixture of both.
//! Every JSON pack is decoded before anything reaches the database; the core
//! merger then applies the same deck/UID rules as the native CLI.

use std::io::{Cursor, Read};

use anyhow::{Context, Result, bail};
use idiosepius_core::content::{Pack, merge_packs};

pub(crate) const MAX_JSON_BYTES: u64 = 32 * 1024 * 1024;
pub(crate) const MAX_TOTAL_BYTES: u64 = 128 * 1024 * 1024;

pub const EXAMPLE_REPOSITORIES: [(&str, &str); 2] = [
    (
        "control systems",
        "https://github.com/0x53A/idiosepius-control-systems",
    ),
    ("maths 2", "https://github.com/0x53A/idiosepius-math-2"),
];

pub struct PickedFile {
    pub name: String,
    pub bytes: Vec<u8>,
}

pub fn decode_packs(files: Vec<PickedFile>) -> Result<Pack> {
    if files.is_empty() {
        bail!("no files selected");
    }

    let mut packs = Vec::new();
    let mut total = 0_u64;
    for file in files {
        if extension(&file.name) == "zip" {
            let mut archive = zip::ZipArchive::new(Cursor::new(file.bytes))
                .with_context(|| format!("opening {}", file.name))?;
            let before = packs.len();
            for i in 0..archive.len() {
                let mut entry = archive
                    .by_index(i)
                    .with_context(|| format!("reading entry {i} from {}", file.name))?;
                if entry.is_dir() || extension(entry.name()) != "json" {
                    continue;
                }
                if entry.size() > MAX_JSON_BYTES {
                    bail!("{} in {} is larger than 32 MiB", entry.name(), file.name);
                }
                total = total.saturating_add(entry.size());
                if total > MAX_TOTAL_BYTES {
                    bail!("selected deck files expand beyond 128 MiB");
                }
                let mut bytes = Vec::with_capacity(entry.size() as usize);
                entry
                    .read_to_end(&mut bytes)
                    .with_context(|| format!("reading {} from {}", entry.name(), file.name))?;
                packs.push(parse_pack(
                    &bytes,
                    &format!("{}:{}", file.name, entry.name()),
                )?);
            }
            if packs.len() == before {
                bail!("{} contains no JSON files", file.name);
            }
        } else if extension(&file.name) == "json" {
            total = total.saturating_add(file.bytes.len() as u64);
            if total > MAX_TOTAL_BYTES {
                bail!("selected deck files are larger than 128 MiB");
            }
            packs.push(parse_pack(&file.bytes, &file.name)?);
        } else {
            bail!("{} is not a JSON or ZIP file", file.name);
        }
    }

    merge_packs(packs)
}

fn parse_pack(bytes: &[u8], name: &str) -> Result<Pack> {
    serde_json::from_slice(bytes).with_context(|| format!("parsing {name}"))
}

fn extension(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    const A: &str = r#"{
      "deck": {"slug":"d","title":"Deck"},
      "questions":[{"uid":"a","prompt":"A?","kind":"true_false","answer":true}]
    }"#;
    const B: &str = r#"{
      "deck": {"slug":"d","title":"Deck"},
      "questions":[{"uid":"b","prompt":"B?","kind":"true_false","answer":false}]
    }"#;

    #[test]
    fn loose_json_files_are_merged() {
        let pack = decode_packs(vec![
            PickedFile {
                name: "a.json".into(),
                bytes: A.as_bytes().to_vec(),
            },
            PickedFile {
                name: "b.JSON".into(),
                bytes: B.as_bytes().to_vec(),
            },
        ])
        .unwrap();
        assert_eq!(pack.questions.len(), 2);
    }

    #[test]
    fn zip_json_files_are_merged_and_other_entries_ignored() {
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut cursor);
            zip.start_file("a.json", SimpleFileOptions::default())
                .unwrap();
            zip.write_all(A.as_bytes()).unwrap();
            zip.start_file("notes.txt", SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"not a pack").unwrap();
            zip.start_file("nested/b.json", SimpleFileOptions::default())
                .unwrap();
            zip.write_all(B.as_bytes()).unwrap();
            zip.finish().unwrap();
        }
        let pack = decode_packs(vec![PickedFile {
            name: "decks.zip".into(),
            bytes: cursor.into_inner(),
        }])
        .unwrap();
        assert_eq!(pack.questions.len(), 2);
    }
}
