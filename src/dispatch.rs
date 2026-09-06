use std::env;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use crate::model::{truncate_chars, Card};

/// Put this marker in a card title or body to force a stub failure (tests / smoke).
pub const STUB_FAIL_MARKER: &str = "[stub:fail]";

pub const DEFAULT_INTERVAL_MS: u64 = 2000;
pub const DEFAULT_TIMEOUT_SECS: u64 = 300;
pub const LOG_MESSAGE_MAX: usize = 4000;

#[derive(Debug, Clone)]
pub struct DispatchConfig {
    /// When false, the interval wake never starts a new job.
    pub enabled: bool,
    /// In-process fake agent: same transitions and `agent_log` / `revision_count` as real grok.
    pub stub: bool,
    /// Stub always fails (env `AGENT_KANBAN_STUB_FAIL`). Per-card `[stub:fail]` also fails.
    pub stub_fail: bool,
    pub grok_bin: String,
    pub cwd: PathBuf,
    pub timeout: Duration,
    pub interval: Duration,
}

impl DispatchConfig {
    pub fn from_env() -> Self {
        let cwd = env::var("AGENT_KANBAN_CWD")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

        let grok_bin = env::var("AGENT_KANBAN_GROK")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "grok".to_string());

        let interval_ms = env_u64("AGENT_KANBAN_DISPATCH_INTERVAL_MS", DEFAULT_INTERVAL_MS);
        let timeout_secs =
            env_u64("AGENT_KANBAN_DISPATCH_TIMEOUT_SECS", DEFAULT_TIMEOUT_SECS).max(1);

        Self {
            enabled: env::var("AGENT_KANBAN_DISPATCH")
                .ok()
                .map(|s| truthy(&s))
                .unwrap_or(true),
            stub: truthy_var("AGENT_KANBAN_STUB_DISPATCH"),
            stub_fail: truthy_var("AGENT_KANBAN_STUB_FAIL"),
            grok_bin,
            cwd,
            timeout: Duration::from_secs(timeout_secs),
            interval: Duration::from_millis(interval_ms),
        }
    }

    /// Immediate-wake stub used by unit tests (no live grok).
    pub fn stub_for_tests() -> Self {
        Self {
            enabled: true,
            stub: true,
            stub_fail: false,
            grok_bin: "grok".into(),
            cwd: env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            timeout: Duration::from_secs(5),
            interval: Duration::from_millis(0),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeKind {
    Success,
    Failure,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchOutcome {
    pub kind: OutcomeKind,
    pub message: String,
}

impl DispatchOutcome {
    pub fn success(message: impl Into<String>) -> Self {
        Self {
            kind: OutcomeKind::Success,
            message: truncate_log(message.into()),
        }
    }

    pub fn failure(message: impl Into<String>) -> Self {
        Self {
            kind: OutcomeKind::Failure,
            message: truncate_log(message.into()),
        }
    }
}

pub fn truthy(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn truthy_var(name: &str) -> bool {
    env::var(name).ok().is_some_and(|v| truthy(&v))
}

fn env_u64(name: &str, default: u64) -> u64 {
    env::var(name)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(default)
}

pub fn truncate_log(message: String) -> String {
    if message.chars().count() <= LOG_MESSAGE_MAX {
        message
    } else {
        truncate_chars(&message, LOG_MESSAGE_MAX)
    }
}

/// Prompt sent to `grok -p`. Includes title, body, and useful card context.
pub fn build_prompt(card: &Card) -> String {
    let body = if card.body.trim().is_empty() {
        "(empty)".to_string()
    } else {
        card.body.clone()
    };
    format!(
        "You are completing one Agent Kanban card. Do the work in the working directory.\n\
         \n\
         Title: {title}\n\
         Status: {status}\n\
         Revision: {rev}\n\
         Card id: {id}\n\
         \n\
         Body / context:\n\
         {body}\n\
         \n\
         When finished, summarize what you changed. If you cannot complete the work, say so clearly.",
        title = card.title,
        status = card.status.title(),
        rev = card.revision_count,
        id = card.id,
        body = body,
    )
}

/// Headless argv after the binary: `grok -p "…" --cwd … --always-approve --json`
pub fn grok_args(prompt: &str, cwd: &str) -> Vec<String> {
    vec![
        "-p".into(),
        prompt.into(),
        "--cwd".into(),
        cwd.into(),
        "--always-approve".into(),
        "--json".into(),
    ]
}

pub fn run_dispatch(config: &DispatchConfig, card: &Card) -> DispatchOutcome {
    if config.stub {
        run_stub(config, card)
    } else {
        run_grok(config, card, None)
    }
}

pub fn stub_should_fail(config: &DispatchConfig, card: &Card) -> bool {
    config.stub_fail
        || card.title.contains(STUB_FAIL_MARKER)
        || card.body.contains(STUB_FAIL_MARKER)
}

fn run_stub(config: &DispatchConfig, card: &Card) -> DispatchOutcome {
    if stub_should_fail(config, card) {
        DispatchOutcome::failure("stub dispatch failed")
    } else {
        DispatchOutcome::success(format!("stub dispatch ok: {}", card.title))
    }
}

/// Spawn a worker thread. The kill sender interrupts a live grok child.
pub fn spawn_dispatch(
    config: DispatchConfig,
    card: Card,
) -> (Receiver<DispatchOutcome>, Sender<()>) {
    let (tx, rx) = mpsc::channel();
    let (kill_tx, kill_rx) = mpsc::channel();
    thread::spawn(move || {
        let outcome = if config.stub {
            run_stub(&config, &card)
        } else {
            run_grok(&config, &card, Some(&kill_rx))
        };
        let _ = tx.send(outcome);
    });
    (rx, kill_tx)
}

fn run_grok(config: &DispatchConfig, card: &Card, kill: Option<&Receiver<()>>) -> DispatchOutcome {
    let prompt = build_prompt(card);
    let cwd = config.cwd.display().to_string();
    let mut cmd = Command::new(&config.grok_bin);
    cmd.args(grok_args(&prompt, &cwd))
        .current_dir(&config.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return DispatchOutcome::failure(format!("missing binary: {}", config.grok_bin));
        }
        Err(err) => {
            return DispatchOutcome::failure(format!("failed to spawn grok: {err}"));
        }
    };

    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let (out_tx, out_rx) = mpsc::channel::<Vec<u8>>();
    let (err_tx, err_rx) = mpsc::channel::<Vec<u8>>();
    if let Some(mut pipe) = stdout_pipe {
        thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = pipe.read_to_end(&mut buf);
            let _ = out_tx.send(buf);
        });
    }
    if let Some(mut pipe) = stderr_pipe {
        thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = pipe.read_to_end(&mut buf);
            let _ = err_tx.send(buf);
        });
    }

    let start = Instant::now();
    let status = loop {
        if let Some(kill) = kill {
            match kill.try_recv() {
                Ok(()) | Err(TryRecvError::Disconnected) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return DispatchOutcome::failure("dispatch cancelled");
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if start.elapsed() >= config.timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return DispatchOutcome::failure("dispatch timed out");
            }
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(err) => {
                return DispatchOutcome::failure(format!("wait failed: {err}"));
            }
        }
    };

    let stdout = String::from_utf8_lossy(
        &out_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap_or_default(),
    )
    .into_owned();
    let stderr = String::from_utf8_lossy(
        &err_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap_or_default(),
    )
    .into_owned();

    if !status.success() {
        let code = status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "signal".into());
        let detail = first_nonempty(&[&stderr, &stdout]).unwrap_or("no output");
        return DispatchOutcome::failure(format!("grok exit {code}: {detail}"));
    }

    match parse_grok_json(&stdout) {
        Ok(summary) => DispatchOutcome::success(summary),
        Err(err) => DispatchOutcome::failure(err),
    }
}

