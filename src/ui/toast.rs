use std::collections::VecDeque;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastType {
    Info,    // Blue
    Success, // Green
    Warning, // Yellow
    Error,   // Red
}

#[derive(Debug, Clone)]
pub struct Toast {
    pub id: u64,
    pub message: String,
    pub toast_type: ToastType,
    pub created_at: Instant,
    pub duration: Duration,
}

impl Toast {
    pub fn new(id: u64, message: impl Into<String>, toast_type: ToastType) -> Self {
        Self {
            id,
            message: message.into(),
            toast_type,
            created_at: Instant::now(),
            duration: Duration::from_secs(3),
        }
    }

    pub fn with_duration(mut self, duration: Duration) -> Self {
        self.duration = duration;
        self
    }

    pub fn is_expired(&self) -> bool {
        self.created_at.elapsed() >= self.duration
    }

    pub fn remaining_ms(&self) -> u64 {
        let elapsed = self.created_at.elapsed().as_millis() as u64;
        let total = self.duration.as_millis() as u64;
        total.saturating_sub(elapsed)
    }
}

pub struct ToastManager {
    queue: VecDeque<Toast>,
    next_id: u64,
    max_visible: usize,
}

impl ToastManager {
    pub fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            next_id: 1,
            max_visible: 5,
        }
    }

    pub fn push(&mut self, message: impl Into<String>, toast_type: ToastType) {
        let toast = Toast::new(self.next_id, message, toast_type);
        self.next_id = self.next_id.wrapping_add(1);
        self.queue.push_back(toast);
        self.trim_queue();
    }

    fn trim_queue(&mut self) {
        // Keep only max_visible toasts
        while self.queue.len() > self.max_visible {
            self.queue.pop_front();
        }
    }

    pub fn update(&mut self) {
        // Remove expired toasts
        self.queue.retain(|t| !t.is_expired());
    }

    pub fn visible_toasts(&self) -> Vec<&Toast> {
        self.queue.iter().collect()
    }

    pub fn clear(&mut self) {
        self.queue.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

impl Default for ToastManager {
    fn default() -> Self {
        Self::new()
    }
}
