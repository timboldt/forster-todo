//! Embedded web view: a tiny_http server on a background thread, sharing the
//! same [`Session`] as the TUI. The browser gets one static page plus a small
//! JSON API:
//!
//! - `GET  /`          the page (embedded at compile time)
//! - `GET  /api/tasks` snapshot: `{version, mode, current, benchmark, cursor, tasks}`
//! - `POST /api/add`   body = task text; appends an open task
//! - `POST /api/act?op=<op>&version=N` applies an FVP operation:
//!   `dot`/`undot`/`skip`/`back`/`finish` during a scan, `complete`/`resume` in action
//!   mode, `purge` (backup + drop done tasks) in any mode. The version guard
//!   (409 on mismatch) prevents acting on a stale view, and ops outside their
//!   mode return 409.
//!
//! `version` is the session's change counter; the page polls and re-renders
//! only when it moves. All mutations go through the same [`Session`] the TUI
//! uses, so the two views cannot diverge. The server binds 0.0.0.0 so other
//! devices on the LAN can use it; pass `--auth user:pass` to require HTTP
//! Basic auth on every route. (Basic auth over plain HTTP keeps casual users
//! out but is not strong security — credentials are base64 on the wire.)

use std::io::Read;
use std::sync::{Arc, Mutex};
use std::thread;

use anyhow::Result;
use tiny_http::{Header, Method, Request, Response, Server};

use crate::fvp::Mode;
use crate::session::Session;
use crate::task::Status;

const INDEX_HTML: &str = include_str!("web/index.html");

/// Maximum accepted request body (task text) in bytes.
const MAX_BODY: u64 = 16 * 1024;

/// Start the web server on 0.0.0.0:`port` (reachable from the LAN) and serve
/// requests on a background thread for the life of the process. When `auth`
/// is `Some("user:pass")`, every route requires matching HTTP Basic auth.
pub fn spawn(session: Arc<Mutex<Session>>, port: u16, auth: Option<String>) -> Result<()> {
    let server = Server::http(("0.0.0.0", port))
        .map_err(|e| anyhow::anyhow!("starting web server on port {port}: {e}"))?;
    // Precompute the exact header value we expect; comparing strings avoids
    // ever decoding untrusted base64.
    let expected = auth.map(|creds| format!("Basic {}", base64_encode(creds.as_bytes())));
    thread::spawn(move || {
        for request in server.incoming_requests() {
            // Never print from this thread: the terminal is in raw mode.
            let _ = if authorized(&request, expected.as_deref()) {
                handle(request, &session)
            } else {
                respond_unauthorized(request)
            };
        }
    });
    Ok(())
}

/// Check the request's `Authorization` header against the expected value.
fn authorized(request: &Request, expected: Option<&str>) -> bool {
    let Some(expected) = expected else {
        return true; // auth not enabled
    };
    request
        .headers()
        .iter()
        .any(|h| h.field.equiv("Authorization") && h.value.as_str() == expected)
}

