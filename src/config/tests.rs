use super::*;
use crate::bindings::Action;

#[test]
fn printed_default_config_parses_to_the_defaults() {
    assert_eq!(Config::parse(DEFAULT_CONFIG).unwrap(), Config::default());
}

#[test]
fn printed_default_config_lists_every_built_in_theme() {
    for name in theme::names() {
        assert!(DEFAULT_CONFIG.contains(name), "{name} is not listed");
    }
}

#[test]
fn printed_default_config_examples_parse() {
    let start = DEFAULT_CONFIG.find("# [[keyboard.bindings]]").unwrap();
    let examples: String = DEFAULT_CONFIG[start..]
        .lines()
        .filter_map(|line| line.strip_prefix("# "))
        .filter(|line| line.starts_with("[[") || line.contains(" = "))
        .map(|line| format!("{line}\n"))
        .collect();
    let config = Config::parse(&examples).unwrap();
    assert_eq!(config.keyboard.bindings.len(), 3);
}

#[test]
fn printed_default_config_names_every_action() {
    let actions = [
        Action::Copy,
        Action::Paste,
        Action::Search,
        Action::ScrollPageUp,
        Action::ScrollPageDown,
        Action::ScrollLineUp,
        Action::ScrollLineDown,
        Action::ScrollToTop,
        Action::ScrollToBottom,
        Action::IncreaseFontSize,
        Action::DecreaseFontSize,
        Action::ResetFontSize,
        Action::ToggleFullscreen,
        Action::ClearSelection,
        Action::None,
    ];
    for action in actions {
        let name = format!("{action:?}");
        assert!(DEFAULT_CONFIG.contains(&name), "{name} is not listed");
    }
}

#[test]
fn empty_file_means_defaults() {
    assert_eq!(Config::parse("").unwrap(), Config::default());
}

#[test]
fn missing_file_means_defaults() {
    let path = std::env::temp_dir().join("batune-no-such-dir/config.toml");
    assert_eq!(load(&path).unwrap(), Config::default());
}

#[test]
fn omitted_keys_keep_their_defaults() {
    let config = Config::parse("[font]\nfamily = \"Iosevka\"\n").unwrap();
    assert_eq!(config.font.family.as_deref(), Some("Iosevka"));
    assert_eq!(config.font.size, Font::default().size);
    assert_eq!(config.window, Window::default());
}

#[test]
fn every_section_is_read() {
    let config = Config::parse(
        r##"
        [font]
        size = 18.5
        line_height = 1.2
        cell_width = 0.9
        [colors]
        theme = "gruvbox-dark"
        bold_is_bright = false
        [cursor]
        shape = "bar"
        blinking = true
        blink_interval = 500
        blink_timeout = 0
        [shell]
        program = "/bin/bash"
        args = ["-l", "-i"]
        working_directory = "/tmp"
        [window]
        columns = 120
        rows = 40
        padding = 0
        backdrop = "mica"
        title = "term"
        dynamic_title = false
        startup_mode = "fullscreen"
        [background_image]
        path = "/srv/bg.png"
        opacity = 0.5
        fit = "stretch"
        [scrolling]
        history = 0
        multiplier = 5
        [bell]
        visual = false
        [clipboard]
        osc52_write = false
        osc52_max_bytes = 100
        [selection]
        semantic_escape_chars = "/"
        save_to_clipboard = false
        [mouse]
        hide_when_typing = true
        [[keyboard.bindings]]
        key = "Insert"
        mods = "Shift"
        action = "None"
        "##,
    )
    .unwrap();
    assert_eq!(config.font().size, 18.5);
    assert_eq!(
        (config.font().line_height, config.font().cell_width),
        (1.2, 0.9)
    );
    assert_eq!(
        config.palette(),
        Palette {
            bold_is_bright: false,
            ..Palette::new(theme::find("gruvbox-dark").unwrap())
        }
    );
    assert_eq!(config.cursor_shape(), CursorShape::Bar);
    assert_eq!(
        config.blink(),
        Some(Timing {
            interval: Duration::from_millis(500),
            timeout: None,
        })
    );
    assert!(config.mouse.hide_when_typing);
    assert_eq!((config.window.columns, config.window.rows), (120, 40));
    assert_eq!(config.window.padding, 0.0);
    assert_eq!(config.window.backdrop, Backdrop::Mica);
    assert_eq!(config.window.title_for(Some("vim")), "term");
    assert_eq!(config.window.startup_mode, StartupMode::Fullscreen);
    assert_eq!(
        config.background_image,
        BackgroundImage {
            path: Some(PathBuf::from("/srv/bg.png")),
            opacity: 0.5,
            fit: Fit::Stretch,
        }
    );
    assert_eq!(config.scrolling.history, 0);
    assert_eq!(config.scrolling.multiplier, 5);
    assert!(!config.bell.visual);
    assert!(!config.clipboard.osc52_write);
    assert_eq!(config.clipboard.osc52_max_bytes, 100);
    assert_eq!(config.selection.semantic_escape_chars, "/");
    assert!(!config.selection.save_to_clipboard);
    assert_eq!(config.keyboard.bindings.len(), 1);
    let command = config.command(None);
    assert_eq!(command.program, "/bin/bash");
    assert_eq!(command.args, ["-l", "-i"]);
    assert_eq!(command.cwd.as_deref(), Some(Path::new("/tmp")));
    assert!(command.env.iter().any(|(k, _)| k == "TERM"));
}

