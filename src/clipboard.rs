//! The system clipboard.

/// Opened on first use and kept open: on X11 the copied text is served
/// from this process, so dropping the handle would drop the text.
#[derive(Default)]
pub struct Clipboard {
    inner: Option<arboard::Clipboard>,
}

impl Clipboard {
    fn handle(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.inner.is_none() {
            match arboard::Clipboard::new() {
                Ok(c) => self.inner = Some(c),
                Err(e) => eprintln!("batune: open clipboard: {e}"),
            }
        }
        self.inner.as_mut()
    }

    pub fn copy(&mut self, text: &str) {
        if let Some(c) = self.handle()
            && let Err(e) = c.set_text(text)
        {
            eprintln!("batune: copy: {e}");
        }
    }

    /// The X11/Wayland primary selection. Other platforms have none, so
    /// the text is dropped there.
    pub fn copy_primary(&mut self, text: &str) {
        #[cfg(target_os = "linux")]
        {
            use arboard::{LinuxClipboardKind, SetExtLinux};
            if let Some(c) = self.handle()
                && let Err(e) = c.set().clipboard(LinuxClipboardKind::Primary).text(text)
            {
                eprintln!("batune: copy to primary: {e}");
            }
        }
        #[cfg(not(target_os = "linux"))]
        let _ = text;
    }

    pub fn paste(&mut self) -> Option<String> {
        match self.handle()?.get_text() {
            Ok(text) => Some(text),
            Err(e) => {
                eprintln!("batune: paste: {e}");
                None
            }
        }
    }
}

/// Where an OSC 52 store goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Targets {
    pub clipboard: bool,
    pub primary: bool,
}

impl Targets {
    /// `c` is the clipboard; `p` and `s` (xterm's "select") are the primary
    /// selection. An empty target means the clipboard: xterm's `s0` default
    /// is a cut buffer that nothing reads any more, and programs that omit
    /// the target (tmux, many editors) mean "copy".
    pub fn of(selection: &str) -> Self {
        if selection.is_empty() {
            return Self {
                clipboard: true,
                primary: false,
            };
        }
        Self {
            clipboard: selection.contains('c'),
            primary: selection.contains(['p', 's']),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc52_targets_follow_the_selection_letters() {
        let t = |clipboard, primary| Targets { clipboard, primary };
        assert_eq!(Targets::of(""), t(true, false));
        assert_eq!(Targets::of("c"), t(true, false));
        assert_eq!(Targets::of("p"), t(false, true));
        assert_eq!(Targets::of("s0"), t(false, true));
        assert_eq!(Targets::of("pc"), t(true, true));
        // Cut buffers alone go nowhere.
        assert_eq!(Targets::of("0"), t(false, false));
    }
}
