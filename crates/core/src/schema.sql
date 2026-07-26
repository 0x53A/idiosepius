-- Idiosepius study database.
--
-- One file holds everything: the study content (decks, topics, questions) and
-- the full record of what the user did with it (sessions, events, attempts,
-- scheduler state). Copying the .db file moves the whole course *and* the
-- learning history with it.
--
-- All timestamps are milliseconds since the Unix epoch, UTC.

CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- ---------------------------------------------------------------- content --

CREATE TABLE IF NOT EXISTS deck (
    id          INTEGER PRIMARY KEY,
    slug        TEXT NOT NULL UNIQUE,
    title       TEXT NOT NULL,
    description TEXT,
    -- When the exam is, if any. The scheduler refuses to park a card past it.
    exam_at     INTEGER,
    created_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS topic (
    id      INTEGER PRIMARY KEY,
    deck_id INTEGER NOT NULL REFERENCES deck(id) ON DELETE CASCADE,
    slug    TEXT NOT NULL,
    title   TEXT NOT NULL,
    ord     INTEGER NOT NULL DEFAULT 0,
    UNIQUE (deck_id, slug)
);

CREATE TABLE IF NOT EXISTS question (
    id          INTEGER PRIMARY KEY,
    deck_id     INTEGER NOT NULL REFERENCES deck(id) ON DELETE CASCADE,
    topic_id    INTEGER REFERENCES topic(id) ON DELETE SET NULL,
    -- Stable authored identifier, so re-importing a pack updates rather than
    -- duplicates, and the history in `attempt` survives content edits.
    uid         TEXT NOT NULL UNIQUE,
    kind        TEXT NOT NULL,
    -- Ordered `ContentBlock` JSON: prose and any number of figures.
    prompt      TEXT NOT NULL,
    -- Kind-specific JSON (the `Body` enum). Kept as a blob so new question
    -- kinds do not need a schema migration.
    payload     TEXT NOT NULL,
    explanation TEXT,
    -- The structured explanation (the `Explain` type): a short and a deep
    -- reading, each a list of raw text and references into `fact`. JSON,
    -- because it is authored content that only the UI ever interprets.
    explain     TEXT,
    difficulty  INTEGER NOT NULL DEFAULT 2,
    source      TEXT,
    tags        TEXT NOT NULL DEFAULT '[]',
    active      INTEGER NOT NULL DEFAULT 1,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS question_deck_idx  ON question(deck_id, active);
CREATE INDEX IF NOT EXISTS question_topic_idx ON question(topic_id);

-- Explanations that several questions share.
--
-- Ten variants of one idea should not carry ten copies of the same paragraph:
-- they drift apart, and fixing a mistake means finding every copy. A question's
-- explanation is therefore a list of raw text and references into this table.
-- Symbols (`kind = 'symbol'`) are the same thing at the smallest scale — one
-- Greek letter, its name, and what it stands for.
CREATE TABLE IF NOT EXISTS fact (
    id         INTEGER PRIMARY KEY,
    deck_id    INTEGER REFERENCES deck(id) ON DELETE CASCADE,
    uid        TEXT NOT NULL UNIQUE,
    kind       TEXT NOT NULL DEFAULT 'note',
    -- For a symbol: the glyph itself ("ζ") and what to call it ("zeta").
    label      TEXT,
    name       TEXT,
    title      TEXT,
    -- Ordered `ContentBlock` JSON: prose and any number of figures.
    body       TEXT NOT NULL,
    source     TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS fact_deck_idx ON fact(deck_id);

-- ------------------------------------------------------------------- log --

CREATE TABLE IF NOT EXISTS session (
    id          INTEGER PRIMARY KEY,
    deck_id     INTEGER REFERENCES deck(id) ON DELETE SET NULL,
    mode        TEXT NOT NULL,
    started_at  INTEGER NOT NULL,
    ended_at    INTEGER,
    app_version TEXT NOT NULL
);

-- Append-only firehose. Everything the user does lands here, including things
-- that never become an attempt (cancelled swipes, reveals, undos, pauses).
-- Nothing in the app is allowed to UPDATE or DELETE from this table.
CREATE TABLE IF NOT EXISTS event (
    id          INTEGER PRIMARY KEY,
    session_id  INTEGER NOT NULL REFERENCES session(id) ON DELETE CASCADE,
    ts          INTEGER NOT NULL,
    -- Milliseconds since session start, from a monotonic clock: survives the
    -- wall clock being changed mid-session.
    mono_ms     INTEGER NOT NULL,
    question_id INTEGER REFERENCES question(id) ON DELETE SET NULL,
    kind        TEXT NOT NULL,
    data        TEXT
);

CREATE INDEX IF NOT EXISTS event_session_idx  ON event(session_id, id);
CREATE INDEX IF NOT EXISTS event_question_idx ON event(question_id);

-- One row per committed answer. Derivable from `event`, but materialised
-- because every stats query and the scheduler want it.
CREATE TABLE IF NOT EXISTS attempt (
    id           INTEGER PRIMARY KEY,
    session_id   INTEGER NOT NULL REFERENCES session(id) ON DELETE CASCADE,
    question_id  INTEGER NOT NULL REFERENCES question(id) ON DELETE CASCADE,
    ts           INTEGER NOT NULL,
    -- Card shown -> answer committed. Hesitation is signal.
    latency_ms   INTEGER NOT NULL,
    correct      INTEGER NOT NULL,
    score        REAL NOT NULL DEFAULT 0.0,
    response     TEXT NOT NULL,
    input_method TEXT,
    -- Scheduler box before this attempt, for reconstructing the curve later.
    box_before   INTEGER
);

CREATE INDEX IF NOT EXISTS attempt_question_idx ON attempt(question_id, ts);
CREATE INDEX IF NOT EXISTS attempt_session_idx  ON attempt(session_id);

-- ------------------------------------------------------------- scheduling --

CREATE TABLE IF NOT EXISTS review_state (
    question_id   INTEGER PRIMARY KEY REFERENCES question(id) ON DELETE CASCADE,
    box           INTEGER NOT NULL DEFAULT 0,
    due_at        INTEGER NOT NULL,
    last_seen_at  INTEGER,
    streak        INTEGER NOT NULL DEFAULT 0,
    lapses        INTEGER NOT NULL DEFAULT 0,
    seen_count    INTEGER NOT NULL DEFAULT 0,
    correct_count INTEGER NOT NULL DEFAULT 0,
    -- Exponential moving average of correctness, 0..1. Reacts faster than the
    -- lifetime ratio, which is what you want the day before an exam.
    ema           REAL NOT NULL DEFAULT 0.0
);

CREATE INDEX IF NOT EXISTS review_due_idx ON review_state(due_at);
