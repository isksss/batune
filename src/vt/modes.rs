/// Terminal modes set by the application. Hosts read them through
/// [`Term::modes`](crate::vt::Term::modes) to decide how to encode keys, mouse
/// events and pastes, and whether to draw the cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Modes {
    /// DECCKM (`?1`): arrow keys send `ESC O A` instead of `ESC [ A`.
    pub cursor_keys_app: bool,
    /// DECKPAM / DECKPNM (`ESC =` / `ESC >`): the numeric keypad sends
    /// application sequences.
    pub keypad_app: bool,
    /// DECSCNM (`?5`): the whole screen is drawn in reverse video.
    pub reverse_video: bool,
    /// DECOM (`?6`): cursor addressing is relative to the scroll region and
    /// the cursor cannot leave it.
    pub origin: bool,
    /// DECAWM (`?7`).
    pub autowrap: bool,
    /// DECTCEM (`?25`): the cursor is drawn.
    pub show_cursor: bool,
    /// IRM (`CSI 4 h`): printed characters push the rest of the row right.
    pub insert: bool,
    /// LNM (`CSI 20 h`): LF, VT and FF also return to the first column, and
    /// the host should send CR LF for the Return key.
    pub linefeed_newline: bool,
    /// `?2004`: pasted text must be wrapped in `ESC [200~` and `ESC [201~`.
    pub bracketed_paste: bool,
    /// `?1004`: the host reports focus changes with `ESC [I` and `ESC [O`.
    pub focus_reporting: bool,
    /// `?2026`: the application is in the middle of a frame; the host should
    /// hold off redrawing until it is reset (or a timeout passes).
    pub synchronized_output: bool,
    /// `?2027`: printed text is grouped into grapheme clusters (UAX #29), so
    /// an emoji ZWJ sequence or a skin tone modifier shares one cell with
    /// its base instead of taking cells of its own.
    pub grapheme_cluster: bool,
    pub mouse_tracking: MouseTracking,
    pub mouse_encoding: MouseEncoding,
}

impl Default for Modes {
    fn default() -> Self {
        Self {
            cursor_keys_app: false,
            keypad_app: false,
            reverse_video: false,
            origin: false,
            autowrap: true,
            show_cursor: true,
            insert: false,
            linefeed_newline: false,
            bracketed_paste: false,
            focus_reporting: false,
            synchronized_output: false,
            // On, as in Ghostty (`grapheme-width-method = unicode`). Why not
            // off, as in the Contour proposal (undefined until set) or foot
            // (wcwidth unless set): almost no application sets it, so off
            // would leave every emoji sequence broken apart. A shell whose
            // wcwidth disagrees can still turn it off with `CSI ? 2027 l`.
            grapheme_cluster: true,
            mouse_tracking: MouseTracking::Off,
            mouse_encoding: MouseEncoding::Default,
        }
    }
}

/// Which mouse events the application wants reported.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MouseTracking {
    #[default]
    Off,
    /// `?9`: button presses only, without modifiers.
    X10,
    /// `?1000`: presses and releases (and the wheel).
    Normal,
    /// `?1002`: also motion while a button is held.
    ButtonEvent,
    /// `?1003`: also motion with no button held.
    AnyEvent,
}

/// How mouse reports are encoded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MouseEncoding {
    /// `ESC [ M` followed by three bytes, each value plus 32; coordinates
    /// beyond 223 cannot be reported.
    #[default]
    Default,
    /// `?1005`: as `Default`, but values above 95 are UTF-8 encoded.
    Utf8,
    /// `?1006`: `ESC [ < b ; x ; y M` (press) or `m` (release).
    Sgr,
}

/// The cursor's appearance, set by DECSCUSR (`CSI Ps SP q`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CursorStyle {
    pub shape: CursorShape,
    pub blinking: bool,
}

impl Default for CursorStyle {
    fn default() -> Self {
        Self {
            shape: CursorShape::Block,
            blinking: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorShape {
    #[default]
    Block,
    Underline,
    Bar,
}
