//! File watcher based hot-reload for scripts.

use super::{Result, ScriptError};
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

/// A handler invoked by [`HotReloader`] when a file change is detected.
///
/// Implementations should update the module registry with the new source and
/// re-instantiate the module graph.
pub trait HotReloadHandler {
    /// Called when a single `.ts` or `.js` file changes or is removed.
    /// `changed_path` is the absolute path of the file that changed.
    fn on_file_changed(&mut self, changed_path: &Path) -> Result<()>;

    /// Called when multiple files change in a single poll batch.
    ///
    /// Default implementation calls `on_file_changed` for each path.
    /// Override to batch-process all changes in a single incremental reload
    /// (more efficient than per-file reloads).
    fn on_files_changed(&mut self, paths: &[PathBuf]) -> Result<()> {
        for path in paths {
            self.on_file_changed(path)?;
        }
        Ok(())
    }
}

/// Watches a directory recursively and notifies a [`HotReloadHandler`] when
/// `.ts`/`.js` files change.
///
/// Unlike the old full-`reload()` approach, this only triggers when a script
/// file changes, leaving V8 context and host API bindings intact.
pub struct HotReloader {
    #[allow(dead_code)]
    watcher: RecommendedWatcher,
    rx: Receiver<notify::Result<Event>>,
    /// Batch the last handler call rejected, retried by later polls so a
    /// mid-write partial save recovers without waiting for the next edit.
    pending: Vec<PathBuf>,
    /// Last time the handler was invoked (throttles event-less retries).
    last_attempt: Instant,
}

/// Minimum interval between event-less retries of a failed batch, so a
/// permanently broken file cannot spin the reload path every frame.
const RETRY_DEBOUNCE: Duration = Duration::from_millis(250);

impl HotReloader {
    /// Start watching `dir` recursively for changes.
    pub fn new<D: AsRef<Path>>(dir: D) -> Result<Self> {
        let (tx, rx) = channel();
        let mut watcher = RecommendedWatcher::new(
            move |res: notify::Result<Event>| {
                let _ = tx.send(res);
            },
            Config::default(),
        )
        .map_err(|e| ScriptError::Execution(format!("watcher: {}", e)))?;

        watcher
            .watch(dir.as_ref(), RecursiveMode::Recursive)
            .map_err(|e| ScriptError::Execution(format!("watch dir: {}", e)))?;

        Ok(Self {
            watcher,
            rx,
            pending: Vec::new(),
            last_attempt: Instant::now(),
        })
    }

    /// Poll for file system events and forward to the handler.
    ///
    /// Collects all pending changes and calls `on_files_changed` once,
    /// enabling batch incremental reload instead of per-file reload.
    /// This is non-blocking; call it each frame in the engine update loop.
    ///
    /// Removals are forwarded too: deleting a module file surfaces as a
    /// reload error (the module is gone) instead of silently keeping its
    /// last code running. A batch the handler rejects stays pending and is
    /// retried by later polls — event-less retries are throttled by
    /// [`RETRY_DEBOUNCE`].
    pub fn poll<H: HotReloadHandler>(&mut self, handler: &mut H) -> Result<()> {
        let mut changed_paths = std::mem::take(&mut self.pending);
        let mut got_new_events = false;
        while let Ok(event) = self.rx.try_recv() {
            let event = event.map_err(|e| ScriptError::Execution(format!("watch event: {}", e)))?;
            if event.kind.is_modify() || event.kind.is_create() || event.kind.is_remove() {
                got_new_events = true;
                for path in event.paths {
                    let is_script = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .map(|e| e == "ts" || e == "js")
                        .unwrap_or(false);
                    if is_script && !changed_paths.contains(&path) {
                        changed_paths.push(path);
                    }
                }
            }
        }
        if changed_paths.is_empty() {
            return Ok(());
        }
        if !got_new_events && self.last_attempt.elapsed() < RETRY_DEBOUNCE {
            // Pure retry still within the debounce window — keep the batch
            // pending and wait for a later poll.
            self.pending = changed_paths;
            return Ok(());
        }
        self.last_attempt = Instant::now();
        match handler.on_files_changed(&changed_paths) {
            Ok(()) => Ok(()),
            Err(e) => {
                self.pending = changed_paths;
                Err(e)
            }
        }
    }

