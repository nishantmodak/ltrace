use crate::model::*;
use anyhow::{Context, Result, bail, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Serialize, de::DeserializeOwned};
use std::{path::Path, sync::Mutex, time::Duration};
use uuid::Uuid;

pub const MAX_SPANS: usize = 50_000;
pub const MAX_RUN_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug)]
pub struct Ingest {
    pub inserted: usize,
    pub rejected: usize,
}
pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        ensure!(
            version <= 2,
            "database was created by a newer ltrace; upgrade this application"
        );
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;
            CREATE TABLE IF NOT EXISTS sessions(id TEXT PRIMARY KEY, body TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS runs(id TEXT PRIMARY KEY, session_id TEXT NOT NULL REFERENCES sessions(id), token TEXT UNIQUE NOT NULL, body TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS spans(run_id TEXT NOT NULL REFERENCES runs(id), trace_id TEXT NOT NULL, span_id TEXT NOT NULL, body TEXT NOT NULL, PRIMARY KEY(run_id, trace_id, span_id));
            CREATE TABLE IF NOT EXISTS notes(id TEXT PRIMARY KEY, session_id TEXT NOT NULL REFERENCES sessions(id), body TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS runs_session ON runs(session_id);
            CREATE INDEX IF NOT EXISTS notes_session ON notes(session_id);
            ")?;
        if version < 2 {
            conn.execute_batch(
                "BEGIN IMMEDIATE; ALTER TABLE spans ADD COLUMN summary TEXT;
                UPDATE spans SET summary=json_remove(body,'$.raw','$.resource','$.scope');
                PRAGMA user_version=2; COMMIT;",
            )?;
        }
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
    /// Project grouping is automatic and atomic across concurrent capture commands.
    pub fn ensure_project(&self, project: &str, title: &str) -> Result<Session> {
        ensure!(
            !project.is_empty() && project.len() <= 4096,
            "invalid project path"
        );
        let conn = self.connection()?;
        let existing: Option<String> = conn.query_row("SELECT body FROM sessions WHERE json_extract(body,'$.project')=?1 ORDER BY rowid LIMIT 1", [project], |r| r.get(0)).optional()?;
        if let Some(body) = existing {
            return Ok(serde_json::from_str(&body)?);
        }
        let session = Session {
            id: Uuid::new_v4().to_string(),
            title: title.chars().take(200).collect(),
            project: project.into(),
            created_ms: now_ms(),
            expectations: vec![],
        };
        conn.execute(
            "INSERT INTO sessions VALUES(?1,?2)",
            params![session.id, json(&session)?],
        )?;
        Ok(session)
    }
    pub fn start_stream(&self) -> Result<(Run, String)> {
        let project = self.ensure_project("ltrace:incoming", "Incoming traces")?;
        let (mut run, token) = self.start_run(
            &project.id,
            "Live telemetry".into(),
            "Direct OTLP export · no test attribution".into(),
            "unavailable".into(),
        )?;
        run.test_status = "not_run".into();
        self.connection()?.execute(
            "UPDATE runs SET body=?2 WHERE id=?1",
            params![run.id, json(&run)?],
        )?;
        Ok((run, token))
    }
    pub fn close_stream(&self, id: &str) -> Result<()> {
        let conn = self.connection()?;
        let mut run: Run = read_one(&conn, "SELECT body FROM runs WHERE id=?1", id)?;
        run.finished_ms = Some(now_ms());
        run.capture_status = if run.issues.is_empty() {
            "settled"
        } else {
            "partial"
        }
        .into();
        conn.execute(
            "UPDATE runs SET body=?2 WHERE id=?1",
            params![run.id, json(&run)?],
        )?;
        Ok(())
    }
    pub fn start_run(
        &self,
        session_id: &str,
        label: String,
        command: String,
        revision: String,
    ) -> Result<(Run, String)> {
        self.start_run_with_expectations(session_id, label, command, revision, None)
    }
    pub fn start_run_with_expectations(
        &self,
        session_id: &str,
        label: String,
        command: String,
        revision: String,
        expectations: Option<Vec<Expectation>>,
    ) -> Result<(Run, String)> {
        let session = self.session(session_id)?;
        let expectations = expectations.unwrap_or(session.expectations);
        ensure!(expectations.len() <= 32, "at most 32 expectations");
        for expectation in &expectations {
            expectation.validate()?;
        }
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
            expectations,
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
    pub fn ingest(&self, token: &str, spans: &[Span]) -> Result<Ingest> {
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
        let mut bytes: i64 = tx.query_row(
            "SELECT COALESCE(SUM(octet_length(body)),0) FROM spans WHERE run_id=?1",
            [&run.id],
            |r| r.get(0),
        )?;
        let mut added = 0;
        let mut rejected = 0;
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
            if n as usize + added >= MAX_SPANS || bytes as usize + body.len() > MAX_RUN_BYTES {
                rejected += 1;
                add_issue(
                    &mut run,
                    "Run storage limit reached; additional spans rejected.",
                );
                continue;
            }
            tx.execute(
                "INSERT INTO spans(run_id,trace_id,span_id,body,summary) VALUES(?1,?2,?3,?4,?5)",
                params![
                    run.id,
                    span.trace_id,
                    span.span_id,
                    body,
                    span_metadata(span)?
                ],
            )?;
            bytes += body.len() as i64;
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
        Ok(Ingest {
            inserted: added,
            rejected,
        })
    }
    pub fn interrupt_unfinished(&self) -> Result<()> {
        let conn = self.connection()?;
        let runs: Vec<Run> = read_many(&conn, "SELECT body FROM runs", [])?;
        for mut run in runs.into_iter().filter(|r| r.finished_ms.is_none()) {
            run.finished_ms = Some(now_ms());
            if run.test_status != "not_run" {
                run.test_status = "interrupted".into();
            }
            run.capture_status = "partial".into();
            add_issue(&mut run, "Receiver restarted before the run finished.");
            conn.execute(
                "UPDATE runs SET body=?2 WHERE id=?1",
                params![run.id, json(&run)?],
            )?;
        }
        Ok(())
    }
    pub fn check_token(&self, token: &str) -> Result<()> {
        let conn = self.connection()?;
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM runs WHERE token=?1)",
            [token],
            |r| r.get(0),
        )?;
        ensure!(exists, "unknown export token");
        Ok(())
    }
    pub fn mark_issue(&self, token: &str, issue: &str) -> Result<()> {
        let conn = self.connection()?;
        let mut run: Run = read_one(&conn, "SELECT body FROM runs WHERE token=?1", token)?;
        add_issue(&mut run, issue);
        if run.finished_ms.is_some() {
            run.capture_status = "partial".into();
        }
        conn.execute(
            "UPDATE runs SET body=?2 WHERE id=?1",
            params![run.id, json(&run)?],
        )?;
        Ok(())
    }
    pub fn analysis_spans(&self, run: &str) -> Result<Vec<Span>> {
        let conn = self.connection()?;
        read_many(
            &conn,
            "SELECT summary FROM spans WHERE run_id=?1 ORDER BY trace_id,span_id",
            [run],
        )
    }
    pub fn finding_spans(&self, run: &str, trace: &str) -> Result<Vec<Span>> {
        let conn = self.connection()?;
        read_many(
            &conn,
            "SELECT json_set(summary,'$.raw',json_object('attributes',json_extract(body,'$.raw.attributes'))) FROM spans WHERE run_id=?1 AND trace_id=?2 ORDER BY span_id",
            params![run, trace],
        )
    }
    /// Page trace identities from compact metadata, then load metadata only for
    /// those traces. Run IDs remain part of identity even for reused trace IDs.
    pub fn recent_traces(
        &self,
        search: &str,
        offset: usize,
        limit: usize,
    ) -> Result<(Vec<serde_json::Value>, usize)> {
        let conn = self.connection()?;
        let groups = "SELECT run_id,trace_id,MIN(printf('%020s',json_extract(summary,'$.start_ns'))) AS started FROM spans GROUP BY run_id,trace_id HAVING ?1='' OR instr(lower(trace_id),?1)>0 OR MAX(instr(lower(json_extract(summary,'$.name')),?1))>0 OR MAX(instr(lower(json_extract(summary,'$.service')),?1))>0";
        let total: i64 =
            conn.query_row(&format!("SELECT COUNT(*) FROM ({groups})"), [search], |r| {
                r.get(0)
            })?;
        let mut statement = conn.prepare(&format!(
            "{groups} ORDER BY started DESC,run_id,trace_id LIMIT ?2 OFFSET ?3"
        ))?;
        let keys: Vec<(String, String)> = statement
            .query_map(
                params![
                    search,
                    limit.clamp(1, 200) as i64,
                    offset.min(i64::MAX as usize) as i64
                ],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?
            .collect::<rusqlite::Result<_>>()?;
        let mut metadata = conn.prepare("SELECT spans.run_id,spans.summary FROM json_each(?1) AS selected JOIN spans ON spans.run_id=json_extract(selected.value,'$[0]') AND spans.trace_id=json_extract(selected.value,'$[1]')")?;
        let rows = metadata.query_map([serde_json::to_string(&keys)?], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut groups = std::collections::BTreeMap::<(String, String), Vec<Span>>::new();
        for row in rows {
            let (run, body) = row?;
            let span: Span = serde_json::from_str(&body)?;
            groups
                .entry((run, span.trace_id.clone()))
                .or_default()
                .push(span);
        }
        let mut result = Vec::new();
        for (run, trace) in keys {
            let spans = groups.remove(&(run.clone(), trace)).unwrap_or_default();
            if let Some(trace) = crate::analysis::traces(&spans).into_iter().next() {
                let mut value = serde_json::to_value(trace)?;
                value["run_id"] = run.into();
                result.push(value);
            }
        }
        Ok((result, total as usize))
    }
    pub fn span_page(
        &self,
        run: &str,
        trace: Option<&str>,
        offset: usize,
        limit: usize,
    ) -> Result<(Vec<Span>, usize)> {
        let conn = self.connection()?;
        let total: i64 = conn.query_row(
            "SELECT COUNT(*) FROM spans WHERE run_id=?1 AND (?2 IS NULL OR trace_id=?2)",
            params![run, trace],
            |r| r.get(0),
        )?;
        let rows = read_many(
            &conn,
            "SELECT body FROM spans WHERE run_id=?1 AND (?2 IS NULL OR trace_id=?2) ORDER BY trace_id,span_id LIMIT ?3 OFFSET ?4",
            params![
                run,
                trace,
                limit.clamp(1, 200) as i64,
                offset.min(i64::MAX as usize) as i64
            ],
        )?;
        Ok((rows, total as usize))
    }
    pub fn span(&self, run: &str, trace: &str, span: &str) -> Result<Span> {
        let conn = self.connection()?;
        let body: Option<String> = conn
            .query_row(
                "SELECT body FROM spans WHERE run_id=?1 AND trace_id=?2 AND span_id=?3",
                params![run, trace, span],
                |r| r.get(0),
            )
            .optional()?;
        Ok(serde_json::from_str(&body.context("not found")?)?)
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

fn span_metadata(span: &Span) -> Result<String> {
    Ok(serde_json::to_string(
        &serde_json::json!({"trace_id":span.trace_id,"span_id":span.span_id,"parent_span_id":span.parent_span_id,"name":span.name,"service":span.service,"start_ns":span.start_ns,"end_ns":span.end_ns,"error":span.error,"dropped":span.dropped}),
    )?)
}
