//! SQLite Sink.
//!
//! Writes structured log records to a local SQLite database.
//!
//! # Design constraints
//!
//! - **Low-throughput auxiliary**: not suitable for >100K rec/s production
//! - `synchronous = OFF` + `journal_mode = WAL` for write performance
//! - Stores Ring 0+1 fields as structured columns
//!
//! # Feature flag
//!
//! Compile with `--features sink-sqlite` to enable the `rusqlite` dependency.

#[cfg(feature = "sink-sqlite")]
use rusqlite::{params, Connection, OpenFlags};

use std::path::PathBuf;
use std::sync::Mutex;

use crate::record::Record;
use crate::sink::{Sink, SinkError, SinkResult};

/// SQLite Sink configuration.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(default)]
pub struct SqliteSinkConfig {
    /// Path to the SQLite database file
    pub path: PathBuf,
    /// Maximum records before auto-vacuum (0 = disabled)
    pub max_records: u64,
    /// Whether to WAL-checkpoint on close
    pub wal_checkpoint_on_close: bool,
}

impl Default for SqliteSinkConfig {
    fn default() -> Self {
        Self {
            path: PathBuf::from("dologger.db"),
            max_records: 0,
            wal_checkpoint_on_close: true,
        }
    }
}

/// SQLite Sink — writes structured records to SQLite.
///
/// Uses WAL journal mode with synchronous=OFF for maximum write throughput.
/// Not suitable for AUDIT domain (use `sink_worm_file` instead).
///
/// The connection is wrapped in a Mutex for `Sync` — only one thread writes at a time.
pub struct SqliteSink {
    config: SqliteSinkConfig,
    #[cfg(feature = "sink-sqlite")]
    conn: Mutex<Option<Connection>>,
    #[cfg(not(feature = "sink-sqlite"))]
    _conn_stub: (),
    records_written: u64,
    is_open: bool,
}

// SAFETY: SqliteSink uses Mutex<Option<Connection>> for interior mutability
// with exclusive write access. All internal state changes happen under lock.
unsafe impl Sync for SqliteSink {}

impl SqliteSink {
    /// Create a new SQLite sink with the given config.
    pub fn new(config: SqliteSinkConfig) -> Self {
        Self {
            config,
            #[cfg(feature = "sink-sqlite")]
            conn: Mutex::new(None),
            #[cfg(not(feature = "sink-sqlite"))]
            _conn_stub: (),
            records_written: 0,
            is_open: false,
        }
    }

    /// Create with a simple database path.
    pub fn with_path(path: impl Into<PathBuf>) -> Self {
        Self::new(SqliteSinkConfig {
            path: path.into(),
            ..Default::default()
        })
    }
}

impl Sink for SqliteSink {
    fn open(&mut self) -> SinkResult {
        #[cfg(feature = "sink-sqlite")]
        {
            // Create parent directory if needed
            if let Some(parent) = self.config.path.parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| SinkError::WriteFailed(format!("mkdir: {e}")))?;
                }
            }

            let conn = Connection::open_with_flags(
                &self.config.path,
                OpenFlags::SQLITE_OPEN_READ_WRITE
                    | OpenFlags::SQLITE_OPEN_CREATE
                    | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )
            .map_err(|e| SinkError::WriteFailed(format!("sqlite open: {e}")))?;

            // Configure for write performance
            conn.execute_batch(
                "PRAGMA journal_mode=WAL;
                 PRAGMA synchronous=OFF;
                 PRAGMA cache_size=-8000;
                 PRAGMA mmap_size=67108864;",
            )
            .map_err(|e| SinkError::WriteFailed(format!("sqlite pragma: {e}")))?;

            // Keep legacy columns nullable so existing databases remain readable.
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS dologger_records (
                    id          INTEGER PRIMARY KEY AUTOINCREMENT,
                    record_id   TEXT,
                    timestamp_ns INTEGER,
                    level       TEXT NOT NULL,
                    message     TEXT NOT NULL,
                    source_file TEXT,
                    source_line INTEGER,
                    source_func TEXT,
                    thread_id   INTEGER,
                    thread_name TEXT,
                    process_id  INTEGER,
                    process_name TEXT,
                    host_name   TEXT,
                    app_name    TEXT,
                    app_version TEXT,
                    environment TEXT,
                    user_id     TEXT,
                    session_id  TEXT,
                    request_id  TEXT,
                    trace_id    TEXT,
                     span_id     TEXT,
                     lsn         INTEGER,
                     signature   BLOB,
                     ext_data    TEXT,
                     content_hash BLOB,
                     created_at  TEXT NOT NULL DEFAULT (datetime('now'))
                );

                CREATE INDEX IF NOT EXISTS idx_level ON dologger_records(level);
                CREATE INDEX IF NOT EXISTS idx_timestamp ON dologger_records(timestamp_ns);
                CREATE INDEX IF NOT EXISTS idx_lsn ON dologger_records(lsn);",
            )
            .map_err(|e| SinkError::WriteFailed(format!("sqlite ddl: {e}")))?;

            ensure_content_hash_column(&conn)?;

            *self.conn.lock().unwrap() = Some(conn);
            self.is_open = true;
            Ok(())
        }
        #[cfg(not(feature = "sink-sqlite"))]
        {
            Err(SinkError::WriteFailed(
                "SQLite Sink: compiled without 'sink-sqlite' feature".into(),
            ))
        }
    }

    fn write(&mut self, formatted: &str) -> SinkResult {
        #[cfg(feature = "sink-sqlite")]
        {
            let guard = self.conn.lock().unwrap();
            let conn = guard.as_ref().ok_or(SinkError::Closed)?;

            // Use the formatted string as the message; extract level from prefix
            // Format: "[secs.millis] [LEVEL] [thread_id] message"
            let (level, message) = parse_formatted_line(formatted);

            conn.execute(
                "INSERT INTO dologger_records (level, message, created_at)
                 VALUES (?1, ?2, datetime('now'))",
                rusqlite::params![level, message],
            )
            .map_err(|e| SinkError::WriteFailed(format!("sqlite insert: {e}")))?;

            self.records_written += 1;
            Ok(())
        }
        #[cfg(not(feature = "sink-sqlite"))]
        {
            let _ = formatted;
            Err(SinkError::WriteFailed(
                "SQLite Sink: compiled without 'sink-sqlite' feature".into(),
            ))
        }
    }

    fn flush(&mut self) -> SinkResult {
        #[cfg(feature = "sink-sqlite")]
        if let Some(ref conn) = *self.conn.lock().unwrap() {
            conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE);")
                .map_err(|e| SinkError::WriteFailed(format!("sqlite checkpoint: {e}")))?;
        }
        Ok(())
    }

    fn close(&mut self) -> SinkResult {
        #[cfg(feature = "sink-sqlite")]
        {
            if let Some(conn) = self.conn.lock().unwrap().take() {
                if self.config.wal_checkpoint_on_close {
                    conn.execute_batch(
                        "PRAGMA wal_checkpoint(TRUNCATE);
                         PRAGMA optimize;",
                    )
                    .map_err(|e| SinkError::WriteFailed(format!("sqlite close: {e}")))?;
                }
            }
        }
        self.is_open = false;
        Ok(())
    }
}

