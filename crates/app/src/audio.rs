//! The background soundscape: an Apteronotus program playing under the study
//! session.
//!
//! The engine is the `apteronotus-lua` sandbox and the `apteronotus-live`
//! scheduler, the same path its own editor uses. What is *not* borrowed is its
//! player: that one exists to keep a live-coding performance sounding across
//! an edit, reconciling revisions at a monotonic scheduling frontier. Nothing
//! here is a performance. A soundscape is chosen once and then left alone for
//! an hour, so every activation is simply a fresh stream at cycle zero, and
//! the whole reconciliation apparatus is not reimplemented.
//!
//! **There is no mute, only stop.** Stopped is the default, so a study session
//! that never asks for sound never opens an audio device at all — no ALSA
//! handle held, no DSP graph running at 48 kHz behind a silent output, and in
//! the browser no AudioContext to be blocked by the autoplay policy. That
//! makes starting again a restart from the beginning rather than a resume,
//! which for a cyclic ambient loop is not a cost anybody can hear — and it is
//! why the control is not called mute. A mute button that silently closes the
//! device and rewinds the piece would be lying about what it does.
//!
//! **Volume is `apteronotus_live::MasterGain`**, a gain stage between the
//! engine and the device rather than part of the score. The obvious
//! alternative was an Apteronotus `control` the score declares and the slider
//! drives, which is how that project does its live faders — but `master` may
//! be declared only once, so a score that already has one cannot be given a
//! gain stage from outside, and every interesting score has one. A master
//! volume that stops working the moment you paste in a real song is not a
//! master volume.
//!
//! This app grew its own version of that node first, and then Apteronotus grew
//! a better one: the engine's is smoothed, so a fader move is not a step in
//! gain, it is calibrated in decibels rather than in amplitude, and it is part
//! of every `AudioOutput` whether or not a host offers a control for it. So
//! this is the engine's, and the copy is gone — along with the direct `fundsp`
//! dependency the copy needed.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use apteronotus_live::{
    AudioOutput, MasterGain, PersistentRuntime, ProgramScheduler, RoutedRuntime, ScheduledRun,
    ScheduledTrack,
};
use apteronotus_lua::{Evaluator, Program};
use web_time::Instant;

#[cfg(not(target_arch = "wasm32"))]
use std::{
    sync::mpsc::RecvTimeoutError,
    thread::{self, JoinHandle},
};

/// How far ahead of the audio clock the scheduler keeps the sequencer filled.
///
/// The native figure is Apteronotus's, and its worker thread keeps to it
/// whatever the interface is doing. The browser has no worker: scheduling runs
/// on the same main thread as egui, the browser's own work and garbage
/// collection, so a single long task there is a dropout. Apteronotus cannot
/// simply buy slack by filling further ahead — it is an instrument, and
/// lookahead is the delay before an edit is heard. Background music has no
/// such constraint. Nobody is playing this, so nobody can feel a longer
/// horizon, and it is the cheapest resilience available on that thread.
#[cfg(not(target_arch = "wasm32"))]
const LOOKAHEAD_SECONDS: f64 = 0.20;
#[cfg(target_arch = "wasm32")]
const LOOKAHEAD_SECONDS: f64 = 0.60;

/// How often the interface must come back while sound is playing.
///
/// In the browser this is not a refresh rate, it is the audio scheduler's
/// clock: `Worker::pump` evaluates and fills lookahead, and it only runs when
/// a frame runs. Native has a worker thread doing that independently, so its
/// frames are needed only to notice a status change — a study app should not
/// repaint at 25 Hz for an hour to keep a word in a corner up to date.
#[cfg(target_arch = "wasm32")]
const PUMP_INTERVAL: Duration = Duration::from_millis(40);
#[cfg(not(target_arch = "wasm32"))]
const PUMP_INTERVAL: Duration = Duration::from_millis(250);

/// The fader's travel, taken from the engine so the two cannot disagree.
///
/// Decibels, not a 0…1 amplitude: a linear-amplitude fader does its whole
/// audible job in the bottom fifth of its travel and spends the rest moving
/// between levels that all sound about the same.
pub(crate) const MIN_DECIBELS: f64 = MasterGain::MIN_DECIBELS as f64;
pub(crate) const MAX_DECIBELS: f64 = MasterGain::MAX_DECIBELS as f64;

