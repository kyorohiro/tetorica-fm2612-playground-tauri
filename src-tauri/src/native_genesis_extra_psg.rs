//! Independent sound-chip instances (PSG, YM2612 and YM2203). Cores are allocated and finally destroyed on the
//! control thread; the device callback owns their exclusive mutable access.
use super::{
    extra_budget, extra_ym2608, extra_ym2610, master_system, ym2203, ym2612, Owner,
    MAX_SAFE_INTEGER,
};
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
    Ym2203(ym2203::Shared),
    Ym2608(extra_ym2608::Shared),
    Ym2610(extra_ym2610::Shared),
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Psg,
    Ym2612,
    Ym2203,
    Ym2608,
    Ym2610,
}
enum Core {
    Psg(master_system::Renderer),
    Ym2612(ym2612::Renderer),
    Ym2203(ym2203::Renderer),
    Ym2608(extra_ym2608::Renderer),
    Ym2610(extra_ym2610::Renderer),
}
pub struct Instance {
    pub id: u64,
    epoch: u64,
    active: AtomicBool,
    pub state: State,
    core: UnsafeCell<Core>,
    _memory: extra_budget::Permit,
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
    pub fn ym2203(&self) -> Result<&ym2203::Shared, String> {
        if let State::Ym2203(state) = &self.state {
            Ok(state)
        } else {
            Err("Additional instance is not a YM2203".into())
        }
    }
    pub fn ym2608(&self) -> Result<&extra_ym2608::Shared, String> {
        if let State::Ym2608(state) = &self.state {
            Ok(state)
        } else {
            Err("Additional instance is not a YM2608".into())
        }
    }
    pub fn ym2610(&self) -> Result<&extra_ym2610::Shared, String> {
        if let State::Ym2610(state) = &self.state {
            Ok(state)
        } else {
            Err("Additional instance is not a YM2610".into())
        }
    }
    fn kind(&self) -> Kind {
        match self.state {
            State::Psg(_) => Kind::Psg,
            State::Ym2612(_) => Kind::Ym2612,
            State::Ym2203(_) => Kind::Ym2203,
            State::Ym2608(_) => Kind::Ym2608,
            State::Ym2610(_) => Kind::Ym2610,
        }
    }
    // This method is private to the callback rack. No control function may use
    // it after the Instance has been inserted into the registry/event queue.
    unsafe fn callback_core(&self) -> &mut Core {
        &mut *self.core.get()
    }
}

