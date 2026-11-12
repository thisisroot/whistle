//! Runs commands for the window: their output streams to it as events, and
//! `sudo` gets the password the user typed into the window.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use whistle_core::runner::{Cmd, Runner};

/// Progress of an install, removal or upgrade, sent as the `job` event.
#[derive(Clone, Serialize)]
pub struct JobEvent {
    /// The app's id, or "upgrade".
    pub job: String,
    /// "step" (a command starts), "note", "log" (a line of output), "done" or "failed".
    pub kind: &'static str,
    pub text: String,
}

pub struct GuiRunner {
    pub app: AppHandle,
    pub job: String,
    pub password: Option<String>,
}

impl GuiRunner {
    pub fn emit(&self, kind: &'static str, text: impl Into<String>) {
        let _ = self.app.emit(
            "job",
            JobEvent {
                job: self.job.clone(),
                kind,
                text: text.into(),
            },
        );
    }

    fn stream(&self, from: impl Read + Send + 'static) -> std::thread::JoinHandle<()> {
        let app = self.app.clone();
        let job = self.job.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(from).lines().map_while(Result::ok) {
                let _ = app.emit(
                    "job",
                    JobEvent {
                        job: job.clone(),
                        kind: "log",
                        text: line,
                    },
                );
            }
        })
    }
}

impl Runner for GuiRunner {
    fn run(&self, cmd: &Cmd) -> Result<()> {
        self.emit("step", cmd.display());
        let mut command = Command::new(&cmd.program);
        if cmd.program == "sudo" {
            match self.password {
                // -k: always read the password, so it never reaches the command's stdin.
                Some(_) => command.args(["-S", "-k", "-p", ""]),
                None => command.arg("-n"),
            };
        }
        command
            .args(&cmd.args)
            .envs(cmd.env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(dir) = &cmd.cwd {
            command.current_dir(dir);
        }
        let mut child = command
            .spawn()
            .with_context(|| format!("can't start {}", cmd.program))?;
        if let Some(mut stdin) = child.stdin.take()
            && cmd.program == "sudo"
            && let Some(password) = &self.password
        {
            let _ = writeln!(stdin, "{password}");
        }
        let out = self.stream(child.stdout.take().context("no stdout")?);
        let err = self.stream(child.stderr.take().context("no stderr")?);
        let deadline = cmd.timeout.map(|limit| Instant::now() + limit);
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if deadline.is_some_and(|d| Instant::now() >= d) {
                let _ = child.kill();
                let _ = child.wait();
                bail!("{} took too long and was stopped", cmd.program);
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        let _ = out.join();
        let _ = err.join();
        if !status.success() {
            bail!("{} failed ({status})", cmd.program);
        }
        Ok(())
    }

    fn is_dry(&self) -> bool {
        false
    }

    fn note(&self, message: &str) {
        self.emit("note", message);
    }
}

/// True when `sudo` would run without asking for a password (or isn't needed).
pub fn sudo_ready() -> bool {
    Command::new("sudo")
        .args(["-n", "true"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}
