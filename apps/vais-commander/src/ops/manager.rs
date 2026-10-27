//! Runs file operations on the background executor and keeps a row per
//! operation (progress, speed samples, result) for the operations panel.

use crate::vfs::Location;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui_kit::*;

use super::{OpKind, Progress, Report, Request, describe, run};

const TICK: Duration = Duration::from_millis(250);
/// Samples kept per operation for the speed sparkline.
pub const SPEED_SAMPLES: usize = 48;
/// Window over which the displayed speed is averaged.
const SPEED_WINDOW: Duration = Duration::from_secs(2);

pub type OperationId = u64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpStatus {
    Running,
    Cancelling,
    Done,
    Failed,
    Cancelled,
}

pub struct Operation {
    pub id: OperationId,
    pub kind: OpKind,
    /// "3 items" or the single file name.
    pub label: String,
    pub dest: Option<Location>,
    pub started: Instant,
    pub finished: Option<Instant>,
    pub status: OpStatus,
    pub progress: Arc<Mutex<Progress>>,
    pub report: Option<Report>,
    /// Bytes per second, one sample per tick, oldest first.
    pub speed_samples: VecDeque<f64>,
    /// Average speed over the last `SPEED_WINDOW`.
    pub speed: f64,
    cancel: Arc<AtomicBool>,
    /// (time, bytes) samples used to compute the speed.
    byte_samples: VecDeque<(Instant, u64)>,
    _task: Task<()>,
}

impl Operation {
    pub fn snapshot(&self) -> Progress {
        self.progress.lock().map(|p| p.clone()).unwrap_or_default()
    }

    pub fn elapsed(&self) -> Duration {
        self.finished.unwrap_or_else(Instant::now) - self.started
    }

    pub fn is_running(&self) -> bool {
        matches!(self.status, OpStatus::Running | OpStatus::Cancelling)
    }

    /// 0.0 … 1.0; by bytes when there are any, else by entries.
    pub fn fraction(&self) -> f32 {
        let p = self.snapshot();
        if p.total_bytes > 0 {
            (p.bytes as f64 / p.total_bytes as f64).min(1.0) as f32
        } else if p.total > 0 {
            (p.done as f64 / p.total as f64).min(1.0) as f32
        } else if self.is_running() {
            0.0
        } else {
            1.0
        }
    }

    fn sample(&mut self, now: Instant) {
        let bytes = self.snapshot().bytes;
        self.byte_samples.push_back((now, bytes));
        while self.byte_samples.len() > SPEED_SAMPLES + 1 {
            self.byte_samples.pop_front();
        }
        let instant = match self.byte_samples.iter().rev().nth(1) {
            Some(&(t, b)) if now > t => (bytes.saturating_sub(b)) as f64 / (now - t).as_secs_f64(),
            _ => 0.0,
        };
        self.speed_samples.push_back(instant);
        while self.speed_samples.len() > SPEED_SAMPLES {
            self.speed_samples.pop_front();
        }
        let window_start = now - SPEED_WINDOW;
        let oldest = self
            .byte_samples
            .iter()
            .find(|(t, _)| *t >= window_start)
            .or_else(|| self.byte_samples.front())
            .copied();
        self.speed = match oldest {
            Some((t, b)) if now > t => (bytes.saturating_sub(b)) as f64 / (now - t).as_secs_f64(),
            _ => 0.0,
        };
    }
}

pub enum ManagerEvent {
    /// A new operation was queued; the panel should show.
    Started,
    /// Progress tick or status change; repaint.
    Changed,
    /// The operation finished (see its `report`).
    Finished(OperationId),
}

pub struct OperationsManager {
    operations: Vec<Operation>,
    next_id: OperationId,
    ticking: bool,
}

impl EventEmitter<ManagerEvent> for OperationsManager {}

impl OperationsManager {
    pub fn new() -> Self {
        Self {
            operations: Vec::new(),
            next_id: 1,
            ticking: false,
        }
    }

    pub fn operations(&self) -> &[Operation] {
        &self.operations
    }

    pub fn get(&self, id: OperationId) -> Option<&Operation> {
        self.operations.iter().find(|o| o.id == id)
    }

    pub fn running_count(&self) -> usize {
        self.operations.iter().filter(|o| o.is_running()).count()
    }

    /// Combined speed of the running operations, bytes per second.
    pub fn total_speed(&self) -> f64 {
        self.operations
            .iter()
            .filter(|o| o.is_running())
            .map(|o| o.speed)
            .sum()
    }

