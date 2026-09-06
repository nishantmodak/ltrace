#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use ltrace_local::local::{self, Client, LocalServer};
use serde_json::{Value, json};
use std::sync::Mutex;
use tauri::Manager;

struct Desktop {
    client: Option<Client>,
    error: Option<String>,
    _server: Mutex<Option<LocalServer>>,
}

#[tauri::command]
async fn status(app: tauri::AppHandle, state: tauri::State<'_, Desktop>) -> Result<Value, String> {
    let client = state.client.as_ref().ok_or_else(|| {
        state
            .error
            .clone()
            .unwrap_or_else(|| "Receiver unavailable".into())
    })?;
    client.health().await.map_err(|e| e.to_string())?;
    let resources = app.path().resource_dir().map_err(|e| e.to_string())?;
    let packaged = resources.join("bin/ltrace-dev");
    let cli = if packaged.is_file() {
        packaged
    } else {
        std::env::current_exe()
            .map_err(|e| e.to_string())?
            .with_file_name("ltrace-dev")
    };
    Ok(
        json!({"url":client.info.url,"version":env!("CARGO_PKG_VERSION"),"data_dir":local::data_dir().map_err(|e|e.to_string())?,"cli_path":cli,"skill_path":resources.join("skills/ltrace-debug/SKILL.md")}),
    )
}
#[tauri::command]
async fn read(path: String, state: tauri::State<'_, Desktop>) -> Result<Value, String> {
    let client = state.client.as_ref().ok_or("Receiver unavailable")?;
    client.read(&path).await.map_err(|e| e.to_string())
}
#[tauri::command]
async fn add_note(
    session: String,
    body: String,
    run: Option<String>,
    state: tauri::State<'_, Desktop>,
) -> Result<Value, String> {
    // IDs originate in our store; restrict path structure even for an IPC caller.
    if !session.chars().all(|c| c.is_ascii_hexdigit() || c == '-') || session.len() != 36 {
        return Err("Invalid session ID".into());
    }
    let client = state.client.as_ref().ok_or("Receiver unavailable")?;
    client
        .write(
            &format!("sessions/{session}/notes"),
            json!({"body":body,"run_id":run}),
        )
        .await
        .map_err(|e| e.to_string())
}
async fn connect() -> Desktop {
    let result: anyhow_result::Result<(Client, Option<LocalServer>)> = async {
        let dir = local::data_dir()?;
        if let Ok(client) = Client::connect(&dir)
            && tokio::time::timeout(std::time::Duration::from_secs(2), client.health())
                .await
                .is_ok_and(|r| r.is_ok())
        {
            return Ok((client, None));
        }
        let port = std::env::var("LTRACE_PORT")
            .ok()
            .map(|s| s.parse())
            .transpose()?
            .unwrap_or(4318);
        let server = local::start(&dir, port).await?;
        let client = Client::new(server.info.clone())?;
        Ok((client, Some(server)))
    }
    .await;
    match result {
        Ok((client, server)) => Desktop {
            client: Some(client),
            error: None,
            _server: Mutex::new(server),
        },
        Err(e) => Desktop {
            client: None,
            error: Some(e.to_string()),
            _server: Mutex::new(None),
        },
    }
}
mod anyhow_result {
    pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
}
fn main() {
    tauri::Builder::default()
        .manage(tauri::async_runtime::block_on(connect()))
        .invoke_handler(tauri::generate_handler![status, read, add_note])
        .run(tauri::generate_context!())
        .expect("failed to run ltrace desktop");
}