#[test]
fn color_overrides_replace_only_the_given_theme_colors() {
    let config = Config::parse(
        r##"
        [colors]
        theme = "solarized-dark"
        background = "#000000"
        brights = ["#010101", "#020202", "#030303", "#040404", "#050505", "#060606", "#070707", "#08090a"]
        "##,
    )
    .unwrap();
    let base = Palette::new(theme::find("solarized-dark").unwrap());
    let palette = config.palette();
    assert_eq!(palette.background, Color::BLACK);
    assert_eq!(palette.foreground, base.foreground);
    assert_eq!(palette.colors[..8], base.colors[..8]);
    assert_eq!(palette.colors[8], Color::rgb(1, 1, 1));
    assert_eq!(palette.colors[15], Color::rgb(8, 9, 10));
    assert_eq!(palette.colors[16..], base.colors[16..]);
}

#[test]
fn unknown_theme_is_rejected_with_the_built_in_names() {
    let err = Config::parse("[colors]\ntheme = \"nope\"").unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("nope") && msg.contains("batune-light"),
        "{msg}"
    );
}

#[test]
fn misspelled_key_is_rejected() {
    assert!(matches!(
        Config::parse("[font]\nsise = 12"),
        Err(Error::Parse(_))
    ));
}

#[test]
fn the_run_time_font_size_moves_in_steps_within_range() {
    let config = Config::parse("[font]\nsize = 2.5").unwrap();
    assert_eq!(config.font_zoomed(0.0).size, 2.5);
    let bigger = config.zoom_by(0.0, FONT_STEP);
    assert_eq!(config.font_zoomed(bigger).size, 3.5);
    let smaller = config.zoom_by(0.0, -FONT_STEP);
    assert_eq!(config.font_zoomed(smaller).size, 1.5);
    let smallest = config.zoom_by(smaller, -FONT_STEP);
    assert_eq!(config.font_zoomed(smallest).size, 1.0);
    assert_eq!(config.zoom_by(smallest, -FONT_STEP), smallest);
    let line_height = config.font_zoomed(bigger).line_height;
    assert_eq!(line_height, config.font.line_height);
}

#[test]
fn a_zoom_kept_across_a_reload_stays_in_range() {
    let config = Config::parse("[font]\nsize = 199").unwrap();
    assert_eq!(config.font_zoomed(10.0).size, 200.0);
}

#[test]
fn broken_bindings_are_reported_with_their_place() {
    let err =
        Config::parse("[[keyboard.bindings]]\nkey = \"C\"\naction = \"Copy\"\nchars = \"x\"\n")
            .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("keyboard.bindings") && msg.contains("not both"),
        "{msg}"
    );
}

