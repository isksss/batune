//! A remote shell over SSH behind the same [`Session`] trait as the local
//! PTY, so the window code does not care which one it drives.
//!
//! The connection runs on a single-threaded tokio runtime in its own thread.
//! Until the shell starts, key input is read by prompts drawn into the
//! terminal (host key confirmation, passwords); after that it is forwarded
//! to the remote PTY.

mod auth;
mod host_key;
mod prompt;
mod public_key;
mod target;

use std::error::Error;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use russh::client::{self, Handle};
use russh::keys::{PublicKey, PublicKeyOrCertificate};
use russh::{ChannelMsg, Disconnect};
use tokio::sync::Notify;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::pty::{Event, ExitStatus, Session, Size};

pub use target::Target;

use prompt::{Edit, LineEditor};

type BoxError = Box<dyn Error + Send + Sync>;

enum Input {
    Data(Vec<u8>),
    Resize(Size),
    Close,
}

type OnEvent = Box<dyn FnMut(Event) + Send>;

/// The event callback, shared by the session task and the russh handler.
#[derive(Clone)]
struct Output(Arc<Mutex<OnEvent>>);

impl Output {
    fn send(&self, event: Event) {
        (self.0.lock().unwrap())(event);
    }

    /// Writes text, turning `\n` into `\r\n` as a cooked PTY would.
    fn print(&self, text: &str) {
        self.send(Event::Output(text.replace('\n', "\r\n").into_bytes()));
    }
}

/// Key input and window size as seen before and during the session.
struct Keys {
    rx: UnboundedReceiver<Input>,
    size: Size,
}

impl Keys {
    /// Shows `prompt` and reads one line. `None` when the user cancels or
    /// the window is closed.
    async fn read_line(&mut self, out: &Output, prompt: &str, echo: bool) -> Option<String> {
        out.print(prompt);
        let mut editor = LineEditor::new(echo);
        loop {
            match self.rx.recv().await? {
                Input::Data(bytes) => {
                    let (shown, edit) = editor.feed(&bytes);
                    if !shown.is_empty() {
                        out.send(Event::Output(shown));
                    }
                    match edit {
                        Edit::Pending => {}
                        Edit::Done(line) => return Some(line),
                        Edit::Cancel => return None,
                    }
                }
                Input::Resize(size) => self.size = size,
                Input::Close => return None,
            }
        }
    }
}

struct Done {
    status: Mutex<Option<ExitStatus>>,
    cond: Condvar,
}

pub struct SshSession {
    tx: UnboundedSender<Input>,
    done: Arc<Done>,
}

impl SshSession {
    /// Starts connecting in the background and returns at once. Events are
    /// delivered like [`crate::pty::LocalSession::spawn`]'s, ending with
    /// one `Closed`. Failures are shown in the terminal, which then waits
    /// for a key so that the message can be read.
    pub fn connect(
        target: Target,
        size: Size,
        on_event: impl FnMut(Event) + Send + 'static,
    ) -> io::Result<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let (tx, rx) = unbounded_channel();
        let done = Arc::new(Done {
            status: Mutex::new(None),
            cond: Condvar::new(),
        });
        let out = Output(Arc::new(Mutex::new(Box::new(on_event))));
        let thread_done = done.clone();
        thread::Builder::new().name("ssh".into()).spawn(move || {
            let keys = Keys { rx, size };
            let status = runtime.block_on(run(target, keys, out.clone()));
            *thread_done.status.lock().unwrap() = Some(status);
            thread_done.cond.notify_all();
            out.send(Event::Closed);
        })?;
        Ok(Self { tx, done })
    }

    fn send(&self, input: Input) -> io::Result<()> {
        self.tx
            .send(input)
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "ssh session has ended"))
    }
}

impl Session for SshSession {
    fn write_all(&mut self, data: &[u8]) -> io::Result<()> {
        self.send(Input::Data(data.to_vec()))
    }

    fn resize(&mut self, size: Size) -> io::Result<()> {
        self.send(Input::Resize(size))
    }

    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        Ok(self.done.status.lock().unwrap().clone())
    }

    fn wait(&mut self) -> io::Result<ExitStatus> {
        let mut status = self.done.status.lock().unwrap();
        loop {
            if let Some(status) = status.as_ref() {
                return Ok(status.clone());
            }
            status = self.done.cond.wait(status).unwrap();
        }
    }

    fn kill(&mut self) -> io::Result<()> {
        // The task may already be gone, which is what kill asks for anyway.
        let _ = self.tx.send(Input::Close);
        Ok(())
    }
}

/// Exit code reported for connection failures, as OpenSSH's `ssh` does.
const FAILED: u32 = 255;

