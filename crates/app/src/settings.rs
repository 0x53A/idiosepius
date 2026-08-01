//! Installation-wide preferences and app-owned font files.
//!
//! The database remains the complete study state. These settings only describe
//! how this installation paints it, and copied fonts live with the settings so
//! a picker grant is never needed again after relaunch.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use eframe::egui::{self, FontData, FontDefinitions, FontFamily};
use serde::{Deserialize, Serialize};

#[cfg(not(target_arch = "wasm32"))]
const SETTINGS_FILE: &str = "settings.json";
#[cfg(not(target_arch = "wasm32"))]
const FONTS_DIR: &str = "fonts";
const MAX_FONT_BYTES: usize = 32 * 1024 * 1024;

#[cfg(target_arch = "wasm32")]
const DEFAULT_FONT_ID: &str = "bundled:jetbrains-mono";
#[cfg(not(target_arch = "wasm32"))]
const DEFAULT_FONT_ID: &str = "bundled:automatic";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
struct PersistedSettings {
    font: String,
    fonts: Vec<StoredFont>,
    /// The background soundscape: whether it is stopped, the document it
    /// plays, and at what level. All stored unconditionally, without regard to
    /// the `audio` feature — a build made without the engine must not quietly
    /// discard the score a build with it saved.
    ///
    /// The alias is the key this was called before the control was renamed
    /// from mute to stop, so an installation that already has a settings file
    /// keeps its choice.
    #[serde(alias = "soundscape_muted")]
    soundscape_stopped: bool,
    /// Empty means "whatever the app ships as its default", so a preset that
    /// is edited upstream keeps improving for anybody who never touched it.
    soundscape: String,
    /// Which library entry the editor was last on, so it reopens where it was
    /// left. The score above is what plays; this is only its identity, and a
    /// name that no longer exists simply reads as an unsaved document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    soundscape_file: Option<String>,
    /// The fader, in decibels — the engine's own scale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    soundscape_decibels: Option<f64>,
    /// What the fader was before it was calibrated: a 0…1 position on a
    /// squared amplitude curve this app used to implement itself. Read once to
    /// carry an existing installation's level over, then dropped on the next
    /// save rather than kept in step with a scale it cannot express.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    soundscape_volume: Option<f64>,
}