pub enum Payload {
    Psg(Arc<master_system::Prepared>),
    Ym2612(Arc<ym2612::Prepared>),
    Ym2203(Arc<ym2203::Prepared>),
    Ym2608(Arc<extra_ym2608::Prepared>),
    Ym2610(Arc<extra_ym2610::Prepared>),
}
impl Payload {
    fn bytes(&self) -> usize {
        match self {
            Self::Psg(v) => v.retained_bytes(),
            Self::Ym2612(v) => v.retained_bytes(),
            Self::Ym2203(v) => v.retained_bytes(),
            Self::Ym2608(v) => v.retained_bytes(),
            Self::Ym2610(v) => v.retained_bytes(),
        }
    }
    fn unique(&self) -> bool {
        match self {
            Self::Psg(v) => Arc::strong_count(v) == 1,
            Self::Ym2612(v) => Arc::strong_count(v) == 1,
            Self::Ym2203(v) => Arc::strong_count(v) == 1,
            Self::Ym2608(v) => Arc::strong_count(v) == 1,
            Self::Ym2610(v) => Arc::strong_count(v) == 1,
        }
    }
}
struct PayloadGuard {
    payload: Payload,
    _memory: extra_budget::Permit,
}
#[derive(Default)]
struct Control {
    instances: Vec<Arc<Instance>>,
    highest: u64,
    payloads: Vec<PayloadGuard>,
}
#[derive(Default)]
pub struct Shared {
    control: Mutex<Control>,
    budget: Arc<extra_budget::Budget>,
}
impl Shared {
    fn collect(control: &mut Control) {
        for instance in &control.instances {
            match &instance.state {
                State::Psg(s) => s.collect_retired(),
                State::Ym2612(s) => s.collect_retired(),
                State::Ym2203(s) => s.collect_retired(),
                State::Ym2608(s) => s.collect_retired(),
                State::Ym2610(s) => s.collect_retired(),
            }
        }

        // With no external Arc, callback/event access has ended. A callback
        // never clones from this registry, and producer serialization prevents
        // new control references while its final lifetime guard is removed.
        // Arc's final-drop acquire fence synchronizes prior owner releases.
        control.instances.retain(|instance| {
            instance.active.load(Ordering::Acquire) || Arc::strong_count(instance) != 1
        });
        control.payloads.retain(|v| !v.payload.unique());
        // Dropping retired Prepared guards may release their immutable page
        // versions. Reclaim those versions on control before new admission.
        for instance in &control.instances {
            if let State::Ym2610(s) = &instance.state {
                s.collect_retired();
            }
        }

        if control.payloads.capacity() > 64.max(control.payloads.len().saturating_mul(2)) {
            control.payloads.shrink_to_fit();
        }
    }
    pub fn reserve_payloads(
        &self,
        sizes: impl Iterator<Item = usize>,
    ) -> Result<Vec<extra_budget::Permit>, String> {
        let mut c = self
            .control
            .lock()
            .map_err(|_| "Additional sound-chip control state is unavailable")?;
        Self::collect(&mut c);
        sizes
            .map(|bytes| {
                self.budget.reserve(
                    bytes
                        .checked_add(extra_budget::PAYLOAD_OVERHEAD)
                        .ok_or("Additional Native sound-chip payload size overflow")?,
                )
            })
            .collect()
    }
    pub fn retain_payloads(
        &self,
        payloads: Vec<Payload>,
        permits: Vec<extra_budget::Permit>,
    ) -> Result<(), String> {
        let mut c = self
            .control
            .lock()
            .map_err(|_| "Additional sound-chip control state is unavailable")?;
        Self::collect(&mut c);
        if payloads.len() != permits.len() {
            return Err("Additional Native sound-chip payload reservation mismatch".into());
        }
        let guards: Vec<_> = payloads
            .into_iter()
            .zip(permits)
            .map(|(payload, memory)| PayloadGuard {
                payload,
                _memory: memory,
            })
            .collect();
        if guards
            .iter()
            .any(|g| g.payload.bytes() + extra_budget::PAYLOAD_OVERHEAD > g._memory.bytes())
        {
            return Err("Additional Native sound-chip payload capacity exceeds reservation".into());
        }
        c.payloads.extend(guards);
        Ok(())
    }
    pub fn create_ym2610(
        &self,
        id: u64,
        epoch: u64,
        rate: u32,
        clock: Option<u32>,
    ) -> Result<Arc<Instance>, String> {
        self.create_kind(id, epoch, rate, clock, Kind::Ym2610)
    }
    pub fn create_ym2608(
        &self,
        id: u64,
        epoch: u64,
        rate: u32,
        clock: Option<u32>,
    ) -> Result<Arc<Instance>, String> {
        self.create_kind(id, epoch, rate, clock, Kind::Ym2608)
    }
    pub fn create(
        &self,
        id: u64,
        epoch: u64,
        rate: u32,
        clock: Option<u32>,
    ) -> Result<Arc<Instance>, String> {
        self.create_kind(id, epoch, rate, clock, Kind::Psg)
    }
    pub fn create_ym2612(
        &self,
        id: u64,
        epoch: u64,
        rate: u32,
        clock: Option<u32>,
    ) -> Result<Arc<Instance>, String> {
        self.create_kind(id, epoch, rate, clock, Kind::Ym2612)
    }
    pub fn create_ym2203(
        &self,
        id: u64,
        epoch: u64,
        rate: u32,
        clock: Option<u32>,
    ) -> Result<Arc<Instance>, String> {
        self.create_kind(id, epoch, rate, clock, Kind::Ym2203)
    }
    fn create_kind(
        &self,
        id: u64,
        epoch: u64,
        rate: u32,
        clock: Option<u32>,
        kind: Kind,
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
        let memory = self.budget.reserve(
            extra_budget::INSTANCE_BYTES
                + if kind == Kind::Ym2608 {
                    extra_budget::YM2608_MEMORY_BYTES
                } else {
                    0
                },
        )?;
        let (state, core) = if kind == Kind::Ym2610 {
            let state = extra_ym2610::Shared::with_budget(self.budget.clone());
            let mut core = extra_ym2610::Renderer::new_additional(rate)?;
            if let Some(hz) = clock {
                let cmd = extra_ym2610::Message {
                    id: None,
                    audition: false,
                    group: None,
                    method: extra_ym2610::Method::Clock { hz },
                }
                .prepare()?;
                core.apply(&cmd, None, &state);
            }
            core.publish(&state);
            (State::Ym2610(state), Core::Ym2610(core))
        } else if kind == Kind::Ym2608 {
            let state = extra_ym2608::Shared::default();
            let mut core = extra_ym2608::Renderer::new_additional(rate)?;
            if let Some(hz) = clock {
                let cmd = extra_ym2608::Message {
                    id: None,
                    audition: false,
                    group: None,
                    method: extra_ym2608::Method::Clock { hz },
                }
                .prepare()?;
                core.apply(&cmd, None, &state);
            }
            core.publish(&state);
            (State::Ym2608(state), Core::Ym2608(core))
        } else if kind == Kind::Ym2612 {
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
        } else if kind == Kind::Ym2203 {
            let state = ym2203::Shared::additional();
            let mut core = ym2203::Renderer::new_additional(rate)?;
            if let Some(hz) = clock {
                let command = ym2203::Message {
                    id: None,
                    audition: false,
                    group: None,
                    method: ym2203::Method::Clock { hz },
                }
                .prepare()?;
                core.apply(&command, None, &state);
            }
            core.publish(&state);
            (State::Ym2203(state), Core::Ym2203(core))
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
            _memory: memory,
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
        self.dispose_kind(id, Kind::Psg)
    }
    pub fn dispose_ym2612(&self, id: u64) -> Result<(), String> {
        self.dispose_kind(id, Kind::Ym2612)
    }
    pub fn dispose_ym2203(&self, id: u64) -> Result<(), String> {
        self.dispose_kind(id, Kind::Ym2203)
    }
    pub fn dispose_ym2610(&self, id: u64) -> Result<(), String> {
        self.dispose_kind(id, Kind::Ym2610)
    }
    pub fn dispose_ym2608(&self, id: u64) -> Result<(), String> {
        self.dispose_kind(id, Kind::Ym2608)
    }
    fn dispose_kind(&self, id: u64, kind: Kind) -> Result<(), String> {
        let mut control = self
            .control
            .lock()
            .map_err(|_| "Additional sound-chip control state is unavailable")?;
        if let Some(instance) = control.instances.iter().find(|instance| instance.id == id) {
            if kind != instance.kind() {
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
                            (Core::Ym2203(core), State::Ym2203(state)) => {
                                core.cancel_invalid(&valid, state)
                            }
                            (Core::Ym2608(core), State::Ym2608(state)) => {
                                core.cancel_invalid(&valid, state)
                            }
                            (Core::Ym2610(core), State::Ym2610(state)) => {
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

    pub fn apply_ym2203(
        &mut self,
        instance: &Instance,
        command: &Arc<ym2203::Prepared>,
        owner: Option<Owner>,
    ) {
        unsafe {
            match (instance.callback_core(), &instance.state) {
                (Core::Ym2203(core), State::Ym2203(state)) => core.apply(command, owner, state),
                _ => unreachable!("Additional YM2203 action requires YM2203 instance"),
            }
        }
    }
    pub fn apply_ym2608(
        &mut self,
        instance: &Instance,
        command: &Arc<extra_ym2608::Prepared>,
        owner: Option<Owner>,
    ) {
        unsafe {
            match (instance.callback_core(), &instance.state) {
                (Core::Ym2608(core), State::Ym2608(state)) => core.apply(command, owner, state),
                _ => unreachable!("Additional YM2608 action requires YM2608 instance"),
            }
        }
    }
    pub fn apply_ym2610(
        &mut self,
        instance: &Instance,
        command: &Arc<extra_ym2610::Prepared>,
        owner: Option<Owner>,
    ) {
        unsafe {
            match (instance.callback_core(), &instance.state) {
                (Core::Ym2610(core), State::Ym2610(state)) => core.apply(command, owner, state),
                _ => unreachable!("Additional YM2610 action requires YM2610 instance"),
            }
        }
    }
    pub fn mix(&mut self, pcm: &mut [f32; 2]) {
        for instance in self.slots.iter().flatten() {
            unsafe {
                match (instance.callback_core(), &instance.state) {
                    (Core::Psg(core), _) => core.mix(pcm),
                    (Core::Ym2612(core), _) => core.mix(pcm),
                    (Core::Ym2610(core), _) => core.mix(pcm),
                    (Core::Ym2203(core), State::Ym2203(state)) => core.mix_additional(pcm, state),
                    (Core::Ym2608(core), State::Ym2608(state)) => core.mix_additional(pcm, state),
                    _ => unreachable!("Matching additional chip state"),
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
    pub fn with_memory_limit(limit: usize) -> Self {
        Self {
            control: Mutex::new(Control::default()),
            budget: Arc::new(extra_budget::Budget::new(limit)),
        }
    }
    pub fn memory_used(&self) -> usize {
        self.budget.used()
    }
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
