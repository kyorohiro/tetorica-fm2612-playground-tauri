//! Independent sound-chip instances (PSG, YM2612 and YM2203). Cores are allocated and finally destroyed on the
//! control thread; the device callback owns their exclusive mutable access.
use super::{
    extra_budget, extra_msx, extra_okim6258, extra_opl, extra_rf5c164, extra_ym2151, extra_ym2608,
    extra_ym2610, master_system, ym2203, ym2612, Owner, MAX_SAFE_INTEGER,
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
    Ym2151(extra_ym2151::Shared),
    Ay8910(extra_msx::Shared),
    Y8950(extra_opl::Shared),
    Ymf262(extra_opl::Shared),
    Rf5c164(extra_rf5c164::Shared),
    Okim6258(extra_okim6258::Shared),
    Ym3812(extra_opl::Shared),
    Ym3526(extra_opl::Shared),
    Ym2413(extra_msx::Shared),
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Psg,
    Ym2612,
    Ym2203,
    Ym2608,
    Ym2610,
    Ym2151,
    Ay8910,
    Y8950,
    Ymf262,
    Rf5c164,
    Okim6258,
    Ym3812,
    Ym3526,
    Ym2413,
}
enum Core {
    Psg(master_system::Renderer),
    Ym2612(ym2612::Renderer),
    Ym2203(ym2203::Renderer),
    Ym2608(extra_ym2608::Renderer),
    Ym2610(extra_ym2610::Renderer),
    Ym2151(extra_ym2151::Renderer),
    Ay8910(extra_msx::Renderer),
    Y8950(extra_opl::Renderer),
    Ymf262(extra_opl::Renderer),
    Rf5c164(extra_rf5c164::Renderer),
    Okim6258(extra_okim6258::Renderer),
    Ym3812(extra_opl::Renderer),
    Ym3526(extra_opl::Renderer),
    Ym2413(extra_msx::Renderer),
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
    pub fn ym2151(&self) -> Result<&extra_ym2151::Shared, String> {
        if let State::Ym2151(state) = &self.state {
            Ok(state)
        } else {
            Err("Additional instance is not a YM2151".into())
        }
    }
    pub fn ym2413(&self) -> Result<&extra_msx::Shared, String> {
        if let State::Ym2413(state) = &self.state {
            Ok(state)
        } else {
            Err("Additional instance is not a YM2413".into())
        }
    }
    pub fn ay8910(&self) -> Result<&extra_msx::Shared, String> {
        if let State::Ay8910(state) = &self.state {
            Ok(state)
        } else {
            Err("Additional instance is not an AY8910".into())
        }
    }
    pub fn ym3526(&self) -> Result<&extra_opl::Shared, String> {
        if let State::Ym3526(state) = &self.state {
            Ok(state)
        } else {
            Err("Additional instance is not a YM3526".into())
        }
    }
    pub fn ym3812(&self) -> Result<&extra_opl::Shared, String> {
        if let State::Ym3812(state) = &self.state {
            Ok(state)
        } else {
            Err("Additional instance is not a YM3812".into())
        }
    }
    pub fn y8950(&self) -> Result<&extra_opl::Shared, String> {
        if let State::Y8950(state) = &self.state {
            Ok(state)
        } else {
            Err("Additional instance is not a Y8950".into())
        }
    }
    pub fn ymf262(&self) -> Result<&extra_opl::Shared, String> {
        if let State::Ymf262(state) = &self.state {
            Ok(state)
        } else {
            Err("Additional instance is not a YMF262".into())
        }
    }
    pub fn rf5c164(&self) -> Result<&extra_rf5c164::Shared, String> {
        if let State::Rf5c164(state) = &self.state {
            Ok(state)
        } else {
            Err("Additional instance is not a RF5C164".into())
        }
    }
    pub fn okim6258(&self) -> Result<&extra_okim6258::Shared, String> {
        if let State::Okim6258(state) = &self.state {
            Ok(state)
        } else {
            Err("Additional instance is not a OKIM6258".into())
        }
    }
    fn kind(&self) -> Kind {
        match self.state {
            State::Psg(_) => Kind::Psg,
            State::Ym2612(_) => Kind::Ym2612,
            State::Ym2203(_) => Kind::Ym2203,
            State::Ym2608(_) => Kind::Ym2608,
            State::Ym2610(_) => Kind::Ym2610,
            State::Ym2151(_) => Kind::Ym2151,
            State::Ay8910(_) => Kind::Ay8910,
            State::Y8950(_) => Kind::Y8950,
            State::Ymf262(_) => Kind::Ymf262,
            State::Rf5c164(_) => Kind::Rf5c164,
            State::Okim6258(_) => Kind::Okim6258,
            State::Ym3812(_) => Kind::Ym3812,
            State::Ym3526(_) => Kind::Ym3526,
            State::Ym2413(_) => Kind::Ym2413,
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
    Ym2151(Arc<extra_ym2151::Prepared>),
    Ay8910(Arc<extra_msx::Prepared>),
    Y8950(Arc<extra_opl::Prepared>),
    Ymf262(Arc<extra_opl::Prepared>),
    Rf5c164(Arc<extra_rf5c164::Prepared>),
    Okim6258(Arc<extra_okim6258::Prepared>),
    Ym3812(Arc<extra_opl::Prepared>),
    Ym3526(Arc<extra_opl::Prepared>),
    Ym2413(Arc<extra_msx::Prepared>),
}
impl Payload {
    fn bytes(&self) -> usize {
        match self {
            Self::Psg(v) => v.retained_bytes(),
            Self::Ym2612(v) => v.retained_bytes(),
            Self::Ym2203(v) => v.retained_bytes(),
            Self::Ym2608(v) => v.retained_bytes(),
            Self::Ym2610(v) => v.retained_bytes(),
            Self::Ym2151(v) => v.retained_bytes(),
            Self::Ay8910(v) => v.retained_bytes(),
            Self::Y8950(v) => v.retained_bytes(),
            Self::Ymf262(v) => v.retained_bytes(),
            Self::Rf5c164(v) => v.retained_bytes(),
            Self::Okim6258(v) => v.retained_bytes(),
            Self::Ym3812(v) => v.retained_bytes(),
            Self::Ym3526(v) => v.retained_bytes(),
            Self::Ym2413(v) => v.retained_bytes(),
        }
    }
    fn unique(&self) -> bool {
        match self {
            Self::Psg(v) => Arc::strong_count(v) == 1,
            Self::Ym2612(v) => Arc::strong_count(v) == 1,
            Self::Ym2203(v) => Arc::strong_count(v) == 1,
            Self::Ym2608(v) => Arc::strong_count(v) == 1,
            Self::Ym2610(v) => Arc::strong_count(v) == 1,
            Self::Ym2151(v) => Arc::strong_count(v) == 1,
            Self::Ay8910(v) => Arc::strong_count(v) == 1,
            Self::Y8950(v) => Arc::strong_count(v) == 1,
            Self::Ymf262(v) => Arc::strong_count(v) == 1,
            Self::Rf5c164(v) => Arc::strong_count(v) == 1,
            Self::Okim6258(v) => Arc::strong_count(v) == 1,
            Self::Ym3812(v) => Arc::strong_count(v) == 1,
            Self::Ym3526(v) => Arc::strong_count(v) == 1,
            Self::Ym2413(v) => Arc::strong_count(v) == 1,
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
                State::Ym2151(s) => s.collect_retired(),
                State::Ay8910(s) => s.collect_retired(),
                State::Y8950(s) => s.collect_retired(),
                State::Ymf262(s) => s.collect_retired(),
                State::Rf5c164(s) => s.collect_retired(),
                State::Okim6258(s) => s.collect_retired(),
                State::Ym3812(s) => s.collect_retired(),
                State::Ym3526(s) => s.collect_retired(),
                State::Ym2413(s) => s.collect_retired(),
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
    pub fn create_ym3526(
        &self,
        id: u64,
        epoch: u64,
        rate: u32,
        clock: Option<u32>,
    ) -> Result<Arc<Instance>, String> {
        self.create_kind(id, epoch, rate, clock, Kind::Ym3526)
    }
    pub fn create_ym3812(
        &self,
        id: u64,
        epoch: u64,
        rate: u32,
        clock: Option<u32>,
    ) -> Result<Arc<Instance>, String> {
        self.create_kind(id, epoch, rate, clock, Kind::Ym3812)
    }
    pub fn create_ymf262(
        &self,
        id: u64,
        epoch: u64,
        rate: u32,
        clock: Option<u32>,
    ) -> Result<Arc<Instance>, String> {
        self.create_kind(id, epoch, rate, clock, Kind::Ymf262)
    }
    pub fn create_rf5c164(
        &self,
        id: u64,
        epoch: u64,
        rate: u32,
        clock: Option<u32>,
    ) -> Result<Arc<Instance>, String> {
        self.create_kind(id, epoch, rate, clock, Kind::Rf5c164)
    }
    pub fn create_okim6258(
        &self,
        id: u64,
        epoch: u64,
        rate: u32,
        clock: Option<u32>,
    ) -> Result<Arc<Instance>, String> {
        self.create_kind(id, epoch, rate, clock, Kind::Okim6258)
    }
    pub fn create_y8950(
        &self,
        id: u64,
        epoch: u64,
        rate: u32,
        clock: Option<u32>,
    ) -> Result<Arc<Instance>, String> {
        self.create_kind(id, epoch, rate, clock, Kind::Y8950)
    }
    pub fn create_ym2413(
        &self,
        id: u64,
        epoch: u64,
        rate: u32,
        clock: Option<u32>,
    ) -> Result<Arc<Instance>, String> {
        self.create_kind(id, epoch, rate, clock, Kind::Ym2413)
    }
    pub fn create_ay8910(
        &self,
        id: u64,
        epoch: u64,
        rate: u32,
        clock: Option<u32>,
    ) -> Result<Arc<Instance>, String> {
        self.create_kind(id, epoch, rate, clock, Kind::Ay8910)
    }
    pub fn create_ym2151(
        &self,
        id: u64,
        epoch: u64,
        rate: u32,
        clock: Option<u32>,
    ) -> Result<Arc<Instance>, String> {
        self.create_kind(id, epoch, rate, clock, Kind::Ym2151)
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
                } else if kind == Kind::Rf5c164 {
                    extra_rf5c164::RAM
                } else {
                    0
                },
        )?;
        let (state, core) = if kind == Kind::Rf5c164 {
            let state = extra_rf5c164::Shared::default();
            let mut core = extra_rf5c164::Renderer::new(rate)?;
            if let Some(hz) = clock {
                let cmd = extra_rf5c164::Message {
                    id: None,
                    audition: false,
                    group: None,
                    method: extra_rf5c164::Method::Clock { hz },
                }
                .prepare()?;
                core.apply(&cmd, None, &state);
            }
            core.publish(&state);
            (State::Rf5c164(state), Core::Rf5c164(core))
        } else if kind == Kind::Okim6258 {
            if let Some(hz) = clock {
                extra_okim6258::validate_clock(hz, rate)?;
            }
            let state = extra_okim6258::Shared::default();
            let mut core = extra_okim6258::Renderer::new(rate)?;
            if let Some(hz) = clock {
                let cmd = extra_okim6258::Message {
                    id: None,
                    audition: false,
                    group: None,
                    method: extra_okim6258::Method::Clock { hz },
                }
                .prepare()?;
                core.apply(&cmd, None, &state);
            }
            core.publish(&state);
            (State::Okim6258(state), Core::Okim6258(core))
        } else if kind == Kind::Ymf262 {
            let chip = extra_opl::Chip::Ymf262;
            let state = extra_opl::Shared::new(chip);
            let mut core = extra_opl::Renderer::new(rate, chip)?;
            if let Some(hz) = clock {
                let cmd = extra_opl::Message {
                    id: None,
                    audition: false,
                    group: None,
                    method: extra_opl::Method::Clock { hz },
                }
                .prepare(chip)?;
                core.apply(&cmd, None, &state);
            }
            core.publish(&state);
            (State::Ymf262(state), Core::Ymf262(core))
        } else if kind == Kind::Y8950 {
            let chip = extra_opl::Chip::Y8950;
            let state = extra_opl::Shared::new(chip);
            let mut core = extra_opl::Renderer::new(rate, chip)?;
            if let Some(hz) = clock {
                let cmd = extra_opl::Message {
                    id: None,
                    audition: false,
                    group: None,
                    method: extra_opl::Method::Clock { hz },
                }
                .prepare(chip)?;
                core.apply(&cmd, None, &state);
            }
            core.publish(&state);
            (State::Y8950(state), Core::Y8950(core))
        } else if kind == Kind::Ym3812 {
            let chip = extra_opl::Chip::Ym3812;
            let state = extra_opl::Shared::new(chip);
            let mut core = extra_opl::Renderer::new(rate, chip)?;
            if let Some(hz) = clock {
                let cmd = extra_opl::Message {
                    id: None,
                    audition: false,
                    group: None,
                    method: extra_opl::Method::Clock { hz },
                }
                .prepare(chip)?;
                core.apply(&cmd, None, &state);
            }
            core.publish(&state);
            (State::Ym3812(state), Core::Ym3812(core))
        } else if kind == Kind::Ym3526 {
            let chip = extra_opl::Chip::Ym3526;
            let state = extra_opl::Shared::new(chip);
            let mut core = extra_opl::Renderer::new(rate, chip)?;
            if let Some(hz) = clock {
                let cmd = extra_opl::Message {
                    id: None,
                    audition: false,
                    group: None,
                    method: extra_opl::Method::Clock { hz },
                }
                .prepare(chip)?;
                core.apply(&cmd, None, &state);
            }
            core.publish(&state);
            (State::Ym3526(state), Core::Ym3526(core))
        } else if kind == Kind::Ay8910 {
            let chip = extra_msx::Chip::Ay8910;
            let state = extra_msx::Shared::new(chip);
            let mut core = extra_msx::Renderer::new(rate, chip)?;
            if let Some(hz) = clock {
                let cmd = extra_msx::Message {
                    id: None,
                    audition: false,
                    group: None,
                    method: extra_msx::Method::Clock { hz },
                }
                .prepare(chip)?;
                core.apply(&cmd, None, &state);
            }
            core.publish(&state);
            (State::Ay8910(state), Core::Ay8910(core))
        } else if kind == Kind::Ym2413 {
            let chip = extra_msx::Chip::Ym2413;
            let state = extra_msx::Shared::new(chip);
            let mut core = extra_msx::Renderer::new(rate, chip)?;
            if let Some(hz) = clock {
                let cmd = extra_msx::Message {
                    id: None,
                    audition: false,
                    group: None,
                    method: extra_msx::Method::Clock { hz },
                }
                .prepare(chip)?;
                core.apply(&cmd, None, &state);
            }
            core.publish(&state);
            (State::Ym2413(state), Core::Ym2413(core))
        } else if kind == Kind::Ym2151 {
            let state = extra_ym2151::Shared::default();
            let mut core = extra_ym2151::Renderer::new_additional(rate)?;
            if let Some(hz) = clock {
                let cmd = extra_ym2151::Message {
                    id: None,
                    audition: false,
                    group: None,
                    method: extra_ym2151::Method::Clock { hz },
                }
                .prepare()?;
                core.apply(&cmd, None, &state);
            }
            core.publish(&state);
            (State::Ym2151(state), Core::Ym2151(core))
        } else if kind == Kind::Ym2610 {
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
    pub fn dispose_ym3526(&self, id: u64) -> Result<(), String> {
        self.dispose_kind(id, Kind::Ym3526)
    }
    pub fn dispose_ym3812(&self, id: u64) -> Result<(), String> {
        self.dispose_kind(id, Kind::Ym3812)
    }
    pub fn dispose_ymf262(&self, id: u64) -> Result<(), String> {
        self.dispose_kind(id, Kind::Ymf262)
    }
    pub fn dispose_rf5c164(&self, id: u64) -> Result<(), String> {
        self.dispose_kind(id, Kind::Rf5c164)
    }
    pub fn dispose_okim6258(&self, id: u64) -> Result<(), String> {
        self.dispose_kind(id, Kind::Okim6258)
    }
    pub fn dispose_y8950(&self, id: u64) -> Result<(), String> {
        self.dispose_kind(id, Kind::Y8950)
    }
    pub fn dispose_ym2413(&self, id: u64) -> Result<(), String> {
        self.dispose_kind(id, Kind::Ym2413)
    }
    pub fn dispose_ay8910(&self, id: u64) -> Result<(), String> {
        self.dispose_kind(id, Kind::Ay8910)
    }
    pub fn dispose_ym2151(&self, id: u64) -> Result<(), String> {
        self.dispose_kind(id, Kind::Ym2151)
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
                            (Core::Ym2151(core), State::Ym2151(state)) => {
                                core.cancel_invalid(&valid, state)
                            }
                            (Core::Ym2413(core), State::Ym2413(state)) => {
                                core.cancel_invalid(&valid, state)
                            }
                            (Core::Ay8910(core), State::Ay8910(state)) => {
                                core.cancel_invalid(&valid, state)
                            }
                            (Core::Y8950(core), State::Y8950(state)) => {
                                core.cancel_invalid(&valid, state)
                            }
                            (Core::Ymf262(core), State::Ymf262(state)) => {
                                core.cancel_invalid(&valid, state)
                            }
                            (Core::Rf5c164(core), State::Rf5c164(state)) => {
                                core.cancel_invalid(&valid, state)
                            }
                            (Core::Okim6258(core), State::Okim6258(state)) => {
                                core.cancel_invalid(&valid, state)
                            }
                            (Core::Ym3812(core), State::Ym3812(state)) => {
                                core.cancel_invalid(&valid, state)
                            }
                            (Core::Ym3526(core), State::Ym3526(state)) => {
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
    pub fn apply_ym2151(
        &mut self,
        instance: &Instance,
        command: &Arc<extra_ym2151::Prepared>,
        owner: Option<Owner>,
    ) {
        unsafe {
            match (instance.callback_core(), &instance.state) {
                (Core::Ym2151(core), State::Ym2151(state)) => core.apply(command, owner, state),
                _ => unreachable!("Additional YM2151 action requires YM2151 instance"),
            }
        }
    }
    pub fn apply_ym2413(
        &mut self,
        instance: &Instance,
        command: &Arc<extra_msx::Prepared>,
        owner: Option<Owner>,
    ) {
        unsafe {
            match (instance.callback_core(), &instance.state) {
                (Core::Ym2413(core), State::Ym2413(state)) => core.apply(command, owner, state),
                _ => unreachable!("Additional YM2413 action requires matching instance"),
            }
        }
    }
    pub fn apply_ay8910(
        &mut self,
        instance: &Instance,
        command: &Arc<extra_msx::Prepared>,
        owner: Option<Owner>,
    ) {
        unsafe {
            match (instance.callback_core(), &instance.state) {
                (Core::Ay8910(core), State::Ay8910(state)) => core.apply(command, owner, state),
                _ => unreachable!("Additional AY8910 action requires matching instance"),
            }
        }
    }
    pub fn apply_ym3526(
        &mut self,
        instance: &Instance,
        command: &Arc<extra_opl::Prepared>,
        owner: Option<Owner>,
    ) {
        unsafe {
            match (instance.callback_core(), &instance.state) {
                (Core::Ym3526(core), State::Ym3526(state)) => core.apply(command, owner, state),
                _ => unreachable!("Additional YM3526 target"),
            }
        }
    }
    pub fn apply_ym3812(
        &mut self,
        instance: &Instance,
        command: &Arc<extra_opl::Prepared>,
        owner: Option<Owner>,
    ) {
        unsafe {
            match (instance.callback_core(), &instance.state) {
                (Core::Ym3812(core), State::Ym3812(state)) => core.apply(command, owner, state),
                _ => unreachable!("Additional YM3812 target"),
            }
        }
    }
    pub fn apply_ymf262(
        &mut self,
        instance: &Instance,
        command: &Arc<extra_opl::Prepared>,
        owner: Option<Owner>,
    ) {
        unsafe {
            match (instance.callback_core(), &instance.state) {
                (Core::Ymf262(core), State::Ymf262(state)) => core.apply(command, owner, state),
                _ => unreachable!("Additional YMF262 target"),
            }
        }
    }
    pub fn apply_rf5c164(
        &mut self,
        instance: &Instance,
        command: &Arc<extra_rf5c164::Prepared>,
        owner: Option<Owner>,
    ) {
        unsafe {
            match (instance.callback_core(), &instance.state) {
                (Core::Rf5c164(core), State::Rf5c164(state)) => core.apply(command, owner, state),
                _ => unreachable!("Additional RF5C164 target"),
            }
        }
    }
    pub fn apply_okim6258(
        &mut self,
        instance: &Instance,
        command: &Arc<extra_okim6258::Prepared>,
        owner: Option<Owner>,
    ) {
        unsafe {
            match (instance.callback_core(), &instance.state) {
                (Core::Okim6258(core), State::Okim6258(state)) => core.apply(command, owner, state),
                _ => unreachable!("Additional OKIM6258 target"),
            }
        }
    }
    pub fn apply_y8950(
        &mut self,
        instance: &Instance,
        command: &Arc<extra_opl::Prepared>,
        owner: Option<Owner>,
    ) {
        unsafe {
            match (instance.callback_core(), &instance.state) {
                (Core::Y8950(core), State::Y8950(state)) => core.apply(command, owner, state),
                _ => unreachable!("Additional Y8950 target"),
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
                    (Core::Ym2151(core), _) => core.mix(pcm),
                    (Core::Ay8910(core), _) => core.mix(pcm),
                    (Core::Y8950(core), _) => core.mix(pcm),
                    (Core::Ymf262(core), _) => core.mix(pcm),
                    (Core::Rf5c164(core), _) => core.mix(pcm),
                    (Core::Okim6258(core), _) => core.mix(pcm),
                    (Core::Ym3812(core), _) => core.mix(pcm),
                    (Core::Ym3526(core), _) => core.mix(pcm),
                    (Core::Ym2413(core), _) => core.mix(pcm),
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