impl SqliteSink {
    /// Write a structured record directly to SQLite (hot path).
    ///
    /// This is the preferred API — it inserts typed columns rather than
    /// relying on text formatting.
    pub fn write_record(&mut self, record: &Record) -> SinkResult {
        #[cfg(feature = "sink-sqlite")]
        {
            let guard = self.conn.lock().unwrap();
            let conn = guard.as_ref().ok_or(SinkError::Closed)?;

            let record_id = format!("{:016x}{:016x}", record.id_hi(), record.id_lo());
            let timestamp_ns = record.timestamp as i64;
            let level = record.level.to_str();
            let message = match record.message.as_utf8() {
                Ok(text) => rusqlite::types::Value::Text(text.to_owned()),
                Err(_) => rusqlite::types::Value::Blob(record.message.as_bytes().to_vec()),
            };
            let source_file = record.source_file();
            let source_func = record.source_function();
            let thread_name = record.thread_name();
            let process_name = record.process_name();
            let host_name = record.host_name();
            let app_name = record.app_name();
            let app_version = record.app_version();
            let environment = record.environment();
            let user_id = record.user_id();
            let session_id = record.session_id();
            let request_id = record.request_id();
            let trace_id = record.trace_id();
            let span_id = record.span_id();
            let content_hash = &record.content_hash;

            conn.execute(
                "INSERT INTO dologger_records
                    (record_id, timestamp_ns, level, message,
                     source_file, source_line, source_func,
                     thread_id, thread_name, process_id, process_name,
                     host_name, app_name, app_version, environment,
                     user_id, session_id, request_id, trace_id, span_id,
                     lsn, content_hash)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22)",
                params![
                    record_id,
                    timestamp_ns,
                    level,
                    message,
                    if source_file.is_empty() { None } else { Some(source_file) },
                    record.source_line() as i64,
                    if source_func.is_empty() { None } else { Some(source_func) },
                    record.thread_id as i64,
                    if thread_name.is_empty() { None } else { Some(thread_name) },
                    record.process_id as i64,
                    if process_name.is_empty() { None } else { Some(process_name) },
                    if host_name.is_empty() { None } else { Some(host_name) },
                    if app_name.is_empty() { None } else { Some(app_name) },
                    if app_version.is_empty() { None } else { Some(app_version) },
                    if environment.is_empty() { None } else { Some(environment) },
                    if user_id.is_empty() { None } else { Some(user_id) },
                    if session_id.is_empty() { None } else { Some(session_id) },
                    if request_id.is_empty() { None } else { Some(request_id) },
                    if trace_id.is_empty() { None } else { Some(trace_id) },
                    if span_id.is_empty() { None } else { Some(span_id) },
                    record.lsn as i64,
                    content_hash.as_slice(),
                ],
            )
            .map_err(|e| SinkError::WriteFailed(format!("sqlite insert: {e}")))?;

            self.records_written += 1;

            // Auto-vacuum check
            if self.config.max_records > 0
                && self.records_written.is_multiple_of(self.config.max_records)
            {
                conn.execute("DELETE FROM dologger_records WHERE id NOT IN (SELECT id FROM dologger_records ORDER BY id DESC LIMIT ?1)",
                    params![self.config.max_records as i64])
                    .map_err(|e| SinkError::WriteFailed(format!("sqlite vacuum: {e}")))?;
            }

            Ok(())
        }
        #[cfg(not(feature = "sink-sqlite"))]
        {
            let _ = record;
            Err(SinkError::WriteFailed(
                "SQLite Sink: compiled without 'sink-sqlite' feature".into(),
            ))
        }
    }

    /// Get the number of records written.
    pub fn records_written(&self) -> u64 {
        self.records_written
    }
}

