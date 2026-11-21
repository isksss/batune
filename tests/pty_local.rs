#![cfg(unix)]

use std::sync::mpsc;
use std::time::{Duration, Instant};

use batune::pty::{Command, Event, LocalSession, Session, Size};

const TIMEOUT: Duration = Duration::from_secs(10);

fn spawn(command: Command) -> (LocalSession, mpsc::Receiver<Event>) {
    let (tx, rx) = mpsc::channel();
    let session = LocalSession::spawn(&command, Size::default(), move |event| {
        let _ = tx.send(event);
    })
    .expect("spawn");
    (session, rx)
}

/// Collects output until `Closed`.
fn read_to_end(rx: &mpsc::Receiver<Event>) -> String {
    let mut out = Vec::new();
    loop {
        match rx.recv_timeout(TIMEOUT).expect("event before timeout") {
            Event::Output(bytes) => out.extend_from_slice(&bytes),
            Event::Closed => return String::from_utf8_lossy(&out).into_owned(),
        }
    }
}

#[test]
fn output_of_the_program_is_delivered_then_closed() {
    let (mut session, rx) = spawn(Command::new("echo").arg("hello pty"));
    assert!(read_to_end(&rx).contains("hello pty"));
    assert!(session.wait().unwrap().success());
}

#[test]
fn written_input_reaches_the_program() {
    let (mut session, rx) = spawn(
        Command::new("sh")
            .arg("-c")
            .arg("read line; echo got:$line"),
    );
    session.write_all(b"abc\r").unwrap();
    assert!(read_to_end(&rx).contains("got:abc"));
    assert!(session.wait().unwrap().success());
}

#[test]
fn writing_to_a_program_that_does_not_read_returns_without_waiting() {
    let (mut session, rx) = spawn(Command::new("sleep").arg("30"));
    let chunk = vec![b'x'; 64 * 1024];
    let started = Instant::now();
    // 64 MiB is far more than the kernel's PTY buffer holds.
    let refused = (0..1024)
        .map(|_| session.write_all(&chunk))
        .filter(|r| {
            r.as_ref()
                .is_err_and(|e| e.kind() == std::io::ErrorKind::WouldBlock)
        })
        .count();
    assert!(
        started.elapsed() < TIMEOUT,
        "write_all waited for the program"
    );
    assert!(refused > 0, "nothing was refused");
    session.kill().unwrap();
    read_to_end(&rx);
}

#[test]
fn killed_program_ends_the_session() {
    let (mut session, rx) = spawn(Command::new("sleep").arg("30"));
    session.kill().unwrap();
    read_to_end(&rx);
    assert!(!session.wait().unwrap().success());
}

#[test]
fn exit_code_of_the_program_is_reported() {
    let (mut session, rx) = spawn(Command::new("sh").arg("-c").arg("exit 3"));
    read_to_end(&rx);
    let status = session.wait().unwrap();
    assert_eq!(status.code, Some(3));
    assert!(!status.success());
}

#[test]
fn try_wait_is_none_while_running_and_some_after_exit() {
    let (mut session, rx) = spawn(Command::new("sh").arg("-c").arg("read x"));
    assert_eq!(session.try_wait().unwrap(), None);
    session.write_all(b"\r").unwrap();
    read_to_end(&rx);
    session.wait().unwrap();
    assert!(session.try_wait().unwrap().is_some_and(|s| s.success()));
}

#[test]
fn resize_is_visible_to_stty_size() {
    let (mut session, rx) = spawn(Command::new("sh").arg("-c").arg("read x; stty size"));
    session.resize(Size::new(40, 120)).unwrap();
    session.write_all(b"\r").unwrap();
    assert!(read_to_end(&rx).contains("40 120"));
}

#[test]
fn default_shell_sees_terminal_env() {
    let (mut session, rx) = spawn(
        batune::pty::default_shell()
            .arg("-c")
            .arg("echo term=$TERM color=$COLORTERM"),
    );
    let out = read_to_end(&rx);
    assert!(out.contains("term=xterm-256color color=truecolor"), "{out}");
    session.wait().unwrap();
}

#[test]
fn env_and_cwd_are_applied() {
    let dir = std::env::temp_dir();
    let (mut session, rx) = spawn(
        Command::new("sh")
            .arg("-c")
            .arg("echo v=$BATUNE_TEST; pwd")
            .env("BATUNE_TEST", "42")
            .cwd(&dir),
    );
    let out = read_to_end(&rx);
    assert!(out.contains("v=42"), "{out}");
    let canonical = dir.canonicalize().unwrap();
    assert!(out.contains(canonical.to_str().unwrap()), "{out}");
    session.wait().unwrap();
}
