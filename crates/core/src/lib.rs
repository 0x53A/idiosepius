//! Idiosepius — study database and study logic.
//!
//! Everything lives in one SQLite file: the content (decks, topics,
//! questions), the complete record of what the user did (sessions, events,
//! attempts), and the scheduler state. The file is the whole application
//! state; there is no other store and no network.
//!
//! ```no_run
//! use idiosepius_core::{Store, Response, Mode, Input, Session, scheduler};
//! use std::rc::Rc;
//!
//! # fn main() -> anyhow::Result<()> {
//! let store = Rc::new(Store::open("study.db")?);
//! let deck = store.deck_id("control-systems")?.expect("deck imported");
//!
//! let mut session = Session::start(store.clone(), deck, Mode::Practice)?;
//! let mut recent = Vec::new();
//!
//! while let Some(q) = scheduler::next_card(&store, deck, Mode::Practice, &recent, None)? {
//!     session.show(q.id);
//!     let outcome = session.answer(&q, &Response::TrueFalse { value: true }, Input::Swipe)?;
//!     println!("{}", if outcome.grade.correct { "right" } else { "wrong" });
//!     recent.push(q.id);
//!     # break;
//! }
//! session.end()?;
//! # Ok(())
//! # }
//! ```

pub mod content;
pub mod db;
pub mod model;
pub mod scheduler;
pub mod session;
pub mod sql;
pub mod stats;

#[cfg(target_arch = "wasm32")]
pub mod browser_io;

pub use db::{NewFact, NewQuestion, Store};
pub use model::{
    Body, Choice, Explain, Fact, FactKind, Grade, Id, Kind, Millis, Question, Response, Seg, now_ms,
};
pub use session::{Input, Mode, Session};
