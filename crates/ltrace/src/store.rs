use crate::model::*;
use anyhow::{Context, Result, bail, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Serialize, de::DeserializeOwned};
use std::{path::Path, sync::Mutex, time::Duration};
use uuid::Uuid;

pub const MAX_SPANS: usize = 50_000;
pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        ensure!(
            version <= 1,
            "database was created by a newer ltrace; upgrade this application"
        );
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;
            CREATE TABLE IF NOT EXISTS sessions(id TEXT PRIMARY KEY, body TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS runs(id TEXT PRIMARY KEY, session_id TEXT NOT NULL REFERENCES sessions(id), token TEXT UNIQUE NOT NULL, body TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS spans(run_id TEXT NOT NULL REFERENCES runs(id), trace_id TEXT NOT NULL, span_id TEXT NOT NULL, body TEXT NOT NULL, PRIMARY KEY(run_id, trace_id, span_id));
            CREATE TABLE IF NOT EXISTS notes(id TEXT PRIMARY KEY, session_id TEXT NOT NULL REFERENCES sessions(id), body TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS runs_session ON runs(session_id);
            CREATE INDEX IF NOT EXISTS notes_session ON notes(session_id);
            PRAGMA user_version=1;")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }
    fn connection(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|_| anyhow::anyhow!("database lock poisoned"))
    }
    pub fn create_session(
        &self,
        title: String,
        project: String,
        expectations: Vec<Expectation>,
    ) -> Result<Session> {
        ensure!(
            !title.trim().is_empty() && title.len() <= 200,
            "title must contain 1–200 characters"
        );
        ensure!(
            !project.trim().is_empty() && project.len() <= 4096,
            "project path is required"
        );
        ensure!(expectations.len() <= 32, "at most 32 expectations");
        for e in &expectations {
            e.validate()?;
        }
        let s = Session {
            id: Uuid::new_v4().to_string(),
            title,
            project,
            created_ms: now_ms(),
            expectations,
        };
        self.connection()?.execute(
            "INSERT INTO sessions VALUES(?1,?2)",
            params![s.id, json(&s)?],
        )?;
        Ok(s)
    }
    pub fn sessions(&self) -> Result<Vec<Session>> {
        let conn = self.connection()?;
        read_many(
            &conn,
            "SELECT body FROM sessions ORDER BY rowid DESC LIMIT 200",
            [],
        )
    }
    pub fn session(&self, id: &str) -> Result<Session> {
        let conn = self.connection()?;
        read_one(&conn, "SELECT body FROM sessions WHERE id=?1", id)
    }
    pub fn start_run(
        &self,
        session_id: &str,
        label: String,
        command: String,
        revision: String,
    ) -> Result<(Run, String)> {
        let session = self.session(session_id)?;
        ensure!(
            label.len() <= 200 && command.len() <= 2000 && revision.len() <= 200,
            "run metadata too long"
        );
        let r = Run {
            id: Uuid::new_v4().to_string(),
            session_id: session_id.into(),
            label,
            command,
            revision,
            started_ms: now_ms(),
            finished_ms: None,
            exit_code: None,
            test_status: "running".into(),
            capture_status: "collecting".into(),
            issues: vec![],
            expectations: session.expectations,
        };
        let token = Uuid::new_v4().to_string();
        self.connection()?.execute(
            "INSERT INTO runs VALUES(?1,?2,?3,?4)",
            params![r.id, session_id, token, json(&r)?],
        )?;
        Ok((r, token))
    }
    pub fn run(&self, id: &str) -> Result<Run> {
        let conn = self.connection()?;
        read_one(&conn, "SELECT body FROM runs WHERE id=?1", id)
    }
    pub fn runs(&self, session: &str) -> Result<Vec<Run>> {
        let conn = self.connection()?;
        read_many(
            &conn,
            "SELECT body FROM runs WHERE session_id=?1 ORDER BY rowid DESC LIMIT 200",
            [session],
        )
    }
    pub fn finish_run(
        &self,
        id: &str,
        exit_code: Option<i32>,
        issue: Option<String>,
    ) -> Result<Run> {
        let conn = self.connection()?;
        let mut run: Run = read_one(&conn, "SELECT body FROM runs WHERE id=?1", id)?;
        ensure!(run.finished_ms.is_none(), "run already finished");
        run.finished_ms = Some(now_ms());
        run.exit_code = exit_code;
        run.test_status = match exit_code {
            Some(0) => "passed",
            Some(_) => "failed",
            None => "interrupted",
        }
        .into();
        if let Some(issue) = issue {
            run.issues.push(issue);
        }
        let n: i64 = conn.query_row("SELECT count(*) FROM spans WHERE run_id=?1", [id], |r| {
            r.get(0)
        })?;
        run.capture_status = if !run.issues.is_empty() {
            "partial"
        } else if n == 0 {
            "empty"
        } else {
            "settled"
        }
        .into();
        conn.execute(
            "UPDATE runs SET body=?2 WHERE id=?1",
            params![id, json(&run)?],
        )?;
        Ok(run)
    }
    /// Atomic batch; retries are idempotent. Conflicts preserve the original and
    /// permanently taint the run instead of silently rewriting history.
    pub fn ingest(&self, token: &str, spans: &[Span]) -> Result<usize> {
        for span in spans {
            span.validate()?;
        }
        let mut conn = self.connection()?;
        let tx = conn.transaction()?;
        let body: Option<String> = tx
            .query_row("SELECT body FROM runs WHERE token=?1", [token], |r| {
                r.get(0)
            })
            .optional()?;
        let mut run: Run = serde_json::from_str(&body.context("unknown export token")?)?;
        let n: i64 = tx.query_row(
            "SELECT count(*) FROM spans WHERE run_id=?1",
            [&run.id],
            |r| r.get(0),
        )?;
        let mut added = 0;
        for span in spans {
            let body = json(span)?;
            let previous: Option<String> = tx
                .query_row(
                    "SELECT body FROM spans WHERE run_id=?1 AND trace_id=?2 AND span_id=?3",
                    params![run.id, span.trace_id, span.span_id],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(previous) = previous {
                if previous != body {
                    add_issue(
                        &mut run,
                        "Conflicting duplicate span; first version preserved.",
                    );
                }
                continue;
            }
            if n as usize + added >= MAX_SPANS {
                add_issue(
                    &mut run,
                    "Run span limit reached; additional spans rejected.",
                );
                continue;
            }
            tx.execute(
                "INSERT INTO spans VALUES(?1,?2,?3,?4)",
                params![run.id, span.trace_id, span.span_id, body],
            )?;
            added += 1;
        }
        if run.finished_ms.is_some() && added > 0 {
            add_issue(
                &mut run,
                "Spans arrived after capture settled; verification requires a fresh run.",
            );
        }
        if run.finished_ms.is_some() && !run.issues.is_empty() {
            run.capture_status = "partial".into();
        }
        tx.execute(
            "UPDATE runs SET body=?2 WHERE id=?1",
            params![run.id, json(&run)?],
        )?;
        tx.commit()?;
        Ok(added)
    }
    pub fn spans(&self, run: &str) -> Result<Vec<Span>> {
        let conn = self.connection()?;
        read_many(
            &conn,
            "SELECT body FROM spans WHERE run_id=?1 ORDER BY trace_id,span_id",
            [run],
        )
    }
    pub fn note(&self, session: &str, run: Option<String>, body: String) -> Result<Note> {
        self.session(session)?;
        ensure!(
            !body.trim().is_empty() && body.len() <= 8000,
            "note must contain 1–8000 bytes"
        );
        if let Some(id) = &run {
            ensure!(
                self.run(id)?.session_id == session,
                "run belongs to another session"
            );
        }
        let note = Note {
            id: Uuid::new_v4().to_string(),
            session_id: session.into(),
            run_id: run,
            created_ms: now_ms(),
            body,
        };
        self.connection()?.execute(
            "INSERT INTO notes VALUES(?1,?2,?3)",
            params![note.id, session, json(&note)?],
        )?;
        Ok(note)
    }
    pub fn notes(&self, session: &str) -> Result<Vec<Note>> {
        let conn = self.connection()?;
        read_many(
            &conn,
            "SELECT body FROM notes WHERE session_id=?1 ORDER BY rowid DESC LIMIT 200",
            [session],
        )
    }
}

fn add_issue(run: &mut Run, issue: &str) {
    if !run.issues.iter().any(|s| s == issue) {
        run.issues.push(issue.into());
    }
}
fn json<T: Serialize>(value: &T) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}
fn read_one<T: DeserializeOwned>(conn: &Connection, sql: &str, id: &str) -> Result<T> {
    let body: Option<String> = conn.query_row(sql, [id], |r| r.get(0)).optional()?;
    match body {
        Some(body) => Ok(serde_json::from_str(&body)?),
        None => bail!("not found"),
    }
}
fn read_many<T: DeserializeOwned, P: rusqlite::Params>(
    conn: &Connection,
    sql: &str,
    params: P,
) -> Result<Vec<T>> {
    let mut statement = conn.prepare(sql)?;
    statement
        .query_map(params, |r| r.get::<_, String>(0))?
        .map(|s| Ok(serde_json::from_str(&s?)?))
        .collect()
}
