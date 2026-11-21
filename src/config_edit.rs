//! Writes settings changed in the settings screen back to the file, keeping
//! the rest of it (comments, order, keys the screen does not show) as is.

use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, Item, Value};

use crate::config::Config;

/// Writes the values that differ between `before` and `after` into the
/// file at `path`, creating it if missing. Nothing is written when the
/// result would not load, and the message says why.
///
/// The running windows pick the file up through the watch, as they would an
/// edit made in a text editor.
pub fn save(path: &Path, before: &Config, after: &Config) -> Result<(), String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let text = apply(&text, before, after)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let target = real_path(path).map_err(|e| format!("{}: {e}", path.display()))?;
    write_replacing(&target, &text).map_err(|e| format!("{}: {e}", target.display()))
}

/// The file a write to `path` should land in: through any symlinks, so
/// renaming over it does not replace a link into a dotfiles repository with
/// a plain file. A missing file (or a link to one) is created where it would
/// be.
fn real_path(path: &Path) -> std::io::Result<PathBuf> {
    match std::fs::canonicalize(path) {
        Ok(real) => Ok(real),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => match std::fs::read_link(path) {
            // shortcut: follows one hop of a dangling link, enough for one link to a not yet created file.
            Ok(link) => Ok(path.parent().unwrap_or(Path::new("")).join(link)),
            Err(_) => Ok(path.to_path_buf()),
        },
        Err(e) => Err(e),
    }
}