#[test]
fn malformed_colors_are_rejected() {
    for bad in ["\"d8d8d8\"", "\"#d8d8d\"", "\"#gggggg\"", "\"#d8d8d8ff\""] {
        let text = format!("[colors]\nforeground = {bad}");
        assert!(Config::parse(&text).is_err(), "{bad} accepted");
    }
    let short = "[colors]\nansi = [\"#000000\"]";
    assert!(Config::parse(short).is_err());
}

#[test]
fn out_of_range_values_are_rejected() {
    for bad in [
        "[font]\nsize = 0",
        "[font]\nsize = nan",
        "[font]\nline_height = 0.4",
        "[font]\ncell_width = 3.5",
        "[font]\nline_height = inf",
        "[window]\ncolumns = 0",
        "[window]\npadding = -1",
        "[window]\npadding = 201",
        "[window]\npadding = 1e30",
        "[cursor]\nblink_timeout = 86401",
        "[cursor]\nblink_timeout = 9223372036854775807",
        "[cursor]\nshape = \"beam\"",
        "[window]\nbackdrop = \"blur\"",
        "[window]\nstartup_mode = \"minimized\"",
        "[scrolling]\nhistory = -1",
        "[scrolling]\nmultiplier = 0",
        "[cursor]\nblink_interval = 0",
        "[cursor]\nblink_interval = 5001",
        "[scrolling]\nmultiplier = 101",
        "[background_image]\nopacity = -0.1",
        "[background_image]\nopacity = 1.5",
        "[background_image]\nopacity = nan",
        "[background_image]\nfit = \"tile\"",
    ] {
        assert!(Config::parse(bad).is_err(), "{bad} accepted");
    }
}

#[test]
fn tilde_expands_to_the_home_directory() {
    let home = Some(PathBuf::from("/home/u"));
    assert_eq!(
        expand_home(Path::new("~/src"), home.clone()),
        Path::new("/home/u/src")
    );
    assert_eq!(
        expand_home(Path::new("~"), home.clone()),
        Path::new("/home/u")
    );
    assert_eq!(expand_home(Path::new("/srv"), home), Path::new("/srv"));
    assert_eq!(expand_home(Path::new("~/x"), None), Path::new("~/x"));
}

#[test]
fn background_image_paths_start_at_the_config_directory() {
    let home = Some(PathBuf::from("/home/u"));
    let resolved = |path: &str| {
        let mut config = Config::parse(&format!("[background_image]\npath = {path:?}")).unwrap();
        config.resolve_paths(Path::new("/etc/batune"), home.clone());
        config.background_image.path.unwrap()
    };
    assert_eq!(resolved("bg.png"), Path::new("/etc/batune/bg.png"));
    assert_eq!(resolved("~/bg.png"), Path::new("/home/u/bg.png"));
    assert_eq!(resolved("/srv/bg.png"), Path::new("/srv/bg.png"));
}

#[test]
fn a_new_background_image_applies_at_once() {
    for text in [
        "[background_image]\npath = \"/srv/bg.png\"",
        "[background_image]\nopacity = 0.8",
        "[background_image]\nfit = \"contain\"",
    ] {
        let new = Config::parse(text).unwrap();
        assert_eq!(
            Changes::between(&Config::default(), &new),
            Changes {
                background_image: true,
                ..Changes::default()
            },
            "{text}"
        );
    }
}

#[test]
fn changes_name_only_what_differs() {
    let old = Config::default();
    assert_eq!(Changes::between(&old, &old), Changes::default());

    let new = Config::parse("[colors]\ntheme = \"batune-light\"").unwrap();
    let changes = Changes::between(&old, &new);
    assert_eq!(
        changes,
        Changes {
            colors: true,
            ..Changes::default()
        }
    );

    let new = Config::parse("[font]\nsize = 20\n[window]\npadding = 0\n[scrolling]\nhistory = 5")
        .unwrap();
    let changes = Changes::between(&old, &new);
    assert!(changes.font && changes.padding && changes.scrollback);
    assert!(!changes.colors && !changes.next_launch);

    let new = Config::parse("[font]\nline_height = 1.5").unwrap();
    assert!(Changes::between(&old, &new).font);
}