/// Where the fader starts before anybody has moved it: background music, so
/// well below the unity a foreground application would open at.
pub(crate) const DEFAULT_DECIBELS: f64 = -12.0;

#[cfg(not(target_arch = "wasm32"))]
const POLL_INTERVAL: Duration = Duration::from_millis(20);

enum Command {
    Play { request: u64, source: String },
    SetVolume { decibels: f32 },
    Stop,
    Shutdown,
}

enum Event {
    Playing { request: u64 },
    Failed { request: u64, message: String },
    Stopped,
    RuntimeError(String),
}

/// What the soundscape is doing, as far as the interface needs to know.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    Silent,
    Starting,
    Playing,
    Failed(String),
}

/// The soundscape as the rest of the app sees it: a stop switch, a level, a
/// source document, and whatever the player last reported about them.
pub(crate) struct Soundscape {
    stopped: bool,
    source: String,
    decibels: f64,
    status: Status,
    command_tx: Sender<Command>,
    event_rx: Receiver<Event>,
    worker: Option<Worker>,
    next_request: u64,
    latest_request: u64,
}

impl Soundscape {
    pub(crate) fn new(source: String, stopped: bool, decibels: f64) -> Self {
        let (worker, command_tx, event_rx) = Worker::spawn();
        let mut soundscape = Self {
            stopped,
            source,
            decibels: decibels.clamp(MIN_DECIBELS, MAX_DECIBELS),
            status: Status::Silent,
            command_tx,
            event_rx,
            worker: Some(worker),
            next_request: 1,
            latest_request: 0,
        };
        soundscape.send(Command::SetVolume {
            decibels: soundscape.decibels as f32,
        });
        // A session that was playing when it was last closed resumes on the
        // desktop, where there is no gesture requirement.
        // `App::new_with_settings` forces `stopped` in the browser and under
        // `--shot`, so this cannot open a device there.
        if !soundscape.stopped {
            soundscape.start();
        }
        soundscape
    }

    pub(crate) fn stopped(&self) -> bool {
        self.stopped
    }

    pub(crate) fn status(&self) -> &Status {
        &self.status
    }

    pub(crate) fn source(&self) -> &str {
        &self.source
    }

    pub(crate) fn source_mut(&mut self) -> &mut String {
        &mut self.source
    }

    /// Where the fader sits, in decibels.
    pub(crate) fn decibels(&self) -> f64 {
        self.decibels
    }

    /// Move the level. This reaches a running graph without restarting it, so
    /// it is safe to call on every frame of a drag.
    pub(crate) fn set_decibels(&mut self, decibels: f64) {
        let decibels = decibels.clamp(MIN_DECIBELS, MAX_DECIBELS);
        if (decibels - self.decibels).abs() < f64::EPSILON {
            return;
        }
        self.decibels = decibels;
        self.send(Command::SetVolume {
            decibels: decibels as f32,
        });
    }

    /// Start or stop. Stopping closes the device; it does not pause.
    pub(crate) fn set_stopped(&mut self, stopped: bool) {
        if self.stopped == stopped {
            return;
        }
        self.stopped = stopped;
        if stopped {
            self.send(Command::Stop);
            self.status = Status::Silent;
        } else {
            self.start();
        }
    }

    /// Adopt `source` and, if already sounding, restart on it.
    ///
    /// Replacing the document while stopped deliberately does not open a
    /// device: editing a soundscape is not a request to hear it.
    pub(crate) fn set_source(&mut self, source: String) {
        self.source = source;
        if !self.stopped {
            self.start();
        }
    }

    /// Restart on the current source, starting the device if it is closed —
    /// the only reason to ask is to hear the result.
    pub(crate) fn restart(&mut self) {
        self.stopped = false;
        self.start();
    }

    fn start(&mut self) {
        let request = self.next_request;
        self.next_request = self.next_request.saturating_add(1);
        self.latest_request = request;
        self.status = Status::Starting;
        self.send(Command::Play {
            request,
            source: self.source.clone(),
        });
    }

    fn send(&mut self, command: Command) {
        if self.command_tx.send(command).is_err() {
            self.status = Status::Failed("the audio worker stopped unexpectedly".into());
        } else if let Some(worker) = &mut self.worker {
            // In the browser this is what keeps AudioContext creation inside
            // the trusted click that started us. Native playback's worker is
            // already running, so its pump is a no-op.
            worker.pump();
        }
    }

