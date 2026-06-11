//! Local MCP shim — exposes the app's tool layer to the Claude Code CLI.
//!
//! A tiny HTTP JSON-RPC server on 127.0.0.1 implementing the MCP
//! streamable-http transport in its plain-JSON mode (no SSE): initialize,
//! tools/list (straight from tools::definitions()), tools/call (dispatched
//! to tools::run(), i.e. the exact same Rust executors the API brain uses).
//! The URL carries a per-run random token, and the listener binds loopback
//! only, so nothing else on the machine can drive the tools.

use serde_json::{json, Value};
use tauri::AppHandle;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::tools;

pub const PORT: u16 = 17823;

pub struct McpInfo {
    pub url: String,
}

fn run_token() -> String {
    // Loopback-only; this just prevents casual cross-process pokes.
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{:x}{:x}", t, std::process::id())
}

/// Start the listener; returns the URL for .mcp.json.
pub fn start(app: &AppHandle) -> McpInfo {
    let token = run_token();
    let path = format!("/mcp-{token}");
    let url = format!("http://127.0.0.1:{PORT}{path}");

    let app = app.clone();
    let route = path.clone();
    tauri::async_runtime::spawn(async move {
        let listener = match TcpListener::bind(("127.0.0.1", PORT)).await {
            Ok(l) => l,
            Err(e) => {
                eprintln!("JARVIS MCP shim: port {PORT} unavailable ({e}) — CLI brain will run toolless");
                return;
            }
        };
        loop {
            let Ok((stream, _)) = listener.accept().await else { continue };
            let app = app.clone();
            let route = route.clone();
            tauri::async_runtime::spawn(async move {
                let _ = handle_conn(app, stream, route).await;
            });
        }
    });

    McpInfo { url }
}

async fn handle_conn(app: AppHandle, mut stream: tokio::net::TcpStream, route: String) -> Result<(), ()> {
    // Serve sequential requests on one connection (the CLI keeps it alive).
    loop {
        let mut buf: Vec<u8> = Vec::with_capacity(8192);
        let mut tmp = [0u8; 4096];

        // Read headers.
        let header_end = loop {
            let n = stream.read(&mut tmp).await.map_err(|_| ())?;
            if n == 0 {
                return Ok(()); // client closed
            }
            buf.extend_from_slice(&tmp[..n]);
            if let Some(pos) = find_subslice(&buf, b"\r\n\r\n") {
                break pos + 4;
            }
            if buf.len() > 64 * 1024 {
                return Err(());
            }
        };

        let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
        let mut lines = head.lines();
        let request_line = lines.next().unwrap_or_default().to_string();
        let mut content_length = 0usize;
        for l in lines {
            if let Some(v) = l.to_ascii_lowercase().strip_prefix("content-length:") {
                content_length = v.trim().parse().unwrap_or(0);
            }
        }

        // Read body.
        let mut body = buf[header_end..].to_vec();
        while body.len() < content_length {
            let n = stream.read(&mut tmp).await.map_err(|_| ())?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(&tmp[..n]);
        }

        let mut parts = request_line.split_whitespace();
        let method = parts.next().unwrap_or("");
        let path = parts.next().unwrap_or("");

        if path != route {
            write_response(&mut stream, 404, "application/json", b"{\"error\":\"not found\"}").await?;
            continue;
        }
        if method == "GET" || method == "DELETE" {
            // No SSE stream / session teardown — plain-JSON mode.
            write_response(&mut stream, 405, "application/json", b"{}").await?;
            continue;
        }
        if method != "POST" {
            write_response(&mut stream, 405, "application/json", b"{}").await?;
            continue;
        }

        let req: Value = serde_json::from_slice(&body).unwrap_or(json!({}));
        // Notifications (no id) get 202 Accepted, empty.
        if req["id"].is_null() {
            write_response(&mut stream, 202, "application/json", b"").await?;
            continue;
        }

        let result = dispatch(&app, &req).await;
        let response = match result {
            Ok(v) => json!({ "jsonrpc": "2.0", "id": req["id"], "result": v }),
            Err(msg) => json!({ "jsonrpc": "2.0", "id": req["id"],
                                 "error": { "code": -32000, "message": msg } }),
        };
        let bytes = response.to_string().into_bytes();
        write_response(&mut stream, 200, "application/json", &bytes).await?;
    }
}

async fn dispatch(app: &AppHandle, req: &Value) -> Result<Value, String> {
    match req["method"].as_str().unwrap_or("") {
        "initialize" => Ok(json!({
            "protocolVersion": req["params"]["protocolVersion"].as_str().unwrap_or("2025-03-26"),
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "jarvis-desktop", "version": env!("CARGO_PKG_VERSION") }
        })),
        "ping" => Ok(json!({})),
        "tools/list" => {
            let defs = tools::definitions();
            let tools: Vec<Value> = defs
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .map(|t| {
                    json!({
                        "name": t["name"],
                        "description": t["description"],
                        "inputSchema": t["input_schema"],
                    })
                })
                .collect();
            Ok(json!({ "tools": tools }))
        }
        "tools/call" => {
            let name = req["params"]["name"].as_str().ok_or("tool name required")?;
            let args = req["params"]["arguments"].clone();
            match tools::run(app, name, &args).await {
                Ok(v) => Ok(json!({
                    "content": [{ "type": "text", "text": v.to_string() }],
                    "isError": false
                })),
                Err(e) => Ok(json!({
                    "content": [{ "type": "text", "text": e }],
                    "isError": true
                })),
            }
        }
        other => Err(format!("Method not supported: {other}")),
    }
}

async fn write_response(
    stream: &mut tokio::net::TcpStream,
    status: u16,
    ctype: &str,
    body: &[u8],
) -> Result<(), ()> {
    let reason = match status {
        200 => "OK",
        202 => "Accepted",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "OK",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await.map_err(|_| ())?;
    stream.write_all(body).await.map_err(|_| ())?;
    stream.flush().await.map_err(|_| ())
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}