    /// Create a reloader fed by a returned channel instead of a real watch
    /// (the watcher is inert), so tests can fire synthetic events.
    #[cfg(test)]
    fn for_test() -> (Self, std::sync::mpsc::Sender<notify::Result<Event>>) {
        let (tx, rx) = channel();
        let watcher = RecommendedWatcher::new(|_: notify::Result<Event>| {}, Config::default())
            .expect("inert watcher");
        (
            Self {
                watcher,
                rx,
                pending: Vec::new(),
                last_attempt: Instant::now(),
            },
            tx,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{ModifyKind, RemoveKind};
    use notify::EventKind;

    /// Records every path the reloader forwards.
    #[derive(Default)]
    struct Recorder(Vec<PathBuf>);

    impl HotReloadHandler for Recorder {
        fn on_file_changed(&mut self, changed_path: &Path) -> Result<()> {
            self.0.push(changed_path.to_path_buf());
            Ok(())
        }
    }

    /// Remove events must reach the handler (previously dropped by the
    /// filter), so deleting a module file surfaces instead of going silent.
    #[test]
    fn remove_events_are_forwarded() {
        let (mut reloader, tx) = HotReloader::for_test();
        let gone = PathBuf::from("gone.js");
        tx.send(Ok(
            Event::new(EventKind::Remove(RemoveKind::File)).add_path(gone.clone())
        ))
        .unwrap();
        // Non-script paths are still filtered out.
        tx.send(Ok(
            Event::new(EventKind::Remove(RemoveKind::File)).add_path(PathBuf::from("notes.txt"))
        ))
        .unwrap();

        let mut recorder = Recorder::default();
        reloader.poll(&mut recorder).expect("poll");
        assert_eq!(recorder.0, vec![gone]);
    }

    /// Reads each forwarded file and rejects anything whose content is not
    /// `ok`, standing in for a backend reload of a broken module.
    struct FileGate {
        attempts: usize,
    }

    impl HotReloadHandler for FileGate {
        fn on_file_changed(&mut self, changed_path: &Path) -> Result<()> {
            self.attempts += 1;
            match std::fs::read_to_string(changed_path) {
                Ok(source) if source.trim() == "ok" => Ok(()),
                _ => Err(ScriptError::Execution(format!(
                    "bad module: {}",
                    changed_path.display()
                ))),
            }
        }
    }

    /// A failed batch stays pending: a later poll retries it without any
    /// new file event (throttled by `RETRY_DEBOUNCE`), so a mid-write
    /// partial save recovers on its own once the file reads cleanly again.
    #[test]
    fn failed_batch_is_retried_without_new_event() {
        let dir = std::env::temp_dir().join(format!(
            "moonfield_hot_reload_test_retry_{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("mod.js");
        std::fs::write(&path, "broken").unwrap();

        let (mut reloader, tx) = HotReloader::for_test();
        let mut gate = FileGate { attempts: 0 };

        // The broken write fails, and the batch becomes pending.
        tx.send(Ok(Event::new(EventKind::Modify(ModifyKind::Data(
            notify::event::DataChange::Content,
        )))
        .add_path(path.clone())))
            .unwrap();
        assert!(reloader.poll(&mut gate).is_err());
        assert_eq!(gate.attempts, 1);

        // Fix the file WITHOUT firing a new event. An immediate poll must
        // not retry yet (debounced) — no tight retry loop.
        std::fs::write(&path, "ok").unwrap();
        reloader.poll(&mut gate).expect("debounced poll is a no-op");
        assert_eq!(gate.attempts, 1);

        // After the debounce window the pending batch is retried and the
        // module recovers.
        std::thread::sleep(RETRY_DEBOUNCE + Duration::from_millis(50));
        reloader.poll(&mut gate).expect("retry recovers");
        assert_eq!(gate.attempts, 2);

        std::fs::remove_dir_all(&dir).ok();
    }
}
