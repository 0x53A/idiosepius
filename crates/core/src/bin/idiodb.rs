//! Command line access to a study database: import packs, inspect progress,
//! dump the log. The UI does not need this, but authoring and evaluation do.

use anyhow::{Result, bail};
use idiosepius_core::{Store, content, content_text, params, scheduler, stats};

const USAGE: &str = "\
idiodb — inspect and load an idiosepius study database

USAGE
    idiodb <db> import <pack.json>…  import or re-import question packs
                                     (several files may share one deck)
    idiodb <db> decks                list decks with progress
    idiodb <db> stats <deck-slug>    accuracy and readiness, per topic
    idiodb <db> weak <deck-slug>     the cards you keep getting wrong
    idiodb <db> facts [deck-slug]    the shared explanations and symbols
    idiodb <db> events               dump the event log as JSON lines
";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 || args[0] == "-h" || args[0] == "--help" {
        print!("{USAGE}");
        return Ok(());
    }

    let mut store = Store::open(&args[0])?;

    match args[1].as_str() {
        "import" => {
            let paths = &args[2..];
            if paths.is_empty() {
                bail!("import needs at least one pack path");
            }
            let packs = paths
                .iter()
                .map(content::load_pack)
                .collect::<Result<Vec<_>>>()?;
            let pack = content::merge_packs(packs)?;
            let report = content::import_pack(&mut store, &pack)?;
            println!(
                "imported {} questions and {} lessons in {} topics, {} facts, into '{}'{}",
                report.questions,
                report.lessons,
                report.topics,
                report.facts,
                pack.deck.slug,
                if report.retired > 0 {
                    format!(" ({} retired)", report.retired)
                } else {
                    String::new()
                }
            );
        }

        "decks" => {
            for d in store.decks()? {
                let c = scheduler::counts(&store, d.id)?;
                let s = stats::deck_stats(&store, d.id)?;
                println!(
                    "{:<24} {:>4} cards  {:>3} new  {:>3} due  {:>5.1}% correct  {:>5.1}% learned{}",
                    d.slug,
                    c.total,
                    c.fresh,
                    c.due,
                    s.accuracy * 100.0,
                    s.readiness * 100.0,
                    match d.exam_at {
                        Some(e) => format!("  exam {}", content::format_rfc3339_ms(e)),
                        None => String::new(),
                    }
                );
            }
        }

        "stats" => {
            let deck = deck_by_slug(&store, args.get(2))?;
            let s = stats::deck_stats(&store, deck)?;
            println!(
                "{} cards, {} seen, {} attempts, {:.1}% correct, {:.1}% learned",
                s.questions,
                s.attempted,
                s.attempts,
                s.accuracy * 100.0,
                s.readiness * 100.0
            );
            println!(
                "median answer time {:.1}s",
                s.median_latency_ms as f64 / 1000.0
            );
            println!();
            for t in stats::topic_stats(&store, deck)? {
                println!(
                    "  {:<22} {:>3} cards  {:>3} attempts  {:>5.1}%  {:>3} learned",
                    t.title,
                    t.questions,
                    t.attempts,
                    t.accuracy * 100.0,
                    t.solid
                );
            }
        }

        "weak" => {
            let deck = deck_by_slug(&store, args.get(2))?;
            for w in stats::weakest(&store, deck, 20)? {
                let prompt: String = w.prompt.chars().take(70).collect();
                println!(
                    "  {:>4.0}%  {}/{}  {}",
                    w.ema * 100.0,
                    w.correct,
                    w.attempts,
                    prompt
                );
            }
        }

        "facts" => {
            let deck = match args.get(2) {
                Some(slug) => deck_by_slug(&store, Some(slug))?,
                None => store.decks()?.first().map(|d| d.id).unwrap_or(0),
            };
            for f in store.facts(deck)? {
                let head = match (&f.label, &f.name, &f.title) {
                    (Some(l), Some(n), _) => format!("{l} ({n})"),
                    (Some(l), None, _) => l.clone(),
                    (None, _, Some(t)) => t.clone(),
                    _ => String::new(),
                };
                println!(
                    "  {:<18} {:<28} {}",
                    f.uid,
                    head,
                    one_line(&content_text(&f.body), 60)
                );
            }
        }

        "events" => {
            let rows = store.conn().query_all(
                "SELECT session_id, ts, mono_ms, question_id, lesson_id, kind, data
                 FROM event ORDER BY id",
                params![],
                |r| {
                    Ok(serde_json::json!({
                        "session": r.get::<i64>(0)?,
                        "ts": r.get::<i64>(1)?,
                        "mono_ms": r.get::<i64>(2)?,
                        "question": r.get::<Option<i64>>(3)?,
                        "lesson": r.get::<Option<i64>>(4)?,
                        "kind": r.get::<String>(5)?,
                        "data": r.get::<Option<String>>(6)?
                            .and_then(|d| serde_json::from_str::<serde_json::Value>(&d).ok()),
                    }))
                },
            )?;
            for row in rows {
                println!("{row}");
            }
        }

        other => bail!("unknown command {other:?}\n\n{USAGE}"),
    }

    Ok(())
}

/// Squash a fact down to something that fits in a terminal column.
fn one_line(s: &str, width: usize) -> String {
    let flat: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= width {
        return flat;
    }
    flat.chars().take(width - 1).chain(['…']).collect()
}

fn deck_by_slug(store: &Store, slug: Option<&String>) -> Result<i64> {
    let Some(slug) = slug else {
        bail!("needs a deck slug")
    };
    match store.deck_id(slug)? {
        Some(id) => Ok(id),
        None => bail!("no deck with slug {slug:?}"),
    }
}
