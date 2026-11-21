//! The row of tabs above the grid.

use crate::config::{Config, Profile};

/// Height of the bar in logical pixels.
pub const HEIGHT: f32 = 30.0;

/// Titles longer than this many characters are cut, so one program's long
/// title does not push the other tabs out of the window.
const MAX_TITLE: usize = 24;

/// What a click on the bar, or a tab key, asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum Clicked {
    Select(usize),
    Close(usize),
    /// A new tab with the profile, or `[shell]` when `None`.
    New(Option<Profile>),
    /// The settings screen.
    Settings,
}

/// Draws the bar for tabs titled `titles`, with `current` shown as
/// selected. + opens the default profile, ▾ lists the others.
pub fn show(
    root: &mut egui::Ui,
    titles: &[String],
    current: usize,
    config: &Config,
) -> Option<Clicked> {
    let mut clicked = None;
    egui::Panel::top("tab_bar")
        .exact_size(HEIGHT)
        .resizable(false)
        .show(root, |ui| {
            ui.horizontal_centered(|ui| {
                for (i, title) in titles.iter().enumerate() {
                    if ui.selectable_label(i == current, shorten(title)).clicked() {
                        clicked = Some(Clicked::Select(i));
                    }
                    if ui.small_button("×").clicked() {
                        clicked = Some(Clicked::Close(i));
                    }
                    ui.separator();
                }
                if ui.button("+").clicked() {
                    clicked = Some(Clicked::New(config.startup_profile(None).ok().flatten()));
                }
                // The list is built only while the menu is open, since on
                // Windows it asks wsl.exe the first time.
                ui.menu_button("▾", |ui| {
                    if ui.button("[shell]").clicked() {
                        clicked = Some(Clicked::New(None));
                    }
                    for profile in config.profiles() {
                        if ui.button(&profile.name).clicked() {
                            clicked = Some(Clicked::New(Some(profile)));
                        }
                    }
                });
                // The Nerd Font gear (nf-fa-cog): the built-in HackGen has
                // no ⚙ (U+2699), and egui is given no other font.
                if ui.button("\u{f013}").on_hover_text("設定").clicked() {
                    clicked = Some(Clicked::Settings);
                }
            });
        });
    clicked
}

fn shorten(title: &str) -> String {
    match title.char_indices().nth(MAX_TITLE) {
        Some((end, _)) => format!("{}…", &title[..end]),
        None => title.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_titles_are_cut_at_a_character_boundary() {
        assert_eq!(shorten("bash"), "bash");
        let long = "あ".repeat(MAX_TITLE + 5);
        assert_eq!(shorten(&long), format!("{}…", "あ".repeat(MAX_TITLE)));
    }
}
