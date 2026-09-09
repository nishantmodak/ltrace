use crate::{
    api::{AppState, router},
    store::Store,
};
use anyhow::{Context, Result, bail, ensure};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{net::TcpListener, task::JoinHandle};
use uuid::Uuid;

pub fn data_dir() -> Result<PathBuf> {
    resolve_home_with(None, default_platform_dirs)
}

pub fn resolve_home(override_: Option<PathBuf>) -> Result<PathBuf> {
    resolve_home_with(override_, default_platform_dirs)
}

fn default_platform_dirs() -> Option<PathBuf> {
    directories::ProjectDirs::from("dev", "ltrace", "ltrace")
        .map(|d| d.data_local_dir().to_path_buf())
}

pub fn resolve_home_with(
    override_: Option<PathBuf>,
    platform_dirs: fn() -> Option<PathBuf>,
) -> Result<PathBuf> {
    if let Some(p) = override_ {
        return Ok(p);
    }
    if let Some(p) = std::env::var_os("LTRACE_HOME").map(PathBuf::from) {
        return Ok(p);
    }
    platform_dirs().context("cannot locate application data directory")
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ConnectionInfo {
    pub url: String,
    pub token: String,
}

pub struct LocalServer {
    pub info: ConnectionInfo,
    task: JoinHandle<()>,
    _lock: File,
    connection_path: PathBuf,
}
impl Drop for LocalServer {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_file(&self.connection_path);
        // Closing our handle alone can leave the lock held by a child between
        // fork and exec. Release it explicitly before allowing a restart.
        let _ = FileExt::unlock(&self._lock);
    }
}

pub async fn start(dir: &Path, port: u16) -> Result<LocalServer> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(dir.join("receiver.lock"))?;
    lock.try_lock_exclusive()
        .context("ltrace is already running for this data directory")?;
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
        .await
        .context("cannot bind local OTLP port; close the conflicting receiver or use --port")?;
    let store = Arc::new(Store::open(dir.join("traces.sqlite"))?);
    store.interrupt_unfinished()?;
    let info = ConnectionInfo {
        url: format!("http://127.0.0.1:{}", listener.local_addr()?.port()),
        token: Uuid::new_v4().to_string(),
    };
    let connection_path = dir.join("connection.json");
    let temporary = dir.join(format!(".connection-{}.tmp", Uuid::new_v4()));
    std::fs::write(&temporary, serde_json::to_vec(&info)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(temporary, &connection_path)?;
    let app = router(AppState {
        store,
        token: info.token.clone(),
        live: Default::default(),
    });
    let task = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            eprintln!("ltrace receiver stopped: {e}");
        }
    });
    Ok(LocalServer {
        info,
        task,
        _lock: lock,
        connection_path,
    })
}

#[derive(Clone)]
pub struct Client {
    pub info: ConnectionInfo,
    http: reqwest::Client,
}
impl Client {
    pub fn connect(dir: &Path) -> Result<Self> {
        let info = serde_json::from_slice(
            &std::fs::read(dir.join("connection.json"))
                .context("ltrace is not running. Open the desktop app or run ltrace-dev serve")?,
        )?;
        Self::new(info)
    }
    pub fn new(info: ConnectionInfo) -> Result<Self> {
        let url = reqwest::Url::parse(&info.url)?;
        ensure!(
            url.scheme() == "http"
                && url.host_str() == Some("127.0.0.1")
                && url.port().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.path() == "/"
                && url.query().is_none()
                && url.fragment().is_none(),
            "connection must be a literal loopback HTTP endpoint"
        );
        Ok(Self {
            info,
            http: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(15))
                .build()?,
        })
    }
    pub async fn read(&self, path: &str) -> Result<Value> {
        self.request(path, None).await
    }
    pub async fn write(&self, path: &str, body: Value) -> Result<Value> {
        self.request(path, Some(body)).await
    }
    async fn request(&self, path: &str, body: Option<Value>) -> Result<Value> {
        ensure!(
            !path.starts_with('/') && !path.contains("..") && !path.contains('#'),
            "invalid API path"
        );
        let url = format!("{}/api/{path}", self.info.url);
        let request = if let Some(body) = body {
            self.http.post(url).json(&body)
        } else {
            self.http.get(url)
        };
        let response = request
            .bearer_auth(&self.info.token)
            .send()
            .await
            .context("local receiver unavailable; open ltrace and retry")?;
        let status = response.status();
        // Read the body as text first so non-JSON error bodies (notably the
        // plain-text 401/403 from the `authorize` middleware) can be surfaced
        // verbatim instead of failing to decode and masking the real reason.
        let text = response
            .text()
            .await
            .context("local receiver unavailable; open ltrace and retry")?;
        if !status.is_success() {
            let message = serde_json::from_str::<Value>(&text)
                .ok()
                .as_ref()
                .and_then(|b| b.get("error").and_then(Value::as_str))
                .unwrap_or(&text)
                .to_owned();
            bail!("{message}");
        }
        Ok(serde_json::from_str(&text)?)
    }
    pub async fn health(&self) -> Result<Value> {
        let health = self.read("health").await?;
        ensure!(
            health["product"] == "ltrace-local" && health["api_version"] == 1,
            "incompatible local receiver"
        );
        Ok(health)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn restart_releases_lock_even_when_a_duplicate_handle_is_alive() {
        let dir = tempfile::tempdir().unwrap();
        let server = start(dir.path(), 0).await.unwrap();
        // A concurrently spawned child can retain this same file description
        // between fork and exec, even when the descriptor is close-on-exec.
        let duplicate = server._lock.try_clone().unwrap();
        assert!(start(dir.path(), 0).await.is_err());

        drop(server);
        let restarted = start(dir.path(), 0).await.unwrap();
        Client::connect(dir.path()).unwrap().health().await.unwrap();
        assert!(start(dir.path(), 0).await.is_err());

        drop(duplicate);
        assert!(start(dir.path(), 0).await.is_err());
        drop(restarted);
    }
}
