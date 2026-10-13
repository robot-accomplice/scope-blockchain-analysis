//! # Prompt input
//!
//! Reads the answer to an interactive prompt (the setup wizard, a token
//! selection) from stdin, so that a cancel can stop the read.
//!
//! In the TUI, a suspend-mode command reads its prompts on the plain
//! terminal. Ctrl-C there must cancel the command and return to the TUI. A
//! blocking `read_line` cannot be stopped, and a read on another thread
//! would stay behind and take the TUI's next keys. So the reader waits for
//! input in short steps and checks [`CANCEL`] between them. It reads one
//! byte at a time, so it never takes bytes after the newline.
//!
//! Outside the TUI nothing sets [`CANCEL`], and the reader acts as
//! `read_line` does.
//!
//! Every prompt goes through [`ask`] (or [`pick`], for a numbered list).
//! It pauses the progress bars, so a spinner never draws over a question.
//! A question, a list to pick from and the text around a prompt are not
//! data: they go to the diagnostic channel of the [`Output`]. So piped
//! data stays clean, and a TUI run keeps only data for Ctrl-Y and `save`.

use crate::cli::output::Output;
use scope::error::ScopeError;
use std::io::{self, BufRead};
use std::sync::atomic::{AtomicBool, Ordering};

/// Set when a signal cancels the suspend-mode command. A prompt read that
/// sees it stops with [`io::ErrorKind::Interrupted`]. The TUI clears it
/// when a suspend-mode command starts.
pub(crate) static CANCEL: AtomicBool = AtomicBool::new(false);

/// How long the reader waits for input before it checks the cancel flag
/// again. It is the longest delay between a cancel and the read's return.
#[cfg(unix)]
const POLL_INTERVAL_MS: libc::c_int = 100;

/// The error a cancelled read returns.
fn cancelled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "cancelled")
}

/// Writes `question` to the diagnostic channel of `out` and reads one
/// line of answer from `reader`. Every progress bar is paused from before
/// the question is written until the answer is read, so no bar draws over
/// it. The question may span lines. The channel is stderr on the CLI,
/// which Rust does not buffer, so the question shows before the read.
///
/// # Errors
///
/// Returns the write or read error.
pub(crate) fn ask(reader: &mut impl BufRead, out: &Output, question: &str) -> io::Result<String> {
    crate::cli::progress::paused(|| {
        crate::err!(out, "{question}")?;
        let mut answer = String::new();
        reader.read_line(&mut answer)?;
        Ok(answer)
    })
}

/// Shows `list` (it ends with a newline), asks `Select <what> (1-<count>): `
/// and returns the zero-based index of the answer.
///
/// # Errors
///
/// The write or read error; `Api` when the answer is not a number
/// from 1 to `count`.
pub(crate) fn pick(
    reader: &mut impl BufRead,
    out: &Output,
    list: &str,
    what: &str,
    count: usize,
) -> scope::error::Result<usize> {
    let question = format!("{list}\nSelect {what} (1-{count}): ");
    let answer = ask(reader, out, &question)?;
    let n: usize = answer
        .trim()
        .parse()
        .map_err(|_| ScopeError::Api("Invalid selection".to_string()))?;
    if !(1..=count).contains(&n) {
        return Err(ScopeError::Api(format!(
            "Selection must be between 1 and {count}"
        )));
    }
    Ok(n - 1)
}

/// The input for a prompt: stdin, cancellable with [`CANCEL`].
#[cfg(unix)]
pub(crate) fn stdin() -> PromptInput<'static> {
    PromptInput::new(libc::STDIN_FILENO, &CANCEL)
}

/// The input for a prompt: stdin. There is no cancel off unix.
#[cfg(not(unix))]
pub(crate) fn stdin() -> io::StdinLock<'static> {
    io::stdin().lock()
}

/// A line reader over a file descriptor that stops when `cancel` is set.
#[cfg(unix)]
pub(crate) struct PromptInput<'a> {
    fd: std::os::fd::RawFd,
    cancel: &'a AtomicBool,
    /// The byte `fill_buf` read and `consume` did not take yet.
    byte: [u8; 1],
    has_byte: bool,
}

#[cfg(unix)]
impl<'a> PromptInput<'a> {
    pub(crate) fn new(fd: std::os::fd::RawFd, cancel: &'a AtomicBool) -> Self {
        Self {
            fd,
            cancel,
            byte: [0],
            has_byte: false,
        }
    }

