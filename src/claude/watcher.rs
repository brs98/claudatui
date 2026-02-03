use anyhow::Result;
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

/// File system watcher for Claude directory changes
#[allow(dead_code)]
pub struct ClaudeWatcher {
    _watcher: RecommendedWatcher,
    rx: Receiver<PathBuf>,
}

#[allow(dead_code)]
impl ClaudeWatcher {
    /// Create a new watcher for the Claude directory
    pub fn new(claude_dir: PathBuf) -> Result<Self> {
        let (tx, rx) = mpsc::channel();

        let mut watcher = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    // Only care about modifications and creates
                    if event.kind.is_modify() || event.kind.is_create() {
                        for path in event.paths {
                            // Filter to only JSONL files
                            if path.extension().map_or(false, |ext| ext == "jsonl") {
                                let _ = tx.send(path);
                            }
                        }
                    }
                }
            },
            Config::default().with_poll_interval(Duration::from_millis(500)),
        )?;

        // Watch the entire Claude directory recursively
        watcher.watch(&claude_dir, RecursiveMode::Recursive)?;

        Ok(Self {
            _watcher: watcher,
            rx,
        })
    }

    /// Try to receive a changed file path (non-blocking)
    pub fn try_recv(&self) -> Option<PathBuf> {
        self.rx.try_recv().ok()
    }
}
