//! Embedded web view: a tiny_http server on a background thread, sharing the
//! same [`Session`] as the TUI. Readonly for now: the browser gets one static
//! page plus a JSON snapshot endpoint.
//!
//! - `GET /`           the page (embedded at compile time)
//! - `GET /api/tasks`  snapshot: `{version, mode, current, tasks}`
//!
//! `version` is the session's change counter; the page polls and re-renders
//! only when it moves. The server binds 127.0.0.1 only: this is a personal,
//! single-user view.

use std::sync::{Arc, Mutex};
use std::thread;

use anyhow::Result;
use tiny_http::{Header, Method, Request, Response, Server};

use crate::fvp::Mode;
use crate::session::Session;
use crate::task::Status;

const INDEX_HTML: &str = include_str!("web/index.html");

/// Start the web server on 127.0.0.1:`port` and serve requests on a
/// background thread for the life of the process.
pub fn spawn(session: Arc<Mutex<Session>>, port: u16) -> Result<()> {
    let server = Server::http(("127.0.0.1", port))
        .map_err(|e| anyhow::anyhow!("starting web server on port {port}: {e}"))?;
    thread::spawn(move || {
        for request in server.incoming_requests() {
            // Never print from this thread: the terminal is in raw mode.
            let _ = handle(request, &session);
        }
    });
    Ok(())
}

fn handle(request: Request, session: &Arc<Mutex<Session>>) -> Result<()> {
    let url = request.url().to_string();
    let path = url.split_once('?').map_or(url.as_str(), |(p, _)| p);

    match (request.method(), path) {
        (Method::Get, "/") => respond(request, 200, "text/html; charset=utf-8", INDEX_HTML),
        (Method::Get, "/api/tasks") => {
            let session = session.lock().expect("session lock poisoned");
            respond_json(request, 200, &snapshot_json(&session))
        }
        _ => respond_json(request, 404, r#"{"error":"not found"}"#),
    }
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
        r#"{{"version":{},"mode":"{}","current":{},"tasks":[{}]}}"#,
        session.version(),
        mode,
        current,
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
    fn snapshot_includes_version_mode_current_and_tasks() {
        let tasks = vec![Task::new("say \"hi\""), Task::new("b")];
        let session = Session::new(PathBuf::from("/dev/null"), tasks);
        // Initial scan dots task 0: Preselect { benchmark: 0, cursor: 1 }.
        let json = snapshot_json(&session);
        assert_eq!(
            json,
            r#"{"version":0,"mode":"preselect","current":null,"tasks":[{"text":"say \"hi\"","status":"dotted"},{"text":"b","status":"open"}]}"#
        );
    }
}
