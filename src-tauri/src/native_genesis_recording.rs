//! A single opt-in preallocated capture. The callback is its sole writer.
//! Result PCM crosses IPC once, after the completion/ownership barrier.
use serde::Serialize;
use std::{
    cell::UnsafeCell,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc,
    },
};
pub const BUDGET: usize = 32 * 1024 * 1024;
#[derive(Debug)]
pub struct Capture {
    data: UnsafeCell<Vec<[f32; 2]>>,
    pub stop: AtomicBool,
    pub completed: AtomicBool,
    frames: AtomicUsize,
    first: AtomicU64,
    rate: u32,
}
// Only Renderer writes data. Control reads only after Renderer transfers its
// Arc through the completion queue, which is the publication/ownership barrier.
unsafe impl Send for Capture {}
unsafe impl Sync for Capture {}
impl Capture {
    pub fn new(rate: u32, seconds: f64) -> Result<Arc<Self>, String> {
        if !seconds.is_finite() || seconds <= 0.0 || seconds > 30.0 {
            return Err(
                "Native recording length must be greater than zero and at most 30 seconds".into(),
            );
        }
        let frames = ((rate as f64 * seconds).ceil() as usize).min(BUDGET / 8);
        Ok(Arc::new(Self {
            data: UnsafeCell::new(vec![[0.0; 2]; frames]),
            stop: AtomicBool::new(false),
            completed: AtomicBool::new(false),
            frames: AtomicUsize::new(0),
            first: AtomicU64::new(0),
            rate,
        }))
    }
    pub fn sample(&self, frame: u64, pcm: [f32; 2]) -> bool {
        let count = self.frames.load(Ordering::Relaxed);
        let data = unsafe { &mut *self.data.get() };
        if self.stop.load(Ordering::Acquire) || count == data.len() {
            self.completed.store(true, Ordering::Release);
            return false;
        }
        if count == 0 {
            self.first.store(frame, Ordering::Relaxed);
        }
        data[count] = pcm;
        self.frames.store(count + 1, Ordering::Relaxed);
        if count + 1 == data.len() {
            self.completed.store(true, Ordering::Release);
            false
        } else {
            true
        }
    }
    pub fn result(&self) -> Result<Recorded, String> {
        if !self.completed.load(Ordering::Acquire) {
            return Err("Native recording is still running".into());
        }
        let count = self.frames.load(Ordering::Acquire);
        let data = unsafe { &*self.data.get() };
        let mut channels = vec![Vec::with_capacity(count), Vec::with_capacity(count)];
        for sample in &data[..count] {
            channels[0].push(sample[0]);
            channels[1].push(sample[1]);
        }
        Ok(Recorded {
            sample_rate: self.rate,
            channels,
            first_frame: self.first.load(Ordering::Acquire),
            frames: count,
            complete: count == data.len(),
        })
    }
}
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Recorded {
    pub sample_rate: u32,
    pub channels: Vec<Vec<f32>>,
    pub first_frame: u64,
    pub frames: usize,
    pub complete: bool,
}
