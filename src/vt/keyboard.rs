//! The kitty keyboard protocol's progressive enhancement flags
//! (<https://sw.kovidgoyal.net/kitty/keyboard-protocol/>): which key
//! encodings the application asked for with `CSI > flags u` and friends.

/// Enhancements the application turned on. Hosts read them through
/// [`Term::keyboard_flags`](crate::vt::Term::keyboard_flags) to decide how to
/// encode keys.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyboardFlags(u8);

impl KeyboardFlags {
    /// Bit 1: keys whose legacy encoding is ambiguous (Esc, modified
    /// Enter/Tab/Backspace, Ctrl or Alt with a text key) are sent as
    /// `CSI code ; mods u`.
    pub const DISAMBIGUATE: Self = Self(1);

    /// The flags this terminal implements. Requested bits outside it are
    /// dropped, so a query reports what keys will really look like instead
    /// of promising encodings (key releases, all keys as escapes, ...) the
    /// host never sends.
    const SUPPORTED: u8 = Self::DISAMBIGUATE.0;

    fn from_request(bits: usize) -> Self {
        Self(bits as u8 & Self::SUPPORTED)
    }

    pub fn bits(self) -> u8 {
        self.0
    }

    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

/// Entries kept by `CSI > u` before the oldest is dropped. The protocol
/// leaves the size to the terminal; a program that pushes more than this
/// without popping is broken anyway, and the bound keeps it from growing
/// memory without limit.
const STACK_LIMIT: usize = 16;

/// The flags in effect and the ones `CSI < u` goes back to. Each screen has
/// its own, as the protocol requires.
#[derive(Clone, Debug, Default)]
pub(crate) struct KeyboardStack {
    current: KeyboardFlags,
    saved: Vec<KeyboardFlags>,
}

impl KeyboardStack {
    pub(crate) fn current(&self) -> KeyboardFlags {
        self.current
    }

    /// `CSI > flags u`.
    pub(crate) fn push(&mut self, bits: usize) {
        if self.saved.len() == STACK_LIMIT {
            self.saved.remove(0);
        }
        self.saved.push(self.current);
        self.current = KeyboardFlags::from_request(bits);
    }

    /// `CSI < n u`. Popping everything resets the flags.
    pub(crate) fn pop(&mut self, n: usize) {
        for _ in 0..n {
            match self.saved.pop() {
                Some(flags) => self.current = flags,
                None => {
                    self.current = KeyboardFlags::default();
                    return;
                }
            }
        }
    }

    /// `CSI = flags ; mode u`: 1 replaces, 2 adds, 3 removes.
    pub(crate) fn set(&mut self, bits: usize, mode: usize) {
        let bits = KeyboardFlags::from_request(bits).0;
        self.current.0 = match mode {
            1 => bits,
            2 => self.current.0 | bits,
            3 => self.current.0 & !bits,
            _ => return,
        };
    }
}