#[test]
fn every_backdrop_is_read() {
    for (text, backdrop) in [
        ("none", Backdrop::None),
        ("mica", Backdrop::Mica),
        ("acrylic", Backdrop::Acrylic),
    ] {
        let config = Config::parse(&format!("[window]\nbackdrop = \"{text}\"")).unwrap();
        assert_eq!(config.window.backdrop, backdrop);
    }
}

#[test]
fn backdrop_applies_without_waiting_for_the_next_launch() {
    let new = Config::parse("[window]\nbackdrop = \"acrylic\"").unwrap();
    assert_eq!(
        Changes::between(&Config::default(), &new),
        Changes {
            backdrop: true,
            ..Changes::default()
        }
    );
}

#[test]
fn settings_read_when_used_need_nothing_redone() {
    let old = Config::default();
    for text in [
        "[scrolling]\nmultiplier = 1",
        "[selection]\nsave_to_clipboard = false",
        "[mouse]\nhide_when_typing = true",
        "[[keyboard.bindings]]\nkey = \"C\"\nmods = \"Control\"\naction = \"Copy\"",
    ] {
        let new = Config::parse(text).unwrap();
        assert_eq!(Changes::between(&old, &new), Changes::default(), "{text}");
    }
}

#[test]
fn the_cursor_does_not_blink_by_default() {
    assert_eq!(Config::default().blink(), None);
    let config = Config::parse("[cursor]\nblinking = true").unwrap();
    assert_eq!(
        config.blink(),
        Some(Timing {
            interval: Duration::from_millis(750),
            timeout: Some(Duration::from_secs(5)),
        })
    );
}

#[test]
fn bold_brightening_is_a_color_change() {
    let new = Config::parse("[colors]\nbold_is_bright = false").unwrap();
    assert!(!new.palette().bold_is_bright);
    assert!(Changes::between(&Config::default(), &new).colors);
}

#[test]
fn the_program_title_wins_unless_dynamic_titles_are_off() {
    let window = Window::default();
    assert_eq!(window.title_for(None), "batune");
    assert_eq!(window.title_for(Some("")), "batune");
    assert_eq!(window.title_for(Some("vim")), "vim");
    let fixed = Window {
        title: "work".to_owned(),
        dynamic_title: false,
        ..Window::default()
    };
    assert_eq!(fixed.title_for(Some("vim")), "work");
}

#[test]
fn title_changes_are_applied_at_once() {
    let old = Config::default();
    for text in [
        "[window]\ntitle = \"work\"",
        "[window]\ndynamic_title = false",
    ] {
        let changes = Changes::between(&old, &Config::parse(text).unwrap());
        assert!(changes.title && !changes.next_launch, "{text}");
    }
}

#[test]
fn shell_and_initial_size_wait_for_the_next_launch() {
    let old = Config::default();
    for text in [
        "[shell]\nprogram = \"/bin/sh\"",
        "[window]\nrows = 50",
        "[window]\nstartup_mode = \"maximized\"",
    ] {
        let new = Config::parse(text).unwrap();
        assert!(Changes::between(&old, &new).next_launch, "{text}");
    }
}

#[test]
fn profiles_wait_for_the_next_launch() {
    let old = Config::default();
    for text in ["default_profile = \"a\"", "[[profiles]]\nname = \"a\""] {
        let new = Config::parse(text).unwrap();
        assert!(Changes::between(&old, &new).next_launch, "{text}");
    }
}

#[test]
fn printed_default_config_profile_examples_parse() {
    let default = "default_profile = \"bash\"\n";
    assert!(DEFAULT_CONFIG.contains(&format!("# {default}")));
    let start = DEFAULT_CONFIG.find("# [[profiles]]").unwrap();
    let end = DEFAULT_CONFIG.find("[window]").unwrap();
    let examples: String = DEFAULT_CONFIG[start..end]
        .lines()
        .filter_map(|line| line.strip_prefix("# "))
        .filter(|line| line.starts_with("[[") || line.contains(" = "))
        .fold(default.to_owned(), |text, line| format!("{text}{line}\n"));
    let config = Config::parse(&examples).unwrap();
    let names: Vec<_> = config.profiles.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["bash", "Ubuntu"]);
    assert_eq!(config.default_profile.as_deref(), Some("bash"));
}