    pub fn start(&mut self, request: Request, cx: &mut Context<Self>) -> OperationId {
        let id = self.next_id;
        self.next_id += 1;
        let progress = Arc::new(Mutex::new(Progress::default()));
        let cancel = Arc::new(AtomicBool::new(false));
        let label = describe(&request.sources);
        let (kind, dest) = (request.kind, request.dest.clone());

        let work = cx.background_spawn({
            let (progress, cancel) = (progress.clone(), cancel.clone());
            async move { run(&request, &progress, &cancel) }
        });
        let task = cx.spawn(async move |this, cx| {
            let report = work.await;
            this.update(cx, |manager, cx| manager.finish(id, report, cx))
                .ok();
        });

        let now = Instant::now();
        self.operations.push(Operation {
            id,
            kind,
            label,
            dest,
            started: now,
            finished: None,
            status: OpStatus::Running,
            progress,
            report: None,
            speed_samples: VecDeque::new(),
            speed: 0.0,
            cancel,
            byte_samples: VecDeque::from([(now, 0)]),
            _task: task,
        });
        self.ensure_ticking(cx);
        cx.emit(ManagerEvent::Started);
        cx.emit(ManagerEvent::Changed);
        cx.notify();
        id
    }

    pub fn cancel(&mut self, id: OperationId, cx: &mut Context<Self>) {
        if let Some(op) = self
            .operations
            .iter_mut()
            .find(|o| o.id == id && o.is_running())
        {
            op.cancel.store(true, Ordering::Relaxed);
            op.status = OpStatus::Cancelling;
            cx.emit(ManagerEvent::Changed);
            cx.notify();
        }
    }

    /// Cancel the most recently started running operation (Esc).
    pub fn cancel_latest(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(id) = self
            .operations
            .iter()
            .rev()
            .find(|o| o.is_running())
            .map(|o| o.id)
        else {
            return false;
        };
        self.cancel(id, cx);
        true
    }

    pub fn clear_finished(&mut self, cx: &mut Context<Self>) {
        self.operations.retain(|o| o.is_running());
        cx.emit(ManagerEvent::Changed);
        cx.notify();
    }

    pub fn remove(&mut self, id: OperationId, cx: &mut Context<Self>) {
        self.operations.retain(|o| o.id != id || o.is_running());
        cx.emit(ManagerEvent::Changed);
        cx.notify();
    }

    fn finish(&mut self, id: OperationId, report: Report, cx: &mut Context<Self>) {
        let now = Instant::now();
        if let Some(op) = self.operations.iter_mut().find(|o| o.id == id) {
            op.sample(now);
            op.finished = Some(now);
            op.status = if report.cancelled {
                OpStatus::Cancelled
            } else if report.errors.is_empty() {
                OpStatus::Done
            } else {
                OpStatus::Failed
            };
            op.report = Some(report);
        }
        cx.emit(ManagerEvent::Finished(id));
        cx.emit(ManagerEvent::Changed);
        cx.notify();
    }

    fn ensure_ticking(&mut self, cx: &mut Context<Self>) {
        if self.ticking {
            return;
        }
        self.ticking = true;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                let keep_going = this
                    .update(cx, |manager, cx| manager.tick(cx))
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
        })
        .detach();
    }

    /// Sample speeds; returns whether anything is still running.
    fn tick(&mut self, cx: &mut Context<Self>) -> bool {
        let now = Instant::now();
        let mut running = false;
        for op in self.operations.iter_mut().filter(|o| o.is_running()) {
            op.sample(now);
            running = true;
        }
        if !running {
            self.ticking = false;
        }
        cx.emit(ManagerEvent::Changed);
        cx.notify();
        running
    }
}

/// One-line result for notifications.
pub fn summary(op: &Operation) -> String {
    let Some(report) = &op.report else {
        return op.kind.progress_text(&op.label);
    };
    if report.cancelled {
        op.kind.cancelled_text(&op.label, report.done)
    } else if report.errors.is_empty() {
        op.kind.done_text(&op.label)
    } else {
        tn!(
            "{result}: {n} item, {failed} failed: {errors}",
            "{result}: {n} items, {failed} failed: {errors}",
            report.done,
            result = op.kind.done_text(&op.label),
            failed = report.errors.len(),
            errors = report
                .errors
                .iter()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join("; ")
        )
    }
}