async fn run(target: Target, keys: Keys, out: Output) -> ExitStatus {
    let keys = Arc::new(tokio::sync::Mutex::new(keys));
    match session(&target, keys.clone(), &out).await {
        Ok(status) => status,
        Err(e) => {
            out.print(&format!(
                "\nbatune: {target}: {e}\n[press any key to close]"
            ));
            let mut keys = keys.lock().await;
            while let Some(Input::Resize(_)) = keys.rx.recv().await {}
            ExitStatus {
                code: Some(FAILED),
                signal: None,
            }
        }
    }
}

async fn session(
    target: &Target,
    keys: Arc<tokio::sync::Mutex<Keys>>,
    out: &Output,
) -> Result<ExitStatus, BoxError> {
    let handler = Handler {
        host: target.host.clone(),
        port: target.port,
        keys: keys.clone(),
        out: out.clone(),
        accepted: None,
        changed_key: Arc::new(AtomicBool::new(false)),
        prompting: Arc::new(Notify::new()),
    };
    let changed_key = handler.changed_key.clone();
    let prompting = handler.prompting.clone();
    out.print(&format!("Connecting to {target}...\n"));
    let mut config = client::Config {
        // Without keepalives a connection that died while the machine slept
        // hangs silently; russh closes it after `keepalive_max` (3) misses.
        keepalive_interval: Some(Duration::from_secs(30)),
        nodelay: true,
        ..Default::default()
    };
    // An unreadable file is reported by the key check itself.
    let recorded: Vec<PublicKey> = host_key::known_hosts_path()
        .and_then(|path| host_key::recorded(&path, &target.host, target.port).ok())
        .into_iter()
        .flatten()
        .map(|(_, key)| key)
        .collect();
    config.preferred.key = host_key::prefer_recorded(&config.preferred.key, &recorded).into();
    let config = Arc::new(config);
    // An unresponsive host keeps the TCP connect or the key exchange waiting
    // for minutes, so the keyboard is watched meanwhile.
    let mut handle = tokio::select! {
        handle = client::connect(config, (target.host.as_str(), target.port), handler) => handle?,
        () = interrupted(&keys, &prompting) => return Err("interrupted".into()),
    };
    // The handler only prompts during key exchange, which is over; from here
    // on this task owns the keyboard.
    let mut keys = keys.lock().await;
    if !auth::authenticate(
        &mut handle,
        target,
        &mut keys,
        out,
        changed_key.load(Ordering::Relaxed),
    )
    .await?
    {
        return Err("authentication failed".into());
    }
    let status = shell(&handle, &mut keys, out).await;
    let _ = handle.disconnect(Disconnect::ByApplication, "", "en").await;
    status
}

/// Resolves when the user presses Ctrl-C or the tab is closed. Other input
/// is dropped: nothing is waiting for it yet. Whenever `prompting` is
/// notified the keyboard is let go, so that the host key prompt can take it
/// (tokio's mutex is fair, so the prompt gets it before this loop does
/// again).
async fn interrupted(keys: &tokio::sync::Mutex<Keys>, prompting: &Notify) {
    loop {
        let mut keys = keys.lock().await;
        loop {
            tokio::select! {
                input = keys.rx.recv() => match input {
                    Some(Input::Data(bytes)) if bytes.contains(&0x03) => return,
                    Some(Input::Data(_)) => {}
                    Some(Input::Resize(size)) => keys.size = size,
                    Some(Input::Close) | None => return,
                },
                () = prompting.notified() => break,
            }
        }
    }
}

async fn shell(
    handle: &Handle<Handler>,
    keys: &mut Keys,
    out: &Output,
) -> Result<ExitStatus, BoxError> {
    let channel = handle.channel_open_session().await?;
    let size = keys.size;
    channel
        .request_pty(
            false,
            "xterm-256color",
            size.cols.into(),
            size.rows.into(),
            size.pixel_width.into(),
            size.pixel_height.into(),
            &[],
        )
        .await?;
    channel.request_shell(false).await?;
    let (mut read, write) = channel.split();
    let mut status = ExitStatus {
        code: None,
        signal: None,
    };
    loop {
        tokio::select! {
            msg = read.wait() => match msg {
                Some(ChannelMsg::Data { data }) | Some(ChannelMsg::ExtendedData { data, .. }) => {
                    out.send(Event::Output(data.to_vec()));
                }
                Some(ChannelMsg::ExitStatus { exit_status }) => status.code = Some(exit_status),
                Some(ChannelMsg::ExitSignal { signal_name, .. }) => {
                    status.signal = Some(format!("{signal_name:?}"));
                }
                Some(ChannelMsg::Eof) => {}
                Some(ChannelMsg::Close) | None => break,
                Some(_) => {}
            },
            input = keys.rx.recv() => match input {
                Some(Input::Data(bytes)) => write.data_bytes(bytes).await?,
                Some(Input::Resize(size)) => {
                    keys.size = size;
                    write
                        .window_change(
                            size.cols.into(),
                            size.rows.into(),
                            size.pixel_width.into(),
                            size.pixel_height.into(),
                        )
                        .await?;
                }
                Some(Input::Close) | None => break,
            },
        }
    }
    Ok(status)
}

