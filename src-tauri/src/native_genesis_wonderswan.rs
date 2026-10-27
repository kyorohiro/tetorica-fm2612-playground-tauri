//! The primary device uses the same callback and lease rules as extra devices.
//! Only the public status envelope and transactional retain signature differ.
use super::{extra_wonderswan as extra, Owner};
use serde::Serialize;
use std::sync::Arc;

pub use extra::{Message, Prepared, CLOCK};

#[derive(Default)]
pub struct Shared {
    inner: extra::Shared,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    #[serde(flatten)]
    pub wonderswan: extra::ChipStatus,
    pub frame: u64,
    pub sample_rate: u32,
    pub revision: u64,
}
impl Shared {
    pub fn clock(&self) -> u32 {
        self.inner.clock()
    }
    pub fn retain(&self, commands: &[Arc<Prepared>]) -> Result<(), String> {
        let commands: Vec<_> = commands.iter().map(|c| (c.clone(), None)).collect();
        self.inner.retain(&commands)
    }
    pub fn wait_applied(&self, id: u64) -> Result<(), String> {
        self.inner.wait_applied(id)
    }
    pub fn retire(&self, command: Arc<Prepared>) {
        self.inner.retire(command)
    }
    pub fn valid_event(&self, command: &Prepared, sequence: u64, owner: Option<Owner>) -> bool {
        self.inner.valid_event(command, sequence, owner)
    }
    pub fn commit_cancel(&self, command: &Prepared) -> bool {
        self.inner.commit_cancel(command)
    }
    pub fn status(&self, frame: u64) -> Result<Status, String> {
        let status = self.inner.status(frame)?;
        Ok(Status {
            wonderswan: status.wonderswan,
            frame: status.frame,
            sample_rate: status.sample_rate,
            revision: status.revision,
        })
    }
}
pub struct Renderer {
    inner: extra::Renderer,
}
impl Renderer {
    pub fn new(rate: u32) -> Result<Self, String> {
        Self::new_with_clock(rate, CLOCK)
    }
    pub fn new_with_clock(rate: u32, clock: u32) -> Result<Self, String> {
        Ok(Self {
            inner: extra::Renderer::new_with_clock(rate, clock)?,
        })
    }
    pub fn apply(&mut self, command: &Prepared, owner: Option<Owner>, shared: &Shared) {
        self.inner.apply(command, owner, &shared.inner)
    }
    pub fn publish(&self, shared: &Shared) {
        self.inner.publish(&shared.inner)
    }
    pub fn cancel_invalid(&mut self, valid: impl Fn(Option<Owner>) -> bool, shared: &Shared) {
        self.inner.cancel_invalid(valid, &shared.inner)
    }
    pub fn stop(&mut self, shared: &Shared) {
        self.inner.stop(&shared.inner)
    }
    pub fn mix(&mut self, pcm: &mut [f32; 2]) {
        self.inner.mix(pcm)
    }
}
