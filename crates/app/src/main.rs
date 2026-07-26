//! Idiosepius — a study deck you swipe through.
//!
//! Single binary, no server: it opens the SQLite study file directly.

mod app;
mod card;
mod coin;
mod explain;
mod import;
mod math;
mod richtext;
mod theme;

use anyhow::{Context, Result};
use idiosepius_core::{Store, content};
use std::path::PathBuf;

const USAGE: &str = "\
idio — study by swiping

USAGE
    idio [<study.db>] [--import <pack.json>…]
    idio <study.db> --shot <out.pam> [--screen <name>] [--card <uid>] [--drag <px>]

    With no path, uses ~/idiosepius/study.db.

KEYS
    ← →   answer false / true        1-5    pick an option
    e     explain (counts as skip)   s      skip
    d     short/deep explanation     r      review answered cards
    u     undo the last answer       enter  confirm / continue
    Ctrl/Cmd+C  copy visible text    Ctrl/Cmd+±  scale interface
    esc   end the session
";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{USAGE}");
        return Ok(());
    }

    let parsed = parse_args(&args)?;
    let db_path = parsed.db_path();
    let imports = parsed.imports.clone();

    if let Some(dir) = db_path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let mut store = Store::open(&db_path)?;

    if !imports.is_empty() {
        let packs = imports
            .iter()
            .map(content::load_pack)
            .collect::<Result<Vec<_>>>()?;
        let pack = content::merge_packs(packs)?;
        let report = content::import_pack(&mut store, &pack)?;
        println!(
            "imported {} questions into '{}'",
            report.questions, pack.deck.slug
        );
    }

    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size([940.0, 720.0])
        .with_min_inner_size([560.0, 480.0])
        .with_title("Idiosepius");
    if let Some(icon) = window_icon() {
        viewport = viewport.with_icon(icon);
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    let shot = parsed.shot.clone().map(|path| {
        app::Shot::new(path, parsed.screen.clone())
            .with_card(parsed.card.clone())
            .with_drag(parsed.drag)
    });

    eframe::run_native(
        "idiosepius",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(&cc.egui_ctx, store, shot)))),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}

#[derive(Debug, Default, PartialEq)]
struct Args {
    db: Option<PathBuf>,
    imports: Vec<PathBuf>,
    shot: Option<PathBuf>,
    screen: Option<String>,
    card: Option<String>,
    drag: f32,
}

impl Args {
    fn db_path(&self) -> PathBuf {
        self.db.clone().unwrap_or_else(default_db_path)
    }
}

fn parse_args(args: &[String]) -> Result<Args> {
    let mut out = Args::default();
    let mut mode = Mode::Positional;

    for arg in args {
        match arg.as_str() {
            "--import" => mode = Mode::Import,
            "--shot" => mode = Mode::Shot,
            "--screen" => mode = Mode::Screen,
            "--card" => mode = Mode::Card,
            "--drag" => mode = Mode::Drag,
            _ => match mode {
                Mode::Import => out.imports.push(PathBuf::from(arg)),
                Mode::Shot => {
                    out.shot = Some(PathBuf::from(arg));
                    mode = Mode::Positional;
                }
                Mode::Screen => {
                    out.screen = Some(arg.clone());
                    mode = Mode::Positional;
                }
                Mode::Card => {
                    out.card = Some(arg.clone());
                    mode = Mode::Positional;
                }
                Mode::Drag => {
                    out.drag = arg
                        .parse()
                        .with_context(|| format!("--drag wants a number, got {arg:?}"))?;
                    mode = Mode::Positional;
                }
                Mode::Positional if out.db.is_none() => out.db = Some(PathBuf::from(arg)),
                Mode::Positional => anyhow::bail!("unexpected argument {arg:?}\n\n{USAGE}"),
            },
        }
    }

    if out.shot.is_none() && (out.screen.is_some() || out.card.is_some() || out.drag != 0.0) {
        anyhow::bail!("--screen, --card and --drag only make sense together with --shot");
    }
    Ok(out)
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Positional,
    Import,
    Shot,
    Screen,
    Card,
    Drag,
}

/// The copper squid coin, decoded for the OS window/taskbar icon.
///
/// The coin is round and copper — deliberately *not* the in-app look — because
/// an OS icon lives outside the app's design language, where it can be as
/// ornamental as it likes. Baked into the binary so the running app needs no
/// asset files beside it. A decode failure is not worth aborting a launch for.
fn window_icon() -> Option<eframe::egui::IconData> {
    const BYTES: &[u8] = include_bytes!("../../../assets/idiosepius-coin-logo-copper-incuse.png");
    let image = image::load_from_memory(BYTES).ok()?.into_rgba8();
    let (width, height) = image.dimensions();
    Some(eframe::egui::IconData {
        rgba: image.into_raw(),
        width,
        height,
    })
}

/// `~/idiosepius/study.db`.
///
/// Deliberately not under `$XDG_DATA_HOME`: the database *is* the course —
/// questions, history and scheduler state in one file — so it wants to sit
/// somewhere you can find it, back it up and copy it between machines, not
/// buried in `.local/share` where it reads like a cache.
fn default_db_path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join("idiosepius").join("study.db")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_bare_path_is_the_database() {
        let a = parse_args(&args(&["study.db"])).unwrap();
        assert_eq!(a.db_path(), PathBuf::from("study.db"));
        assert!(a.imports.is_empty());
    }

    #[test]
    fn import_takes_every_following_path() {
        let a = parse_args(&args(&["s.db", "--import", "a.json", "b.json"])).unwrap();
        assert_eq!(a.db_path(), PathBuf::from("s.db"));
        assert_eq!(a.imports.len(), 2);
    }

    #[test]
    fn importing_without_a_database_path_uses_the_default() {
        let a = parse_args(&args(&["--import", "a.json"])).unwrap();
        assert!(a.db_path().ends_with("idiosepius/study.db"));
        assert_eq!(a.imports.len(), 1);
    }

    #[test]
    fn a_second_stray_path_is_an_error() {
        assert!(parse_args(&args(&["a.db", "b.db"])).is_err());
    }

    #[test]
    fn shot_takes_exactly_one_path_and_does_not_swallow_the_rest() {
        let a = parse_args(&args(&["s.db", "--shot", "out.pam"])).unwrap();
        assert_eq!(a.shot, Some(PathBuf::from("out.pam")));
        assert_eq!(a.db_path(), PathBuf::from("s.db"));
    }

    #[test]
    fn screen_selects_the_view_to_capture() {
        let a = parse_args(&args(&["s.db", "--shot", "o.pam", "--screen", "study"])).unwrap();
        assert_eq!(a.screen.as_deref(), Some("study"));
    }

    #[test]
    fn screen_without_shot_is_rejected() {
        assert!(parse_args(&args(&["s.db", "--screen", "study"])).is_err());
    }

    #[test]
    fn card_and_drag_are_parsed() {
        let a = parse_args(&args(&[
            "s.db",
            "--shot",
            "o.pam",
            "--card",
            "cs-mod-001",
            "--drag",
            "-90",
        ]))
        .unwrap();
        assert_eq!(a.card.as_deref(), Some("cs-mod-001"));
        assert_eq!(a.drag, -90.0);
    }

    #[test]
    fn a_non_numeric_drag_is_rejected() {
        assert!(parse_args(&args(&["s.db", "--shot", "o.pam", "--drag", "left"])).is_err());
    }
}