impl Default for PersistedSettings {
    fn default() -> Self {
        Self {
            font: DEFAULT_FONT_ID.into(),
            fonts: Vec::new(),
            // Sound is never sprung on anybody. Stopped is the state a fresh
            // installation is in, and the only way out of it is a deliberate
            // click.
            soundscape_stopped: true,
            soundscape: String::new(),
            soundscape_file: None,
            soundscape_decibels: None,
            soundscape_volume: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct StoredFont {
    id: String,
    label: String,
    file_name: String,
}

#[derive(Clone)]
enum FontSource {
    Automatic,
    Bundled(&'static str),
    #[cfg(not(target_arch = "wasm32"))]
    System {
        family: String,
        path: std::path::PathBuf,
    },
    Stored {
        record: StoredFont,
        bytes: Arc<Vec<u8>>,
    },
}

#[derive(Clone)]
struct FontOption {
    id: String,
    label: String,
    source: FontSource,
}

/// A validated local font waiting to be copied into app-owned storage.
#[derive(Clone, Debug)]
pub(crate) struct PreparedFont {
    record: StoredFont,
    bytes: Vec<u8>,
}

impl PreparedFont {
    pub(crate) fn file_name(&self) -> &str {
        &self.record.file_name
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Clone)]
pub(crate) struct FontSettings {
    persisted: PersistedSettings,
    options: Vec<FontOption>,
    #[cfg(not(target_arch = "wasm32"))]
    filter: String,
}

impl Default for FontSettings {
    fn default() -> Self {
        Self::from_persisted(PersistedSettings::default(), HashMap::new()).0
    }
}

impl FontSettings {
    fn from_persisted(
        mut persisted: PersistedSettings,
        stored_bytes: HashMap<String, Vec<u8>>,
    ) -> (Self, Option<String>) {
        let mut options = bundled_options();

        #[cfg(not(target_arch = "wasm32"))]
        {
            options.extend(system_options());
        }

        let mut missing = Vec::new();
        for record in &persisted.fonts {
            if let Some(bytes) = stored_bytes.get(&record.file_name) {
                match validate_font(bytes) {
                    Ok(()) => options.push(FontOption {
                        id: record.id.clone(),
                        label: record.label.clone(),
                        source: FontSource::Stored {
                            record: record.clone(),
                            bytes: Arc::new(bytes.clone()),
                        },
                    }),
                    Err(error) => missing.push(format!("{} ({error})", record.label)),
                }
            } else {
                missing.push(format!("{} (file missing)", record.label));
            }
        }
        options.sort_by(|a, b| {
            option_rank(&a.source)
                .cmp(&option_rank(&b.source))
                .then_with(|| a.label.to_lowercase().cmp(&b.label.to_lowercase()))
        });

        let selected_exists = options.iter().any(|option| option.id == persisted.font);
        if !selected_exists {
            persisted.font = DEFAULT_FONT_ID.into();
        }
        let warning = (!missing.is_empty()).then(|| {
            format!(
                "Could not load copied font{}: {}",
                if missing.len() == 1 { "" } else { "s" },
                missing.join(", ")
            )
        });

        (
            Self {
                persisted,
                options,
                #[cfg(not(target_arch = "wasm32"))]
                filter: String::new(),
            },
            warning,
        )
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn load_native(root: &Path) -> (Self, Option<String>) {
        let path = root.join(SETTINGS_FILE);
        let persisted = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<PersistedSettings>(&bytes) {
                Ok(settings) => settings,
                Err(error) => {
                    return (
                        Self::default(),
                        Some(format!("Could not read {}: {error}", path.display())),
                    );
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                PersistedSettings::default()
            }
            Err(error) => {
                return (
                    Self::default(),
                    Some(format!("Could not read {}: {error}", path.display())),
                );
            }
        };

        let mut bytes = HashMap::new();
        let mut read_errors = Vec::new();
        for font in &persisted.fonts {
            let path = root.join(FONTS_DIR).join(&font.file_name);
            match std::fs::read(&path) {
                Ok(font_bytes) => {
                    bytes.insert(font.file_name.clone(), font_bytes);
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => read_errors.push(format!("{}: {error}", path.display())),
            }
        }
        let (settings, missing) = Self::from_persisted(persisted, bytes);
        let warning = match (missing, read_errors.is_empty()) {
            (Some(missing), false) => Some(format!("{missing}; {}", read_errors.join("; "))),
            (Some(missing), true) => Some(missing),
            (None, false) => Some(read_errors.join("; ")),
            (None, true) => None,
        };
        (settings, warning)
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn load_browser(
        settings_bytes: &[u8],
        font_files: Vec<(String, Vec<u8>)>,
    ) -> (Self, Option<String>) {
        let persisted = if settings_bytes.is_empty() {
            PersistedSettings::default()
        } else {
            match serde_json::from_slice(settings_bytes) {
                Ok(settings) => settings,
                Err(error) => {
                    return (
                        Self::default(),
                        Some(format!("Could not read browser settings: {error}")),
                    );
                }
            }
        };
        Self::from_persisted(persisted, font_files.into_iter().collect())
    }

    pub(crate) fn apply(&self, ctx: &egui::Context) -> Result<()> {
        let option = self
            .options
            .iter()
            .find(|option| option.id == self.persisted.font)
            .or_else(|| self.options.first())
            .context("no fonts are available")?;
        apply_source(ctx, &option.source)
    }

    pub(crate) fn select(&mut self, id: &str, ctx: &egui::Context) -> Result<bool> {
        if self.persisted.font == id {
            return Ok(false);
        }
        let source = self
            .options
            .iter()
            .find(|option| option.id == id)
            .map(|option| option.source.clone())
            .with_context(|| format!("unknown font choice {id:?}"))?;
        apply_source(ctx, &source)?;
        self.persisted.font = id.into();
        Ok(true)
    }

    pub(crate) fn selected_id(&self) -> &str {
        &self.persisted.font
    }

    pub(crate) fn selected_label(&self) -> &str {
        self.options
            .iter()
            .find(|option| option.id == self.persisted.font)
            .map_or("Automatic", |option| option.label.as_str())
    }

    pub(crate) fn choices(&self) -> Vec<(String, String)> {
        self.options
            .iter()
            .map(|option| (option.id.clone(), option.label.clone()))
            .collect()
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn filter_mut(&mut self) -> &mut String {
        &mut self.filter
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn filter(&self) -> &str {
        &self.filter
    }

    pub(crate) fn json(&self) -> Result<Vec<u8>> {
        serde_json::to_vec_pretty(&self.persisted).context("encoding settings")
    }

    pub(crate) fn soundscape_stopped(&self) -> bool {
        self.persisted.soundscape_stopped
    }

    /// The stored fader position in decibels, or `None` for "wherever the app
    /// starts one". An installation that only ever saw the old 0…1 fader is
    /// carried over rather than reset.
    pub(crate) fn soundscape_decibels(&self) -> Option<f64> {
        self.persisted
            .soundscape_decibels
            .or_else(|| self.persisted.soundscape_volume.map(legacy_decibels))
    }

    /// The stored score, or `None` while the installation is still on
    /// whichever preset the app ships as its default.
    pub(crate) fn soundscape(&self) -> Option<&str> {
        (!self.persisted.soundscape.is_empty()).then_some(self.persisted.soundscape.as_str())
    }

    /// The library entry the score came from, if it came from one.
    pub(crate) fn soundscape_file(&self) -> Option<&str> {
        self.persisted.soundscape_file.as_deref()
    }

    /// Record the soundscape state. A document identical to the shipped
    /// default is stored as "the default" rather than as a copy of it.
    #[cfg(feature = "audio")]
    pub(crate) fn set_soundscape(
        &mut self,
        source: &str,
        file: Option<&str>,
        stopped: bool,
        decibels: f64,
    ) {
        self.persisted.soundscape_stopped = stopped;
        self.persisted.soundscape_decibels = Some(decibels);
        // The legacy key cannot express this scale, so it goes rather than
        // sitting in the file disagreeing with the one that can.
        self.persisted.soundscape_volume = None;
        self.persisted.soundscape_file = file.map(str::to_owned);
        self.persisted.soundscape = if source == crate::soundscape::default_source() {
            String::new()
        } else {
            source.to_owned()
        };
    }

    pub(crate) fn prepare_import(name: &str, bytes: Vec<u8>) -> Result<PreparedFont> {
        if bytes.is_empty() {
            bail!("the selected font file is empty");
        }
        if bytes.len() > MAX_FONT_BYTES {
            bail!("the selected font is larger than 32 MiB");
        }
        let extension = Path::new(name)
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .filter(|extension| extension == "ttf" || extension == "otf")
            .context("choose a .ttf or .otf font file")?;
        validate_font(&bytes)?;

        let label = Path::new(name)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(str::trim)
            .filter(|stem| !stem.is_empty())
            .unwrap_or("Imported font")
            .to_owned();
        let hash = stable_hash(&bytes);
        let file_name = format!("{hash:016x}.{extension}");
        Ok(PreparedFont {
            record: StoredFont {
                id: format!("custom:{file_name}"),
                label,
                file_name,
            },
            bytes,
        })
    }

    // The browser writes settings and font bytes in one act; natively the
    // settings file is rewritten after the import is committed.
    #[cfg(any(test, target_arch = "wasm32"))]
    pub(crate) fn json_with_import(&self, prepared: &PreparedFont) -> Result<Vec<u8>> {
        let mut copy = self.clone();
        copy.register(prepared);
        copy.json()
    }

    pub(crate) fn commit_import(&mut self, prepared: PreparedFont) {
        self.register(&prepared);
    }

    fn register(&mut self, prepared: &PreparedFont) {
        if let Some(record) = self
            .persisted
            .fonts
            .iter_mut()
            .find(|font| font.file_name == prepared.record.file_name)
        {
            *record = prepared.record.clone();
        } else {
            self.persisted.fonts.push(prepared.record.clone());
        }

        let option = FontOption {
            id: prepared.record.id.clone(),
            label: prepared.record.label.clone(),
            source: FontSource::Stored {
                record: prepared.record.clone(),
                bytes: Arc::new(prepared.bytes.clone()),
            },
        };
        if let Some(existing) = self
            .options
            .iter_mut()
            .find(|existing| existing.id == option.id)
        {
            *existing = option;
        } else {
            self.options.push(option);
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn copy_into_native_storage(root: &Path, prepared: &PreparedFont) -> Result<()> {
        let fonts = root.join(FONTS_DIR);
        std::fs::create_dir_all(&fonts).with_context(|| format!("creating {}", fonts.display()))?;
        let destination = fonts.join(prepared.file_name());
        if !destination.exists() {
            std::fs::write(&destination, prepared.bytes())
                .with_context(|| format!("copying font to {}", destination.display()))?;
        }
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn save_native(&self, root: &Path) -> Result<()> {
        Self::save_native_json(root, &self.json()?)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn save_native_json(root: &Path, json: &[u8]) -> Result<()> {
        std::fs::create_dir_all(root).with_context(|| format!("creating {}", root.display()))?;
        let path = root.join(SETTINGS_FILE);
        std::fs::write(&path, json).with_context(|| format!("writing {}", path.display()))
    }
}

fn bundled_options() -> Vec<FontOption> {
    let mut options = Vec::new();
    #[cfg(target_arch = "wasm32")]
    options.push(FontOption {
        id: "bundled:jetbrains-mono".into(),
        label: "JetBrains Mono · bundled".into(),
        source: FontSource::Automatic,
    });
    #[cfg(not(target_arch = "wasm32"))]
    options.push(FontOption {
        id: "bundled:automatic".into(),
        label: "Automatic · system preference".into(),
        source: FontSource::Automatic,
    });
    options.push(FontOption {
        id: "bundled:hack".into(),
        label: "Hack · bundled".into(),
        source: FontSource::Bundled("Hack"),
    });
    options.push(FontOption {
        id: "bundled:ubuntu-light".into(),
        label: "Ubuntu Light · bundled".into(),
        source: FontSource::Bundled("Ubuntu-Light"),
    });
    options
}

#[cfg(not(target_arch = "wasm32"))]
fn system_options() -> Vec<FontOption> {
    // Forking fc-list and scanning every installed face is not free, and
    // `FontSettings::default()` runs on paths that never look at the list —
    // every unit test, and every `--shot` capture. The set does not change
    // under a running app, so enumerate it once per process.
    static SYSTEM: std::sync::OnceLock<Vec<FontOption>> = std::sync::OnceLock::new();
    SYSTEM.get_or_init(enumerate_system_fonts).clone()
}

#[cfg(not(target_arch = "wasm32"))]
fn enumerate_system_fonts() -> Vec<FontOption> {
    let output = match std::process::Command::new("fc-list")
        .args(["-f", "%{family}\t%{file}\n"])
        .output()
    {
        Ok(output) if output.status.success() => output,
        _ => return Vec::new(),
    };
    let mut families = std::collections::BTreeMap::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Some((names, path)) = line.split_once('\t') else {
            continue;
        };
        let path = std::path::PathBuf::from(path.trim());
        let supported = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("ttf") || extension.eq_ignore_ascii_case("otf")
            });
        if !supported {
            continue;
        }
        for family in names
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
        {
            families
                .entry(family.to_lowercase())
                .or_insert_with(|| (family.to_owned(), path.clone()));
        }
    }
    families
        .into_iter()
        .map(|(_, (family, path))| FontOption {
            id: format!("system:{}", family.to_lowercase()),
            label: family.clone(),
            source: FontSource::System { family, path },
        })
        .collect()
}

/// The old fader's 0…1 position, in the decibels it was actually producing.
///
/// That fader squared its position to get an amplitude, so this is the exact
/// level the installation was hearing — not an approximation of the feel of
/// the control. Silence is `-inf`, which the fader's own clamp turns into the
/// bottom of its travel.
fn legacy_decibels(position: f64) -> f64 {
    let amplitude = position.clamp(0.0, 1.0).powi(2);
    if amplitude <= 0.0 {
        f64::NEG_INFINITY
    } else {
        20.0 * amplitude.log10()
    }
}

fn option_rank(source: &FontSource) -> u8 {
    match source {
        FontSource::Automatic | FontSource::Bundled(_) => 0,
        FontSource::Stored { .. } => 1,
        #[cfg(not(target_arch = "wasm32"))]
        FontSource::System { .. } => 2,
    }
}

fn apply_source(ctx: &egui::Context, source: &FontSource) -> Result<()> {
    match source {
        FontSource::Automatic => {
            crate::theme::install_fonts(ctx);
            return Ok(());
        }
        FontSource::Bundled(name) => {
            let mut fonts = FontDefinitions::default();
            make_primary(&mut fonts, name);
            ctx.set_fonts(fonts);
        }
        #[cfg(not(target_arch = "wasm32"))]
        FontSource::System { family, path } => {
            let bytes =
                std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
            validate_font(&bytes)
                .with_context(|| format!("{family} is not a usable TTF/OTF font"))?;
            install_bytes(ctx, bytes)?;
        }
        FontSource::Stored { record, bytes } => {
            let _ = record;
            install_bytes(ctx, bytes.as_ref().clone())?;
        }
    }
    Ok(())
}

fn install_bytes(ctx: &egui::Context, bytes: Vec<u8>) -> Result<()> {
    validate_font(&bytes)?;
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "user-selected".into(),
        Arc::new(FontData::from_owned(bytes)),
    );
    make_primary(&mut fonts, "user-selected");
    ctx.set_fonts(fonts);
    Ok(())
}

fn make_primary(fonts: &mut FontDefinitions, name: &str) {
    for family in [FontFamily::Monospace, FontFamily::Proportional] {
        let choices = fonts.families.entry(family).or_default();
        choices.retain(|choice| choice != name);
        choices.insert(0, name.to_owned());
    }
}

fn stable_hash(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Validate through the same parser egui 0.35 uses before handing bytes to
/// `Context::set_fonts`. In particular, `ttf-parser` accepts WOFF2 containers,
/// while egui's `skrifa::FontRef` deliberately accepts raw SFNT TTF/OTF only.
fn validate_font(bytes: &[u8]) -> Result<()> {
    if bytes.starts_with(b"wOF2") || bytes.starts_with(b"wOFF") {
        bail!("WOFF/WOFF2 web fonts are not supported; choose a desktop TTF or OTF file");
    }
    skrifa::FontRef::from_index(bytes, 0)
        .map(|_| ())
        .map_err(|error| anyhow::anyhow!("that file is not a usable TTF/OTF font: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_defaults_choose_a_bundled_font() {
        assert!(PersistedSettings::default().font.starts_with("bundled:"));
    }

    #[test]
    fn imported_file_name_is_content_addressed() {
        // This is not a font, so validation must happen before a storage name
        // is ever accepted.
        assert!(FontSettings::prepare_import("pretend.ttf", b"not a font".to_vec()).is_err());
    }

    #[test]
    fn woff2_is_rejected_before_egui_can_defer_a_panic() {
        let mut bytes = b"wOF2".to_vec();
        bytes.resize(32, 0);
        let error = FontSettings::prepare_import("JetBrainsMono.ttf", bytes)
            .unwrap_err()
            .to_string();
        assert!(error.contains("WOFF2"));
    }

    #[test]
    fn a_previously_copied_woff2_falls_back_on_relaunch() {
        let record = StoredFont {
            id: "custom:bad.ttf".into(),
            label: "Bad web font".into(),
            file_name: "bad.ttf".into(),
        };
        let persisted = PersistedSettings {
            font: record.id.clone(),
            fonts: vec![record],
            ..PersistedSettings::default()
        };
        let mut files = HashMap::new();
        files.insert(
            "bad.ttf".into(),
            b"wOF2 followed by web font bytes".to_vec(),
        );
        let (settings, warning) = FontSettings::from_persisted(persisted, files);
        assert_eq!(settings.selected_id(), DEFAULT_FONT_ID);
        assert!(warning.unwrap().contains("WOFF"));
    }

    #[test]
    fn a_valid_font_is_added_without_becoming_the_active_choice() {
        let definitions = FontDefinitions::default();
        let bytes = definitions.font_data["Hack"].font.to_vec();
        let prepared = FontSettings::prepare_import("My Font.ttf", bytes).unwrap();
        let defaults = FontSettings::default();
        let original = defaults.selected_id().to_owned();
        let json = defaults.json_with_import(&prepared).unwrap();
        let saved: PersistedSettings = serde_json::from_slice(&json).unwrap();
        assert_eq!(saved.font, original);
        assert_eq!(saved.fonts[0].label, "My Font");
        assert_eq!(saved.fonts[0].file_name, prepared.record.file_name);
    }

    #[test]
    fn stable_hash_changes_with_content() {
        assert_ne!(stable_hash(b"one"), stable_hash(b"two"));
        assert_eq!(stable_hash(b"one"), stable_hash(b"one"));
    }
}
