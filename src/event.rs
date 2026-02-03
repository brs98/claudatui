pub use crossterm::event::{Event as CrosstermEvent, KeyEvent, MouseEvent};
use std::path::PathBuf;

/// Application events
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum Event {
    /// Terminal tick for regular updates
    Tick,
    /// Keyboard input
    Key(KeyEvent),
    /// Mouse input
    Mouse(MouseEvent),
    /// Terminal resize
    Resize(u16, u16),
    /// PTY output data received
    PtyOutput(Vec<u8>),
    /// File system change detected
    FileChanged(PathBuf),
    /// Error occurred
    Error(String),
}

impl From<CrosstermEvent> for Event {
    fn from(event: CrosstermEvent) -> Self {
        match event {
            CrosstermEvent::Key(key) => Event::Key(key),
            CrosstermEvent::Mouse(mouse) => Event::Mouse(mouse),
            CrosstermEvent::Resize(w, h) => Event::Resize(w, h),
            _ => Event::Tick,
        }
    }
}
