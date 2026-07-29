//! The soundscape library: the scores this installation has saved.
//!
//! The shipped presets are templates — they are compiled into the binary and
//! cannot be written to, so editing one and saving produces a *file*, which is
//! why the button over a template says "save as". Everything the library holds
//! is a file the user made.
//!
//! Where those files live is the one thing that differs by platform, and it
//! differs in the same way everything else does: natively they are ordinary
//! `.eod` documents in `~/idiosepius/soundscapes/`, beside the database and
//! the copied fonts, where a text editor can reach them. In the browser there
//! is no such place, so they sit in the same origin-private storage the
//! database does, and the library offers a download for each one instead —
//! that download *is* the way out of the sandbox.
//!
//! The rest of the app treats them identically: the shell serves a
//! `Request`, exactly as it does for importing a deck or exporting the
//! database, and this module never touches a filesystem or a browser API of
//! its own.

/// Where saved scores live under the settings root. The browser's own copy of
/// this name is in the storage script beside its `fonts` sibling; there is no
/// way to share a constant with an inline module.
#[cfg(not(target_arch = "wasm32"))]
const DIR: &str = "soundscapes";

/// Apteronotus's document extension. A saved score is one of its documents,
/// not a format of ours, and naming it as such is what makes the round trip
/// through that editor obvious.
pub(crate) const EXTENSION: &str = "eod";

/// The longest name that still fits a row and a file system.
const MAX_NAME: usize = 48;

/// One saved score.
pub(crate) struct Score {
    /// The name as shown and as typed — the file's stem, not a separate
    /// label. One name, so a file found on disk is the file seen in the app.
    pub(crate) name: String,
    pub(crate) source: String,
}

/// Every saved score, in name order.
#[derive(Default)]
pub(crate) struct Library {
    scores: Vec<Score>,
}

impl Library {
    /// Adopt what storage returned: `(file name, contents)` pairs, in any
    /// order. Anything that is not an `.eod` document is ignored rather than
    /// reported — the directory belongs to the user, and a stray note in it is
    /// not an error.
    pub(crate) fn new(files: Vec<(String, String)>) -> Self {
        let mut scores: Vec<Score> = files
            .into_iter()
            .filter_map(|(file, source)| {
                Some(Score {
                    name: score_name(&file)?,
                    source,
                })
            })
            .collect();
        scores.sort_by(|a, b| a.name.cmp(&b.name));
        Self { scores }
    }

    pub(crate) fn scores(&self) -> &[Score] {
        &self.scores
    }

    pub(crate) fn source(&self, name: &str) -> Option<&str> {
        self.scores
            .iter()
            .find(|score| score.name == name)
            .map(|score| score.source.as_str())
    }

    pub(crate) fn contains(&self, name: &str) -> bool {
        self.scores.iter().any(|score| score.name == name)
    }

    /// Write a score into the library, replacing one of the same name.
    pub(crate) fn insert(&mut self, name: &str, source: &str) {
        match self.scores.iter_mut().find(|score| score.name == name) {
            Some(score) => score.source = source.to_owned(),
            None => {
                self.scores.push(Score {
                    name: name.to_owned(),
                    source: source.to_owned(),
                });
                self.scores.sort_by(|a, b| a.name.cmp(&b.name));
            }
        }
    }

    pub(crate) fn remove(&mut self, name: &str) {
        self.scores.retain(|score| score.name != name);
    }

    /// The name a save should use: `base` itself when it is the file being
    /// written back to, and otherwise the first free variation of it.
    pub(crate) fn free_name_unless(&self, base: &str, overwrite: bool) -> String {
        if overwrite {
            clean_name(base)
        } else {
            self.free_name(base)
        }
    }

    /// `base`, or `base-2`, `base-3`… — the first that is free.
    ///
    /// Saving a template under its own name is the ordinary case, and it must
    /// not silently overwrite the copy made last week.
    pub(crate) fn free_name(&self, base: &str) -> String {
        let base = clean_name(base);
        if !self.contains(&base) {
            return base;
        }
        (2..)
            .map(|suffix| clean_name(&format!("{base}-{suffix}")))
            .find(|candidate| !self.contains(candidate))
            .expect("an unbounded search finds a free name")
    }
}

/// The file a score is stored as.
pub(crate) fn file_name(name: &str) -> String {
    format!("{}.{EXTENSION}", clean_name(name))
}

/// The score a stored file is, or `None` if the file is not one.
fn score_name(file: &str) -> Option<String> {
    let stem = file.strip_suffix(&format!(".{EXTENSION}"))?;
    let name = clean_name(stem);
    (!name.is_empty()).then_some(name)
}

/// A typed name reduced to one that is safe as a file name and readable as a
/// label.
///
/// Deliberately not a display name mapped onto a hidden file name: the name in
/// the list is the name on disk, so a file dropped into the directory by hand
/// appears exactly as it is called, and a score saved here is findable without
/// consulting the app. Everything outside `[a-z0-9_-]` becomes a hyphen —
/// including the dot, so the only one in a stored file is the one before its
/// extension, and a name can neither hide an extension nor reach outside its
/// own directory.
pub(crate) fn clean_name(name: &str) -> String {
    let mut cleaned = String::with_capacity(name.len());
    for character in name.trim().chars() {
        let character = character.to_ascii_lowercase();
        if character.is_ascii_alphanumeric() || character == '_' {
            cleaned.push(character);
        } else if !cleaned.is_empty() && !cleaned.ends_with('-') {
            cleaned.push('-');
        }
    }
    let cleaned = cleaned.trim_matches('-').to_owned();
    match cleaned.char_indices().nth(MAX_NAME) {
        Some((at, _)) => cleaned[..at].trim_end_matches('-').to_owned(),
        None => cleaned,
    }
}

