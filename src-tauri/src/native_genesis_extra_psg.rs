//! Independent sound-chip instances (PSG and YM2612). Cores are allocated and finally destroyed on the
//! control thread; the device callback owns their exclusive mutable access.
use super::{master_system, ym2612, Owner, MAX_SAFE_INTEGER};
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

pub enum State {
    Psg(master_system::Shared),
    Ym2612(ym2612::Shared),
}
enum Core {
    Psg(master_system::Renderer),
    Ym2612(ym2612::Renderer),
}
pub struct Instance {
    pub id: u64,
    epoch: u64,
    active: AtomicBool,
    pub state: State,
    core: UnsafeCell<Core>,
}
impl fmt::Debug for Instance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExtraSoundChip")
            .field("id", &self.id)
            .finish()
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
        self.psg()?;
        message.prepare()
    }
    pub fn psg(&self) -> Result<&master_system::Shared, String> {
        if let State::Psg(state) = &self.state {
            Ok(state)
        } else {
            Err("Additional instance is not a Sega PSG".into())
        }
    }
    pub fn ym2612(&self) -> Result<&ym2612::Shared, String> {
        if let State::Ym2612(state) = &self.state {
            Ok(state)
        } else {
            Err("Additional instance is not a YM2612".into())
        }
    }
    // This method is private to the callback rack. No control function may use
    // it after the Instance has been inserted into the registry/event queue.
    unsafe fn callback_core(&self) -> &mut Core {
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
        self.create_kind(id, epoch, rate, clock, false)
    }
    pub fn create_ym2612(
        &self,
        id: u64,
        epoch: u64,
        rate: u32,
        clock: Option<u32>,
    ) -> Result<Arc<Instance>, String> {
        self.create_kind(id, epoch, rate, clock, true)
    }
    fn create_kind(
        &self,
        id: u64,
        epoch: u64,
        rate: u32,
        clock: Option<u32>,
        fm: bool,
    ) -> Result<Arc<Instance>, String> {
        let mut control = self
            .control
            .lock()
            .map_err(|_| "Additional sound-chip control state is unavailable")?;
        Self::collect(&mut control);
        if id == 0 || id > MAX_SAFE_INTEGER || id <= control.highest {
            return Err(
                "Additional sound-chip instance ID must be positive, unique and increasing".into(),
            );
        }
        if control.instances.len() >= LIMIT {
            return Err(format!("Additional sound-chip instance limit ({LIMIT}) reached; wait for retired instances to drain"));
        }
        let (state, core) = if fm {
            let state = ym2612::Shared::default();
            let mut core = ym2612::Renderer::new(rate)?;
            if let Some(hz) = clock {
                let command = ym2612::Message {
                    id: None,
                    audition: false,
                    method: ym2612::Method::Clock { hz },
                }
                .prepare()?;
                core.apply(&command, None, &state);
            }
            core.publish(&state);
            (State::Ym2612(state), Core::Ym2612(core))
        } else {
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
            (State::Psg(state), Core::Psg(core))
        };
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
            .map_err(|_| "Additional sound-chip control state is unavailable")?;
        Self::collect(&mut control);
        control
            .instances
            .iter()
            .find(|instance| instance.id == id && instance.valid(epoch))
            .cloned()
            .ok_or_else(|| "Additional sound-chip instance has expired".into())
    }
    pub fn dispose(&self, id: u64) -> Result<(), String> {
        self.dispose_kind(id, false)
    }
    pub fn dispose_ym2612(&self, id: u64) -> Result<(), String> {
        self.dispose_kind(id, true)
    }
    fn dispose_kind(&self, id: u64, fm: bool) -> Result<(), String> {
        let mut control = self
            .control
            .lock()
            .map_err(|_| "Additional sound-chip control state is unavailable")?;
        if let Some(instance) = control.instances.iter().find(|instance| instance.id == id) {
            if fm != matches!(instance.state, State::Ym2612(_)) {
                return Err("Additional sound-chip disposal must match its chip type".into());
            }
            instance.active.store(false, Ordering::Release);
        }
        Self::collect(&mut control);
        Ok(())
    }
    pub fn deactivate_all(&self) -> Result<(), String> {
        let mut control = self
            .control
            .lock()
            .map_err(|_| "Additional sound-chip control state is unavailable")?;
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
            .map_err(|_| "Additional sound-chip control state is unavailable")?;
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
            .expect("Bounded additional sound-chip rack") = Some(instance);
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
                        match (instance.callback_core(), &instance.state) {
                            (Core::Psg(core), State::Psg(state)) => {
                                core.cancel_invalid(&valid, state)
                            }
                            (Core::Ym2612(core), State::Ym2612(state)) => {
                                core.cancel_invalid(&valid, state)
                            }
                            _ => unreachable!("Matching additional chip state"),
                        }
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
            match (instance.callback_core(), &instance.state) {
                (Core::Psg(core), State::Psg(state)) => core.apply(command, owner, state),
                _ => unreachable!("Additional PSG action requires PSG instance"),
            }
        }
    }
    pub fn apply_ym2612(
        &mut self,
        instance: &Instance,
        command: &ym2612::Prepared,
        owner: Option<Owner>,
    ) {
        unsafe {
            match (instance.callback_core(), &instance.state) {
                (Core::Ym2612(core), State::Ym2612(state)) => core.apply(command, owner, state),
                _ => unreachable!("Additional YM2612 action requires YM2612 instance"),
            }
        }
    }

    pub fn mix(&mut self, pcm: &mut [f32; 2]) {
        for instance in self.slots.iter().flatten() {
            unsafe {
                match instance.callback_core() {
                    Core::Psg(core) => core.mix(pcm),
                    Core::Ym2612(core) => core.mix(pcm),
                }
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
