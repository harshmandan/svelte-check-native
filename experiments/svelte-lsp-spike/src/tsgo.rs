//! A warm `tsgo --lsp` child.
//!
//! This is the half of the design the batch path cannot demonstrate. Spawning
//! tsgo per query costs a few hundred milliseconds; keeping one alive against a
//! narrow project costs a few hundred megabytes and answers in about a
//! millisecond. The child is started on the first hover and lives until the
//! editor disconnects.
//!
//! tsgo sends requests back to the client (`client/registerCapability` on
//! startup). Leaving one unanswered deadlocks it — silently, with no error
//! and no timeout.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub struct Tsgo {
    child: Child,
    /// Shared with the reader thread, which answers server-initiated requests
    /// on its own rather than routing them through the request path.
    stdin: Arc<Mutex<ChildStdin>>,
    responses: Receiver<serde_json::Value>,
    /// Overlay files already handed to tsgo: the version last sent, and the
    /// text it carried. The text is kept so an unchanged overlay can be
    /// skipped — tsgo declares `interFileDependencies`, so every didChange
    /// invalidates the whole program and makes the next pull re-check it.
    opened: HashMap<String, (i64, String)>,
    next_id: i64,
}

impl Tsgo {
    /// Start tsgo and initialize it against `root` — the cache directory
    /// holding the overlay tsconfig, which is the project tsgo will resolve
    /// for every overlay file we open.
    pub fn start(binary: &Path, needs_node: bool, root: &Path) -> Result<Self, String> {
        let mut command = if needs_node {
            let mut c = Command::new("node");
            c.arg(binary);
            c
        } else {
            Command::new(binary)
        };
        let mut child = command
            .args(["--lsp", "-stdio"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("spawning tsgo: {e}"))?;

        let stdin = Arc::new(Mutex::new(
            child.stdin.take().ok_or("tsgo stdin unavailable")?,
        ));
        let stdout = child.stdout.take().ok_or("tsgo stdout unavailable")?;
        let (tx, responses) = channel();
        spawn_reader(stdout, tx, Arc::clone(&stdin));

        let mut tsgo = Self {
            child,
            stdin,
            responses,
            opened: HashMap::new(),
            next_id: 1,
        };

        let root_uri = uri(root);
        tsgo.request(
            "initialize",
            serde_json::json!({
                "processId": std::process::id(),
                "rootUri": root_uri,
                "workspaceFolders": [{ "uri": root_uri, "name": "overlay" }],
                "capabilities": {
                    "workspace": { "configuration": true, "workspaceFolders": true },
                    "textDocument": {
                        "synchronization": {},
                        "hover": { "contentFormat": ["markdown", "plaintext"] },
                        "diagnostic": { "dynamicRegistration": false }
                    }
                }
            }),
            Duration::from_secs(30),
        )?;
        tsgo.notify("initialized", serde_json::json!({}));
        Ok(tsgo)
    }

    /// Make tsgo's view of an overlay match what is on disk — didOpen the
    /// first time, didChange after that. Overlays are rewritten on every
    /// check, so re-opening an already-open document would leave tsgo on the
    /// stale text.
    pub fn sync(&mut self, overlay: &Path) -> Result<(), String> {
        let key = overlay.to_string_lossy().into_owned();
        let text = std::fs::read_to_string(overlay)
            .map_err(|e| format!("reading overlay {}: {e}", overlay.display()))?;
        if let Some((_, previous)) = self.opened.get(&key)
            && previous == &text
        {
            return Ok(());
        }
        match self.opened.get(&key).map(|(v, _)| *v) {
            None => {
                self.notify(
                    "textDocument/didOpen",
                    serde_json::json!({
                        "textDocument": {
                            "uri": uri(overlay),
                            "languageId": "typescript",
                            "version": 1,
                            "text": text.clone()
                        }
                    }),
                );
                self.opened.insert(key, (1, text));
            }
            Some(version) => {
                self.notify(
                    "textDocument/didChange",
                    serde_json::json!({
                        "textDocument": { "uri": uri(overlay), "version": version + 1 },
                        "contentChanges": [{ "text": text.clone() }]
                    }),
                );
                self.opened.insert(key, (version + 1, text));
            }
        }
        Ok(())
    }

    /// Tell tsgo the overlay tsconfig changed on disk. Adding a tab rewrites
    /// the project's file list, and the alternative to this notification is
    /// killing the process and rebuilding the program from nothing — which on
    /// a heavy scope is a second of dead editor per tab switch.
    pub fn config_changed(&mut self, tsconfig: &Path) {
        self.notify(
            "workspace/didChangeWatchedFiles",
            serde_json::json!({ "changes": [{ "uri": uri(tsconfig), "type": 2 }] }),
        );
    }

    /// Pull diagnostics for one overlay. tsgo advertises
    /// `diagnosticProvider`, so this is a request with an answer rather than
    /// a push we would have to wait for and guess the end of.
    pub fn diagnostics(&mut self, overlay: &Path) -> Result<Vec<serde_json::Value>, String> {
        let response = self.request(
            "textDocument/diagnostic",
            serde_json::json!({ "textDocument": { "uri": uri(overlay) } }),
            Duration::from_secs(60),
        )?;
        Ok(response
            .pointer("/result/items")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default())
    }

    pub fn hover(
        &mut self,
        overlay: &Path,
        line: u32,
        character: u32,
    ) -> Result<Option<serde_json::Value>, String> {
        let response = self.request(
            "textDocument/hover",
            serde_json::json!({
                "textDocument": { "uri": uri(overlay) },
                "position": { "line": line, "character": character }
            }),
            Duration::from_secs(20),
        )?;
        Ok(response.get("result").cloned().filter(|v| !v.is_null()))
    }

    fn request(
        &mut self,
        method: &str,
        params: serde_json::Value,
        timeout: Duration,
    ) -> Result<serde_json::Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&serde_json::json!({
            "jsonrpc": "2.0", "id": id, "method": method, "params": params
        }));

