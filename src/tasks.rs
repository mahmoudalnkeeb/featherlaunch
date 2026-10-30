// SPDX-License-Identifier: GPL-3.0-only
//! Progress shared between a background task and the UI.

use std::sync::{Arc, Mutex};

use futures_util::future::AbortHandle;
use std::time::{Duration, Instant};

pub type Waker = Arc<dyn Fn() + Send + Sync>;

const SPEED_WINDOW: Duration = Duration::from_millis(1000);

#[derive(Clone, PartialEq, Eq)]
pub enum Outcome {
    Running,
    Done,
    Cancelled,
    Failed(String),
}

#[derive(Clone)]
pub struct TaskState {
    /// What the task is, fixed for its whole life ("Installing Fabulously Optimized").
    pub title: String,
    /// The step it is on right now ("Downloading libraries").
    pub label: String,
    /// Items (usually files) in the current step.
    pub done: u64,
    pub total: u64,
    /// Bytes of the current step. `bytes_total` is 0 when sizes are unknown.
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub speed: f64,
    pub active: Vec<String>,
    pub started: Instant,
    pub ended: Option<Instant>,
    pub outcome: Outcome,
    pub cancellable: bool,
    pub critical: u32,
    speed_mark: (Instant, u64),
}

impl TaskState {
    pub fn finished(&self) -> bool {
        self.outcome != Outcome::Running
    }

    pub fn can_cancel(&self) -> bool {
        self.cancellable && self.critical == 0 && !self.finished()
    }

    /// Progress of the current step from 0 to 1, by bytes when known, else by items.
    pub fn fraction(&self) -> Option<f32> {
        if self.bytes_total > 0 {
            Some((self.bytes_done as f64 / self.bytes_total as f64).min(1.0) as f32)
        } else if self.total > 0 {
            Some((self.done as f64 / self.total as f64).min(1.0) as f32)
        } else {
            None
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.ended.unwrap_or_else(Instant::now) - self.started
    }

    pub fn remaining(&self) -> Option<Duration> {
        if self.bytes_total == 0 || self.speed < 1.0 {
            return None;
        }
        let left = self.bytes_total.saturating_sub(self.bytes_done) as f64;
        Some(Duration::from_secs_f64(left / self.speed))
    }
}

#[derive(Clone)]
pub struct Progress {
    state: Arc<Mutex<TaskState>>,
    wake: Waker,
    abort: Arc<Mutex<Option<AbortHandle>>>,
}

pub struct Critical(Progress);

impl Drop for Critical {
    fn drop(&mut self) {
        self.0.update(|s| s.critical -= 1);
    }
}

impl Progress {
    pub fn new(title: impl Into<String>, wake: Waker) -> Self {
        let title = title.into();
        let now = Instant::now();
        let state = TaskState {
            label: title.clone(),
            title,
            done: 0,
            total: 0,
            bytes_done: 0,
            bytes_total: 0,
            speed: 0.0,
            active: Vec::new(),
            started: now,
            ended: None,
            outcome: Outcome::Running,
            cancellable: false,
            critical: 0,
            speed_mark: (now, 0),
        };
        Self { state: Arc::new(Mutex::new(state)), wake, abort: Arc::new(Mutex::new(None)) }
    }

    /// Lets the user cancel this task. Cancelling drops the task's future at its next await, so
    /// only tasks whose half-done state is cleaned up on drop should opt in.
    pub fn set_cancellable(&self, abort: AbortHandle) {
        *self.abort.lock().unwrap() = Some(abort);
        self.update(|s| s.cancellable = true);
    }

    /// Marks a step that must run to the end, such as extracting files on a blocking thread or an
    /// external installer process, which dropping the future would not stop. Cancel is hidden
    /// and refused while the returned guard lives.
    pub fn critical(&self) -> Critical {
        self.update(|s| s.critical += 1);
        Critical(self.clone())
    }

    pub fn cancel(&self) {
        if !self.snapshot().can_cancel() {
            return;
        }
        if let Some(abort) = self.abort.lock().unwrap().as_ref() {
            abort.abort();
        }
    }

    fn update(&self, f: impl FnOnce(&mut TaskState)) {
        f(&mut self.state.lock().unwrap());
        (self.wake)();
    }

