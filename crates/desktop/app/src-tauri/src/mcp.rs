//! In die App eingebauter MCP-Server (opt-in über die Umgebungsvariable `VINDEX_MCP`).
//! Erlaubt einem MCP-Client (z. B. einem Agenten), die laufende Desktop-App zu prüfen und
//! zu steuern, OHNE Browser/localhost-Vorschau: ein Screenshot der echten App, Navigation
//! zwischen Ansichten und Abfrage des UI-Zustands.
//!
//! Transport: MCP „Streamable HTTP" minimal — ein `POST /mcp` beantwortet die JSON-RPC-
//! Anfragen (initialize, tools/list, tools/call); `GET /mcp` liefert 405 (kein
//! server-seitiger Stream). Bindung nur an 127.0.0.1. Der Screenshot greift das echte
//! Fenster compositor-nativ ab (spectacle/grim/import), kein Portal-Dialog.

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use base64::Engine;
use serde_json::{json, Value};
use std::net::Ipv4Addr;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

/// Vom Frontend gemeldeter UI-Zustand (aktuelle Ansicht o. Ä.) für das `ui_state`-Tool.
static REPORTED_STATE: Mutex<Option<Value>> = Mutex::new(None);

/// Wird vom Tauri-Command `mcp_report_state` aufgerufen, wenn sich die Ansicht ändert.
pub fn set_reported_state(v: Value) {
    if let Ok(mut g) = REPORTED_STATE.lock() {
        *g = Some(v);
    }
}

fn reported_state() -> Value {
    REPORTED_STATE
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .unwrap_or_else(|| json!({}))
}

/// Startet den MCP-Server, falls `VINDEX_MCP` gesetzt ist. Der Wert kann ein Port sein
/// (sonst Default 4599). Gibt den tatsächlichen Port zurück; `None`, wenn deaktiviert.
pub async fn start(app: AppHandle) -> std::io::Result<Option<u16>> {
    let Some(raw) = std::env::var("VINDEX_MCP")
        .ok()
        .filter(|s| !s.trim().is_empty())
    else {
        return Ok(None);
    };
    let port: u16 = raw.trim().parse().ok().filter(|p| *p > 0).unwrap_or(4599);
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await?;
    let actual = listener.local_addr()?.port();
    let router = Router::new()
        .route(
            "/mcp",
            post(handle).get(|| async { StatusCode::METHOD_NOT_ALLOWED }),
        )
        .route("/health", get(|| async { "ok" }))
        .with_state(app);
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    Ok(Some(actual))
}

/// Tool-Definitionen für `tools/list`.
fn tool_defs() -> Value {
    json!([
        {
            "name": "screenshot",
            "description": "Screenshot der laufenden vindex-Desktop-App. mode: 'window' (nur das App-Fenster, Default) oder 'full' (ganzer Bildschirm).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "mode": { "type": "string", "enum": ["window", "full"] }
                }
            }
        },
        {
            "name": "navigate",
            "description": "Wechselt die Ansicht der App. view: home|library|downloads|aniworld|serienstream|filmo|settings.",
            "inputSchema": {
                "type": "object",
                "properties": { "view": { "type": "string" } },
                "required": ["view"]
            }
        },
        {
            "name": "ui_state",
            "description": "Liefert den aktuell vom Frontend gemeldeten UI-Zustand (aktive Ansicht usw.) als JSON.",
            "inputSchema": { "type": "object", "properties": {} }
        }
    ])
}

async fn handle(State(app): State<AppHandle>, Json(req): Json<Value>) -> Response {
    let id = req.get("id").cloned();
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");
    // Benachrichtigungen (ohne id) bestätigen wir nur.
    if id.is_none() {
        return StatusCode::ACCEPTED.into_response();
    }
    match method {
        "initialize" => rpc_ok(
            id,
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "vindex-desktop", "version": env!("CARGO_PKG_VERSION") }
            }),
        ),
        "ping" => rpc_ok(id, json!({})),
        "tools/list" => rpc_ok(id, json!({ "tools": tool_defs() })),
        "tools/call" => {
            tool_call(
                app,
                id,
                req.get("params").cloned().unwrap_or_else(|| json!({})),
            )
            .await
        }
        _ => rpc_error(id, -32601, "Methode nicht gefunden"),
    }
}