#[cfg(feature = "sink-sqlite")]
fn ensure_content_hash_column(conn: &Connection) -> Result<(), SinkError> {
    let mut statement = conn
        .prepare("PRAGMA table_info(dologger_records)")
        .map_err(|e| SinkError::WriteFailed(format!("sqlite schema inspect: {e}")))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| SinkError::WriteFailed(format!("sqlite schema inspect: {e}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| SinkError::WriteFailed(format!("sqlite schema inspect: {e}")))?;

    if !columns.iter().any(|column| column == "content_hash") {
        conn.execute(
            "ALTER TABLE dologger_records ADD COLUMN content_hash BLOB",
            [],
        )
        .map_err(|e| SinkError::WriteFailed(format!("sqlite schema migrate: {e}")))?;
    }

    Ok(())
}

#[cfg(all(test, feature = "sink-sqlite"))]
mod tests {
    use super::*;
    use crate::record::Record;
    use crate::sink::Sink;

    static TEST_COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    fn test_db_path(test_name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "dologger_sqlite_{test_name}_{}_{}.db",
            std::process::id(),
            TEST_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ))
    }

    fn remove_db(path: &PathBuf) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    #[test]
    fn open_migrates_legacy_schema_and_writes_new_record() {
        let path = test_db_path("legacy");
        remove_db(&path);

        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE dologger_records (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                record_id TEXT,
                timestamp_ns INTEGER,
                level TEXT NOT NULL,
                message TEXT NOT NULL,
                source_file TEXT,
                source_line INTEGER,
                source_func TEXT,
                thread_id INTEGER,
                thread_name TEXT,
                process_id INTEGER,
                process_name TEXT,
                host_name TEXT,
                app_name TEXT,
                app_version TEXT,
                environment TEXT,
                user_id TEXT,
                session_id TEXT,
                request_id TEXT,
                trace_id TEXT,
                span_id TEXT,
                lsn INTEGER,
                signature BLOB,
                ext_data TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO dologger_records
                (record_id, level, message, signature, ext_data)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                "legacy-id",
                "INFO",
                "legacy row",
                vec![0xabu8, 0xcdu8],
                "legacy extension",
            ],
        )
        .unwrap();
        drop(conn);

        let mut sink = SqliteSink::with_path(&path);
        sink.open().unwrap();
        let mut record = Record::new(0);
        record.message.set("legacy migration");
        record.set_source_line(17);
        sink.write_record(&record).unwrap();
        sink.close().unwrap();

        let conn = Connection::open(&path).unwrap();
        let content_hash: Vec<u8> = conn
            .query_row(
                "SELECT content_hash FROM dologger_records WHERE message = ?1",
                ["legacy migration"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(content_hash, vec![0; 32]);
        let (legacy_record_id, legacy_signature, legacy_ext_data, legacy_content_hash): (
            String,
            Vec<u8>,
            String,
            Option<Vec<u8>>,
        ) = conn
            .query_row(
                "SELECT record_id, signature, ext_data, content_hash
                 FROM dologger_records WHERE message = ?1",
                ["legacy row"],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(legacy_record_id, "legacy-id");
        assert_eq!(legacy_signature, vec![0xab, 0xcd]);
        assert_eq!(legacy_ext_data, "legacy extension");
        assert!(legacy_content_hash.is_none());
        let record_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM dologger_records", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(record_count, 2);
        remove_db(&path);
    }
}

/// Parse a ConsoleSink-formatted line into (level, message).
/// Format: `[secs.millis] [LEVEL] [thread_id] message`
fn parse_formatted_line(line: &str) -> (&str, &str) {
    // Find the level (second `[ ]` pair)
    if let Some(rest) = line.strip_prefix('[') {
        if let Some(rest) = rest.split_once("] [") {
            if let Some(rest) = rest.1.split_once("] [") {
                let (level, rest) = rest.1.split_once("] ").unwrap_or(("UNKNOWN", rest.1));
                return (level, rest);
            }
        }
    }
    ("UNKNOWN", line)
}