struct Handler {
    host: String,
    port: u16,
    keys: Arc<tokio::sync::Mutex<Keys>>,
    out: Output,
    /// The key accepted at the first exchange. A re-exchange must present
    /// the same key: prompting then would wait on the keyboard, which the
    /// shell loop holds.
    accepted: Option<PublicKey>,
    /// Set when the user accepted a key that differs from the recorded one.
    changed_key: Arc<AtomicBool>,
    /// Asks [`interrupted`] to hand over the keyboard.
    prompting: Arc<Notify>,
}

impl Handler {
    async fn verify(&mut self, key: PublicKey) -> Result<bool, BoxError> {
        if let Some(accepted) = &self.accepted {
            return Ok(*accepted == key);
        }
        let path = host_key::known_hosts_path().ok_or("cannot find the home directory")?;
        let fingerprint = host_key::fingerprint(&key);
        let algorithm = key.algorithm();
        let host = match self.port {
            22 => self.host.clone(),
            port => format!("[{}]:{port}", self.host),
        };
        let status = host_key::check(&path, &self.host, self.port, &key)?;
        let warning = match status {
            host_key::Status::Known => {
                self.accepted = Some(key);
                return Ok(true);
            }
            host_key::Status::Revoked => {
                self.out.print(&format!(
                    "The {algorithm} host key for '{host}' ({fingerprint}) is marked as revoked in {}.\n",
                    path.display()
                ));
                return Err("host key verification failed".into());
            }
            host_key::Status::Unknown => format!(
                "The authenticity of host '{host}' can't be established.\n\
                 {algorithm} key fingerprint is {fingerprint}.\n"
            ),
            // OpenSSH refuses outright here; the issue asks for a prompt, so
            // the default answer is no and known_hosts is left untouched.
            host_key::Status::Changed { line } => format!(
                "WARNING: THE HOST KEY FOR '{host}' HAS CHANGED!\n\
                 Someone could be eavesdropping on you right now.\n\
                 The {algorithm} key sent by the remote host is {fingerprint}.\n\
                 The old key is in {} line {line}; it is kept as is.\n",
                path.display()
            ),
        };
        self.out.print(&warning);
        self.prompting.notify_one();
        let answer = self
            .keys
            .lock()
            .await
            .read_line(
                &self.out,
                "Are you sure you want to continue connecting (yes/no)? ",
                true,
            )
            .await;
        if !answer.is_some_and(|a| a.eq_ignore_ascii_case("yes")) {
            return Err("host key verification failed".into());
        }
        if status == host_key::Status::Unknown {
            match host_key::learn(&path, &self.host, self.port, &key) {
                Ok(()) => self.out.print(&format!(
                    "Permanently added '{host}' ({algorithm}) to {}.\n",
                    path.display()
                )),
                Err(e) => self.out.print(&format!(
                    "Could not add the host key to {}: {e}\n",
                    path.display()
                )),
            }
        }
        if matches!(status, host_key::Status::Changed { .. }) {
            self.changed_key.store(true, Ordering::Relaxed);
        }
        self.accepted = Some(key);
        Ok(true)
    }
}

impl client::Handler for Handler {
    type Error = BoxError;

    async fn check_server_key(&mut self, key: &PublicKeyOrCertificate) -> Result<bool, BoxError> {
        let key = match key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key.clone(),
            PublicKeyOrCertificate::Certificate(cert) => {
                PublicKey::new(cert.public_key().clone(), "")
            }
        };
        self.verify(key).await
    }

    async fn auth_banner(&mut self, banner: &str, _: &mut client::Session) -> Result<(), BoxError> {
        self.out.print(banner);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(f)
    }

    #[test]
    fn ctrl_c_interrupts_the_connection_and_a_prompt_can_still_read_keys() {
        block_on(async {
            let (tx, rx) = unbounded_channel();
            let size = Size {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            };
            let keys = tokio::sync::Mutex::new(Keys { rx, size });
            let prompting = Notify::new();
            let watch = interrupted(&keys, &prompting);
            tokio::pin!(watch);
            tx.send(Input::Data(b"x".to_vec())).unwrap();
            let prompt = async {
                prompting.notify_one();
                let mut keys = keys.lock().await;
                tx.send(Input::Data(b"y".to_vec())).unwrap();
                match keys.rx.recv().await {
                    Some(Input::Data(bytes)) => bytes,
                    _ => Vec::new(),
                }
            };
            // `biased`: the watch takes the keyboard (and "x") first.
            tokio::select! {
                biased;
                () = &mut watch => panic!("ended without Ctrl-C"),
                got = prompt => assert_eq!(got, b"y"),
            }
            tx.send(Input::Data(b"a\x03".to_vec())).unwrap();
            tokio::time::timeout(Duration::from_secs(5), watch)
                .await
                .expect("Ctrl-C did not interrupt");
        });
    }
}