        // Responses arrive in order for our purposes; anything with a
        // different id belongs to a request we already gave up on.
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return Err(format!("{method} timed out"));
            }
            match self.responses.recv_timeout(remaining) {
                Ok(msg) if msg.get("id").and_then(|v| v.as_i64()) == Some(id) => return Ok(msg),
                Ok(_) => continue,
                Err(_) => return Err(format!("{method} timed out")),
            }
        }
    }

    fn notify(&mut self, method: &str, params: serde_json::Value) {
        self.send(&serde_json::json!({
            "jsonrpc": "2.0", "method": method, "params": params
        }));
    }

    fn send(&mut self, message: &serde_json::Value) {
        if let Ok(mut stdin) = self.stdin.lock() {
            write_message(&mut stdin, message);
        }
    }
}

impl Drop for Tsgo {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn write_message(stdin: &mut ChildStdin, message: &serde_json::Value) {
    let body = message.to_string();
    let _ = write!(stdin, "Content-Length: {}\r\n\r\n{}", body.len(), body);
    let _ = stdin.flush();
}

/// Read tsgo's stream forever: responses go to the channel, server-initiated
/// requests get an immediate reply so the server never blocks on us.
fn spawn_reader(
    stdout: std::process::ChildStdout,
    tx: Sender<serde_json::Value>,
    stdin: Arc<Mutex<ChildStdin>>,
) {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let mut content_length = None;
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => return,
                    Ok(_) => {}
                }
                let trimmed = line.trim_end();
                if trimmed.is_empty() {
                    break;
                }
                if let Some(value) = trimmed.strip_prefix("Content-Length:") {
                    content_length = value.trim().parse::<usize>().ok();
                }
            }
            let Some(len) = content_length else { continue };
            let mut body = vec![0u8; len];
            if reader.read_exact(&mut body).is_err() {
                return;
            }
            let Ok(msg) = serde_json::from_slice::<serde_json::Value>(&body) else {
                continue;
            };
            // A message with a method is a notification or a request from the
            // server. Requests must be answered; `workspace/configuration`
            // wants one settings object per item, everything else takes null.
            if let Some(method) = msg.get("method").and_then(|m| m.as_str()) {
                if let Some(id) = msg.get("id") {
                    let result = if method == "workspace/configuration" {
                        let count = msg
                            .pointer("/params/items")
                            .and_then(|i| i.as_array())
                            .map(|a| a.len())
                            .unwrap_or(0);
                        serde_json::Value::Array(vec![serde_json::json!({}); count])
                    } else {
                        serde_json::Value::Null
                    };
                    if let Ok(mut out) = stdin.lock() {
                        write_message(
                            &mut out,
                            &serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result }),
                        );
                    }
                }
                continue;
            }
            if tx.send(msg).is_err() {
                return;
            }
        }
    });
}

/// One encoder for both directions of the server — tsgo has to be handed the
/// same URI shape the editor uses, or its answers key against a document we
/// cannot match back.
pub use crate::rpc::path_to_uri as uri;
