//! JSON-RPC over stdio, hand-rolled.
//!
//! An LSP framework would bring an async runtime for a server that does one
//! blocking thing at a time. Framing is a length header and a body.

use std::io::{BufRead, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::path::{Path, PathBuf};

/// Read one message, or `None` when the client closes the stream.
pub fn read_message() -> Option<serde_json::Value> {
    let stdin = std::io::stdin();
    let mut handle = stdin.lock();
    let mut content_length: Option<usize> = None;

    loop {
        let mut line = String::new();
        if handle.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break; // end of headers
        }
        if let Some(value) = trimmed.strip_prefix("Content-Length:") {
            content_length = value.trim().parse().ok();
        }
    }

    let len = content_length?;
    let mut body = vec![0u8; len];
    handle.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

fn send(message: &serde_json::Value) {
    let body = message.to_string();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let _ = write!(out, "Content-Length: {}\r\n\r\n{}", body.len(), body);
    let _ = out.flush();
}

pub fn respond(id: Option<serde_json::Value>, result: serde_json::Value) {
    let Some(id) = id else { return };
    send(&serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result }));
}

pub fn notify(method: &str, params: serde_json::Value) {
    send(&serde_json::json!({ "jsonrpc": "2.0", "method": method, "params": params }));
}

/// True once the LSP loop owns stdout.
///
/// Everything else in the process has to keep off it: stdout carries
/// length-framed JSON-RPC with no trailing newlines, so a stray `println!`
/// lands mid-frame. `--selftest` prints its results there, which is why
/// logging has to know which mode it is in rather than always notifying —
/// a debug line glued to the front of `DIAGS ...` silently broke the parity
/// harness, which reported every file as a mismatch.
static SERVING: AtomicBool = AtomicBool::new(false);

pub fn begin_serving() {
    SERVING.store(true, Ordering::Relaxed);
}

/// Log to stderr always, and to the editor's output panel when there is one.
pub fn log(message: &str) {
    eprintln!("[spike] {message}");
    if SERVING.load(Ordering::Relaxed) {
        notify(
            "window/logMessage",
            serde_json::json!({ "type": 3, "message": message }),
        );
    }
}

pub fn publish_diagnostics(path: &Path, diags: &[&svn_typecheck::CheckDiagnostic]) {
    let items: Vec<serde_json::Value> = diags.iter().map(|d| to_lsp(d)).collect();
    notify(
        "textDocument/publishDiagnostics",
        serde_json::json!({ "uri": path_to_uri(path), "diagnostics": items }),
    );
}

fn to_lsp(d: &svn_typecheck::CheckDiagnostic) -> serde_json::Value {
    // LSP positions are 0-based; ours are 1-based on both axes.
    let start = serde_json::json!({ "line": d.line.saturating_sub(1), "character": d.column.saturating_sub(1) });
    let end = serde_json::json!({ "line": d.end_line.saturating_sub(1), "character": d.end_column.saturating_sub(1) });
    let severity = match d.severity {
        svn_typecheck::Severity::Error => 1,
        svn_typecheck::Severity::Warning => 2,
        _ => 4,
    };
    let code = match &d.code {
        svn_typecheck::DiagnosticCode::Numeric(n) => serde_json::json!(n),
        svn_typecheck::DiagnosticCode::Slug(s) => serde_json::json!(s),
    };
    serde_json::json!({
        "range": { "start": start, "end": end },
        "severity": severity,
        "code": code,
        "source": "svelte-lsp-spike",
        "message": d.message,
    })
}

pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let decoded = percent_decode(rest);
    Some(PathBuf::from(decoded))
}

/// Percent-encode a path the way editors do, which is far less than a naive
/// "encode anything unusual" rule.
///
/// This matters concretely for SvelteKit, whose route directories use both
/// `(group)` and `[param]` — and the two are treated differently: parentheses
/// stay literal, brackets are encoded. Over-encoding produces a URI the editor
/// never matches to the file it asked about, so diagnostics for those routes
/// silently go nowhere. Matches Node's `pathToFileURL`: unreserved characters,
/// sub-delims and `:@/` pass through; everything else, including every
/// non-ASCII byte, is encoded.
pub fn path_to_uri(path: &Path) -> String {
    let mut out = String::from("file://");
    for ch in path.to_string_lossy().chars() {
        match ch {
            'A'..='Z'
            | 'a'..='z'
            | '0'..='9'
            | '-' | '_' | '.' | '~'
            | '!' | '$' | '&' | '\'' | '(' | ')' | '*' | '+' | ',' | ';' | '='
            | ':' | '@' | '/' => out.push(ch),
            _ => {
                let mut buf = [0u8; 4];
                for b in ch.encode_utf8(&mut buf).as_bytes() {
                    out.push_str(&format!("%{b:02X}"));
                }
            }
        }
    }
    out
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(byte) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
