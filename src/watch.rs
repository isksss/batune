//! Watches the configuration file for changes.

use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

/// How long the file has to stay untouched before it is read. Editors often
/// truncate and then write, and reading in between would see an empty file.
const SETTLE: Duration = Duration::from_millis(150);

/// Calls `on_change` from a background thread after `path` is created or
/// written. The watch stops when the returned watcher is dropped.
pub fn watch(
    path: &Path,
    on_change: impl Fn() + Send + 'static,
) -> notify::Result<RecommendedWatcher> {
    // Watching the directory rather than the file: editors that save by
    // writing a new file and renaming it over the old one would otherwise
    // leave the watch on the deleted inode.
    let dir = path.parent().unwrap_or(Path::new("."));
    let target = path.to_path_buf();
    let (tx, rx) = mpsc::channel();
    let mut watcher =
        notify::recommended_watcher(move |event: notify::Result<notify::Event>| match event {
            Ok(event) if is_change_to(&event, &target) => {
                let _ = tx.send(());
            }
            Ok(_) => {}
            Err(e) => eprintln!("batune: watch config: {e}"),
        })?;
    watcher.watch(dir, RecursiveMode::NonRecursive)?;
    thread::spawn(move || {
        while rx.recv().is_ok() {
            while rx.recv_timeout(SETTLE).is_ok() {}
            on_change();
        }
    });
    Ok(watcher)
}

/// Removal is ignored so that deleting the file (or the first half of a
/// rename-based save) does not snap the window back to the defaults.
fn is_change_to(event: &notify::Event, target: &Path) -> bool {
    matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_))
        && event
            .paths
            .iter()
            .any(|p| p.file_name() == target.file_name())
}

#[cfg(test)]
mod tests {
    use notify::event::{CreateKind, ModifyKind, RemoveKind};

    use std::path::PathBuf;

    use super::*;

    fn event(kind: EventKind, path: &str) -> notify::Event {
        notify::Event::new(kind).add_path(PathBuf::from(path))
    }

    #[test]
    fn writes_and_creations_of_the_file_count() {
        let target = PathBuf::from("/c/batune/config.toml");
        let modify = EventKind::Modify(ModifyKind::Any);
        let create = EventKind::Create(CreateKind::File);
        assert!(is_change_to(
            &event(modify, "/c/batune/config.toml"),
            &target
        ));
        assert!(is_change_to(
            &event(create, "/c/batune/config.toml"),
            &target
        ));
    }

    #[test]
    fn other_files_and_removal_are_ignored() {
        let target = PathBuf::from("/c/batune/config.toml");
        let modify = EventKind::Modify(ModifyKind::Any);
        let remove = EventKind::Remove(RemoveKind::File);
        assert!(!is_change_to(
            &event(modify, "/c/batune/.config.toml.swp"),
            &target
        ));
        assert!(!is_change_to(
            &event(remove, "/c/batune/config.toml"),
            &target
        ));
    }

    #[test]
    fn saving_the_file_calls_back() {
        let dir = std::env::temp_dir().join(format!("batune-watch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let (tx, rx) = mpsc::channel();
        let _watcher = watch(&path, move || {
            let _ = tx.send(());
        })
        .unwrap();
        std::fs::write(&path, "[font]\nsize = 20\n").unwrap();
        let fired = rx.recv_timeout(Duration::from_secs(5));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
        assert!(fired.is_ok(), "no callback after writing the file");
    }
}