    /// How soon the interface must paint again for the player's sake, or
    /// `None` if nothing is playing and the screen may go back to sleep.
    ///
    /// Without this the frame loop is free to settle, and on the web that
    /// silences the scheduler — the pump only runs inside a frame. Before it
    /// existed the only thing keeping frames coming was the ocean background
    /// animating, which made audio depend on a decoration.
    ///
    /// `Failed` keeps pumping, which looks wrong and is not: a score that
    /// fails to compile leaves the *previous* one playing, so a refusal is
    /// exactly the state in which audio is still running and still needs
    /// filling. Only `Silent` means the device is closed.
    pub(crate) fn repaint_interval(&self) -> Option<Duration> {
        (self.status != Status::Silent).then_some(PUMP_INTERVAL)
    }

    /// Collect whatever the player has reported. Call once per frame.
    pub(crate) fn poll(&mut self) {
        if let Some(worker) = &mut self.worker {
            worker.pump();
        }
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                Event::Playing { request } if request >= self.latest_request => {
                    self.latest_request = request;
                    self.status = Status::Playing;
                }
                Event::Failed { request, message } if request >= self.latest_request => {
                    self.latest_request = request;
                    self.status = Status::Failed(message);
                }
                Event::Stopped => self.status = Status::Silent,
                Event::RuntimeError(message) => self.status = Status::Failed(message),
                Event::Playing { .. } | Event::Failed { .. } => {}
            }
        }
    }
}

impl Drop for Soundscape {
    fn drop(&mut self) {
        let _ = self.command_tx.send(Command::Shutdown);
        if let Some(worker) = self.worker.take() {
            worker.join();
        }
    }
}

// ---------------------------------------------------------------------------
// The worker
//
// Native gets a thread, so neither Lua evaluation nor scheduler lookahead can
// stutter the study interface. The browser has no `std::thread::spawn` without
// cross-origin isolation, so it does the same work from the frame callback.
// ---------------------------------------------------------------------------