    /// Reads one byte. `None` at end of input. Waits in steps of
    /// [`POLL_INTERVAL_MS`] and stops with `Interrupted` when `cancel` is
    /// set.
    fn read_byte(&mut self) -> io::Result<Option<u8>> {
        if std::mem::take(&mut self.has_byte) {
            return Ok(Some(self.byte[0]));
        }
        loop {
            if self.cancel.load(Ordering::SeqCst) {
                return Err(cancelled());
            }
            let mut pfd = libc::pollfd {
                fd: self.fd,
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: one valid pollfd, and poll writes only its revents.
            let ready = unsafe { libc::poll(&mut pfd, 1, POLL_INTERVAL_MS) };
            if ready < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(e);
            }
            if ready == 0 {
                continue;
            }
            // Input, end of input or an error: the read tells which.
            let mut b = 0u8;
            // SAFETY: reads at most one byte into a valid one-byte buffer.
            let n = unsafe { libc::read(self.fd, (&mut b as *mut u8).cast(), 1) };
            match n {
                1 => return Ok(Some(b)),
                0 => return Ok(None),
                _ => {
                    let e = io::Error::last_os_error();
                    if matches!(
                        e.kind(),
                        io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                    ) {
                        continue;
                    }
                    return Err(e);
                }
            }
        }
    }
}

#[cfg(unix)]
impl io::Read for PromptInput<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let Some(first) = buf.first_mut() else {
            return Ok(0);
        };
        Ok(match self.read_byte()? {
            Some(b) => {
                *first = b;
                1
            }
            None => 0,
        })
    }
}

#[cfg(unix)]
impl BufRead for PromptInput<'_> {
    /// The prompts use `read_line`; this exists because `BufRead` needs
    /// it. A cancel returns `Other`, not `Interrupted`: the std helpers
    /// over `fill_buf` retry on `Interrupted`, so they would never stop.
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if !self.has_byte {
            match self.read_byte() {
                Ok(Some(b)) => {
                    self.byte[0] = b;
                    self.has_byte = true;
                }
                Ok(None) => {}
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {
                    return Err(io::Error::other(e));
                }
                Err(e) => return Err(e),
            }
        }
        Ok(&self.byte[..usize::from(self.has_byte)])
    }

    fn consume(&mut self, amt: usize) {
        if amt > 0 {
            self.has_byte = false;
        }
    }

    /// Reads up to and including the newline, as `BufRead::read_line`
    /// does, and appends it to `buf`. Returns the number of bytes read; 0
    /// at end of input.
    ///
    /// # Errors
    ///
    /// `Interrupted` when the cancel flag is set; `InvalidData` when the
    /// line is not UTF-8 (`buf` is then unchanged); or the read error.
    fn read_line(&mut self, buf: &mut String) -> io::Result<usize> {
        let mut line = Vec::new();
        while let Some(b) = self.read_byte()? {
            line.push(b);
            if b == b'\n' {
                break;
            }
        }
        let text = String::from_utf8(line).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "stream did not contain valid UTF-8",
            )
        })?;
        buf.push_str(&text);
        Ok(text.len())
    }
}

#[cfg(test)]
mod ask_tests {
    use super::*;
    use crate::cli::progress::testing::{REGISTRY_LOCK, Screen, drawing_spinner};
    use std::io::Write;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;

