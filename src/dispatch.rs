use std::io::ErrorKind;
use std::process::Command;

use uuid::Uuid;

use crate::model::summarize_output;

pub const SUMMARY_MAX_CHARS: usize = 2000;

const GROK_NOT_FOUND: &str = "grok not found on PATH. Install the Grok CLI (https://x.ai/cli) and ensure `grok` is available, then press r to retry.";

/// Binary used for dispatch. Override with `AGENT_KANBAN_GROK` (default: `grok`).
pub fn grok_bin() -> String {
    std::env::var("AGENT_KANBAN_GROK")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "grok".to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchOutcome {
    pub card_id: String,
    pub run_id: String,
    pub success: bool,
    pub summary: String,
}

pub fn new_run_id() -> String {
    format!("ak-{}", Uuid::new_v4())
}

/// Headless `grok -p` (plus flags so grok does not steal the TUI).
pub fn run_headless(card_id: &str, prompt: &str, run_id: &str) -> DispatchOutcome {
    let bin = grok_bin();
    match Command::new(&bin)
        .arg("--no-auto-update")
        .arg("--no-alt-screen")
        .arg("--output-format")
        .arg("plain")
        .arg("-s")
        .arg(run_id)
        .arg("-p")
        .arg(prompt)
        .output()
    {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            if output.status.success() {
                let summary = if stdout.trim().is_empty() {
                    summarize_output(&stderr, SUMMARY_MAX_CHARS)
                } else {
                    summarize_output(&stdout, SUMMARY_MAX_CHARS)
                };
                DispatchOutcome {
                    card_id: card_id.to_string(),
                    run_id: run_id.to_string(),
                    success: true,
                    summary,
                }
            } else {
                let combined = if stderr.trim().is_empty() {
                    format!(
                        "grok exited {}.\n{}",
                        output.status.code().unwrap_or(-1),
                        stdout
                    )
                } else {
                    format!(
                        "grok exited {}.\n{}",
                        output.status.code().unwrap_or(-1),
                        stderr
                    )
                };
                DispatchOutcome {
                    card_id: card_id.to_string(),
                    run_id: run_id.to_string(),
                    success: false,
                    summary: summarize_output(&combined, SUMMARY_MAX_CHARS),
                }
            }
        }
        Err(err) if err.kind() == ErrorKind::NotFound => DispatchOutcome {
            card_id: card_id.to_string(),
            run_id: run_id.to_string(),
            success: false,
            summary: GROK_NOT_FOUND.to_string(),
        },
        Err(err) => DispatchOutcome {
            card_id: card_id.to_string(),
            run_id: run_id.to_string(),
            success: false,
            summary: format!("failed to start grok: {err}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn missing_binary_is_a_clear_failure() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("AGENT_KANBAN_GROK", "agent-kanban-no-such-grok-binary");
        let outcome = run_headless("card-1", "hello", "run-1");
        std::env::remove_var("AGENT_KANBAN_GROK");
        assert!(!outcome.success);
        assert!(outcome.summary.contains("not found"));
        assert_eq!(outcome.card_id, "card-1");
    }

    #[test]
    fn successful_stub_command_is_ok() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("AGENT_KANBAN_GROK", "true");
        let outcome = run_headless("card-2", "hello", "run-2");
        std::env::remove_var("AGENT_KANBAN_GROK");
        assert!(outcome.success);
        assert_eq!(outcome.card_id, "card-2");
    }

    #[test]
    fn failing_stub_command_is_fail() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("AGENT_KANBAN_GROK", "false");
        let outcome = run_headless("card-3", "hello", "run-3");
        std::env::remove_var("AGENT_KANBAN_GROK");
        assert!(!outcome.success);
        assert!(outcome.summary.contains("exited"));
    }
}