async fn tool_call(app: AppHandle, id: Option<Value>, params: Value) -> Response {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    match name {
        "screenshot" => {
            let full = args.get("mode").and_then(Value::as_str) == Some("full");
            // Fenster nach vorn holen, damit der Fensterschnitt die App trifft.
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
            match capture(full).await {
                Ok(b64) => rpc_ok(
                    id,
                    json!({ "content": [{ "type": "image", "data": b64, "mimeType": "image/png" }] }),
                ),
                Err(e) => rpc_ok(
                    id,
                    json!({ "content": [{ "type": "text", "text": format!("Screenshot fehlgeschlagen: {e}") }], "isError": true }),
                ),
            }
        }
        "navigate" => {
            let view = args
                .get("view")
                .and_then(Value::as_str)
                .unwrap_or("home")
                .to_string();
            let _ = app.emit("mcp-navigate", view.clone());
            rpc_ok(
                id,
                json!({ "content": [{ "type": "text", "text": format!("navigiert zu {view}") }] }),
            )
        }
        "ui_state" => rpc_ok(
            id,
            json!({ "content": [{ "type": "text", "text": reported_state().to_string() }] }),
        ),
        other => rpc_error(id, -32602, &format!("Unbekanntes Tool: {other}")),
    }
}

/// Fängt den Bildschirm/das aktive Fenster compositor-nativ ab und liefert Base64-PNG.
/// Reihenfolge nach Verfügbarkeit: spectacle (KDE), grim (wlroots), import (X11).
async fn capture(full: bool) -> Result<String, String> {
    let path = std::env::temp_dir().join(format!("vindex-mcp-{}.png", std::process::id()));
    let p = path.to_string_lossy().to_string();
    let _ = tokio::fs::remove_file(&path).await;

    let attempts: Vec<(&str, Vec<String>)> = if full {
        vec![
            (
                "spectacle",
                vec!["-b", "-f", "-n", "-o", &p]
                    .into_iter()
                    .map(String::from)
                    .collect(),
            ),
            ("grim", vec![p.clone()]),
            (
                "import",
                vec!["-window".to_string(), "root".to_string(), p.clone()],
            ),
        ]
    } else {
        vec![
            (
                "spectacle",
                vec!["-b", "-a", "-n", "-o", &p]
                    .into_iter()
                    .map(String::from)
                    .collect(),
            ),
            (
                "spectacle",
                vec!["-b", "-f", "-n", "-o", &p]
                    .into_iter()
                    .map(String::from)
                    .collect(),
            ),
            ("grim", vec![p.clone()]),
        ]
    };

    let mut last = String::from("kein Screenshot-Werkzeug (spectacle/grim/import)");
    for (bin, a) in attempts {
        match tokio::process::Command::new(bin).args(&a).output().await {
            Ok(out) if out.status.success() => {
                if let Ok(bytes) = tokio::fs::read(&path).await {
                    if !bytes.is_empty() {
                        return Ok(base64::engine::general_purpose::STANDARD.encode(bytes));
                    }
                }
                last = format!("{bin}: leere Datei");
            }
            Ok(out) => {
                last = format!("{bin}: {}", String::from_utf8_lossy(&out.stderr).trim());
            }
            Err(e) => last = format!("{bin}: {e}"),
        }
    }
    Err(last)
}

fn rpc_ok(id: Option<Value>, result: Value) -> Response {
    Json(json!({ "jsonrpc": "2.0", "id": id, "result": result })).into_response()
}

fn rpc_error(id: Option<Value>, code: i64, msg: &str) -> Response {
    Json(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": msg } }))
        .into_response()
}
