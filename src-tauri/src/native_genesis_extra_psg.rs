//! Independent PSG instances. Cores are allocated and finally destroyed on the
//! control thread; the device callback owns their exclusive mutable access.
use super::{master_system, Owner, MAX_SAFE_INTEGER};
use std::{
    cell::UnsafeCell,
    fmt,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

pub const LIMIT: usize = 16;
pub const CLOCK_HELD: &str =
    "Release the manually held additional PSG voices before changing the clock";

pub fn error(message: String) -> String {
    if message == "Release the manually held Game Gear voices before changing the clock" {
        CLOCK_HELD.into()
    } else {
        message
    }
}

pub struct Instance {
    pub id: u64,
    epoch: u64,
    active: AtomicBool,
    pub state: master_system::Shared,
    core: UnsafeCell<master_system::Renderer>,
}
impl fmt::Debug for Instance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExtraPsg").field("id", &self.id).finish()
    }
}
// After publication, only the one device callback reads or mutates `core`.
// Control/status paths access atomics in `state`, never the core. The control
// registry retains every Instance until all callback/event references vanish.
// Consequently its UnsafeCell and C++ core are finally dropped off callback.
unsafe impl Send for Instance {}
unsafe impl Sync for Instance {}
impl Instance {
    pub fn valid(&self, epoch: u64) -> bool {
        self.epoch == epoch && self.active.load(Ordering::Acquire)
    }
    pub fn prepare(
        &self,
        message: &master_system::Message,
    ) -> Result<Arc<master_system::Prepared>, String> {
        if message.chip != master_system::Chip::Segapsg {
            return Err("Additional PSG instances only accept Sega PSG commands".into());
        }
        message.prepare()
    }
    // This method is private to the callback rack. No control function may use
    // it after the Instance has been inserted into the registry/event queue.
    unsafe fn callback_core(&self) -> &mut master_system::Renderer {
        &mut *self.core.get()
    }
}

#[derive(Default)]
struct Control {
    instances: Vec<Arc<Instance>>,
    highest: u64,
}
#[derive(Default)]
pub struct Shared {
    control: Mutex<Control>,
}
impl Shared {
    fn collect(control: &mut Control) {
        // With no external Arc, callback/event access has ended. A callback
        // never clones from this registry, and producer serialization prevents
        // new control references while its final lifetime guard is removed.
        // Arc's final-drop acquire fence synchronizes prior owner releases.
        control.instances.retain(|instance| {
            instance.active.load(Ordering::Acquire) || Arc::strong_count(instance) != 1
        });
    }
    pub fn create(
        &self,
        id: u64,
        epoch: u64,
        rate: u32,
        clock: Option<u32>,
    ) -> Result<Arc<Instance>, String> {
        let mut control = self
            .control
            .lock()
            .map_err(|_| "Additional PSG control state is unavailable")?;
        Self::collect(&mut control);
        if id == 0 || id > MAX_SAFE_INTEGER || id <= control.highest {
            return Err(
                "Additional PSG instance ID must be positive, unique and increasing".into(),
            );
        }
        if control.instances.len() >= LIMIT {
            return Err(format!("Additional PSG instance limit ({LIMIT}) reached; wait for retired instances to drain"));
        }
        let state = master_system::Shared::game_gear();
        let mut core = master_system::Renderer::new_game_gear(rate)?;
        if let Some(hz) = clock {
            let command = master_system::Message {
                chip: master_system::Chip::Segapsg,
                id: None,
                audition: false,
                method: master_system::Method::Clock { hz },
            }
            .prepare()?;
            // Still unpublished and uniquely owned on the control thread.
            core.apply(&command, None, &state);
        }
        core.publish(&state);
        let instance = Arc::new(Instance {
            id,
            epoch,
            active: AtomicBool::new(true),
            state,
            core: UnsafeCell::new(core),
        });
        control.highest = id;
        control.instances.push(instance.clone());
        Ok(instance)
    }
    pub fn get(&self, id: u64, epoch: u64) -> Result<Arc<Instance>, String> {
        let mut control = self
            .control
            .lock()
            .map_err(|_| "Additional PSG control state is unavailable")?;
        Self::collect(&mut control);
        control
            .instances
            .iter()
            .find(|instance| instance.id == id && instance.valid(epoch))
            .cloned()
            .ok_or_else(|| "Additional PSG instance has expired".into())
    }
    pub fn dispose(&self, id: u64) -> Result<(), String> {
        let mut control = self
            .control
            .lock()
            .map_err(|_| "Additional PSG control state is unavailable")?;
        if let Some(instance) = control.instances.iter().find(|instance| instance.id == id) {
            instance.active.store(false, Ordering::Release);
        }
        Self::collect(&mut control);
        Ok(())
    }
    pub fn deactivate_all(&self) -> Result<(), String> {
        let mut control = self
            .control
            .lock()
            .map_err(|_| "Additional PSG control state is unavailable")?;
        for instance in &control.instances {
            instance.active.store(false, Ordering::Release);
        }
        Self::collect(&mut control);
        Ok(())
    }
    pub fn collect_retired(&self) -> Result<(), String> {
        let mut control = self
            .control
            .lock()
            .map_err(|_| "Additional PSG control state is unavailable")?;
        Self::collect(&mut control);
        Ok(())
    }
}

pub struct Renderer {
    slots: [Option<Arc<Instance>>; LIMIT],
}
impl Default for Renderer {
    fn default() -> Self {
        Self {
            slots: std::array::from_fn(|_| None),
        }
    }
}
impl Renderer {
    pub fn install(&mut self, instance: Arc<Instance>) {
        // Admission counts all registered instances, including pending and
        // retiring cores, so a free callback slot is guaranteed.
        *self
            .slots
            .iter_mut()
            .find(|slot| slot.is_none())
            .expect("Bounded additional PSG rack") = Some(instance);
    }
    pub fn prune(&mut self, epoch: u64, valid: impl Fn(Option<Owner>) -> bool, fading: bool) {
        for slot in &mut self.slots {
            if let Some(instance) = slot {
                if !instance.valid(epoch) && !fading {
                    // The registry is a control-side lifetime guard, so this
                    // Arc release cannot destroy the Instance on callback.
                    *slot = None;
                } else if !fading {
                    unsafe {
                        instance
                            .callback_core()
                            .cancel_invalid(&valid, &instance.state);
                    }
                }
            }
        }
    }
    pub fn apply(
        &mut self,
        instance: &Instance,
        command: &master_system::Prepared,
        owner: Option<Owner>,
    ) {
        unsafe {
            instance
                .callback_core()
                .apply(command, owner, &instance.state);
        }
    }
    pub fn mix(&mut self, pcm: &mut [f32; 2]) {
        for instance in self.slots.iter().flatten() {
            unsafe {
                instance.callback_core().mix(pcm);
            }
        }
    }
    pub fn clear(&mut self) {
        self.slots.fill_with(|| None);
    }
}

#[cfg(test)]
impl Shared {
    pub fn counts(&self) -> (usize, usize) {
        let control = self.control.lock().unwrap();
        (
            control.instances.len(),
            control
                .instances
                .iter()
                .filter(|instance| instance.active.load(Ordering::Acquire))
                .count(),
        )
    }
}