    pub fn set_label(&self, label: impl Into<String>) {
        let label = label.into();
        self.update(|s| {
            s.label = label;
            s.done = 0;
            s.total = 0;
            s.bytes_done = 0;
            s.bytes_total = 0;
            s.speed = 0.0;
            s.speed_mark = (Instant::now(), 0);
            s.active.clear();
        });
    }

    pub fn add_total(&self, n: u64) {
        self.update(|s| s.total += n);
    }

    pub fn inc(&self) {
        self.update(|s| s.done += 1);
    }

    pub fn add_bytes_total(&self, n: u64) {
        self.update(|s| s.bytes_total += n);
    }

    /// Counts received bytes. Called per network chunk, so it doesn't wake the UI: the UI
    /// redraws on a timer while tasks run.
    pub fn add_bytes(&self, n: u64) {
        let mut s = self.state.lock().unwrap();
        s.bytes_done += n;
        let (at, bytes) = s.speed_mark;
        let dt = at.elapsed();
        if dt >= SPEED_WINDOW {
            s.speed = (s.bytes_done.saturating_sub(bytes)) as f64 / dt.as_secs_f64();
            s.speed_mark = (Instant::now(), s.bytes_done);
        }
    }

    pub fn undo_bytes(&self, received: u64, total: u64) {
        let mut s = self.state.lock().unwrap();
        s.bytes_done = s.bytes_done.saturating_sub(received);
        s.bytes_total = s.bytes_total.saturating_sub(total);
    }

    pub fn file_started(&self, name: &str) {
        let name = name.to_string();
        self.update(|s| s.active.push(name));
    }

    pub fn file_finished(&self, name: &str) {
        self.update(|s| {
            if let Some(i) = s.active.iter().position(|a| a == name) {
                s.active.remove(i);
            }
        });
    }

    pub fn finish(&self, result: &anyhow::Result<()>) {
        self.update(|s| {
            s.ended = Some(Instant::now());
            s.active.clear();
            s.outcome = match result {
                Ok(()) => Outcome::Done,
                Err(e) => Outcome::Failed(format!("{e:#}")),
            };
        });
    }

    pub fn cancelled(&self) {
        self.update(|s| {
            s.ended = Some(Instant::now());
            s.active.clear();
            s.outcome = Outcome::Cancelled;
        });
    }

    pub fn snapshot(&self) -> TaskState {
        self.state.lock().unwrap().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn progress() -> Progress {
        Progress::new("Installing", Arc::new(|| {}))
    }

    #[test]
    fn cancel_is_offered_only_outside_critical_steps() {
        let p = progress();
        assert!(!p.snapshot().can_cancel(), "tasks must opt in");
        let (abort, _registration) = AbortHandle::new_pair();
        p.set_cancellable(abort);
        assert!(p.snapshot().can_cancel());
        let guard = p.critical();
        assert!(!p.snapshot().can_cancel());
        drop(guard);
        assert!(p.snapshot().can_cancel());
        p.cancelled();
        assert!(!p.snapshot().can_cancel());
        assert!(p.snapshot().outcome == Outcome::Cancelled);
    }

    #[test]
    fn fraction_prefers_bytes_and_undo_takes_back_a_failed_attempt() {
        let p = progress();
        assert_eq!(p.snapshot().fraction(), None);
        p.add_total(4);
        p.inc();
        assert_eq!(p.snapshot().fraction(), Some(0.25));
        p.add_bytes_total(1000);
        p.add_bytes(500);
        assert_eq!(p.snapshot().fraction(), Some(0.5));
        p.add_bytes_total(200);
        p.add_bytes(100);
        p.undo_bytes(100, 200);
        let s = p.snapshot();
        assert_eq!((s.bytes_done, s.bytes_total), (500, 1000));
    }

    #[test]
    fn a_new_step_starts_from_zero() {
        let p = progress();
        p.add_total(3);
        p.inc();
        p.add_bytes_total(10);
        p.file_started("a.jar");
        p.set_label("Downloading assets");
        let s = p.snapshot();
        assert_eq!((s.done, s.total, s.bytes_total, s.active.len()), (0, 0, 0, 0));
        assert_eq!(s.label, "Downloading assets");
        assert_eq!(s.title, "Installing");
    }
}