// ---------------------------------------------------------------------------
// Native storage
// ---------------------------------------------------------------------------

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::{DIR, EXTENSION};
    use anyhow::{Context as _, Result};
    use std::path::Path;

    /// Read every saved score under `root`, with a warning for anything that
    /// could not be read.
    ///
    /// A missing directory is not a warning: an installation that has never
    /// saved a score is the ordinary case, not a broken one.
    pub(crate) fn load(root: &Path) -> (Vec<(String, String)>, Option<String>) {
        let directory = root.join(DIR);
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return (Vec::new(), None);
            }
            Err(error) => {
                return (
                    Vec::new(),
                    Some(format!("Could not read {}: {error}", directory.display())),
                );
            }
        };

        let mut files = Vec::new();
        let mut problems = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some(EXTENSION) {
                continue;
            }
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            match std::fs::read_to_string(&path) {
                Ok(source) => files.push((name.to_owned(), source)),
                Err(error) => problems.push(format!("{}: {error}", path.display())),
            }
        }
        let warning = (!problems.is_empty())
            .then(|| format!("Could not read saved soundscapes: {}", problems.join("; ")));
        (files, warning)
    }

    pub(crate) fn save(root: &Path, file: &str, source: &str) -> Result<()> {
        let directory = root.join(DIR);
        std::fs::create_dir_all(&directory)
            .with_context(|| format!("creating {}", directory.display()))?;
        let path = directory.join(file);
        std::fs::write(&path, source).with_context(|| format!("writing {}", path.display()))
    }

    pub(crate) fn delete(root: &Path, file: &str) -> Result<()> {
        let path = root.join(DIR).join(file);
        match std::fs::remove_file(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            result => result.with_context(|| format!("removing {}", path.display())),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use native::{delete as delete_native, load as load_native, save as save_native};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_typed_name_becomes_one_file_can_hold() {
        assert_eq!(clean_name("Night Drive"), "night-drive");
        assert_eq!(clean_name("  spaced  out  "), "spaced-out");
        assert_eq!(clean_name("my/../escape"), "my-escape");
        assert_eq!(clean_name("under_score.1"), "under_score-1");
        assert_eq!(clean_name("-!-"), "");
        assert_eq!(clean_name(".."), "");
        assert!(clean_name(&"x".repeat(200)).len() <= MAX_NAME);
    }

    #[test]
    fn a_name_round_trips_through_its_file() {
        assert_eq!(file_name("Night Drive"), "night-drive.eod");
        assert_eq!(
            score_name("night-drive.eod").as_deref(),
            Some("night-drive")
        );
        assert_eq!(score_name("notes.txt"), None);
        assert_eq!(score_name(".eod"), None);
    }

    #[test]
    fn the_library_is_sorted_and_replaces_rather_than_duplicates() {
        let mut library = Library::new(vec![
            ("zither.eod".into(), "tempo(1)".into()),
            ("acid.eod".into(), "tempo(2)".into()),
            ("notes.md".into(), "not a score".into()),
        ]);
        assert_eq!(
            library
                .scores()
                .iter()
                .map(|score| score.name.as_str())
                .collect::<Vec<_>>(),
            ["acid", "zither"]
        );

        library.insert("acid", "tempo(3)");
        assert_eq!(library.scores().len(), 2);
        assert_eq!(library.source("acid"), Some("tempo(3)"));

        library.remove("acid");
        assert!(!library.contains("acid"));
    }

    /// The whole native round trip: a missing directory reads as an empty
    /// library, a save creates it, and a delete of something already gone is
    /// the outcome that was asked for rather than an error.
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn scores_survive_a_trip_through_the_directory() {
        let root = std::env::temp_dir().join(format!("idiosepius-library-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        assert!(load_native(&root).0.is_empty());

        save_native(&root, &file_name("night drive"), "tempo(90)").expect("saving creates the dir");
        std::fs::write(root.join(DIR).join("notes.txt"), "not a score").expect("a stray file");

        let (files, warning) = load_native(&root);
        assert!(warning.is_none());
        let library = Library::new(files);
        assert_eq!(library.scores().len(), 1, "only .eod documents are scores");
        assert_eq!(library.source("night-drive"), Some("tempo(90)"));

        delete_native(&root, &file_name("night-drive")).expect("deleting");
        delete_native(&root, &file_name("night-drive")).expect("deleting twice is not an error");
        assert!(Library::new(load_native(&root).0).scores().is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Saving a template under its own name twice must not overwrite the first
    /// copy — the whole point of "save as" is that nothing is replaced.
    #[test]
    fn a_free_name_steps_past_what_is_taken() {
        let mut library = Library::default();
        assert_eq!(library.free_name("Waves"), "waves");
        library.insert("waves", "");
        assert_eq!(library.free_name("Waves"), "waves-2");
        library.insert("waves-2", "");
        assert_eq!(library.free_name("waves"), "waves-3");
    }
}
