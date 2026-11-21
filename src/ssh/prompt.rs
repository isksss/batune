//! Line input for prompts shown inside the terminal before the remote shell
//! exists (host key confirmation, passwords). The terminal sends raw key
//! bytes, so echo and editing are done here.

/// Result of feeding key bytes to a [`LineEditor`].
#[derive(Debug, PartialEq, Eq)]
pub enum Edit {
    /// More input is needed.
    Pending,
    /// Enter was pressed.
    Done(String),
    /// Ctrl-C, or Ctrl-D on an empty line.
    Cancel,
}

pub struct LineEditor {
    line: String,
    echo: bool,
}

impl LineEditor {
    /// `echo: false` hides the typed text, for passwords.
    pub fn new(echo: bool) -> Self {
        Self {
            line: String::new(),
            echo,
        }
    }

    /// Applies key bytes and returns what to show on screen. Input after
    /// Enter in the same chunk is discarded.
    pub fn feed(&mut self, bytes: &[u8]) -> (Vec<u8>, Edit) {
        let mut shown = Vec::new();
        for c in String::from_utf8_lossy(bytes).chars() {
            match c {
                '\r' | '\n' => {
                    shown.extend_from_slice(b"\r\n");
                    return (shown, Edit::Done(std::mem::take(&mut self.line)));
                }
                '\x03' => {
                    shown.extend_from_slice(b"^C\r\n");
                    return (shown, Edit::Cancel);
                }
                '\x04' if self.line.is_empty() => {
                    shown.extend_from_slice(b"\r\n");
                    return (shown, Edit::Cancel);
                }
                '\x7f' | '\x08' => {
                    if self.line.pop().is_some() && self.echo {
                        shown.extend_from_slice(b"\x08 \x08");
                    }
                }
                '\x15' => {
                    if self.echo {
                        for _ in 0..self.line.chars().count() {
                            shown.extend_from_slice(b"\x08 \x08");
                        }
                    }
                    self.line.clear();
                }
                // Arrow and function keys arrive as one escape sequence per
                // chunk; inserting their tail as text would corrupt the line.
                '\x1b' => break,
                c if c.is_control() => {}
                c => {
                    self.line.push(c);
                    if self.echo {
                        let mut buf = [0; 4];
                        shown.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                    }
                }
            }
        }
        (shown, Edit::Pending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enter_returns_the_typed_line_and_echoes_it() {
        let mut ed = LineEditor::new(true);
        assert_eq!(ed.feed(b"ye"), (b"ye".to_vec(), Edit::Pending));
        assert_eq!(
            ed.feed(b"s\r"),
            (b"s\r\n".to_vec(), Edit::Done("yes".into()))
        );
    }

    #[test]
    fn hidden_input_shows_nothing_but_the_newline() {
        let mut ed = LineEditor::new(false);
        assert_eq!(ed.feed(b"secret"), (Vec::new(), Edit::Pending));
        assert_eq!(ed.feed(b"\x7f"), (Vec::new(), Edit::Pending));
        assert_eq!(
            ed.feed(b"\r"),
            (b"\r\n".to_vec(), Edit::Done("secre".into()))
        );
    }

    #[test]
    fn backspace_removes_a_whole_character() {
        let mut ed = LineEditor::new(true);
        ed.feed("aé".as_bytes());
        assert_eq!(ed.feed(b"\x7f"), (b"\x08 \x08".to_vec(), Edit::Pending));
        assert_eq!(ed.feed(b"\r").1, Edit::Done("a".into()));
    }

    #[test]
    fn ctrl_u_clears_the_line() {
        let mut ed = LineEditor::new(true);
        ed.feed(b"ab");
        assert_eq!(ed.feed(b"\x15").0, b"\x08 \x08\x08 \x08".to_vec());
        assert_eq!(ed.feed(b"c\r").1, Edit::Done("c".into()));
    }

    #[test]
    fn ctrl_c_cancels_and_ctrl_d_cancels_only_an_empty_line() {
        assert_eq!(LineEditor::new(true).feed(b"x\x03").1, Edit::Cancel);
        let mut ed = LineEditor::new(true);
        assert_eq!(ed.feed(b"\x04").1, Edit::Cancel);
        let mut ed = LineEditor::new(true);
        assert_eq!(ed.feed(b"x\x04\r").1, Edit::Done("x".into()));
    }

    #[test]
    fn escape_sequences_are_ignored() {
        let mut ed = LineEditor::new(true);
        assert_eq!(ed.feed(b"\x1b[A"), (Vec::new(), Edit::Pending));
        assert_eq!(ed.feed(b"\r").1, Edit::Done(String::new()));
    }
}
