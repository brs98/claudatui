use anyhow::Result;
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

/// File system watcher for sessions-index.json changes
pub struct SessionsWatcher {
    _watcher: RecommendedWatcher,
    rx: Receiver<PathBuf>,
}

impl SessionsWatcher {
    /// Create a new watcher for sessions-index.json files
    pub fn new(claude_dir: &Path) -> Result<Self> {
        let (tx, rx) = mpsc::channel();

        let mut watcher = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    // Only care about modifications and creates
                    if event.kind.is_modify() || event.kind.is_create() {
                        for path in event.paths {
                            // Watch for sessions-index.json and .jsonl file changes
                            // This ensures new sessions appear immediately when their file is created
                            let is_index =
                                path.file_name().is_some_and(|n| n == "sessions-index.json");
                            let is_jsonl = path.extension().is_some_and(|ext| ext == "jsonl");

                            if is_index || is_jsonl {
                                let _ = tx.send(path);
                            }
                        }
                    }
                }
            },
            Config::default().with_poll_interval(Duration::from_millis(500)),
        )?;

        // Watch the projects directory recursively
        let projects_dir = claude_dir.join("projects");
        if projects_dir.exists() {
            watcher.watch(&projects_dir, RecursiveMode::Recursive)?;
        }

        Ok(Self {
            _watcher: watcher,
            rx,
        })
    }

    /// Try to receive a change notification (non-blocking)
    pub fn try_recv(&self) -> Option<PathBuf> {
        self.rx.try_recv().ok()
    }
}