#[cfg(not(target_arch = "wasm32"))]
struct Worker {
    thread: Option<JoinHandle<()>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Worker {
    fn spawn() -> (Worker, Sender<Command>, Receiver<Event>) {
        let (command_tx, command_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("idiosepius-soundscape".into())
            .spawn(move || run_worker(command_rx, event_tx))
            .expect("failed to start the soundscape worker");
        (
            Worker {
                thread: Some(thread),
            },
            command_tx,
            event_rx,
        )
    }

    fn pump(&mut self) {}

    fn join(mut self) {
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn run_worker(command_rx: Receiver<Command>, event_tx: Sender<Event>) {
    let mut player = Player::new();
    loop {
        match command_rx.recv_timeout(POLL_INTERVAL) {
            Ok(Command::Play { request, source }) => {
                let event = match player.play(&source) {
                    Ok(()) => Event::Playing { request },
                    Err(message) => Event::Failed { request, message },
                };
                if event_tx.send(event).is_err() {
                    break;
                }
            }
            Ok(Command::SetVolume { decibels }) => player.set_volume(decibels),
            Ok(Command::Stop) => {
                player.stop();
                if event_tx.send(Event::Stopped).is_err() {
                    break;
                }
            }
            Ok(Command::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {}
        }
        if let Err(message) = player.fill_lookahead()
            && event_tx.send(Event::RuntimeError(message)).is_err()
        {
            break;
        }
    }
}

#[cfg(target_arch = "wasm32")]
struct Worker {
    command_rx: Receiver<Command>,
    event_tx: Sender<Event>,
    player: Player,
}

#[cfg(target_arch = "wasm32")]
impl Worker {
    fn spawn() -> (Worker, Sender<Command>, Receiver<Event>) {
        let (command_tx, command_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        (
            Worker {
                command_rx,
                event_tx,
                player: Player::new(),
            },
            command_tx,
            event_rx,
        )
    }

    fn pump(&mut self) {
        while let Ok(command) = self.command_rx.try_recv() {
            let event = match command {
                Command::Play { request, source } => match self.player.play(&source) {
                    Ok(()) => Event::Playing { request },
                    Err(message) => Event::Failed { request, message },
                },
                Command::SetVolume { decibels } => {
                    self.player.set_volume(decibels);
                    continue;
                }
                Command::Stop => {
                    self.player.stop();
                    Event::Stopped
                }
                Command::Shutdown => break,
            };
            if self.event_tx.send(event).is_err() {
                return;
            }
        }
        if let Err(message) = self.player.fill_lookahead() {
            let _ = self.event_tx.send(Event::RuntimeError(message));
        }
    }

    fn join(self) {}
}

// ---------------------------------------------------------------------------
// The player
// ---------------------------------------------------------------------------

struct Player {
    evaluator: Evaluator,
    program: Option<Arc<Program>>,
    scheduler: ProgramScheduler,
    output: Option<AudioOutput>,
    persistent: Option<PersistentRuntime>,
    started: Option<Instant>,
    /// Where the fader sits. The engine's `MasterGain` handle belongs to one
    /// stream, so what survives a restart is the *number*, reapplied to the
    /// new output before it is ever unpaused — which is also what stops a
    /// replacement from rendering a single block at the wrong gain.
    decibels: f32,
}

impl Player {
    fn new() -> Player {
        Player {
            evaluator: Evaluator::default(),
            program: None,
            scheduler: ProgramScheduler::default(),
            output: None,
            persistent: None,
            started: None,
            decibels: DEFAULT_DECIBELS as f32,
        }
    }

    /// Evaluate `source` and put it on the device, replacing whatever was
    /// playing.
    ///
    /// The replacement is built completely — evaluated, lowered, opened and
    /// filled — before the previous stream is touched. A soundscape that fails
    /// to compile therefore leaves the previous one playing rather than
    /// dropping the room into silence, which is the same promise Apteronotus's
    /// own player makes and the one property of it worth keeping here.
    fn play(&mut self, source: &str) -> Result<(), String> {
        let program = self
            .evaluator
            .evaluate(source)
            .map_err(|error| error.to_string())?;
        let channels = playable_channels(&program)?;
        let mut persistent = needs_persistent_runtime(&program)
            .then(|| persistent_runtime(&program))
            .transpose()?;

        // The master fader is inside every `AudioOutput`, so the plain route
        // needs no processor of its own any more.
        let mut output = match &mut persistent {
            Some(runtime) => AudioOutput::open_processed(
                runtime.layout().total_channels(),
                runtime.layout().main_channels(),
                runtime.take_processor(),
            ),
            None => AudioOutput::open(channels),
        }
        .map_err(|error| error.to_string())?;
        // Before a single block is rendered: an output opens at unity, and
        // this one must not be heard there even for one buffer.
        output.master().set_decibels(self.decibels);

        let mut scheduler = ProgramScheduler::default();
        fill(
            &mut scheduler,
            &program,
            persistent.as_ref(),
            LOOKAHEAD_SECONDS,
            &mut output,
        )?;

        // Everything that can fail has now succeeded. Silence the old stream.
        self.stop();
        output.play().map_err(|error| error.to_string())?;

        self.program = Some(Arc::new(program));
        self.scheduler = scheduler;
        self.output = Some(output);
        self.persistent = persistent;
        self.started = Some(Instant::now());
        Ok(())
    }

    /// Move the level. An atomic store into a node the running graph already
    /// holds, so it never restarts, re-lowers or re-evaluates anything, and
    /// the engine smooths it so the move is not heard as an edge.
    fn set_volume(&mut self, decibels: f32) {
        self.decibels = decibels.clamp(MasterGain::MIN_DECIBELS, MasterGain::MAX_DECIBELS);
        if let Some(output) = &self.output {
            output.master().set_decibels(self.decibels);
        }
    }

    fn stop(&mut self) {
        if let Some(output) = self.output.take() {
            let _ = output.pause();
        }
        self.program = None;
        self.scheduler = ProgramScheduler::default();
        self.persistent = None;
        self.started = None;
    }

    fn fill_lookahead(&mut self) -> Result<(), String> {
        let (Some(started), Some(program)) = (self.started, self.program.clone()) else {
            return Ok(());
        };
        let target_seconds = started.elapsed().as_secs_f64() + LOOKAHEAD_SECONDS;
        if target_seconds <= program.tempo.cycle_to_seconds(self.scheduler.frontier()) {
            return Ok(());
        }
        let output = self
            .output
            .as_mut()
            .expect("a started player holds its output");
        fill(
            &mut self.scheduler,
            &program,
            self.persistent.as_ref(),
            target_seconds,
            output,
        )
    }
}

/// Schedule every track and finite run of `program` up to `target_seconds`.
///
/// This takes the whole `AudioOutput` rather than the sequencer inside it so
/// that fundsp stays an implementation detail of the engine: naming
/// `Sequencer` here would make it a direct dependency of the study app.
fn fill(
    scheduler: &mut ProgramScheduler,
    program: &Program,
    persistent: Option<&PersistentRuntime>,
    target_seconds: f64,
    output: &mut AudioOutput,
) -> Result<(), String> {
    let tracks = scheduled_tracks(program)?;
    let sequencer = output.sequencer_mut();
    match persistent {
        Some(runtime) => scheduler
            .fill_routed_program_to_seconds_tempo_map(
                target_seconds,
                tracks,
                scheduled_runs(program)?,
                &program.tempo,
                sequencer,
                RoutedRuntime::new(runtime.layout(), runtime.controls()),
            )
            .map(|_| ())
            .map_err(|error| error.to_string()),
        None => scheduler
            .fill_to_seconds_tempo_map(target_seconds, tracks, &program.tempo, sequencer)
            .map(|_| ())
            .map_err(|error| error.to_string()),
    }
}

// The three functions below mirror `crates/app/src/player.rs` in the
// Apteronotus tree, which keeps them `pub(crate)`. They are preflight checks
// over public `Program` data rather than engine internals, so restating them
// is cheaper than asking that repository for an embedding surface it does not
// yet have — and `tests::every_preset_is_playable` is what catches it if the
// two ever disagree about what "playable" means.

fn playable_channels(program: &Program) -> Result<usize, String> {
    let persistent = needs_persistent_runtime(program);
    if program.tracks.is_empty() && program.runs.is_empty() {
        return Err("this soundscape has nothing to play".into());
    }

    let mut channels = persistent.then(|| program.buses.main_channels());
    for (index, track) in program.tracks.iter().enumerate() {
        let graph = program
            .voice(track.voice)
            .ok_or_else(|| format!("track {index} refers to a missing voice"))?;
        if graph.inputs != 0 {
            return Err(format!(
                "track {index} wants a live audio input, which the study app does not open"
            ));
        }
        if graph.channels() == 0 {
            return Err(format!("track {index} has no audio outputs"));
        }
        match channels {
            // Routed lowering broadcasts a mono voice across the persistent
            // main layout, so a mono track is not a mismatch there.
            Some(expected)
                if graph.channels() != expected && !(persistent && graph.channels() == 1) =>
            {
                return Err(format!(
                    "track {index} outputs {} channels, but earlier tracks output {expected}",
                    graph.channels()
                ));
            }
            None => channels = Some(graph.channels()),
            Some(_) => {}
        }
    }
    Ok(channels.expect("a playable program has a track or a persistent main layout"))
}

fn needs_persistent_runtime(program: &Program) -> bool {
    !program.runs.is_empty()
        || !program.controls.specs().is_empty()
        || program.buses.total_channels() != program.buses.main_channels()
        || program.voices.iter().any(|voice| !voice.sends.is_empty())
}

fn persistent_runtime(program: &Program) -> Result<PersistentRuntime, String> {
    let patches = program
        .runs
        .iter()
        .filter(|run| run.span.is_none())
        .enumerate()
        .map(|(index, run)| {
            program
                .patch(run.patch)
                .ok_or_else(|| format!("run {index} refers to a missing patch"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    PersistentRuntime::new(&program.buses, &program.controls, patches)
        .map_err(|error| error.to_string())
}

fn scheduled_runs(program: &Program) -> Result<Vec<ScheduledRun<'_>>, String> {
    program
        .runs
        .iter()
        .enumerate()
        .filter_map(|(index, run)| {
            run.span.map(|span| {
                program
                    .patch(run.patch)
                    .map(|patch| ScheduledRun::with_routing(patch, span, &run.routing))
                    .ok_or_else(|| format!("run {index} refers to a missing patch"))
            })
        })
        .collect()
}

fn scheduled_tracks(program: &Program) -> Result<Vec<ScheduledTrack<'_>>, String> {
    program
        .tracks
        .iter()
        .enumerate()
        .map(|(index, track)| {
            let template = program
                .voice(track.voice)
                .ok_or_else(|| format!("track {index} refers to a missing voice"))?;
            Ok(ScheduledTrack::with_routing_and_bindings(
                &track.pattern,
                template,
                &track.routing,
                &track.onset_bindings,
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::soundscape::PRESETS;

    /// A preset that does not evaluate is worse than no preset, and the engine
    /// is a sibling repository that moves. This is the guard.
    #[test]
    fn every_preset_is_playable() {
        for preset in PRESETS {
            let program = apteronotus_lua::evaluate(preset.source).unwrap_or_else(|error| {
                panic!("preset {} failed to evaluate: {error}", preset.name)
            });
            let channels = playable_channels(&program)
                .unwrap_or_else(|error| panic!("preset {} is not playable: {error}", preset.name));
            assert_eq!(
                channels, 2,
                "preset {} should reach the stereo output the app opens",
                preset.name
            );
            if needs_persistent_runtime(&program) {
                persistent_runtime(&program).unwrap_or_else(|error| {
                    panic!("preset {} could not build its arena: {error}", preset.name)
                });
            }
        }
    }

    /// "New" hands over a document, and a document that does not evaluate is
    /// an error message where a starting point was promised.
    #[test]
    fn a_new_document_is_playable() {
        let program = apteronotus_lua::evaluate(crate::soundscape::NEW_SOURCE)
            .expect("the new-document template evaluates");
        assert_eq!(
            playable_channels(&program).expect("it is playable"),
            2,
            "it should reach the stereo output the app opens"
        );
    }

    /// The reason the master fader is a stage in the output rather than an
    /// Apteronotus `control` the score declares. Every real song ends with its
    /// own `master(...)`, and `master` may be declared only once, so there is
    /// no way to add a gain stage to one from outside. A score is *expected*
    /// to have no `volume` control; the fader must work anyway.
    #[test]
    fn the_presets_declare_no_volume_control_and_do_not_need_to() {
        for preset in PRESETS {
            let program = apteronotus_lua::evaluate(preset.source).expect("a preset evaluates");
            assert!(
                program.controls.id("volume").is_none(),
                "{} should be a verbatim copy of its song, with no volume control added",
                preset.name
            );
        }
    }

    /// The fader travels in decibels, and the app's own bounds are the
    /// engine's — a slider that could ask for a level the engine clamps away
    /// would be a slider with a dead end on it.
    #[test]
    fn the_fader_travels_the_engines_range_and_starts_below_unity() {
        assert_eq!(MIN_DECIBELS, f64::from(MasterGain::MIN_DECIBELS));
        assert_eq!(MAX_DECIBELS, f64::from(MasterGain::MAX_DECIBELS));
        assert!(MIN_DECIBELS < DEFAULT_DECIBELS && DEFAULT_DECIBELS < MAX_DECIBELS);
    }

    /// A level set while nothing is playing has to survive to the next stream,
    /// because "start" and "how loud" are separate acts and either may come
    /// first.
    #[test]
    fn the_level_is_kept_across_a_stream_that_does_not_exist_yet() {
        let mut player = Player::new();
        player.set_volume(-24.0);
        assert_eq!(player.decibels, -24.0);
        player.set_volume(20.0);
        assert_eq!(player.decibels, MasterGain::MAX_DECIBELS, "never a boost");
        player.set_volume(-400.0);
        assert_eq!(player.decibels, MasterGain::MIN_DECIBELS);
    }

    #[test]
    fn a_program_with_nothing_in_it_is_refused_before_a_device_is_opened() {
        let program = apteronotus_lua::evaluate("tempo(90)").expect("an empty program evaluates");
        assert!(playable_channels(&program).is_err());
    }

    /// The whole path, on a real device: evaluate, lower, open, fill, sound.
    ///
    /// Ignored by default because it needs an audio device — under Xvfb or in
    /// CI there is none, and a test that fails for want of hardware teaches
    /// nothing. Run it deliberately, on a machine with sound:
    ///
    /// ```text
    /// cargo test -p idiosepius-app -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "opens the audio device"]
    #[cfg(not(target_arch = "wasm32"))]
    fn the_default_soundscape_reaches_a_real_device() {
        let mut player = Player::new();
        player
            .play(crate::soundscape::default_source())
            .expect("the default soundscape should play");
        player.set_volume(-18.0);
        // Drive the lookahead for a moment, exactly as the worker loop does,
        // so a scheduling fault past the first window is caught too.
        for _ in 0..60 {
            player
                .fill_lookahead()
                .expect("lookahead should keep filling");
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        player.stop();
    }
}
