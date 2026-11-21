//! The TOML configuration file.

use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use crate::bindings::Binding;
use crate::blink::Timing;
use crate::palette::Palette;
use crate::pty::{Command, default_shell, with_terminal_env};
use crate::render::{BackgroundFit, Color, FontConfig};
use crate::theme;
use crate::vt::{CursorShape, DEFAULT_SCROLLBACK, MAX_CLIPBOARD_BYTES};

/// Sizes `font.size` may take, and the size changed at run time stays in.
const FONT_SIZES: std::ops::RangeInclusive<f32> = 1.0..=200.0;
/// Points one IncreaseFontSize or DecreaseFontSize changes the size by.
pub const FONT_STEP: f32 = 1.0;
/// Largest `window.padding`, in logical pixels.
const MAX_PADDING: f32 = 200.0;
/// Largest `cursor.blink_timeout`, one day in seconds.
const MAX_BLINK_TIMEOUT: u64 = 86_400;

/// What `--print-default-config` prints. Every value equals the default, so
/// the file can be copied as a starting point.
pub const DEFAULT_CONFIG: &str = include_str!("config/default.toml");

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Started when no `--profile` is given; `None` starts [`Config::shell`].
    pub default_profile: Option<String>,
    pub font: Font,
    pub colors: Colors,
    pub cursor: Cursor,
    pub shell: Shell,
    /// As written in the file; [`Config::profiles`] adds the detected ones.
    pub profiles: Vec<Profile>,
    pub window: Window,
    pub background_image: BackgroundImage,
    pub scrolling: Scrolling,
    pub bell: Bell,
    pub clipboard: Clipboard,
    pub selection: Selection,
    pub mouse: Mouse,
    pub keyboard: Keyboard,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Keyboard {
    /// Checked before the defaults; see [`crate::bindings::resolve`].
    pub bindings: Vec<Binding>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Mouse {
    pub hide_when_typing: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Font {
    pub family: Option<String>,
    pub size: f32,
    pub line_height: f32,
    pub cell_width: f32,
}

impl Default for Font {
    fn default() -> Self {
        let font = FontConfig::default();
        Self {
            family: None,
            size: font.size,
            line_height: font.line_height,
            cell_width: font.cell_width,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Colors {
    pub theme: String,
    pub foreground: Option<Rgb>,
    pub background: Option<Rgb>,
    pub cursor: Option<Rgb>,
    pub ansi: Option<[Rgb; 8]>,
    pub brights: Option<[Rgb; 8]>,
    pub bold_is_bright: bool,
}

impl Default for Colors {
    fn default() -> Self {
        Self {
            theme: theme::DEFAULT.to_owned(),
            foreground: None,
            background: None,
            cursor: None,
            ansi: None,
            brights: None,
            bold_is_bright: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Cursor {
    pub shape: Shape,
    pub blinking: bool,
    /// Milliseconds.
    pub blink_interval: u64,
    /// Seconds; 0 never stops.
    pub blink_timeout: u64,
}

impl Default for Cursor {
    fn default() -> Self {
        Self {
            shape: Shape::default(),
            blinking: false,
            blink_interval: 750,
            blink_timeout: 5,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Shape {
    #[default]
    Block,
    Underline,
    Bar,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Shell {
    pub program: Option<String>,
    pub args: Vec<String>,
    pub working_directory: Option<PathBuf>,
}

/// A named program to start: a `[[profiles]]` entry or a detected WSL
/// distribution.
// Not `[shell]` flattened in: serde cannot combine `flatten` with
// `deny_unknown_fields`, and a typo here should still be an error.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub name: String,
    pub program: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    pub working_directory: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Window {
    pub columns: u16,
    pub rows: u16,
    pub padding: f32,
    pub backdrop: Backdrop,
    pub title: String,
    pub dynamic_title: bool,
    pub startup_mode: StartupMode,
}

impl Default for Window {
    fn default() -> Self {
        Self {
            columns: 80,
            rows: 24,
            padding: 6.0,
            backdrop: Backdrop::default(),
            title: "batune".to_owned(),
            dynamic_title: true,
            startup_mode: StartupMode::default(),
        }
    }
}

/// The material DWM draws behind the title bar on Windows 11.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backdrop {
    /// The title bar takes the theme's background color.
    #[default]
    None,
    Mica,
    Acrylic,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BackgroundImage {
    /// As written in the file until [`load`] resolves it against the
    /// file's directory and the home directory.
    pub path: Option<PathBuf>,
    pub opacity: f32,
    pub fit: Fit,
}

impl Default for BackgroundImage {
    fn default() -> Self {
        Self {
            path: None,
            opacity: 0.3,
            fit: Fit::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Fit {
    #[default]
    Cover,
    Contain,
    Stretch,
}

impl From<Fit> for BackgroundFit {
    fn from(fit: Fit) -> Self {
        match fit {
            Fit::Cover => BackgroundFit::Cover,
            Fit::Contain => BackgroundFit::Contain,
            Fit::Stretch => BackgroundFit::Stretch,
        }
    }
}

impl Window {
    /// The window title, given the one the program set (OSC 0/2) if any.
    pub fn title_for<'a>(&'a self, program: Option<&'a str>) -> &'a str {
        program
            .filter(|title| self.dynamic_title && !title.is_empty())
            .unwrap_or(&self.title)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StartupMode {
    #[default]
    Windowed,
    Maximized,
    Fullscreen,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Scrolling {
    pub history: usize,
    /// Lines per wheel notch.
    pub multiplier: u8,
}

impl Default for Scrolling {
    fn default() -> Self {
        Self {
            history: DEFAULT_SCROLLBACK,
            multiplier: crate::input::WHEEL_LINES,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Bell {
    pub visual: bool,
}

impl Default for Bell {
    fn default() -> Self {
        Self { visual: true }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Clipboard {
    pub osc52_write: bool,
    pub osc52_max_bytes: usize,
}

impl Default for Clipboard {
    fn default() -> Self {
        Self {
            osc52_write: true,
            osc52_max_bytes: MAX_CLIPBOARD_BYTES,
        }
    }
}

impl Clipboard {
    /// Whether an OSC 52 store of `text` may go on the clipboard.
    pub fn accepts(&self, text: &str) -> bool {
        self.osc52_write && text.len() <= self.osc52_max_bytes
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Selection {
    pub semantic_escape_chars: String,
    pub save_to_clipboard: bool,
}

impl Default for Selection {
    fn default() -> Self {
        Self {
            semantic_escape_chars: crate::selection::WORD_DELIMITERS.to_owned(),
            save_to_clipboard: true,
        }
    }
}

/// A color written as `"#rrggbb"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub Color);

impl<'de> Deserialize<'de> for Rgb {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        parse_hex(&s)
            .map(Rgb)
            .ok_or_else(|| serde::de::Error::custom(format!("{s:?} is not a #rrggbb color")))
    }
}

fn parse_hex(s: &str) -> Option<Color> {
    let hex = s.strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some(Color::rgb(channel(0)?, channel(2)?, channel(4)?))
}

#[derive(Debug)]
pub enum Error {
    Read(std::io::Error),
    Parse(toml::de::Error),
    Invalid(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Read(e) => write!(f, "cannot read: {e}"),
            Error::Parse(e) => write!(f, "{e}"),
            Error::Invalid(msg) => f.write_str(msg),
        }
    }
}

/// `batune/config.toml` under the platform's config directory
/// (`$XDG_CONFIG_HOME` or `~/.config` on Linux, `%APPDATA%` on Windows).
pub fn path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("batune").join("config.toml"))
}

/// Reads the file at `path`. A missing file is not an error: it means the
/// defaults.
pub fn load(path: &Path) -> Result<Config, Error> {
    let mut config = match std::fs::read_to_string(path) {
        Ok(text) => Config::parse(&text)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(e) => return Err(Error::Read(e)),
    };
    let dir = path.parent().unwrap_or(Path::new(""));
    config.resolve_paths(dir, dirs::home_dir());
    Ok(config)
}

impl Config {
    pub fn parse(text: &str) -> Result<Self, Error> {
        let config: Config = toml::from_str(text).map_err(Error::Parse)?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), Error> {
        if theme::find(&self.colors.theme).is_none() {
            let names: Vec<_> = theme::names().collect();
            return Err(Error::Invalid(format!(
                "colors.theme: unknown theme {:?} (built-in: {})",
                self.colors.theme,
                names.join(", ")
            )));
        }
        if !(self.font.size.is_finite() && FONT_SIZES.contains(&self.font.size)) {
            return Err(Error::Invalid(format!(
                "font.size: {} is outside 1-200",
                self.font.size
            )));
        }
        for (name, value) in [
            ("font.line_height", self.font.line_height),
            ("font.cell_width", self.font.cell_width),
        ] {
            if !(value.is_finite() && (0.5..=3.0).contains(&value)) {
                return Err(Error::Invalid(format!(
                    "{name}: {value} is outside 0.5-3.0"
                )));
            }
        }
        if self.window.columns == 0 || self.window.rows == 0 {
            return Err(Error::Invalid(
                "window.columns and window.rows must be at least 1".to_owned(),
            ));
        }
        // The renderer turns the padding into u32 pixels and doubles it.
        if !(self.window.padding.is_finite() && (0.0..=MAX_PADDING).contains(&self.window.padding))
        {
            return Err(Error::Invalid(format!(
                "window.padding: {} is outside 0-{MAX_PADDING}",
                self.window.padding
            )));
        }
        let opacity = self.background_image.opacity;
        if !(opacity.is_finite() && (0.0..=1.0).contains(&opacity)) {
            return Err(Error::Invalid(format!(
                "background_image.opacity: {opacity} is outside 0.0-1.0"
            )));
        }
        if !(50..=5000).contains(&self.cursor.blink_interval) {
            return Err(Error::Invalid(format!(
                "cursor.blink_interval: {} is outside 50-5000",
                self.cursor.blink_interval
            )));
        }
        // Added to an Instant, which panics far enough in the future.
        if self.cursor.blink_timeout > MAX_BLINK_TIMEOUT {
            return Err(Error::Invalid(format!(
                "cursor.blink_timeout: {} is over {MAX_BLINK_TIMEOUT}",
                self.cursor.blink_timeout
            )));
        }
        if !(1..=100).contains(&self.scrolling.multiplier) {
            return Err(Error::Invalid(format!(
                "scrolling.multiplier: {} is outside 1-100",
                self.scrolling.multiplier
            )));
        }
        // vt drops longer stores before they reach the config, so a higher
        // value would silently not apply.
        if self.clipboard.osc52_max_bytes > MAX_CLIPBOARD_BYTES {
            return Err(Error::Invalid(format!(
                "clipboard.osc52_max_bytes: {} is over {MAX_CLIPBOARD_BYTES}",
                self.clipboard.osc52_max_bytes
            )));
        }
        for (i, profile) in self.profiles.iter().enumerate() {
            if profile.name.is_empty() {
                return Err(Error::Invalid(
                    "profiles: name must not be empty".to_owned(),
                ));
            }
            // Which of two same-named entries `--profile` meant cannot be
            // guessed, so neither is picked silently.
            if self.profiles[..i].iter().any(|p| p.name == profile.name) {
                return Err(Error::Invalid(format!(
                    "profiles: {:?} is defined twice",
                    profile.name
                )));
            }
        }
        Ok(())
    }

    /// Makes the background image path absolute: `~` is the home
    /// directory and relative paths start at `dir`, the config file's
    /// directory. Done here and not where the image is read so that a
    /// reload compares the files actually meant.
    fn resolve_paths(&mut self, dir: &Path, home: Option<PathBuf>) {
        if let Some(path) = self.background_image.path.as_mut() {
            *path = dir.join(expand_home(path, home));
        }
    }

    pub fn font(&self) -> FontConfig {
        FontConfig {
            family: self.font.family.clone(),
            size: self.font.size,
            line_height: self.font.line_height,
            cell_width: self.font.cell_width,
        }
    }

    /// [`Config::font`] with the size `zoom` points off `font.size`, the
    /// change made at run time with IncreaseFontSize and DecreaseFontSize.
    pub fn font_zoomed(&self, zoom: f32) -> FontConfig {
        FontConfig {
            size: (self.font.size + zoom).clamp(*FONT_SIZES.start(), *FONT_SIZES.end()),
            ..self.font()
        }
    }

    /// `zoom` moved `by` points, stopping where the size reaches the end of
    /// the allowed range so that going back takes as many steps.
    pub fn zoom_by(&self, zoom: f32, by: f32) -> f32 {
        let size = self.font.size;
        (size + zoom + by).clamp(*FONT_SIZES.start(), *FONT_SIZES.end()) - size
    }

    /// The theme's colors with the `[colors]` overrides applied.
    pub fn palette(&self) -> Palette {
        let mut theme = *theme::find(&self.colors.theme)
            .unwrap_or_else(|| theme::find(theme::DEFAULT).expect("default theme"));
        let c = &self.colors;
        if let Some(Rgb(v)) = c.foreground {
            theme.foreground = v;
        }
        if let Some(Rgb(v)) = c.background {
            theme.background = v;
        }
        if let Some(Rgb(v)) = c.cursor {
            theme.cursor = v;
        }
        for (offset, colors) in [(0, &c.ansi), (8, &c.brights)] {
            if let Some(colors) = colors {
                for (i, Rgb(v)) in colors.iter().enumerate() {
                    theme.ansi[offset + i] = *v;
                }
            }
        }
        let mut palette = Palette::new(&theme);
        palette.bold_is_bright = c.bold_is_bright;
        palette
    }

    pub fn cursor_shape(&self) -> CursorShape {
        match self.cursor.shape {
            Shape::Block => CursorShape::Block,
            Shape::Underline => CursorShape::Underline,
            Shape::Bar => CursorShape::Bar,
        }
    }

    /// How the cursor blinks, or `None` when it does not.
    pub fn blink(&self) -> Option<Timing> {
        let c = &self.cursor;
        c.blinking.then(|| Timing {
            interval: Duration::from_millis(c.blink_interval),
            timeout: (c.blink_timeout > 0).then(|| Duration::from_secs(c.blink_timeout)),
        })
    }

    /// Every profile that can be started: the `[[profiles]]` entries, then
    /// the WSL distributions on Windows once a background `wsl.exe` has
    /// listed them (the first call starts it).
    pub fn profiles(&self) -> Vec<Profile> {
        self.profiles_with(crate::wsl::distros())
    }

    /// An entry with a distribution's name replaces it, so its command can
    /// be changed (e.g. to add `--cd ~`) without a second, same-named item.
    fn profiles_with(&self, distros: &[String]) -> Vec<Profile> {
        let detected = distros
            .iter()
            .filter(|d| !self.profiles.iter().any(|p| &p.name == *d))
            .map(|d| Profile {
                name: d.clone(),
                program: Some("wsl.exe".to_owned()),
                args: vec!["-d".to_owned(), d.clone()],
                working_directory: None,
            });
        self.profiles.iter().cloned().chain(detected).collect()
    }

    /// The profile to start: `name` (from `--profile`) or else
    /// `default_profile`. `Ok(None)` means `[shell]`.
    ///
    /// An unknown `name` is an error, since the user asked for it just now.
    /// An unknown `default_profile` only warns and falls back to `[shell]`,
    /// so that a profile that went away does not keep the terminal from
    /// opening.
    pub fn startup_profile(&self, name: Option<&str>) -> Result<Option<Profile>, Error> {
        // Started here, at launch, so the tab menu has the list when opened.
        crate::wsl::distros();
        let Some(wanted) = name.or(self.default_profile.as_deref()) else {
            return Ok(None);
        };
        if let Some(profile) = self.profiles.iter().find(|p| p.name == wanted) {
            return Ok(Some(profile.clone()));
        }
        // Only a name the config does not have waits for `wsl.exe`.
        let profiles = self.profiles_with(crate::wsl::distros_wait());
        if let Some(profile) = profiles.iter().find(|p| p.name == wanted) {
            return Ok(Some(profile.clone()));
        }
        let names: Vec<_> = profiles.iter().map(|p| p.name.as_str()).collect();
        let message = format!(
            "unknown profile {wanted:?} (profiles: {})",
            names.join(", ")
        );
        if name.is_some() {
            return Err(Error::Invalid(message));
        }
        eprintln!("batune: default_profile: {message}; starting [shell]");
        Ok(None)
    }

    /// The program to run in the terminal: `profile`, or `[shell]` when
    /// `None`.
    pub fn command(&self, profile: Option<&Profile>) -> Command {
        match profile {
            Some(p) => command(
                p.program.as_deref(),
                &p.args,
                p.working_directory.as_deref(),
            ),
            None => command(
                self.shell.program.as_deref(),
                &self.shell.args,
                self.shell.working_directory.as_deref(),
            ),
        }
    }
}

fn command(program: Option<&str>, args: &[String], cwd: Option<&Path>) -> Command {
    let mut command = match program {
        Some(program) => with_terminal_env(Command::new(program)),
        None => default_shell(),
    };
    command.args.extend(args.iter().map(OsString::from));
    command.cwd = cwd.map(|dir| expand_home(dir, dirs::home_dir()));
    command
}

/// What a running window has to redo when the configuration changes from
/// `old` to `new`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    pub font: bool,
    pub padding: bool,
    pub colors: bool,
    pub cursor: bool,
    pub scrollback: bool,
    pub backdrop: bool,
    pub background_image: bool,
    pub title: bool,
    /// Settings read only when a window or tab opens (the shell and profiles,
    /// the initial grid size and the startup mode) changed.
    pub next_launch: bool,
}

impl Changes {
    pub fn between(old: &Config, new: &Config) -> Self {
        Self {
            font: old.font != new.font,
            padding: old.window.padding != new.window.padding,
            colors: old.colors != new.colors,
            cursor: old.cursor != new.cursor,
            scrollback: old.scrolling.history != new.scrolling.history,
            backdrop: old.window.backdrop != new.window.backdrop,
            background_image: old.background_image != new.background_image,
            title: (&old.window.title, old.window.dynamic_title)
                != (&new.window.title, new.window.dynamic_title),
            next_launch: old.shell != new.shell
                || old.profiles != new.profiles
                || old.default_profile != new.default_profile
                || (old.window.columns, old.window.rows) != (new.window.columns, new.window.rows)
                || old.window.startup_mode != new.window.startup_mode,
        }
    }
}

/// Replaces a leading `~` with the home directory, as a shell would.
fn expand_home(path: &Path, home: Option<PathBuf>) -> PathBuf {
    match (path.strip_prefix("~"), home) {
        (Ok(rest), Some(home)) => home.join(rest),
        _ => path.to_path_buf(),
    }
}

#[cfg(test)]
mod tests;
