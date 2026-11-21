//! Key bindings: which keys the terminal handles itself, from the defaults
//! and the `[[keyboard.bindings]]` of the configuration file.

use std::sync::LazyLock;

use serde::Deserialize;
use winit::keyboard::{Key, ModifiersState, NamedKey};

/// What a bound key does instead of being sent to the program.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub enum Action {
    Copy,
    Paste,
    /// Opens the scrollback search.
    Search,
    ScrollPageUp,
    ScrollPageDown,
    ScrollLineUp,
    ScrollLineDown,
    /// The oldest line of the scrollback.
    ScrollToTop,
    /// The live screen.
    ScrollToBottom,
    /// Changes the font size for this run only; `font.size` in the file is
    /// left as it is.
    IncreaseFontSize,
    DecreaseFontSize,
    /// Back to `font.size`.
    ResetFontSize,
    ToggleFullscreen,
    ClearSelection,
    /// A tab with the default shell or profile.
    NewTab,
    CloseTab,
    /// The tab to the right, from the last back to the first.
    NextTab,
    PreviousTab,
    /// The settings screen.
    OpenSettings,
    /// Removes a default binding; the key goes to the program.
    None,
}

/// What a binding does: an action, or text sent to the program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Bound {
    Action(Action),
    Chars(String),
}

/// When a binding applies besides its key and modifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub enum Mode {
    /// Only while text is selected.
    Selection,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(try_from = "RawBinding")]
pub struct Binding {
    pub key: KeyName,
    pub mods: Mods,
    pub mode: Option<Mode>,
    pub bound: Bound,
}

/// A binding as written in the file, before `action` and `chars` are
/// checked against each other.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBinding {
    key: KeyName,
    #[serde(default)]
    mods: Mods,
    mode: Option<Mode>,
    action: Option<Action>,
    chars: Option<String>,
}

impl TryFrom<RawBinding> for Binding {
    type Error = String;

    fn try_from(raw: RawBinding) -> Result<Self, String> {
        let bound = match (raw.action, raw.chars) {
            (Some(action), None) => Bound::Action(action),
            (None, Some(chars)) => Bound::Chars(chars),
            (Some(_), Some(_)) => return Err("give either action or chars, not both".to_owned()),
            (None, None) => return Err("give action or chars".to_owned()),
        };
        Ok(Self {
            key: raw.key,
            mods: raw.mods,
            mode: raw.mode,
            bound,
        })
    }
}

impl Binding {
    const fn action(key: KeyName, mods: ModifiersState, action: Action) -> Self {
        Self {
            key,
            mods: Mods(mods),
            mode: None,
            bound: Bound::Action(action),
        }
    }

    fn matches(&self, key: &Key, mods: ModifiersState, selecting: bool) -> bool {
        self.mods.0 == mods
            && self.key.matches(key)
            && match self.mode {
                Some(Mode::Selection) => selecting,
                None => true,
            }
    }
}

/// Named keys that can be bound, by the name used in the file.
const NAMED: &[(&str, NamedKey)] = &[
    ("PageUp", NamedKey::PageUp),
    ("PageDown", NamedKey::PageDown),
    ("Home", NamedKey::Home),
    ("End", NamedKey::End),
    ("Insert", NamedKey::Insert),
    ("Delete", NamedKey::Delete),
    ("Up", NamedKey::ArrowUp),
    ("Down", NamedKey::ArrowDown),
    ("Left", NamedKey::ArrowLeft),
    ("Right", NamedKey::ArrowRight),
    ("Tab", NamedKey::Tab),
    ("Enter", NamedKey::Enter),
    ("Escape", NamedKey::Escape),
    ("Backspace", NamedKey::Backspace),
    ("Space", NamedKey::Space),
    ("F1", NamedKey::F1),
    ("F2", NamedKey::F2),
    ("F3", NamedKey::F3),
    ("F4", NamedKey::F4),
    ("F5", NamedKey::F5),
    ("F6", NamedKey::F6),
    ("F7", NamedKey::F7),
    ("F8", NamedKey::F8),
    ("F9", NamedKey::F9),
    ("F10", NamedKey::F10),
    ("F11", NamedKey::F11),
    ("F12", NamedKey::F12),
];

/// Characters that are awkward to write as themselves in a binding.
const CHAR_NAMES: &[(&str, char)] = &[("Plus", '+'), ("Minus", '-'), ("Equals", '=')];

