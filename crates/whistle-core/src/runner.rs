//! Running external commands. Everything Whistle executes goes through a
//! `Runner`, as an argv (never a shell string), so `--dry-run` and tests can
//! see exactly what would happen.

use std::cell::RefCell;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cmd {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: Option<PathBuf>,
    pub timeout: Option<Duration>,
}

impl Cmd {
    pub fn new(program: impl Into<String>) -> Self {
        Self { program: program.into(), args: vec![], env: vec![], cwd: None, timeout: None }
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn args<I: IntoIterator<Item = S>, S: Into<String>>(mut self, args: I) -> Self {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn envs<'a>(mut self, vars: impl IntoIterator<Item = &'a (String, String)>) -> Self {
        self.env.extend(vars.into_iter().cloned());
        self
    }

    pub fn cwd(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// The argv as one line, quoted for reading (not for a shell).
    pub fn argv(&self) -> Vec<&str> {
        std::iter::once(self.program.as_str()).chain(self.args.iter().map(String::as_str)).collect()
    }

    pub fn display(&self) -> String {
        let quote = |s: &str| {
            if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_./=:+,@%".contains(c)) {
                s.to_owned()
            } else {
                format!("'{}'", s.replace('\'', r"'\''"))
            }
        };
        let env = self.env.iter().map(|(k, v)| format!("{k}={}", quote(v)));
        env.chain(self.argv().into_iter().map(quote)).collect::<Vec<_>>().join(" ")
    }
}

pub trait Runner {
    fn run(&self, cmd: &Cmd) -> Result<()>;
    /// True when nothing is actually executed or written.
    fn is_dry(&self) -> bool;
    /// Say what is happening (or would happen).
    fn note(&self, message: &str);
}

/// Executes commands, with the terminal attached so sudo and installers can prompt.
pub struct RealRunner;

impl Runner for RealRunner {
    fn run(&self, cmd: &Cmd) -> Result<()> {
        eprintln!("+ {}", cmd.display());
        let mut command = Command::new(&cmd.program);
        command.args(&cmd.args).envs(cmd.env.iter().map(|(k, v)| (k, v)));
        if let Some(dir) = &cmd.cwd {
            command.current_dir(dir);
        }
        let mut child = command.spawn().with_context(|| format!("can't start {}", cmd.program))?;
        let status = match cmd.timeout {
            None => child.wait()?,
            Some(limit) => {
                let deadline = Instant::now() + limit;
                loop {
                    if let Some(status) = child.try_wait()? {
                        break status;
                    }
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        bail!("{} timed out after {}s", cmd.program, limit.as_secs());
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
            }
        };
        if !status.success() {
            bail!("{} failed ({status})", cmd.program);
        }
        Ok(())
    }

    fn is_dry(&self) -> bool {
        false
    }

    fn note(&self, message: &str) {
        eprintln!("{message}");
    }
}

/// Records commands instead of running them; optionally prints them.
#[derive(Default)]
pub struct DryRunner {
    pub echo: bool,
    pub log: RefCell<Vec<Cmd>>,
    pub notes: RefCell<Vec<String>>,
}

impl DryRunner {
    pub fn printing() -> Self {
        Self { echo: true, ..Self::default() }
    }

    pub fn commands(&self) -> Vec<String> {
        self.log.borrow().iter().map(Cmd::display).collect()
    }
}

impl Runner for DryRunner {
    fn run(&self, cmd: &Cmd) -> Result<()> {
        if self.echo {
            println!("would run: {}", cmd.display());
        }
        self.log.borrow_mut().push(cmd.clone());
        Ok(())
    }

    fn is_dry(&self) -> bool {
        true
    }

    fn note(&self, message: &str) {
        if self.echo {
            println!("would {message}");
        }
        self.notes.borrow_mut().push(message.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_quotes_only_when_needed() {
        let cmd = Cmd::new("wine").arg(r"C:\Program Files\x.exe").arg("/S");
        assert_eq!(cmd.display(), r"wine 'C:\Program Files\x.exe' /S");
    }

    #[test]
    fn real_runner_reports_failure() {
        if which::which("false").is_ok() {
            assert!(RealRunner.run(&Cmd::new("false")).is_err());
        }
    }
}