    /// An answer that takes a while to type. It notes how many draws the
    /// screen had when the answer came.
    struct SlowAnswer<'a> {
        screen: &'a Screen,
        draws_at_answer: Option<usize>,
    }

    impl io::Read for SlowAnswer<'_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.draws_at_answer.is_some() {
                return Ok(0);
            }
            std::thread::sleep(Duration::from_millis(40));
            self.draws_at_answer = Some(self.screen.ops().len());
            buf[..2].copy_from_slice(b"y\n");
            Ok(2)
        }
    }

    /// A diagnostic channel that notes how many draws the screen had when
    /// the question was written.
    struct Question {
        screen: Screen,
        draws_at_question: Arc<AtomicUsize>,
    }

    impl Write for Question {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.draws_at_question
                .store(self.screen.ops().len(), Ordering::SeqCst);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn test_ask_keeps_a_ticking_spinner_off_the_question() {
        // crawl by symbol: the "Searching..." spinner drew over "Save this
        // token for future use? [y/N]" until the answer came.
        let _lock = REGISTRY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let screen = Screen::default();
        let sp = drawing_spinner("Searching...", &Output::capture().0, &screen);
        std::thread::sleep(Duration::from_millis(30));
        let mut answer = SlowAnswer {
            screen: &screen,
            draws_at_answer: None,
        };
        let mut reader = io::BufReader::new(&mut answer);
        let draws_at_question = Arc::new(AtomicUsize::new(0));
        let out = Output::from_writers(
            Vec::new(),
            Question {
                screen: screen.clone(),
                draws_at_question: draws_at_question.clone(),
            },
        );
        let got = ask(&mut reader, &out, "Save? [y/N]: ").unwrap();
        drop(reader);
        assert_eq!(got, "y\n");
        assert_eq!(
            answer.draws_at_answer,
            Some(draws_at_question.load(Ordering::SeqCst)),
            "the spinner drew between the question and the answer"
        );
        sp.finish_and_clear();
    }

    #[test]
    fn test_a_prompt_never_reaches_the_data_channel() {
        // `scope crawl USDC > out.json`: the question must show on the
        // terminal and stay out of the file.
        let (out, cap) = Output::capture();
        let got = ask(&mut io::Cursor::new(b"y\n".to_vec()), &out, "Save? [y/N]: ").unwrap();
        assert_eq!(got, "y\n");
        assert_eq!(cap.out(), "");
        assert_eq!(cap.err(), "Save? [y/N]: ");
    }

    #[test]
    fn test_ask_returns_the_write_error() {
        let out = Output::from_writers(Vec::new(), crate::cli::output::tests::ClosedPipe);
        let e = ask(&mut io::Cursor::new(b"y\n".to_vec()), &out, "q? ").unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn test_pick_returns_the_index_and_rejects_bad_answers() {
        let pick_with = |answer: &str| {
            let (out, cap) = Output::capture();
            let got = pick(
                &mut io::Cursor::new(answer.as_bytes().to_vec()),
                &out,
                "1 a\n2 b\n",
                "token",
                2,
            );
            assert_eq!(cap.err(), "1 a\n2 b\n\nSelect token (1-2): ");
            assert_eq!(cap.out(), "", "the list is not data");
            got.map_err(|e| e.to_string())
        };
        assert_eq!(pick_with("2\n"), Ok(1));
        assert!(pick_with("x\n").unwrap_err().contains("Invalid selection"));
        assert!(
            pick_with("3\n")
                .unwrap_err()
                .contains("Selection must be between 1 and 2")
        );
        assert!(
            pick_with("0\n")
                .unwrap_err()
                .contains("Selection must be between 1 and 2")
        );
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::Read;
    use std::os::fd::RawFd;
    use std::time::{Duration, Instant};

    /// A pipe: (read end, write end). Closed on drop.
    struct Pipe(RawFd, RawFd);

    impl Pipe {
        fn new() -> Self {
            let mut fds = [0; 2];
            // SAFETY: pipe writes two fds into a valid two-element array.
            assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
            Pipe(fds[0], fds[1])
        }

        fn write(&self, bytes: &[u8]) {
            // SAFETY: writes from a valid buffer of the given length.
            let n = unsafe { libc::write(self.1, bytes.as_ptr().cast(), bytes.len()) };
            assert_eq!(n, bytes.len() as isize);
        }

        fn close_write(&mut self) {
            // SAFETY: closes the write end once; -1 marks it closed.
            unsafe { libc::close(self.1) };
            self.1 = -1;
        }

        /// What is left in the pipe, read without the prompt reader.
        fn rest(&mut self) -> Vec<u8> {
            self.close_write();
            let mut out = Vec::new();
            let mut b = [0u8; 64];
            loop {
                // SAFETY: reads into a valid buffer of the given length.
                let n = unsafe { libc::read(self.0, b.as_mut_ptr().cast(), b.len()) };
                if n <= 0 {
                    return out;
                }
                out.extend_from_slice(&b[..n as usize]);
            }
        }
    }

    impl Drop for Pipe {
        fn drop(&mut self) {
            for fd in [self.0, self.1] {
                if fd >= 0 {
                    // SAFETY: each open fd is closed once.
                    unsafe { libc::close(fd) };
                }
            }
        }
    }

    #[test]
    fn test_reads_one_line_and_leaves_the_rest() {
        // The next prompt reads the next line: the reader must not take
        // bytes after the newline.
        let flag = AtomicBool::new(false);
        let mut pipe = Pipe::new();
        pipe.write(b"yes\nnext\n");
        let mut input = PromptInput::new(pipe.0, &flag);
        let mut line = String::from("old:");
        assert_eq!(input.read_line(&mut line).unwrap(), 4);
        assert_eq!(line, "old:yes\n", "appends, as read_line does");
        assert_eq!(pipe.rest(), b"next\n");
    }

    #[test]
    fn test_end_of_input_returns_the_partial_line_then_zero() {
        // A closed stdin must end the prompt as read_line does, not hang.
        let flag = AtomicBool::new(false);
        let mut pipe = Pipe::new();
        pipe.write(b"abc");
        pipe.close_write();
        let mut input = PromptInput::new(pipe.0, &flag);
        let mut line = String::new();
        assert_eq!(input.read_line(&mut line).unwrap(), 3);
        assert_eq!(line, "abc");
        assert_eq!(input.read_line(&mut line).unwrap(), 0);
    }

    #[test]
    fn test_invalid_utf8_is_an_error_and_leaves_the_buffer() {
        let flag = AtomicBool::new(false);
        let pipe = Pipe::new();
        pipe.write(b"\xff\n");
        let mut input = PromptInput::new(pipe.0, &flag);
        let mut line = String::from("kept");
        let e = input.read_line(&mut line).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::InvalidData);
        assert_eq!(line, "kept");
    }

    #[test]
    fn test_cancel_stops_a_waiting_read_and_takes_no_later_input() {
        // Ctrl-C at a prompt in the TUI: the read must stop soon, and no
        // reader may stay behind to take the TUI's next keys.
        let flag = AtomicBool::new(false);
        let mut pipe = Pipe::new();
        let started = Instant::now();
        let e = std::thread::scope(|s| {
            s.spawn(|| {
                std::thread::sleep(Duration::from_millis(150));
                flag.store(true, Ordering::SeqCst);
            });
            let mut line = String::new();
            PromptInput::new(pipe.0, &flag)
                .read_line(&mut line)
                .unwrap_err()
        });
        assert_eq!(e.kind(), io::ErrorKind::Interrupted);
        assert!(started.elapsed() < Duration::from_secs(1), "stopped late");
        pipe.write(b"k");
        std::thread::sleep(Duration::from_millis(2 * POLL_INTERVAL_MS as u64));
        assert_eq!(pipe.rest(), b"k", "the key after the cancel is still there");
    }

    #[test]
    fn test_a_set_flag_stops_the_read_before_it_waits() {
        let flag = AtomicBool::new(true);
        let mut pipe = Pipe::new();
        pipe.write(b"y\n");
        let mut line = String::new();
        let e = PromptInput::new(pipe.0, &flag)
            .read_line(&mut line)
            .unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::Interrupted);
        assert_eq!(pipe.rest(), b"y\n");
    }

    #[test]
    fn test_read_and_fill_buf_take_one_byte_at_a_time() {
        let flag = AtomicBool::new(false);
        let mut pipe = Pipe::new();
        pipe.write(b"ab");
        let mut input = PromptInput::new(pipe.0, &flag);
        assert_eq!(input.read(&mut []).unwrap(), 0);
        assert_eq!(input.fill_buf().unwrap(), b"a");
        assert_eq!(input.fill_buf().unwrap(), b"a", "not consumed yet");
        let mut buf = [0u8; 4];
        assert_eq!(input.read(&mut buf).unwrap(), 1, "the buffered byte");
        assert_eq!(buf[0], b'a');
        input.consume(0);
        assert_eq!(input.fill_buf().unwrap(), b"b");
        input.consume(1);
        assert_eq!(pipe.rest(), b"");
        assert_eq!(input.fill_buf().unwrap(), b"", "end of input");
        assert_eq!(input.read(&mut buf).unwrap(), 0);
    }

    #[test]
    fn test_fill_buf_cancel_is_not_interrupted() {
        // std's read_until retries Interrupted forever; fill_buf must not
        // return it.
        let flag = AtomicBool::new(true);
        let pipe = Pipe::new();
        let mut input = PromptInput::new(pipe.0, &flag);
        let e = input.fill_buf().unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::Other);
        let mut v = Vec::new();
        assert!(input.read_until(b'\n', &mut v).is_err(), "no endless retry");
    }

    #[test]
    fn test_a_read_error_is_returned() {
        // A directory polls as ready, and reading it fails (EISDIR).
        let flag = AtomicBool::new(false);
        // SAFETY: a valid NUL-terminated path; the fd is closed below.
        let fd = unsafe { libc::open(c"/".as_ptr(), libc::O_RDONLY) };
        assert!(fd >= 0);
        let mut line = String::new();
        let r = PromptInput::new(fd, &flag).read_line(&mut line);
        // SAFETY: closes the fd opened above, once.
        unsafe { libc::close(fd) };
        assert!(r.is_err());
        assert_ne!(r.unwrap_err().kind(), io::ErrorKind::Interrupted);
    }

    #[test]
    fn test_stdin_reads_from_the_process_cancel_flag() {
        let input = stdin();
        assert_eq!(input.fd, libc::STDIN_FILENO);
        assert!(std::ptr::eq(input.cancel, &CANCEL));
    }
}