/// A bindable key. Characters are kept in lower case and matched without
/// regard to case, since Shift turns the key's `c` into `C`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyName {
    Named(NamedKey),
    Char(char),
}

impl KeyName {
    fn parse(s: &str) -> Option<Self> {
        if let Some(&(_, named)) = NAMED.iter().find(|(name, _)| name.eq_ignore_ascii_case(s)) {
            return Some(Self::Named(named));
        }
        if let Some(&(_, c)) = CHAR_NAMES
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(s))
        {
            return Some(Self::Char(c));
        }
        let mut chars = s.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) if !c.is_whitespace() && !c.is_control() => {
                Some(Self::Char(c.to_ascii_lowercase()))
            }
            _ => None,
        }
    }

    fn matches(self, key: &Key) -> bool {
        match (self, key) {
            (Self::Named(named), Key::Named(pressed)) => named == *pressed,
            (Self::Char(c), Key::Character(s)) => {
                let mut chars = s.chars();
                matches!((chars.next(), chars.next()), (Some(p), None) if p.to_ascii_lowercase() == c)
            }
            _ => false,
        }
    }
}

impl<'de> Deserialize<'de> for KeyName {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Self::parse(&s).ok_or_else(|| {
            let names: Vec<_> = NAMED
                .iter()
                .map(|(name, _)| *name)
                .chain(CHAR_NAMES.iter().map(|(name, _)| *name))
                .collect();
            serde::de::Error::custom(format!(
                "unknown key {s:?} (a single character, or one of: {})",
                names.join(", ")
            ))
        })
    }
}

/// Modifiers written as `"Control|Shift"`; empty or `"None"` for none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods(pub ModifiersState);

impl Mods {
    fn parse(s: &str) -> Result<Self, String> {
        let mut mods = ModifiersState::empty();
        if s.trim().is_empty() || s.trim().eq_ignore_ascii_case("none") {
            return Ok(Self(mods));
        }
        for part in s.split('|').map(str::trim) {
            mods |= match part.to_ascii_lowercase().as_str() {
                "control" | "ctrl" => ModifiersState::CONTROL,
                "shift" => ModifiersState::SHIFT,
                "alt" => ModifiersState::ALT,
                "super" => ModifiersState::SUPER,
                _ => {
                    return Err(format!(
                        "unknown modifier {part:?} in {s:?} (Control, Shift, Alt, Super, joined with |)"
                    ));
                }
            };
        }
        Ok(Self(mods))
    }
}

impl<'de> Deserialize<'de> for Mods {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Self::parse(&s).map_err(serde::de::Error::custom)
    }
}

const CTRL: ModifiersState = ModifiersState::CONTROL;
const SHIFT: ModifiersState = ModifiersState::SHIFT;
const CTRL_SHIFT: ModifiersState = CTRL.union(SHIFT);

/// The bindings that apply unless the file changes them. `windows` adds the
/// Windows Terminal habit of copying with a bare Ctrl+C while text is
/// selected; Ctrl+C still interrupts the program otherwise.
pub fn defaults(windows: bool) -> Vec<Binding> {
    let mut bindings = vec![
        Binding::action(KeyName::Char('c'), CTRL_SHIFT, Action::Copy),
        Binding::action(KeyName::Char('v'), CTRL_SHIFT, Action::Paste),
        Binding::action(KeyName::Named(NamedKey::Insert), SHIFT, Action::Paste),
        Binding::action(KeyName::Char('f'), CTRL_SHIFT, Action::Search),
        Binding::action(
            KeyName::Named(NamedKey::PageUp),
            SHIFT,
            Action::ScrollPageUp,
        ),
        Binding::action(
            KeyName::Named(NamedKey::PageDown),
            SHIFT,
            Action::ScrollPageDown,
        ),
        // Shift gives `+` on most layouts, so the binding without it is for
        // the keypad and layouts with a `+` key of its own.
        Binding::action(KeyName::Char('='), CTRL, Action::IncreaseFontSize),
        Binding::action(KeyName::Char('+'), CTRL, Action::IncreaseFontSize),
        Binding::action(KeyName::Char('+'), CTRL_SHIFT, Action::IncreaseFontSize),
        Binding::action(KeyName::Char('-'), CTRL, Action::DecreaseFontSize),
        Binding::action(KeyName::Char('0'), CTRL, Action::ResetFontSize),
        Binding::action(KeyName::Char('t'), CTRL_SHIFT, Action::NewTab),
        Binding::action(KeyName::Char('w'), CTRL_SHIFT, Action::CloseTab),
        Binding::action(KeyName::Named(NamedKey::Tab), CTRL, Action::NextTab),
        Binding::action(
            KeyName::Named(NamedKey::Tab),
            CTRL_SHIFT,
            Action::PreviousTab,
        ),
        Binding::action(KeyName::Char(','), CTRL, Action::OpenSettings),
    ];
    if windows {
        bindings.push(Binding {
            mode: Some(Mode::Selection),
            ..Binding::action(KeyName::Char('c'), CTRL, Action::Copy)
        });
    }
    bindings
}