fn respond_unauthorized(request: Request) -> Result<()> {
    let header = Header::from_bytes(
        &b"WWW-Authenticate"[..],
        &b"Basic realm=\"forster-todo\""[..],
    )
    .expect("static header is valid");
    request.respond(
        Response::from_string(r#"{"error":"unauthorized"}"#)
            .with_status_code(401)
            .with_header(header),
    )?;
    Ok(())
}

/// Standard base64 (RFC 4648, with padding). Small enough to hand-roll rather
/// than pull in a dependency for one header comparison.
fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let n = u32::from_be_bytes([
            0,
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ]);
        out.push(ALPHABET[(n >> 18 & 63) as usize] as char);
        out.push(ALPHABET[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6 & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

fn handle(mut request: Request, session: &Arc<Mutex<Session>>) -> Result<()> {
    let url = request.url().to_string();
    let (path, query) = url.split_once('?').unwrap_or((url.as_str(), ""));

    match (request.method().clone(), path) {
        (Method::Get, "/") => respond(request, 200, "text/html; charset=utf-8", INDEX_HTML),
        (Method::Get, "/api/tasks") => {
            let session = session.lock().expect("session lock poisoned");
            respond_json(request, 200, &snapshot_json(&session))
        }
        (Method::Post, "/api/add") => {
            let mut text = String::new();
            request
                .as_reader()
                .take(MAX_BODY)
                .read_to_string(&mut text)?;
            let text = text.trim().to_string();
            if text.is_empty() {
                return respond_json(request, 400, r#"{"error":"empty task text"}"#);
            }
            let mut session = session.lock().expect("session lock poisoned");
            match session.add(text) {
                Ok(()) => respond_json(request, 200, &snapshot_json(&session)),
                Err(_) => respond_json(request, 500, r#"{"error":"failed to save"}"#),
            }
        }
        (Method::Post, "/api/act") => {
            let Some(op) = query_param(query, "op") else {
                return respond_json(request, 400, r#"{"error":"op parameter required"}"#);
            };
            let op = op.to_string();
            let Some(expected) = query_param(query, "version").and_then(|v| v.parse::<u64>().ok())
            else {
                return respond_json(request, 400, r#"{"error":"version parameter required"}"#);
            };
            let mut session = session.lock().expect("session lock poisoned");
            if session.version() != expected {
                return respond_json(request, 409, r#"{"error":"stale version"}"#);
            }
            let scanning = matches!(session.mode, Mode::Preselect { .. });
            let result = match op.as_str() {
                "dot" | "undot" | "skip" | "back" | "finish" => {
                    if !scanning {
                        return respond_json(request, 409, r#"{"error":"not scanning"}"#);
                    }
                    match op.as_str() {
                        "dot" => session.dot(),
                        "undot" => session.undot(),
                        "skip" => {
                            session.move_down();
                            Ok(())
                        }
                        "back" => {
                            session.move_up();
                            Ok(())
                        }
                        _ => {
                            session.finish_scan();
                            Ok(())
                        }
                    }
                }
                "complete" | "resume" => {
                    if session.action_task().is_none() {
                        return respond_json(request, 409, r#"{"error":"no action task"}"#);
                    }
                    if op == "complete" {
                        session.complete()
                    } else {
                        session.resume_scan();
                        Ok(())
                    }
                }
                // Modeless: back up the file, then drop done tasks.
                "purge" => session.purge_done().map(|_| ()),
                _ => return respond_json(request, 400, r#"{"error":"unknown op"}"#),
            };
            match result {
                Ok(()) => respond_json(request, 200, &snapshot_json(&session)),
                Err(_) => respond_json(request, 500, r#"{"error":"failed to save"}"#),
            }
        }
        _ => respond_json(request, 404, r#"{"error":"not found"}"#),
    }
}

/// First value for `name` in a query string like `a=1&b=2`.
fn query_param<'a>(query: &'a str, name: &str) -> Option<&'a str> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v)
}

fn respond(request: Request, status: u16, content_type: &str, body: &str) -> Result<()> {
    let header = Header::from_bytes(&b"Content-Type"[..], content_type.as_bytes())
        .expect("static header is valid");
    request.respond(
        Response::from_string(body)
            .with_status_code(status)
            .with_header(header),
    )?;
    Ok(())
}

fn respond_json(request: Request, status: u16, body: &str) -> Result<()> {
    respond(request, status, "application/json", body)
}

/// Serialize the session state as JSON for the web client.
fn snapshot_json(session: &Session) -> String {
    let mode = match session.mode {
        Mode::Empty => "empty",
        Mode::Preselect { .. } => "preselect",
        Mode::Action { .. } => "action",
    };
    let current = match session.action_task() {
        Some(i) => i.to_string(),
        None => "null".to_string(),
    };
    let (benchmark, cursor) = match session.mode {
        Mode::Preselect { benchmark, cursor } => (benchmark.to_string(), cursor.to_string()),
        _ => ("null".to_string(), "null".to_string()),
    };
    let tasks: Vec<String> = session
        .tasks
        .iter()
        .map(|t| {
            let status = match t.status {
                Status::Open => "open",
                Status::Dotted => "dotted",
                Status::Done => "done",
            };
            format!(
                r#"{{"text":"{}","status":"{}"}}"#,
                json_escape(&t.text),
                status
            )
        })
        .collect();
    format!(
        r#"{{"version":{},"mode":"{}","current":{},"benchmark":{},"cursor":{},"tasks":[{}]}}"#,
        session.version(),
        mode,
        current,
        benchmark,
        cursor,
        tasks.join(",")
    )
}

/// Escape a string for inclusion in a JSON string literal.
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::Task;
    use std::path::PathBuf;

    #[test]
    fn json_escape_handles_specials() {
        assert_eq!(json_escape(r#"say "hi""#), r#"say \"hi\""#);
        assert_eq!(json_escape(r"back\slash"), r"back\\slash");
        assert_eq!(json_escape("tab\there"), "tab\\there");
        assert_eq!(json_escape("ctrl\u{1}"), "ctrl\\u0001");
        assert_eq!(json_escape("plain — unicode ok"), "plain — unicode ok");
    }

    #[test]
    fn base64_encodes_rfc4648_vectors() {
        // RFC 4648 test vectors.
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        // The value a browser would send for user:pass.
        assert_eq!(base64_encode(b"user:pass"), "dXNlcjpwYXNz");
    }

    #[test]
    fn query_param_finds_values() {
        assert_eq!(query_param("version=42", "version"), Some("42"));
        assert_eq!(query_param("a=1&version=7", "version"), Some("7"));
        assert_eq!(query_param("a=1", "version"), None);
        assert_eq!(query_param("", "version"), None);
    }

    #[test]
    fn snapshot_includes_version_mode_current_and_tasks() {
        let tasks = vec![Task::new("say \"hi\""), Task::new("b")];
        let session = Session::new(PathBuf::from("/dev/null"), tasks);
        // Initial scan dots task 0: Preselect { benchmark: 0, cursor: 1 }.
        let json = snapshot_json(&session);
        assert_eq!(
            json,
            r#"{"version":0,"mode":"preselect","current":null,"benchmark":0,"cursor":1,"tasks":[{"text":"say \"hi\"","status":"dotted"},{"text":"b","status":"open"}]}"#
        );
    }
}