fn first_nonempty<'a>(parts: &[&'a str]) -> Option<&'a str> {
    parts.iter().map(|s| s.trim()).find(|s| !s.is_empty())
}

/// Accept a single JSON object, or the last JSON line of mixed stdout.
pub fn parse_grok_json(stdout: &str) -> Result<String, String> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Err("bad JSON from grok: empty stdout".into());
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        return Ok(summarize_json(&value));
    }
    if let Some(line) = trimmed.lines().rev().find(|l| !l.trim().is_empty()) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(line.trim()) {
            return Ok(summarize_json(&value));
        }
    }
    Err("bad JSON from grok".into())
}

fn summarize_json(value: &serde_json::Value) -> String {
    const KEYS: &[&str] = &["text", "message", "result", "output", "content", "summary"];
    if let Some(s) = value.as_str() {
        return s.to_string();
    }
    if let Some(obj) = value.as_object() {
        for key in KEYS {
            if let Some(s) = obj.get(*key).and_then(|v| v.as_str()) {
                if !s.is_empty() {
                    return s.to_string();
                }
            }
        }
    }
    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Status;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    fn writable_script(dir: &std::path::Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, body).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn card(title: &str, body: &str) -> Card {
        let mut c = Card::new(title);
        c.status = Status::Todo;
        c.body = body.into();
        c
    }

    #[test]
    fn grok_argv_matches_ac() {
        let args = grok_args("do the thing", "/tmp/work");
        assert_eq!(
            args,
            vec![
                "-p",
                "do the thing",
                "--cwd",
                "/tmp/work",
                "--always-approve",
                "--json"
            ]
        );
    }

    #[test]
    fn prompt_includes_title_body_and_context() {
        let mut c = card("Ship login", "Use existing auth crate");
        c.revision_count = 2;
        let prompt = build_prompt(&c);
        assert!(prompt.contains("Ship login"), "{prompt}");
        assert!(prompt.contains("Use existing auth crate"), "{prompt}");
        assert!(prompt.contains("Revision: 2"), "{prompt}");
        assert!(prompt.contains(&c.id), "{prompt}");
    }

    #[test]
    fn stub_success_and_fail_paths() {
        let mut cfg = DispatchConfig::stub_for_tests();
        let ok = run_dispatch(&cfg, &card("A", "body"));
        assert_eq!(ok.kind, OutcomeKind::Success);
        assert!(ok.message.contains("stub dispatch ok"));

        cfg.stub_fail = true;
        let fail = run_dispatch(&cfg, &card("A", "body"));
        assert_eq!(fail.kind, OutcomeKind::Failure);

        cfg.stub_fail = false;
        let marked = run_dispatch(&cfg, &card("x [stub:fail]", ""));
        assert_eq!(marked.kind, OutcomeKind::Failure);
    }

    #[test]
    fn parse_json_object_and_trailing_line() {
        assert!(parse_grok_json("").is_err());
        assert!(parse_grok_json("not json").is_err());
        assert_eq!(parse_grok_json(r#"{"text":"hello"}"#).unwrap(), "hello");
        assert_eq!(
            parse_grok_json("noise\n{\"message\":\"ok\"}\n").unwrap(),
            "ok"
        );
    }

    #[test]
    fn missing_binary_is_failure() {
        let mut cfg = DispatchConfig::stub_for_tests();
        cfg.stub = false;
        cfg.grok_bin = "/this/does/not/exist/grok-agent-kanban".into();
        let out = run_dispatch(&cfg, &card("X", ""));
        assert_eq!(out.kind, OutcomeKind::Failure);
        assert!(out.message.contains("missing binary"), "{}", out.message);
    }

    #[test]
    fn nonzero_exit_is_failure() {
        let dir = tempfile::tempdir().unwrap();
        let bin = writable_script(dir.path(), "fail.sh", "#!/bin/sh\necho nope\nexit 7\n");
        let mut cfg = DispatchConfig::stub_for_tests();
        cfg.stub = false;
        cfg.grok_bin = bin.display().to_string();
        cfg.cwd = dir.path().to_path_buf();
        let out = run_dispatch(&cfg, &card("X", ""));
        assert_eq!(out.kind, OutcomeKind::Failure);
        assert!(out.message.contains("exit 7"), "{}", out.message);
    }

    #[test]
    fn bad_json_is_failure_even_on_zero_exit() {
        let dir = tempfile::tempdir().unwrap();
        let bin = writable_script(dir.path(), "bad.sh", "#!/bin/sh\necho not-json\nexit 0\n");
        let mut cfg = DispatchConfig::stub_for_tests();
        cfg.stub = false;
        cfg.grok_bin = bin.display().to_string();
        cfg.cwd = dir.path().to_path_buf();
        let out = run_dispatch(&cfg, &card("X", ""));
        assert_eq!(out.kind, OutcomeKind::Failure);
        assert!(out.message.contains("bad JSON"), "{}", out.message);
    }

    #[test]
    fn success_json_is_ok() {
        let dir = tempfile::tempdir().unwrap();
        let bin = writable_script(
            dir.path(),
            "ok.sh",
            "#!/bin/sh\necho '{\"text\":\"patched\"}'\nexit 0\n",
        );
        let mut cfg = DispatchConfig::stub_for_tests();
        cfg.stub = false;
        cfg.grok_bin = bin.display().to_string();
        cfg.cwd = dir.path().to_path_buf();
        let out = run_dispatch(&cfg, &card("X", ""));
        assert_eq!(out.kind, OutcomeKind::Success);
        assert_eq!(out.message, "patched");
    }

    #[test]
    fn timeout_kills_child() {
        let dir = tempfile::tempdir().unwrap();
        let bin = writable_script(dir.path(), "slow.sh", "#!/bin/sh\nsleep 30\n");
        let mut cfg = DispatchConfig::stub_for_tests();
        cfg.stub = false;
        cfg.grok_bin = bin.display().to_string();
        cfg.cwd = dir.path().to_path_buf();
        cfg.timeout = Duration::from_millis(200);
        let out = run_dispatch(&cfg, &card("X", ""));
        assert_eq!(out.kind, OutcomeKind::Failure);
        assert!(out.message.contains("timed out"), "{}", out.message);
    }

    #[test]
    fn truthy_flags() {
        assert!(truthy("1"));
        assert!(truthy("true"));
        assert!(truthy("YES"));
        assert!(!truthy("0"));
        assert!(!truthy(""));
    }
}
