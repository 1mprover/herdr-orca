//! The card sidebar's optional status line: the first line printed by
//! `[ui.sidebar.cards].footer_command`, refreshed on an interval.

/// Runs `[ui.sidebar.cards].footer_command` off the input thread and keeps its
/// latest first line for the sidebar's status row.
#[derive(Default)]
pub(crate) struct CardsFooterRuntime {
    command: String,
    text: Option<String>,
    next_run_at: Option<std::time::Instant>,
    pending: Option<std::sync::mpsc::Receiver<Option<String>>>,
}

const FOOTER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const FOOTER_MAX_CHARS: usize = 120;

impl CardsFooterRuntime {
    pub(super) fn text(&self) -> Option<&str> {
        self.text.as_deref()
    }

    /// Collects a finished run and starts the next one when due. Returns
    /// whether the visible text changed.
    pub(super) fn tick(
        &mut self,
        now: std::time::Instant,
        enabled: bool,
        command: &str,
        interval_seconds: u64,
    ) -> bool {
        let command = command.trim();
        if !enabled || command.is_empty() {
            self.pending = None;
            self.next_run_at = None;
            self.command.clear();
            return self.text.take().is_some();
        }
        let mut changed = false;
        if self.command != command {
            self.command = command.to_owned();
            self.pending = None;
            self.next_run_at = None;
            changed |= self.text.take().is_some();
        }
        if let Some(receiver) = &self.pending {
            match receiver.try_recv() {
                Ok(output) => {
                    self.pending = None;
                    changed |= self.text != output;
                    self.text = output;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.pending = None,
            }
        }
        if self.pending.is_none() && self.next_run_at.is_none_or(|deadline| now >= deadline) {
            let interval = std::time::Duration::from_secs(interval_seconds.clamp(1, 3600));
            self.next_run_at = Some(now + interval);
            self.pending = spawn_footer_command(self.command.clone());
        }
        changed
    }
}

#[cfg(unix)]
fn spawn_footer_command(command: String) -> Option<std::sync::mpsc::Receiver<Option<String>>> {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("herdr-cards-footer".to_owned())
        .spawn(move || {
            let _ = sender.send(run_footer_command(&command));
        })
        .ok()?;
    Some(receiver)
}

#[cfg(not(unix))]
fn spawn_footer_command(_command: String) -> Option<std::sync::mpsc::Receiver<Option<String>>> {
    None
}

#[cfg(unix)]
fn run_footer_command(command: &str) -> Option<String> {
    use std::io::Read as _;
    use std::process::{Command, Stdio};

    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < FOOTER_TIMEOUT => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut output = String::new();
    child.stdout.take()?.read_to_string(&mut output).ok()?;
    footer_line(&output)
}

/// First non-empty output line with terminal escapes and control characters
/// removed, so a script written for a colored prompt still renders cleanly.
fn footer_line(output: &str) -> Option<String> {
    let line = output.lines().find(|line| !line.trim().is_empty())?;
    let mut clean = String::new();
    let mut chars = line.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            }
            continue;
        }
        if !character.is_control() {
            clean.push(character);
        }
    }
    let clean = clean.trim();
    (!clean.is_empty()).then(|| clean.chars().take(FOOTER_MAX_CHARS).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn footer_line_strips_escapes_and_keeps_the_first_line() {
        assert_eq!(
            footer_line("\n\u{1b}[32m5h 80%\u{1b}[0m · 7d 91%\nsecond\n").as_deref(),
            Some("5h 80% · 7d 91%")
        );
        assert_eq!(footer_line("  \n\t\n"), None);
    }
}
