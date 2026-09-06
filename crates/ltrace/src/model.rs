use serde::{Deserialize, Serialize};
use serde_json::Value;

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Expectation {
    pub name: String,
    pub reason: String,
    /// Exact span name and service; no implicit query normalization.
    pub operation: String,
    pub service: String,
    pub min_count: u32,
    pub max_count: u32,
}

impl Expectation {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(!self.name.trim().is_empty(), "expectation needs a name");
        anyhow::ensure!(
            !self.reason.trim().is_empty(),
            "expectation needs a source/reason"
        );
        anyhow::ensure!(
            !self.operation.is_empty() && !self.service.is_empty(),
            "expectation needs an operation and service"
        );
        anyhow::ensure!(
            self.min_count <= self.max_count,
            "invalid expectation count range"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub project: String,
    pub created_ms: u64,
    pub expectations: Vec<Expectation>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Run {
    pub id: String,
    pub session_id: String,
    pub label: String,
    /// Store a user-provided description, never environment variables or command output.
    pub command: String,
    pub revision: String,
    pub started_ms: u64,
    pub finished_ms: Option<u64>,
    pub exit_code: Option<i32>,
    pub test_status: String,
    pub capture_status: String,
    pub issues: Vec<String>,
    pub expectations: Vec<Expectation>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Span {
    pub trace_id: String,
    pub span_id: String,
    pub parent_span_id: String,
    pub name: String,
    pub service: String,
    /// Strings preserve nanosecond precision across JavaScript and JSON.
    pub start_ns: String,
    pub end_ns: String,
    pub error: bool,
    pub dropped: bool,
    pub raw: Value,
    pub resource: Value,
    pub scope: Value,
}

impl Span {
    pub fn interval(&self) -> Option<(u64, u64)> {
        let start = self.start_ns.parse().ok()?;
        let end = self.end_ns.parse().ok()?;
        (start > 0 && end >= start).then_some((start, end))
    }
    pub fn validate(&self) -> anyhow::Result<()> {
        fn id(value: &str, len: usize) -> bool {
            value.len() == len
                && value.bytes().all(|b| b.is_ascii_hexdigit())
                && value.bytes().any(|b| b != b'0')
        }
        anyhow::ensure!(
            id(&self.trace_id, 32) && id(&self.span_id, 16),
            "invalid trace/span ID"
        );
        anyhow::ensure!(
            self.parent_span_id.is_empty() || id(&self.parent_span_id, 16),
            "invalid parent ID"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Note {
    pub id: String,
    pub session_id: String,
    pub run_id: Option<String>,
    pub created_ms: u64,
    pub body: String,
}