static DEFAULTS: LazyLock<Vec<Binding>> = LazyLock::new(|| defaults(cfg!(windows)));

/// What a pressed key does, or `None` when it goes to the program. The
/// user's bindings come first, the last matching one winning, so a later
/// line overrides an earlier one as it would in most config files; an
/// `action = "None"` there hides the default for the same key.
pub fn resolve<'a>(
    user: &'a [Binding],
    defaults: &'a [Binding],
    key: &Key,
    mods: ModifiersState,
    selecting: bool,
) -> Option<&'a Bound> {
    let bound = user
        .iter()
        .rev()
        .chain(defaults)
        .find(|b| b.matches(key, mods, selecting))
        .map(|b| &b.bound)?;
    (*bound != Bound::Action(Action::None)).then_some(bound)
}

/// [`resolve`] against the defaults for this platform.
pub fn lookup<'a>(
    user: &'a [Binding],
    key: &Key,
    mods: ModifiersState,
    selecting: bool,
) -> Option<&'a Bound> {
    resolve(user, &DEFAULTS, key, mods, selecting)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: ModifiersState = ModifiersState::empty();

    #[derive(Deserialize)]
    struct File {
        bindings: Vec<Binding>,
    }

    fn parse(text: &str) -> Result<Vec<Binding>, String> {
        toml::from_str::<File>(text)
            .map(|f| f.bindings)
            .map_err(|e| e.to_string())
    }

    fn chr(s: &str) -> Key {
        Key::Character(s.into())
    }

    fn named(key: NamedKey) -> Key {
        Key::Named(key)
    }

    fn action(
        user: &[Binding],
        windows: bool,
        key: &Key,
        mods: ModifiersState,
        selecting: bool,
    ) -> Option<Action> {
        match resolve(user, &defaults(windows), key, mods, selecting)? {
            Bound::Action(action) => Some(*action),
            Bound::Chars(_) => panic!("chars bound"),
        }
    }

    #[test]
    fn a_binding_reads_key_mods_and_action() {
        let bindings = parse(
            r#"
            [[bindings]]
            key = "PageUp"
            mods = "Control|Shift"
            action = "ScrollPageUp"
            [[bindings]]
            key = "t"
            mods = "alt"
            chars = "\u001b[A"
            [[bindings]]
            key = "Plus"
            action = "Paste"
            mode = "Selection"
            "#,
        )
        .unwrap();
        assert_eq!(
            bindings,
            [
                Binding::action(
                    KeyName::Named(NamedKey::PageUp),
                    CTRL_SHIFT,
                    Action::ScrollPageUp
                ),
                Binding {
                    key: KeyName::Char('t'),
                    mods: Mods(ModifiersState::ALT),
                    mode: None,
                    bound: Bound::Chars("\x1b[A".to_owned()),
                },
                Binding {
                    mode: Some(Mode::Selection),
                    ..Binding::action(KeyName::Char('+'), NONE, Action::Paste)
                },
            ]
        );
    }

    #[test]
    fn key_names_cover_letters_digits_symbols_and_named_keys() {
        assert_eq!(KeyName::parse("C"), Some(KeyName::Char('c')));
        assert_eq!(KeyName::parse("0"), Some(KeyName::Char('0')));
        assert_eq!(KeyName::parse("="), Some(KeyName::Char('=')));
        assert_eq!(KeyName::parse("plus"), Some(KeyName::Char('+')));
        assert_eq!(KeyName::parse("Minus"), Some(KeyName::Char('-')));
        assert_eq!(
            KeyName::parse("Insert"),
            Some(KeyName::Named(NamedKey::Insert))
        );
        assert_eq!(KeyName::parse("F12"), Some(KeyName::Named(NamedKey::F12)));
        assert_eq!(
            KeyName::parse("Space"),
            Some(KeyName::Named(NamedKey::Space))
        );
        assert_eq!(KeyName::parse(""), None);
        assert_eq!(KeyName::parse("CC"), None);
    }

    #[test]
    fn modifiers_are_joined_with_a_bar() {
        assert_eq!(Mods::parse("Control|Shift"), Ok(Mods(CTRL_SHIFT)));
        assert_eq!(
            Mods::parse("ctrl | alt"),
            Ok(Mods(CTRL | ModifiersState::ALT))
        );
        assert_eq!(Mods::parse("Super"), Ok(Mods(ModifiersState::SUPER)));
        assert_eq!(Mods::parse("None"), Ok(Mods(NONE)));
        assert_eq!(Mods::parse(""), Ok(Mods(NONE)));
    }

    #[test]
    fn unknown_keys_modifiers_modes_and_actions_are_rejected() {
        for (fields, expected) in [
            ("key = \"Foo\"\naction = \"Copy\"", "unknown key \"Foo\""),
            (
                "key = \"C\"\nmods = \"Hyper\"\naction = \"Copy\"",
                "unknown modifier \"Hyper\"",
            ),
            (
                "key = \"C\"\naction = \"Explode\"",
                "unknown variant `Explode`",
            ),
            (
                "key = \"C\"\nmode = \"Vi\"\naction = \"Copy\"",
                "unknown variant `Vi`",
            ),
            ("key = \"C\"\ncommand = \"ls\"", "unknown field `command`"),
        ] {
            let err = parse(&format!("[[bindings]]\n{fields}\n")).unwrap_err();
            assert!(err.contains(expected), "{fields}: {err}");
        }
    }

    #[test]
    fn action_and_chars_are_exclusive_and_one_is_required() {
        let both = parse("[[bindings]]\nkey = \"C\"\naction = \"Copy\"\nchars = \"x\"\n");
        assert!(both.unwrap_err().contains("not both"));
        let neither = parse("[[bindings]]\nkey = \"C\"\n");
        assert!(neither.unwrap_err().contains("give action or chars"));
    }

    #[test]
    fn defaults_are_the_former_fixed_shortcuts() {
        let c = chr("C");
        assert_eq!(
            action(&[], false, &c, CTRL_SHIFT, false),
            Some(Action::Copy)
        );
        assert_eq!(
            action(&[], false, &chr("c"), CTRL_SHIFT, false),
            Some(Action::Copy)
        );
        assert_eq!(action(&[], false, &c, SHIFT, true), None);
        assert_eq!(action(&[], false, &chr("c"), NONE, true), None);
        let v = chr("V");
        assert_eq!(
            action(&[], false, &v, CTRL_SHIFT, false),
            Some(Action::Paste)
        );
        assert_eq!(action(&[], false, &v, CTRL, false), None);
        let f = chr("F");
        assert_eq!(
            action(&[], false, &f, CTRL_SHIFT, false),
            Some(Action::Search)
        );
        assert_eq!(action(&[], false, &f, CTRL, false), None);
        let insert = named(NamedKey::Insert);
        assert_eq!(
            action(&[], false, &insert, SHIFT, false),
            Some(Action::Paste)
        );
        assert_eq!(action(&[], false, &insert, NONE, false), None);
        let up = named(NamedKey::PageUp);
        let down = named(NamedKey::PageDown);
        assert_eq!(
            action(&[], false, &up, SHIFT, false),
            Some(Action::ScrollPageUp)
        );
        assert_eq!(
            action(&[], false, &down, SHIFT, false),
            Some(Action::ScrollPageDown)
        );
        assert_eq!(action(&[], false, &up, NONE, false), None);
    }

    #[test]
    fn tabs_are_opened_closed_and_cycled_with_ctrl_shift_and_ctrl_tab() {
        for (key, mods, expected) in [
            (chr("T"), CTRL_SHIFT, Action::NewTab),
            (chr("W"), CTRL_SHIFT, Action::CloseTab),
            (named(NamedKey::Tab), CTRL, Action::NextTab),
            (named(NamedKey::Tab), CTRL_SHIFT, Action::PreviousTab),
        ] {
            assert_eq!(action(&[], false, &key, mods, false), Some(expected));
        }
        assert_eq!(action(&[], false, &named(NamedKey::Tab), NONE, false), None);
        assert_eq!(action(&[], false, &chr("w"), CTRL, false), None);
    }

    #[test]
    fn ctrl_comma_opens_the_settings_and_can_be_rebound() {
        let comma = chr(",");
        assert_eq!(
            action(&[], false, &comma, CTRL, false),
            Some(Action::OpenSettings)
        );
        assert_eq!(action(&[], false, &comma, NONE, false), None);
        let user = parse("[[bindings]]\nkey = \",\"\nmods = \"Control\"\naction = \"None\"\n\n[[bindings]]\nkey = \"F2\"\naction = \"OpenSettings\"\n").unwrap();
        assert_eq!(action(&user, false, &comma, CTRL, false), None);
        assert_eq!(
            action(&user, false, &named(NamedKey::F2), NONE, false),
            Some(Action::OpenSettings)
        );
    }

    #[test]
    fn on_windows_ctrl_c_copies_only_while_text_is_selected() {
        let c = chr("c");
        assert_eq!(action(&[], true, &c, CTRL, true), Some(Action::Copy));
        assert_eq!(action(&[], true, &c, CTRL, false), None);
        assert_eq!(action(&[], false, &c, CTRL, true), None);
    }

    #[test]
    fn user_bindings_override_and_remove_defaults() {
        let user = parse(
            r#"
            [[bindings]]
            key = "V"
            mods = "Control|Shift"
            action = "Copy"
            [[bindings]]
            key = "Insert"
            mods = "Shift"
            action = "None"
            [[bindings]]
            key = "C"
            mods = "Control"
            action = "None"
            "#,
        )
        .unwrap();
        let v = chr("V");
        assert_eq!(
            action(&user, false, &v, CTRL_SHIFT, false),
            Some(Action::Copy)
        );
        let insert = named(NamedKey::Insert);
        assert_eq!(action(&user, false, &insert, SHIFT, false), None);
        assert_eq!(action(&user, true, &chr("c"), CTRL, true), None);
        let f = chr("F");
        assert_eq!(
            action(&user, false, &f, CTRL_SHIFT, false),
            Some(Action::Search)
        );
    }

    #[test]
    fn the_last_matching_user_binding_wins() {
        let user = parse(
            r#"
            [[bindings]]
            key = "T"
            mods = "Control"
            chars = "first"
            [[bindings]]
            key = "T"
            mods = "Control"
            chars = "second"
            "#,
        )
        .unwrap();
        assert_eq!(
            resolve(&user, &defaults(false), &chr("t"), CTRL, false),
            Some(&Bound::Chars("second".to_owned()))
        );
    }

    #[test]
    fn selection_bindings_wait_for_a_selection() {
        let user = parse(
            r#"
            [[bindings]]
            key = "Escape"
            mode = "Selection"
            action = "Copy"
            "#,
        )
        .unwrap();
        let escape = named(NamedKey::Escape);
        assert_eq!(
            action(&user, false, &escape, NONE, true),
            Some(Action::Copy)
        );
        assert_eq!(action(&user, false, &escape, NONE, false), None);
    }

    #[test]
    fn bare_ctrl_c_copies_a_selection_only_on_windows() {
        let c = chr("c");
        assert_eq!(lookup(&[], &c, CTRL, false), None);
        let expected = cfg!(windows).then_some(&Bound::Action(Action::Copy));
        assert_eq!(lookup(&[], &c, CTRL, true), expected);
    }

    #[test]
    fn ctrl_plus_minus_and_zero_change_the_font_size() {
        for (key, mods, expected) in [
            ("=", CTRL, Action::IncreaseFontSize),
            ("+", CTRL, Action::IncreaseFontSize),
            ("+", CTRL_SHIFT, Action::IncreaseFontSize),
            ("-", CTRL, Action::DecreaseFontSize),
            ("0", CTRL, Action::ResetFontSize),
        ] {
            assert_eq!(
                action(&[], false, &chr(key), mods, false),
                Some(expected),
                "{key}"
            );
        }
        assert_eq!(action(&[], false, &chr("_"), CTRL_SHIFT, false), None);
    }

    #[test]
    fn every_action_can_be_named_in_the_file() {
        for name in [
            "Copy",
            "Paste",
            "Search",
            "ScrollPageUp",
            "ScrollPageDown",
            "ScrollLineUp",
            "ScrollLineDown",
            "ScrollToTop",
            "ScrollToBottom",
            "IncreaseFontSize",
            "DecreaseFontSize",
            "ResetFontSize",
            "ToggleFullscreen",
            "ClearSelection",
            "None",
        ] {
            let text = format!("[[bindings]]\nkey = \"F1\"\naction = \"{name}\"\n");
            assert!(parse(&text).is_ok(), "{name}");
        }
    }
}