#[test]
fn only_shell_starts_the_shell_as_before() {
    let config = Config::parse("[shell]\nprogram = \"/bin/zsh\"\nargs = [\"-l\"]").unwrap();
    assert_eq!(config.startup_profile(None).unwrap(), None);
    let command = config.command(None);
    assert_eq!(command.program, "/bin/zsh");
    assert_eq!(command.args, ["-l"]);
}

#[test]
fn profile_is_picked_by_name_then_by_default_profile() {
    let config = Config::parse(
        r#"
        default_profile = "b"
        [[profiles]]
        name = "a"
        program = "/bin/a"
        [[profiles]]
        name = "b"
        program = "/bin/b"
        args = ["-x"]
        working_directory = "/tmp"
        "#,
    )
    .unwrap();
    let a = config.startup_profile(Some("a")).unwrap().unwrap();
    assert_eq!(config.command(Some(&a)).program, "/bin/a");
    let b = config.startup_profile(None).unwrap().unwrap();
    let command = config.command(Some(&b));
    assert_eq!(command.program, "/bin/b");
    assert_eq!(command.args, ["-x"]);
    assert_eq!(command.cwd.as_deref(), Some(Path::new("/tmp")));
}

#[test]
fn unknown_profile_is_an_error_only_when_asked_for_by_name() {
    let config = Config::parse("default_profile = \"gone\"\n[[profiles]]\nname = \"a\"").unwrap();
    assert!(matches!(
        config.startup_profile(Some("gone")),
        Err(Error::Invalid(_))
    ));
    assert_eq!(
        config.startup_profile(None).unwrap(),
        None,
        "falls back to [shell]"
    );
}

#[test]
fn wsl_distributions_follow_the_configured_profiles() {
    let config = Config::parse(
        "[[profiles]]\nname = \"Ubuntu\"\nprogram = \"wsl.exe\"\nargs = [\"-d\", \"Ubuntu\", \"--cd\", \"~\"]\n\
         [[profiles]]\nname = \"pwsh\"\nprogram = \"pwsh.exe\"",
    )
    .unwrap();
    let profiles = config.profiles_with(&["Ubuntu".to_owned(), "Debian".to_owned()]);
    let names: Vec<_> = profiles.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        names,
        ["Ubuntu", "pwsh", "Debian"],
        "configured Ubuntu wins"
    );
    assert!(profiles[0].args.contains(&"--cd".to_owned()));
    let debian = config.command(Some(&profiles[2]));
    assert_eq!(debian.program, "wsl.exe");
    assert_eq!(debian.args, ["-d", "Debian"]);
}

#[test]
fn profile_names_must_be_unique_and_not_empty() {
    let twice = "[[profiles]]\nname = \"a\"\n[[profiles]]\nname = \"a\"";
    assert!(matches!(Config::parse(twice), Err(Error::Invalid(_))));
    assert!(matches!(
        Config::parse("[[profiles]]\nname = \"\""),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        Config::parse("[[profiles]]\nprogram = \"/bin/sh\""),
        Err(Error::Parse(_))
    ));
}

#[test]
fn osc_52_stores_are_limited_by_size_and_can_be_turned_off() {
    let clipboard = Clipboard {
        osc52_write: true,
        osc52_max_bytes: 3,
    };
    assert!(clipboard.accepts("abc"));
    assert!(!clipboard.accepts("abcd"));
    assert!(!clipboard.accepts("ああ"), "counted in UTF-8 bytes");
    let off = Clipboard {
        osc52_write: false,
        ..Clipboard::default()
    };
    assert!(!off.accepts("a"));
}

#[test]
fn osc_52_limit_cannot_exceed_what_vt_accepts() {
    let text = format!("[clipboard]\nosc52_max_bytes = {}", MAX_CLIPBOARD_BYTES + 1);
    assert!(matches!(Config::parse(&text), Err(Error::Invalid(_))));
    let text = format!("[clipboard]\nosc52_max_bytes = {MAX_CLIPBOARD_BYTES}");
    assert!(Config::parse(&text).is_ok());
}
