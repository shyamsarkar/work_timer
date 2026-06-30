#![allow(dead_code)]
use notify_rust::{CloseReason, Notification, NotificationResponse, Timeout};
use std::sync::mpsc::Sender;
use std::thread;

/// Result of the idle warning notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleWarningResult {
    Continue,
    Pause,
    Ignore, // Dismissed, timed out, or closed
}

/// Result of the resume prompt notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumePromptResult {
    Resume,
    RemainPaused,
}

/// Helper to show a simple, transient notification (does not expect actions).
pub fn show_simple(summary: &str, body: &str) {
    let result = Notification::new()
        .summary(summary)
        .body(body)
        .timeout(Timeout::Milliseconds(4000))
        .show();

    if let Err(e) = result {
        tracing::error!("Failed to show simple notification: {:?}", e);
    }
}

/// Shows the idle detection warning notification. Blocks in a separate thread,
/// sending the outcome to `action_tx`.
pub fn show_idle_warning(
    idle_time_str: String,
    timeout_seconds: u32,
    action_tx: Sender<IdleWarningResult>,
) {
    thread::spawn(move || {
        let handle = Notification::new()
            .summary("Idle Warning")
            .body(&format!(
                "You have been idle for {}. Are you still working?",
                idle_time_str
            ))
            .action("continue", "Continue Working")
            .action("pause", "Pause Timer")
            .timeout(Timeout::Milliseconds(timeout_seconds * 1000))
            .show();

        match handle {
            Ok(h) => {
                let action_tx_clone = action_tx.clone();
                let response_res = h.wait_for_response(move |response: &NotificationResponse| {
                    let result = match response {
                        NotificationResponse::Action(key) => match key.as_str() {
                            "continue" => IdleWarningResult::Continue,
                            "pause" => IdleWarningResult::Pause,
                            _ => IdleWarningResult::Ignore,
                        },
                        NotificationResponse::Closed(CloseReason::Expired) => {
                            tracing::info!("Idle warning notification expired (timed out)");
                            IdleWarningResult::Ignore
                        }
                        _ => IdleWarningResult::Ignore,
                    };
                    if let Err(e) = action_tx_clone.send(result) {
                        tracing::error!("Failed to send idle warning result: {:?}", e);
                    }
                });

                if let Err(e) = response_res {
                    tracing::error!("Error waiting for idle warning response: {:?}", e);
                    let _ = action_tx.send(IdleWarningResult::Ignore);
                }
            }
            Err(e) => {
                tracing::error!("Failed to display idle warning notification: {:?}", e);
                let _ = action_tx.send(IdleWarningResult::Ignore);
            }
        }
    });
}

/// Shows the resume prompt notification. Blocks in a separate thread,
/// sending the outcome to `action_tx`.
pub fn show_resume_prompt(inactive_time_str: String, action_tx: Sender<ResumePromptResult>) {
    thread::spawn(move || {
        let handle = Notification::new()
            .summary("Resume Timer?")
            .body(&format!(
                "Activity resumed. You were inactive for {}.\nDo you want to resume the timer?",
                inactive_time_str
            ))
            .action("resume", "Resume Timer")
            .action("remain", "Remain Paused")
            .timeout(Timeout::Never)
            .show();

        match handle {
            Ok(h) => {
                let action_tx_clone = action_tx.clone();
                let response_res = h.wait_for_response(move |response: &NotificationResponse| {
                    let result = match response {
                        NotificationResponse::Action(key) => match key.as_str() {
                            "resume" => ResumePromptResult::Resume,
                            _ => ResumePromptResult::RemainPaused,
                        },
                        _ => ResumePromptResult::RemainPaused,
                    };
                    if let Err(e) = action_tx_clone.send(result) {
                        tracing::error!("Failed to send resume prompt result: {:?}", e);
                    }
                });

                if let Err(e) = response_res {
                    tracing::error!("Error waiting for resume prompt response: {:?}", e);
                    let _ = action_tx.send(ResumePromptResult::RemainPaused);
                }
            }
            Err(e) => {
                tracing::error!("Failed to display resume prompt notification: {:?}", e);
                let _ = action_tx.send(ResumePromptResult::RemainPaused);
            }
        }
    });
}
