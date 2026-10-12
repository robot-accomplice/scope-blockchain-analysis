//! A scope TUI process in a PTY, with a vt100 screen model.

use anyhow::{Context, Result};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

const READ_CHUNK: usize = 8192;
const READY_TIMEOUT: Duration = Duration::from_secs(10);
const RECV_SLICE: Duration = Duration::from_millis(50);

pub struct StartOpts {
    pub binary: PathBuf,
    pub home: PathBuf,
    pub cols: u16,
    pub rows: u16,
    pub env: Vec<(String, String)>,
}

pub struct Session {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    rx: Receiver<Vec<u8>>,
    parser: vt100::Parser,
    raw: Vec<u8>,
    status: Option<i32>,
}

/// Where scope keeps its journal under a given HOME.
pub fn data_dir(home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/scope")
    } else {
        home.join(".local/share/scope")
    }
}

fn size(cols: u16, rows: u16) -> PtySize {
    PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

impl Session {
    pub fn start(o: &StartOpts) -> Result<Self> {
        let pair = native_pty_system().openpty(size(o.cols, o.rows))?;
        let mut cmd = CommandBuilder::new(&o.binary);
        cmd.args(["tui", "--no-banner"]);
        cmd.env_clear();
        cmd.env("PATH", std::env::var("PATH").unwrap_or_default());
        cmd.env("HOME", &o.home);
        cmd.env("TZ", "UTC");
        cmd.env("LANG", "C.UTF-8");
        cmd.env("TERM", "xterm-256color");
        for (k, v) in &o.env {
            cmd.env(k, v);
        }
        let child = pair.slave.spawn_command(cmd).context("spawning scope")?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let mut buf = [0u8; READ_CHUNK];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        let mut session = Self {
            master: pair.master,
            writer,
            child,
            rx,
            parser: vt100::Parser::new(o.rows, o.cols, 0),
            raw: Vec::new(),
            status: None,
        };
        session.wait_for_first_paint()?;
        Ok(session)
    }

    /// Waits until the TUI has entered the alternate screen. Input sent
    /// earlier is lost when the app switches the terminal to raw mode, which
    /// is slow on a cold start. Text on the normal screen (for example a
    /// warning on stderr before the TUI starts) does not count: ratatui
    /// enters the alternate screen only after it enables raw mode.
    fn wait_for_first_paint(&mut self) -> Result<()> {
        let until = Instant::now() + READY_TIMEOUT;
        while !self.parser.screen().alternate_screen() {
            anyhow::ensure!(
                Instant::now() < until,
                "scope did not start the TUI within {READY_TIMEOUT:?}; screen:\n{}",
                self.screen()
            );
            self.pump(RECV_SLICE)?;
            anyhow::ensure!(
                self.exit_status().is_none(),
                "scope exited before it drew; screen:\n{}",
                self.screen()
            );
        }
        Ok(())
    }

    pub fn send(&mut self, bytes: &[u8]) -> Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()?;
        Ok(())
    }

    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<()> {
        self.master.resize(size(cols, rows))?;
        self.parser.screen_mut().set_size(rows, cols);
        Ok(())
    }

    /// Reads output for up to `wait`, feeding the screen model.
    pub fn pump(&mut self, wait: Duration) -> Result<()> {
        let until = Instant::now() + wait;
        loop {
            let left = until.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left.min(RECV_SLICE)) {
                Ok(chunk) => {
                    self.parser.process(&chunk);
                    self.raw.extend_from_slice(&chunk);
                    if Instant::now() >= until {
                        return Ok(());
                    }
                }
                Err(RecvTimeoutError::Disconnected) => return Ok(()),
                Err(RecvTimeoutError::Timeout) if Instant::now() >= until => return Ok(()),
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
    }

    pub fn screen(&self) -> String {
        crate::screen::text(&self.parser)
    }

    pub fn raw(&self) -> &[u8] {
        &self.raw
    }

    /// The exit code once the process has ended.
    pub fn exit_status(&mut self) -> Option<i32> {
        if self.status.is_none()
            && let Ok(Some(s)) = self.child.try_wait()
        {
            self.status = Some(s.exit_code() as i32);
        }
        self.status
    }

    /// Kills the child and reaps it. Waits only after a successful kill, so it cannot hang.
    pub fn kill(&mut self) {
        if self.child.kill().is_ok() {
            let _ = self.child.wait();
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.kill();
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A stand-in for scope that prints a line on the normal screen and
    /// exits without starting the TUI.
    fn fake_binary(dir: &Path) -> PathBuf {
        let path = dir.join("fake-scope");
        std::fs::write(
            &path,
            "#!/bin/sh\necho 'warning: before the TUI'\nsleep 1\n",
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn test_text_before_the_alternate_screen_is_not_ready() {
        // Pre-TUI stderr text must not count as the first paint: input sent
        // then arrives before raw mode. The error must show that text.
        let dir = tempfile::tempdir().unwrap();
        let opts = StartOpts {
            binary: fake_binary(dir.path()),
            home: dir.path().to_path_buf(),
            cols: 80,
            rows: 24,
            env: vec![],
        };
        let err = Session::start(&opts).err().expect("must not be ready");
        let msg = format!("{err:#}");
        assert!(msg.contains("exited before it drew"), "{msg}");
        assert!(msg.contains("warning: before the TUI"), "{msg}");
    }
}
