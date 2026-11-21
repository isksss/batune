use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::thread;

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::pty::{Event, ExitStatus, Session, Size};

/// A program to run in a [`LocalSession`].
#[derive(Debug, Clone)]
pub struct Command {
    pub program: OsString,
    pub args: Vec<OsString>,
    /// Working directory; inherits the current one when `None`.
    pub cwd: Option<PathBuf>,
    /// Variables added to (or overriding) the inherited environment.
    pub env: Vec<(OsString, OsString)>,
}

impl Command {
    pub fn new(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            env: Vec::new(),
        }
    }

    pub fn arg(mut self, arg: impl Into<OsString>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn cwd(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }

    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    fn to_builder(&self) -> CommandBuilder {
        let mut builder = CommandBuilder::new(&self.program);
        builder.args(&self.args);
        if let Some(cwd) = &self.cwd {
            builder.cwd(cwd);
        }
        for (key, value) in &self.env {
            builder.env(key, value);
        }
        builder
    }
}

/// Writes waiting for the writer thread. Input is keystrokes, pastes and
/// replies to queries, so this many only piles up when the program has
/// stopped reading.
const PENDING_WRITES: usize = 256;

/// A program running on an OS pseudo terminal.
pub struct LocalSession {
    master: Box<dyn MasterPty + Send>,
    writes: SyncSender<Vec<u8>>,
    child: Box<dyn Child + Send + Sync>,
}

impl LocalSession {
    /// Starts `command` on a new pseudo terminal of `size`. `on_event` is
    /// called from a dedicated reader thread until it receives
    /// [`Event::Closed`].
    pub fn spawn(
        command: &Command,
        size: Size,
        mut on_event: impl FnMut(Event) + Send + 'static,
    ) -> io::Result<Self> {
        let pair = native_pty_system()
            .openpty(to_pty_size(size))
            .map_err(io::Error::other)?;
        let child = pair
            .slave
            .spawn_command(command.to_builder())
            .map_err(io::Error::other)?;
        // Keeping our copy of the slave open would stop the reader from ever
        // seeing EOF after the child exits.
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().map_err(io::Error::other)?;
        let writer = pair.master.take_writer().map_err(io::Error::other)?;
        let writes = spawn_writer(writer)?;

        thread::Builder::new()
            .name("batune-pty-reader".into())
            .spawn(move || {
                let mut buf = vec![0u8; 64 * 1024];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => on_event(Event::Output(buf[..n].to_vec())),
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                        // Linux reports EIO once the last slave fd closes; that
                        // is the normal end of output, not a failure to surface.
                        Err(_) => break,
                    }
                }
                on_event(Event::Closed);
            })?;

        Ok(Self {
            master: pair.master,
            writes,
            child,
        })
    }
}

impl Session for LocalSession {
    /// Queues `data` for the writer thread and returns at once, so a
    /// program that does not read its input cannot stall the caller. Fails
    /// with `WouldBlock` (dropping `data`) once that program has left
    /// [`PENDING_WRITES`] writes unread.
    fn write_all(&mut self, data: &[u8]) -> io::Result<()> {
        match self.writes.try_send(data.to_vec()) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "the program is not reading its input",
            )),
            Err(TrySendError::Disconnected(_)) => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "the terminal no longer accepts input",
            )),
        }
    }

    fn resize(&mut self, size: Size) -> io::Result<()> {
        self.master
            .resize(to_pty_size(size))
            .map_err(io::Error::other)
    }

    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        Ok(self.child.try_wait()?.map(to_exit_status))
    }

    fn wait(&mut self) -> io::Result<ExitStatus> {
        self.child.wait().map(to_exit_status)
    }

    fn kill(&mut self) -> io::Result<()> {
        self.child.kill()
    }
}

/// Moves writes to their own thread: a write to a PTY blocks once the
/// kernel buffer is full, which happens when the program stops reading.
/// The thread ends when the session is dropped or a write fails; later
/// writes then report `BrokenPipe`.
fn spawn_writer(mut writer: Box<dyn Write + Send>) -> io::Result<SyncSender<Vec<u8>>> {
    let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(PENDING_WRITES);
    thread::Builder::new()
        .name("batune-pty-writer".into())
        .spawn(move || {
            for data in rx {
                if writer
                    .write_all(&data)
                    .and_then(|()| writer.flush())
                    .is_err()
                {
                    break;
                }
            }
        })?;
    Ok(tx)
}

fn to_pty_size(size: Size) -> PtySize {
    PtySize {
        rows: size.rows,
        cols: size.cols,
        pixel_width: size.pixel_width,
        pixel_height: size.pixel_height,
    }
}

fn to_exit_status(status: portable_pty::ExitStatus) -> ExitStatus {
    match status.signal() {
        Some(signal) => ExitStatus {
            code: None,
            signal: Some(signal.to_string()),
        },
        None => ExitStatus {
            code: Some(status.exit_code()),
            signal: None,
        },
    }
}
