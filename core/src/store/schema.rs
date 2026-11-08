//! Versioned SQLite schema. Each entry in [`MIGRATIONS`] runs once, in order,
//! tracked by `PRAGMA user_version`. Never edit a shipped migration — append a
//! new one. Mirrors the macOS GRDB schema (snake_case, Unix-second timestamps).

pub const MIGRATIONS: &[&str] = &[
    // v1 — the original desktop tables (kept so old databases upgrade in place).
    "CREATE TABLE IF NOT EXISTS transcriptions (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        text        TEXT NOT NULL,
        app         TEXT,
        duration_ms INTEGER NOT NULL DEFAULT 0,
        word_count  INTEGER NOT NULL DEFAULT 0,
        created_at  INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS personal_memory (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        kind        TEXT NOT NULL,
        trigger     TEXT NOT NULL,
        replacement TEXT NOT NULL
    );",
    // v2 — Mac-parity transcription table with per-stage latency telemetry.
    "CREATE TABLE transcription (
        id                  INTEGER PRIMARY KEY AUTOINCREMENT,
        created_at          INTEGER NOT NULL,
        text                TEXT NOT NULL,
        raw_text            TEXT,
        app_name            TEXT,
        app_id              TEXT,
        recording_ms        INTEGER NOT NULL DEFAULT 0,
        processing_ms       INTEGER NOT NULL DEFAULT 0,
        transcription_ms    INTEGER,
        text_processing_ms  INTEGER,
        polish_ms           INTEGER,
        injection_ms        INTEGER,
        model_id            TEXT NOT NULL DEFAULT '',
        audio_device        TEXT,
        injection_method    TEXT,
        entry_kind          TEXT NOT NULL DEFAULT 'dictation',
        edit_instruction    TEXT,
        word_count          INTEGER NOT NULL DEFAULT 0,
        time_zone           TEXT,
        polished            INTEGER NOT NULL DEFAULT 0
    );
    CREATE INDEX idx_transcription_created ON transcription (created_at DESC);
    CREATE INDEX idx_transcription_app ON transcription (app_name);
    INSERT INTO transcription (id, created_at, text, app_name, recording_ms, word_count)
        SELECT id, created_at, text, app, duration_ms, word_count FROM transcriptions;
    DROP TABLE transcriptions;

    CREATE TABLE dictionary_entry (
        id                INTEGER PRIMARY KEY AUTOINCREMENT,
        source            TEXT NOT NULL,
        target            TEXT NOT NULL,
        normalized_source TEXT NOT NULL UNIQUE,
        is_case_sensitive INTEGER NOT NULL DEFAULT 0,
        is_active         INTEGER NOT NULL DEFAULT 1,
        usage_count       INTEGER NOT NULL DEFAULT 0,
        created_at        INTEGER NOT NULL,
        updated_at        INTEGER NOT NULL
    );
    CREATE TABLE snippet_entry (
        id                 INTEGER PRIMARY KEY AUTOINCREMENT,
        trigger            TEXT NOT NULL,
        expansion          TEXT NOT NULL,
        normalized_trigger TEXT NOT NULL UNIQUE,
        is_active          INTEGER NOT NULL DEFAULT 1,
        usage_count        INTEGER NOT NULL DEFAULT 0,
        created_at         INTEGER NOT NULL,
        updated_at         INTEGER NOT NULL
    );
    INSERT OR IGNORE INTO dictionary_entry (source, target, normalized_source, created_at, updated_at)
        SELECT trigger, replacement, lower(trim(trigger)), strftime('%s','now'), strftime('%s','now')
        FROM personal_memory WHERE kind != 'snippet';
    INSERT OR IGNORE INTO snippet_entry (trigger, expansion, normalized_trigger, created_at, updated_at)
        SELECT trigger, replacement, lower(trim(trigger)), strftime('%s','now'), strftime('%s','now')
        FROM personal_memory WHERE kind = 'snippet';
    DROP TABLE personal_memory;

    CREATE TABLE learning_suggestion (
        id                INTEGER PRIMARY KEY AUTOINCREMENT,
        kind              TEXT NOT NULL,
        candidate_source  TEXT NOT NULL,
        candidate_target  TEXT NOT NULL,
        normalized_source TEXT NOT NULL,
        confidence        REAL NOT NULL DEFAULT 0,
        status            TEXT NOT NULL DEFAULT 'pending',
        evidence          TEXT,
        created_at        INTEGER NOT NULL,
        updated_at        INTEGER NOT NULL,
        UNIQUE (kind, normalized_source, candidate_target)
    );
    CREATE TABLE edit_check (
        id            INTEGER PRIMARY KEY AUTOINCREMENT,
        checked_at    INTEGER NOT NULL,
        app_name      TEXT,
        outcome       TEXT NOT NULL,
        learned_count INTEGER NOT NULL DEFAULT 0
    );

    CREATE TABLE writing_insight_snapshot (
        id            INTEGER PRIMARY KEY AUTOINCREMENT,
        kind          TEXT NOT NULL DEFAULT 'writing',
        generated_at  INTEGER NOT NULL,
        analyzer_name TEXT NOT NULL,
        used_fallback INTEGER NOT NULL DEFAULT 0,
        sample_count  INTEGER NOT NULL,
        request_json  TEXT NOT NULL,
        snapshot_json TEXT NOT NULL,
        is_pinned     INTEGER NOT NULL DEFAULT 0
    );
    CREATE INDEX idx_wis_kind ON writing_insight_snapshot (kind, generated_at DESC);

    CREATE TABLE chat_conversation (
        id         INTEGER PRIMARY KEY AUTOINCREMENT,
        title      TEXT NOT NULL,
        model      TEXT,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
    );
    CREATE TABLE chat_message (
        id              INTEGER PRIMARY KEY AUTOINCREMENT,
        conversation_id INTEGER NOT NULL REFERENCES chat_conversation(id) ON DELETE CASCADE,
        role            TEXT NOT NULL,
        content         TEXT NOT NULL,
        reasoning       TEXT,
        sources_json    TEXT,
        created_at      INTEGER NOT NULL
    );
    CREATE INDEX idx_chat_message_conv ON chat_message (conversation_id, id);

    CREATE TABLE semantic_chunk (
        id               INTEGER PRIMARY KEY AUTOINCREMENT,
        transcription_id INTEGER NOT NULL,
        chunk_index      INTEGER NOT NULL,
        text             TEXT NOT NULL,
        text_hash        TEXT NOT NULL,
        source_created_at INTEGER NOT NULL,
        app_name         TEXT,
        word_count       INTEGER NOT NULL,
        created_at       INTEGER NOT NULL,
        updated_at       INTEGER NOT NULL,
        UNIQUE (transcription_id, chunk_index)
    );
    CREATE TABLE semantic_embedding (
        id            INTEGER PRIMARY KEY AUTOINCREMENT,
        chunk_id      INTEGER NOT NULL REFERENCES semantic_chunk(id) ON DELETE CASCADE,
        model_id      TEXT NOT NULL,
        provider_name TEXT NOT NULL,
        dimension     INTEGER NOT NULL,
        vector        BLOB NOT NULL,
        generated_at  INTEGER NOT NULL,
        UNIQUE (chunk_id, model_id)
    );
    CREATE TABLE semantic_graph_node (
        node_id      TEXT PRIMARY KEY,
        kind         TEXT NOT NULL,
        title        TEXT NOT NULL,
        subtitle     TEXT,
        weight       REAL NOT NULL,
        last_seen_at INTEGER,
        updated_at   INTEGER NOT NULL
    );
    CREATE TABLE semantic_graph_edge (
        id             INTEGER PRIMARY KEY AUTOINCREMENT,
        source_node_id TEXT NOT NULL,
        target_node_id TEXT NOT NULL,
        kind           TEXT NOT NULL,
        weight         REAL NOT NULL,
        evidence       TEXT,
        updated_at     INTEGER NOT NULL,
        UNIQUE (source_node_id, target_node_id, kind)
    );
    CREATE TABLE semantic_insight_snapshot (
        id                 INTEGER PRIMARY KEY AUTOINCREMENT,
        generated_at       INTEGER NOT NULL,
        graph_signature    TEXT NOT NULL,
        analyzer_name      TEXT NOT NULL,
        summary_model_name TEXT,
        source_node_count  INTEGER NOT NULL,
        source_edge_count  INTEGER NOT NULL,
        source_chunk_count INTEGER NOT NULL,
        report_json        TEXT NOT NULL
    );
    CREATE TABLE semantic_signal (
        id           INTEGER PRIMARY KEY AUTOINCREMENT,
        chunk_id     INTEGER NOT NULL,
        family       TEXT NOT NULL,
        value        TEXT NOT NULL,
        confidence   REAL NOT NULL,
        model_id     TEXT NOT NULL,
        extracted_at INTEGER NOT NULL
    );
    CREATE INDEX idx_semantic_signal_chunk ON semantic_signal (chunk_id);
    CREATE TABLE insight_finding (
        id                 INTEGER PRIMARY KEY AUTOINCREMENT,
        kind               TEXT NOT NULL,
        subject_key        TEXT NOT NULL,
        title              TEXT NOT NULL,
        detail             TEXT NOT NULL,
        magnitude          REAL NOT NULL DEFAULT 0,
        confidence         REAL NOT NULL DEFAULT 0.5,
        window_start       INTEGER,
        window_end         INTEGER,
        evidence_chunk_ids TEXT NOT NULL DEFAULT '[]',
        first_seen_at      INTEGER NOT NULL,
        last_computed_at   INTEGER NOT NULL,
        last_shown_at      INTEGER,
        status             TEXT NOT NULL DEFAULT 'active',
        UNIQUE (kind, subject_key)
    );
    CREATE TABLE concept_extraction (
        transcription_id  INTEGER PRIMARY KEY,
        text_hash         TEXT NOT NULL,
        extractor_version INTEGER NOT NULL,
        concepts_json     TEXT NOT NULL,
        extracted_at      INTEGER NOT NULL
    );
    CREATE TABLE session_analysis (
        session_key    TEXT PRIMARY KEY,
        prompt_version INTEGER NOT NULL,
        model          TEXT NOT NULL,
        analysis_json  TEXT NOT NULL,
        session_end    INTEGER NOT NULL,
        analyzed_at    INTEGER NOT NULL
    );
    CREATE TABLE semantic_index_queue (
        transcription_id INTEGER PRIMARY KEY,
        enqueued_at      INTEGER NOT NULL,
        attempts         INTEGER NOT NULL DEFAULT 0,
        last_error       TEXT
    );

    CREATE TABLE milestone_unlock (
        badge_id      TEXT PRIMARY KEY,
        unlocked_at   INTEGER NOT NULL,
        celebrated_at INTEGER
    );

    CREATE TABLE coach_metric (
        transcription_id    INTEGER PRIMARY KEY,
        metrics_version     INTEGER NOT NULL,
        words               INTEGER NOT NULL,
        um_uh               INTEGER NOT NULL,
        phrase_fillers      INTEGER NOT NULL,
        wpm                 INTEGER,
        sentence_lengths    TEXT NOT NULL,
        repeats             INTEGER NOT NULL,
        polish_change_ratio REAL,
        raw_known           INTEGER NOT NULL
    );
    CREATE TABLE coach_habit (
        id         INTEGER PRIMARY KEY AUTOINCREMENT,
        measure    TEXT NOT NULL,
        mode       TEXT,
        started_at INTEGER NOT NULL,
        ended_at   INTEGER
    );
    CREATE TABLE coach_rep (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        skill_id    TEXT NOT NULL,
        kind        TEXT NOT NULL,
        spoken      INTEGER NOT NULL,
        created_at  INTEGER NOT NULL,
        score       REAL,
        skill_met   INTEGER,
        duration_ms INTEGER NOT NULL,
        words       INTEGER NOT NULL,
        checks      TEXT NOT NULL,
        persona     TEXT NOT NULL,
        transcript  TEXT
    );
    CREATE INDEX idx_coach_rep_skill ON coach_rep (skill_id, created_at);
    CREATE TABLE coach_judgement (
        transcription_id INTEGER NOT NULL,
        question         TEXT NOT NULL,
        model            TEXT NOT NULL,
        answer           TEXT NOT NULL,
        confidence       REAL NOT NULL,
        judged_at        INTEGER NOT NULL,
        PRIMARY KEY (transcription_id, question, model)
    );

    CREATE TABLE kv (
        key        TEXT PRIMARY KEY,
        value      TEXT,
        updated_at INTEGER NOT NULL
    );",

    // v3 — Orttaai Link (docs/orttaai-link.md): sync identities on the synced
    // tables, the sync ledger, and triggers that note every local change while
    // this PC is linked to a Mac (sync_state.capture = '1'; sync turns it off
    // while it applies records from the Mac).
    "ALTER TABLE transcription ADD COLUMN sync_id TEXT;
    ALTER TABLE transcription ADD COLUMN source_device_id TEXT;
    ALTER TABLE transcription ADD COLUMN source_device_kind TEXT;
    CREATE UNIQUE INDEX idx_transcription_sync_id ON transcription (sync_id);
    ALTER TABLE learning_suggestion ADD COLUMN sync_id TEXT;
    CREATE UNIQUE INDEX idx_learning_suggestion_sync_id ON learning_suggestion (sync_id);
    ALTER TABLE writing_insight_snapshot ADD COLUMN sync_id TEXT;
    CREATE UNIQUE INDEX idx_wis_sync_id ON writing_insight_snapshot (sync_id);

    CREATE TABLE sync_state (
        key   TEXT PRIMARY KEY,
        value TEXT
    );
    INSERT INTO sync_state (key, value) VALUES ('capture', '0');

    CREATE TABLE sync_record (
        record_name      TEXT PRIMARY KEY,
        zone             TEXT NOT NULL,
        kind             TEXT NOT NULL,
        local_table      TEXT,
        local_id         INTEGER,
        rev_counter      INTEGER NOT NULL,
        rev_modified_ms  INTEGER NOT NULL,
        rev_writer       TEXT NOT NULL,
        generation       INTEGER NOT NULL DEFAULT 0,
        is_deleted       INTEGER NOT NULL DEFAULT 0,
        deleted_at_ms    INTEGER,
        origin_device_id TEXT,
        origin_kind      TEXT,
        created_at_ms    INTEGER,
        schema_version   INTEGER NOT NULL DEFAULT 1,
        payload          BLOB
    );
    CREATE INDEX idx_sync_record_local ON sync_record (local_table, local_id);

    CREATE TABLE sync_outbox (
        record_name     TEXT PRIMARY KEY,
        rev_counter     INTEGER NOT NULL,
        rev_modified_ms INTEGER NOT NULL,
        rev_writer      TEXT NOT NULL,
        failed_reason   TEXT,
        enqueued_at     INTEGER NOT NULL
    );

    CREATE TABLE sync_pending (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        local_table TEXT NOT NULL,
        local_id    INTEGER NOT NULL,
        deleted     INTEGER NOT NULL DEFAULT 0,
        sync_key    TEXT
    );

    CREATE TRIGGER sync_transcription_insert AFTER INSERT ON transcription
    WHEN (SELECT value FROM sync_state WHERE key = 'capture') = '1'
    BEGIN INSERT INTO sync_pending (local_table, local_id) VALUES ('transcription', NEW.id); END;
    CREATE TRIGGER sync_transcription_update AFTER UPDATE OF text, raw_text, app_name, app_id, recording_ms,
        processing_ms, transcription_ms, text_processing_ms, injection_ms, model_id, audio_device,
        injection_method, entry_kind, edit_instruction, time_zone, created_at ON transcription
    WHEN (SELECT value FROM sync_state WHERE key = 'capture') = '1'
    BEGIN INSERT INTO sync_pending (local_table, local_id) VALUES ('transcription', NEW.id); END;
    CREATE TRIGGER sync_transcription_delete AFTER DELETE ON transcription
    WHEN (SELECT value FROM sync_state WHERE key = 'capture') = '1'
    BEGIN INSERT INTO sync_pending (local_table, local_id, deleted, sync_key) VALUES ('transcription', OLD.id, 1, OLD.sync_id); END;

    CREATE TRIGGER sync_dictionary_insert AFTER INSERT ON dictionary_entry
    WHEN (SELECT value FROM sync_state WHERE key = 'capture') = '1'
    BEGIN INSERT INTO sync_pending (local_table, local_id) VALUES ('dictionary_entry', NEW.id); END;
    CREATE TRIGGER sync_dictionary_update AFTER UPDATE OF source, target, normalized_source, is_case_sensitive, is_active ON dictionary_entry
    WHEN (SELECT value FROM sync_state WHERE key = 'capture') = '1'
    BEGIN INSERT INTO sync_pending (local_table, local_id) VALUES ('dictionary_entry', NEW.id); END;
    CREATE TRIGGER sync_dictionary_delete AFTER DELETE ON dictionary_entry
    WHEN (SELECT value FROM sync_state WHERE key = 'capture') = '1'
    BEGIN INSERT INTO sync_pending (local_table, local_id, deleted, sync_key) VALUES ('dictionary_entry', OLD.id, 1, OLD.normalized_source); END;

    CREATE TRIGGER sync_snippet_insert AFTER INSERT ON snippet_entry
    WHEN (SELECT value FROM sync_state WHERE key = 'capture') = '1'
    BEGIN INSERT INTO sync_pending (local_table, local_id) VALUES ('snippet_entry', NEW.id); END;
    CREATE TRIGGER sync_snippet_update AFTER UPDATE OF trigger, expansion, normalized_trigger, is_active ON snippet_entry
    WHEN (SELECT value FROM sync_state WHERE key = 'capture') = '1'
    BEGIN INSERT INTO sync_pending (local_table, local_id) VALUES ('snippet_entry', NEW.id); END;
    CREATE TRIGGER sync_snippet_delete AFTER DELETE ON snippet_entry
    WHEN (SELECT value FROM sync_state WHERE key = 'capture') = '1'
    BEGIN INSERT INTO sync_pending (local_table, local_id, deleted, sync_key) VALUES ('snippet_entry', OLD.id, 1, OLD.normalized_trigger); END;

    CREATE TRIGGER sync_suggestion_insert AFTER INSERT ON learning_suggestion
    WHEN (SELECT value FROM sync_state WHERE key = 'capture') = '1'
    BEGIN INSERT INTO sync_pending (local_table, local_id) VALUES ('learning_suggestion', NEW.id); END;
    CREATE TRIGGER sync_suggestion_update AFTER UPDATE OF candidate_target, confidence, status, evidence ON learning_suggestion
    WHEN (SELECT value FROM sync_state WHERE key = 'capture') = '1'
    BEGIN INSERT INTO sync_pending (local_table, local_id) VALUES ('learning_suggestion', NEW.id); END;
    CREATE TRIGGER sync_suggestion_delete AFTER DELETE ON learning_suggestion
    WHEN (SELECT value FROM sync_state WHERE key = 'capture') = '1'
    BEGIN INSERT INTO sync_pending (local_table, local_id, deleted, sync_key) VALUES ('learning_suggestion', OLD.id, 1, OLD.sync_id); END;

    CREATE TRIGGER sync_insight_insert AFTER INSERT ON writing_insight_snapshot
    WHEN NEW.kind = 'writing' AND (SELECT value FROM sync_state WHERE key = 'capture') = '1'
    BEGIN INSERT INTO sync_pending (local_table, local_id) VALUES ('writing_insight_snapshot', NEW.id); END;
    CREATE TRIGGER sync_insight_update AFTER UPDATE OF is_pinned ON writing_insight_snapshot
    WHEN NEW.kind = 'writing' AND (SELECT value FROM sync_state WHERE key = 'capture') = '1'
    BEGIN INSERT INTO sync_pending (local_table, local_id) VALUES ('writing_insight_snapshot', NEW.id); END;
    CREATE TRIGGER sync_insight_delete AFTER DELETE ON writing_insight_snapshot
    WHEN OLD.kind = 'writing' AND (SELECT value FROM sync_state WHERE key = 'capture') = '1'
    BEGIN INSERT INTO sync_pending (local_table, local_id, deleted, sync_key) VALUES ('writing_insight_snapshot', OLD.id, 1, OLD.sync_id); END;",
];