/// Writes a temporary file beside `target` and renames it over, so a failed
/// write (a full disk, a crash) leaves the old file whole instead of empty
/// or cut short.
fn write_replacing(target: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    let tmp = target.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    let result = (|| {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(text.as_bytes())?;
        // The new file would otherwise get the default mode and could make
        // a file the user had kept private readable by others.
        if let Ok(meta) = std::fs::metadata(target) {
            file.set_permissions(meta.permissions())?;
        }
        file.sync_all()?;
        std::fs::rename(&tmp, target)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// `text` with the changed values written in, checked as the file would be
/// on load.
pub fn apply(text: &str, before: &Config, after: &Config) -> Result<String, String> {
    let mut doc: DocumentMut = text.parse().map_err(|e| format!("config.toml: {e}"))?;
    // Only changed keys are touched, so a value the user wrote as `14` does
    // not turn into `14.0` and a file without a key does not gain it.
    for (old, new) in fields(before).into_iter().zip(fields(after)) {
        if old.2.as_ref().map(Value::to_string) != new.2.as_ref().map(Value::to_string) {
            set(&mut doc, new.0, new.1, new.2)?;
        }
    }
    let text = doc.to_string();
    Config::parse(&text).map_err(|e| e.to_string())?;
    Ok(text)
}

fn set(
    doc: &mut DocumentMut,
    section: &str,
    key: &str,
    value: Option<Value>,
) -> Result<(), String> {
    let table = if section.is_empty() {
        doc.as_table_mut() as &mut dyn toml_edit::TableLike
    } else {
        doc.entry(section)
            .or_insert(toml_edit::table())
            .as_table_like_mut()
            .ok_or_else(|| format!("{section} is not a table"))?
    };
    match (value, table.get_mut(key)) {
        (None, _) => {
            table.remove(key);
        }
        // Replacing the item, not the entry, keeps the key's decor, which
        // holds the comment lines above it.
        (Some(mut value), Some(item)) => {
            if let Some(old) = item.as_value() {
                *value.decor_mut() = old.decor().clone();
            }
            *item = Item::Value(value);
        }
        (Some(value), None) => {
            table.insert(key, Item::Value(value));
        }
    }
    Ok(())
}

/// The keys the settings screen edits, as (section, key, value); `None`
/// leaves the key out.
fn fields(c: &Config) -> Vec<(&'static str, &'static str, Option<Value>)> {
    let float = |v: f32| Some(Value::from(v.to_string().parse::<f64>().unwrap_or(0.0)));
    let int = |v: u64| Some(Value::from(i64::try_from(v).unwrap_or(i64::MAX)));
    let bool = |v: bool| Some(Value::from(v));
    let str = |v: &str| Some(Value::from(v));
    // The enums are deserialized with `rename_all = "lowercase"`.
    let name = |v: &dyn std::fmt::Debug| str(&format!("{v:?}").to_lowercase());
    vec![
        (
            "",
            "default_profile",
            c.default_profile.as_deref().and_then(str),
        ),
        ("font", "family", c.font.family.as_deref().and_then(str)),
        ("font", "size", float(c.font.size)),
        ("font", "line_height", float(c.font.line_height)),
        ("font", "cell_width", float(c.font.cell_width)),
        ("colors", "theme", str(&c.colors.theme)),
        ("colors", "bold_is_bright", bool(c.colors.bold_is_bright)),
        ("cursor", "shape", name(&c.cursor.shape)),
        ("cursor", "blinking", bool(c.cursor.blinking)),
        ("cursor", "blink_interval", int(c.cursor.blink_interval)),
        ("cursor", "blink_timeout", int(c.cursor.blink_timeout)),
        ("window", "padding", float(c.window.padding)),
        ("window", "backdrop", name(&c.window.backdrop)),
        ("window", "title", str(&c.window.title)),
        ("window", "dynamic_title", bool(c.window.dynamic_title)),
        (
            "background_image",
            "opacity",
            float(c.background_image.opacity),
        ),
        ("background_image", "fit", name(&c.background_image.fit)),
        ("scrolling", "history", int(c.scrolling.history as u64)),
        (
            "scrolling",
            "multiplier",
            int(c.scrolling.multiplier.into()),
        ),
        ("bell", "visual", bool(c.bell.visual)),
        ("clipboard", "osc52_write", bool(c.clipboard.osc52_write)),
        ("mouse", "hide_when_typing", bool(c.mouse.hide_when_typing)),
        (
            "selection",
            "save_to_clipboard",
            bool(c.selection.save_to_clipboard),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Shape;

    const FILE: &str = "# my settings\n\
        default_profile = \"bash\"\n\
        \n\
        [font]\n\
        # big letters\n\
        size = 18 # for the laptop\n\
        family = \"DejaVu Sans Mono\"\n\
        \n\
        [[profiles]]\n\
        name = \"bash\"\n\
        program = \"/bin/bash\"\n\
        \n\
        [[keyboard.bindings]]\n\
        key = \"F11\"\n\
        action = \"ToggleFullscreen\"\n";

    #[test]
    fn a_change_keeps_comments_order_and_other_keys() {
        let before = Config::parse(FILE).unwrap();
        let mut after = before.clone();
        after.font.size = 20.0;
        after.cursor.shape = Shape::Bar;
        let text = apply(FILE, &before, &after).unwrap();
        assert!(text.starts_with("# my settings\ndefault_profile = \"bash\"\n"));
        assert!(text.contains(
            "# big letters\nsize = 20.0 # for the laptop\nfamily = \"DejaVu Sans Mono\"\n"
        ));
        assert!(text.contains("[[profiles]]\nname = \"bash\""));
        assert!(text.contains("[[keyboard.bindings]]\nkey = \"F11\""));
        assert!(text.contains("[cursor]\nshape = \"bar\"\n"));
        let saved = Config::parse(&text).unwrap();
        assert_eq!(saved, after);
    }

    #[test]
    fn unchanged_values_are_not_rewritten() {
        let before = Config::parse(FILE).unwrap();
        assert_eq!(apply(FILE, &before, &before).unwrap(), FILE);
    }

    #[test]
    fn clearing_an_optional_value_removes_the_key() {
        let before = Config::parse(FILE).unwrap();
        let mut after = before.clone();
        after.default_profile = None;
        after.font.family = None;
        let text = apply(FILE, &before, &after).unwrap();
        assert!(!text.contains("default_profile"));
        assert!(!text.contains("family"));
        assert!(text.contains("# big letters\nsize = 18"));
    }

    #[test]
    fn an_empty_file_gets_only_the_changed_keys() {
        let before = Config::default();
        let mut after = before.clone();
        after.background_image.opacity = 0.3 + 0.2;
        let text = apply("", &before, &after).unwrap();
        assert_eq!(text, "[background_image]\nopacity = 0.5\n");
    }

    #[test]
    fn invalid_values_are_refused_with_the_reason() {
        let before = Config::parse(FILE).unwrap();
        let mut after = before.clone();
        after.font.size = 500.0;
        let e = apply(FILE, &before, &after).unwrap_err();
        assert!(e.contains("font.size"), "{e}");
        after = before.clone();
        after.cursor.blink_interval = 10;
        assert!(
            apply(FILE, &before, &after)
                .unwrap_err()
                .contains("blink_interval")
        );
    }

    #[test]
    fn values_over_the_limits_are_refused_not_written() {
        let before = Config::default();
        let mut after = before.clone();
        after.cursor.blink_timeout = u64::MAX;
        assert!(
            apply("", &before, &after)
                .unwrap_err()
                .contains("blink_timeout")
        );
        after = before.clone();
        after.window.padding = f32::MAX;
        assert!(apply("", &before, &after).unwrap_err().contains("padding"));
    }

    #[test]
    fn saving_replaces_the_file_whole_and_leaves_no_temporary_file() {
        let dir =
            std::env::temp_dir().join(format!("batune-config-replace-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, FILE).unwrap();
        let before = Config::parse(FILE).unwrap();
        let mut after = before.clone();
        after.font.size = 20.0;
        save(&path, &before, &after).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        let entries = std::fs::read_dir(&dir).unwrap().count();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(Config::parse(&written).unwrap(), after);
        assert_eq!(entries, 1);
    }

    #[cfg(unix)]
    #[test]
    fn saving_through_a_symlink_writes_the_linked_file_and_keeps_the_link() {
        let dir = std::env::temp_dir().join(format!("batune-config-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("dotfiles")).unwrap();
        let real = dir.join("dotfiles").join("batune.toml");
        let link = dir.join("config.toml");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let before = Config::default();
        let mut after = before.clone();
        after.scrolling.multiplier = 5;
        // The linked file does not exist yet, then it does.
        save(&link, &before, &after).unwrap();
        let first = std::fs::read_to_string(&real);
        let before = after.clone();
        after.scrolling.multiplier = 7;
        save(&link, &before, &after).unwrap();
        let second = std::fs::read_to_string(&real);
        let still_link = std::fs::symlink_metadata(&link).map(|m| m.file_type().is_symlink());
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(first.unwrap(), "[scrolling]\nmultiplier = 5\n");
        assert_eq!(second.unwrap(), "[scrolling]\nmultiplier = 7\n");
        assert!(still_link.unwrap());
    }

    #[test]
    fn save_does_not_write_an_invalid_file_and_creates_a_missing_one() {
        let dir = std::env::temp_dir().join(format!("batune-config-edit-{}", std::process::id()));
        let path = dir.join("sub").join("config.toml");
        let before = Config::default();
        let mut after = before.clone();
        after.scrolling.multiplier = 0;
        assert!(save(&path, &before, &after).is_err());
        assert!(!path.exists());
        after.scrolling.multiplier = 5;
        save(&path, &before, &after).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(path.parent().unwrap());
        let _ = std::fs::remove_dir(&dir);
        assert_eq!(written, "[scrolling]\nmultiplier = 5\n");
    }
}
