//! Independently allocated OPLL and AY cores. Shared lease and retirement rules
//! do not instantiate the primary MSX pair or alter its implementation.
//! Allocation and reclamation stay on the control side of the event queue.
use super::{Owner, CAPACITY, MAX_SAFE_INTEGER, OWNERS};
use crossbeam_queue::ArrayQueue;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    ffi::c_void,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering},
        Arc, Mutex,
    },
};
pub const OPLL_CLOCK: u32 = 3_579_545;
pub const AY_CLOCK: u32 = 1_789_773;
const VOICES: usize = 14;
const PENDING_BYTES: usize = 4 * 1024 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chip {
    Ym2413,
    Ay8910,
}
impl Chip {
    fn clock(self) -> u32 {
        if self == Self::Ym2413 {
            OPLL_CLOCK
        } else {
            AY_CLOCK
        }
    }
    fn channels(self) -> usize {
        if self == Self::Ym2413 {
            14
        } else {
            3
        }
    }
    fn registers(self) -> usize {
        if self == Self::Ym2413 {
            64
        } else {
            16
        }
    }
    fn name(self) -> &'static str {
        if self == Self::Ym2413 {
            "YM2413"
        } else {
            "AY8910"
        }
    }
    fn physical(self, ch: usize) -> usize {
        if self == Self::Ym2413 && ch >= 9 {
            [6, 7, 8, 8, 7][ch - 9]
        } else {
            ch
        }
    }
}
#[repr(C)]
struct OpllState {
    registers: [u8; 64],
    clock: u32,
    status: u8,
    irq: u8,
}
impl Default for OpllState {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}
#[repr(C)]
#[derive(Default)]
struct AyState {
    registers: [u8; 16],
    clock: u32,
    levels: [u8; 3],
}
struct CoreState {
    registers: [u8; 64],
    clock: u32,
    levels: [u8; 3],
    status: u8,
    irq: u8,
}
extern "C" {
    fn native_ym2413_create(rate: u32) -> *mut c_void;
    fn native_ym2413_destroy(p: *mut c_void);
    fn native_ym2413_reset(p: *mut c_void);
    fn native_ym2413_stop(p: *mut c_void);
    fn native_ym2413_write(p: *mut c_void, reg: u8, value: u8);
    fn native_ym2413_key_off(p: *mut c_void, voice: u8);
    fn native_ym2413_preview_key_off(p: *mut c_void, voice: u8);
    fn native_ym2413_audition_key_on(p: *mut c_void, voice: u8);
    fn native_ym2413_sample(p: *mut c_void, l: *mut f32, r: *mut f32);
    fn native_ym2413_set_clock(p: *mut c_void, hz: u32) -> i32;
    fn native_ym2413_get_state(p: *mut c_void, state: *mut OpllState);
    fn native_ay8910_create(rate: u32) -> *mut c_void;
    fn native_ay8910_destroy(p: *mut c_void);
    fn native_ay8910_reset(p: *mut c_void);
    fn native_ay8910_stop(p: *mut c_void);
    fn native_ay8910_write(p: *mut c_void, reg: u8, value: u8);
    fn native_ay8910_key_off(p: *mut c_void, voice: u8);
    fn native_ay8910_preview_key_off(p: *mut c_void, voice: u8);
    fn native_ay8910_all_notes_off(p: *mut c_void);
    fn native_ay8910_audition_key_on(p: *mut c_void, voice: u8);
    fn native_ay8910_set_retained_level(p: *mut c_void, ch: u8, level: u8);
    fn native_ay8910_sample(p: *mut c_void, l: *mut f32, r: *mut f32);
    fn native_ay8910_set_clock(p: *mut c_void, hz: u32) -> i32;
    fn native_ay8910_get_state(p: *mut c_void, state: *mut AyState);
}
#[derive(Clone, Debug, Deserialize)]
pub struct Message {
    #[serde(default)]
    pub id: Option<u64>,
    #[serde(default)]
    pub audition: bool,
    #[serde(default)]
    pub group: Option<u64>,
    #[serde(flatten)]
    pub method: Method,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "method", rename_all = "kebab-case")]
pub enum Method {
    Write {
        register: u8,
        value: u8,
        #[serde(default, rename = "intentMask")]
        intent_mask: Option<u8>,
    },
    KeyOff {
        channel: u8,
    },
    AuditionKeyOn {
        channel: u8,
    },
    SetRetainedLevel {
        channel: u8,
        level: u8,
    },
    Clock {
        hz: u32,
    },
    Mix {
        volume: f32,
        pan: f32,
    },
    Reset,
    StopVoices,
    AllNotesOff,
    CancelMainOwner {
        #[serde(rename = "targetGroup")]
        target_group: u64,
    },
    Batch {
        commands: Vec<Op>,
        channel: u8,
        voice: Option<u64>,
        action: LeaseAction,
    },
}
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(tag = "method", rename_all = "kebab-case")]
pub enum Op {
    Write {
        register: u8,
        value: u8,
        #[serde(default, rename = "intentMask")]
        intent_mask: Option<u8>,
    },
    KeyOff {
        channel: u8,
    },
    Clock {
        hz: u32,
    },
    AuditionKeyOn {
        channel: u8,
    },
    SetRetainedLevel {
        channel: u8,
        level: u8,
    },
}
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum LeaseAction {
    On,
    Off,
    Edit,
}
#[derive(Debug)]
pub struct Prepared {
    pub id: Option<u64>,
    chip: Chip,
    audition: bool,
    group: Option<u64>,
    method: Method,
    complete: AtomicBool,
    clock_rejected: AtomicBool,
    outcome_observed: AtomicBool,
}
fn valid_id(id: Option<u64>) -> bool {
    id.is_none_or(|v| v > 0 && v <= MAX_SAFE_INTEGER)
}
fn declared_channel(chip: Chip, op: &Op) -> Option<usize> {
    match *op {
        Op::KeyOff { channel }
        | Op::AuditionKeyOn { channel }
        | Op::SetRetainedLevel { channel, .. } => Some(channel as usize),
        Op::Write { register, .. } if chip == Chip::Ay8910 => match register {
            0..=5 => Some(register as usize / 2),
            8..=10 => Some(register as usize - 8),
            _ => None,
        },
        Op::Write {
            register: 0x10..=0x18,
            ..
        }
        | Op::Write {
            register: 0x20..=0x28,
            ..
        }
        | Op::Write {
            register: 0x30..=0x38,
            ..
        } => {
            if let Op::Write { register, .. } = *op {
                Some((register & 15) as usize)
            } else {
                None
            }
        }
        _ => None,
    }
}
fn validate_op(chip: Chip, op: &Op) -> Result<(), String> {
    match *op {
        Op::Write {
            intent_mask: Some(mask),
            register,
            ..
        } if chip != Chip::Ym2413 || register != 14 || mask > 31 => {
            Err("Invalid Native YM2413 rhythm intent mask".into())
        }
        Op::Write { register, .. } if register as usize >= chip.registers() => {
            Err(format!("Native {} register is out of range", chip.name()))
        }
        Op::KeyOff { channel } | Op::AuditionKeyOn { channel }
            if channel as usize >= chip.channels() =>
        {
            Err(format!("Native {} channel is out of range", chip.name()))
        }
        Op::SetRetainedLevel { channel, level }
            if chip != Chip::Ay8910 || channel >= 3 || level > 31 =>
        {
            Err("Invalid Native AY8910 retained level".into())
        }
        Op::Clock { hz } if !(100_000..=100_000_000).contains(&hz) => Err(format!(
            "Native {} clock must be 100000–100000000 Hz",
            chip.name()
        )),
        _ => Ok(()),
    }
}
impl Message {
    pub fn prepared_bytes(&self) -> usize {
        std::mem::size_of::<Prepared>()
            + if let Method::Batch { commands, .. } = &self.method {
                commands.len() * std::mem::size_of::<Op>()
            } else {
                0
            }
    }
    pub fn prepare(&self, chip: Chip) -> Result<Arc<Prepared>, String> {
        if !valid_id(self.id) || !valid_id(self.group) {
            return Err(format!("Invalid Native {} command ID", chip.name()));
        }
        match &self.method {
            Method::Write {
                register,
                value,
                intent_mask,
            } => validate_op(
                chip,
                &Op::Write {
                    register: *register,
                    value: *value,
                    intent_mask: *intent_mask,
                },
            )?,
            Method::KeyOff { channel } => validate_op(chip, &Op::KeyOff { channel: *channel })?,
            Method::AuditionKeyOn { channel } => {
                validate_op(chip, &Op::AuditionKeyOn { channel: *channel })?
            }
            Method::SetRetainedLevel { channel, level } => validate_op(
                chip,
                &Op::SetRetainedLevel {
                    channel: *channel,
                    level: *level,
                },
            )?,
            Method::Clock { hz } => validate_op(chip, &Op::Clock { hz: *hz })?,
            Method::Mix { volume, pan }
                if !volume.is_finite()
                    || !(0.0..=4.).contains(volume)
                    || !pan.is_finite()
                    || !(-1.0..=1.).contains(pan) =>
            {
                return Err(format!(
                    "Native {} mixer volume must be 0–4, pan −1–1",
                    chip.name()
                ))
            }
            Method::CancelMainOwner { target_group } if !valid_id(Some(*target_group)) => {
                return Err("Invalid Native simple chip Main owner group".into())
            }
            Method::Batch {
                commands,
                channel,
                voice,
                action,
            } => {
                if *channel as usize >= chip.channels()
                    || commands.is_empty()
                    || commands.len() > 512
                    || !valid_id(*voice)
                    || self.audition && *action != LeaseAction::Edit && voice.is_none()
                {
                    return Err(format!(
                        "Invalid Native {} batch or voice lease",
                        chip.name()
                    ));
                }
                for op in commands {
                    validate_op(chip, op)?;
                    if declared_channel(chip, op).is_some_and(|target| {
                        chip.physical(target) != chip.physical(*channel as usize)
                    }) {
                        return Err(format!(
                            "Native {} batch must target its declared physical channel",
                            chip.name()
                        ));
                    }
                }
            }
            _ => {}
        }
        Ok(Arc::new(Prepared {
            id: self.id,
            chip,
            audition: self.audition,
            group: self.group,
            method: self.method.clone(),
            complete: AtomicBool::new(false),
            clock_rejected: AtomicBool::new(false),
            outcome_observed: AtomicBool::new(false),
        }))
    }
}
impl Prepared {
    pub fn starts(&self) -> bool {
        match &self.method {
            Method::Write { .. } | Method::AuditionKeyOn { .. } => true,
            Method::Batch { action, .. } => *action != LeaseAction::Off,
            _ => false,
        }
    }
    pub fn stops(&self) -> bool {
        matches!(self.method, Method::StopVoices) && self.group.is_none()
    }
    pub fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + if let Method::Batch { commands, .. } = &self.method {
                commands.capacity() * std::mem::size_of::<Op>()
            } else {
                0
            }
    }
    fn bytes(&self) -> usize {
        self.retained_bytes()
    }
    fn affects_layout(&self) -> bool {
        self.chip == Chip::Ym2413
            && match &self.method {
                Method::Write { register: 14, .. } | Method::Reset => true,
                Method::Batch { commands, .. } => commands
                    .iter()
                    .any(|op| matches!(op, Op::Write { register: 14, .. })),
                _ => false,
            }
    }
}
#[derive(Default)]
struct Control {
    prepared: Vec<Arc<Prepared>>,
    rejected_clocks: HashSet<u64>,
}
pub struct Shared {
    chip: Chip,
    revision: AtomicU64,
    layout_revision: AtomicU64,
    layout_progress: AtomicU64,
    registers: [AtomicU8; 64],
    levels: [AtomicU8; 3],
    clock: AtomicU32,
    status: AtomicU8,
    irq: AtomicU8,
    held: AtomicU32,
    rate: AtomicU32,
    control: Mutex<Control>,
    retired: ArrayQueue<Arc<Prepared>>,
    highest_id: AtomicU64,
    cancel_before: AtomicU64,
    owner_cancel_before: [AtomicU64; OWNERS],
}
impl Shared {
    pub fn new(chip: Chip) -> Self {
        Self {
            chip,
            revision: AtomicU64::new(0),
            layout_revision: AtomicU64::new(0),
            layout_progress: AtomicU64::new(0),
            registers: std::array::from_fn(|_| AtomicU8::new(0)),
            levels: std::array::from_fn(|_| AtomicU8::new(0)),
            clock: AtomicU32::new(chip.clock()),
            status: AtomicU8::new(0),
            irq: AtomicU8::new(0),
            held: AtomicU32::new(0),
            rate: AtomicU32::new(0),
            control: Mutex::new(Control::default()),
            retired: ArrayQueue::new(CAPACITY),
            highest_id: AtomicU64::new(0),
            cancel_before: AtomicU64::new(0),
            owner_cancel_before: std::array::from_fn(|_| AtomicU64::new(0)),
        }
    }
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChipStatus {
    pub clock: u32,
    pub registers: Vec<u8>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub levels: Vec<u8>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub gates: Vec<bool>,
    pub status: u8,
    pub irq: bool,
    pub held: Vec<u8>,
    pub pending_commands: Vec<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout_revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout_pending: Option<bool>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Ym2413Status {
    pub ym2413: ChipStatus,
    pub frame: u64,
    pub sample_rate: u32,
    pub revision: u64,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Ay8910Status {
    pub ay8910: ChipStatus,
    pub frame: u64,
    pub sample_rate: u32,
    pub revision: u64,
}
pub struct Status {
    pub chip: ChipStatus,
    pub frame: u64,
    pub sample_rate: u32,
    pub revision: u64,
}
impl Shared {
    fn clock_held_error(&self) -> &'static str {
        if self.chip == Chip::Ym2413 {
            "Release the manually held additional YM2413 voices before changing the clock"
        } else {
            "Release the manually held additional AY8910 voices before changing the clock"
        }
    }
    pub fn prepare_message(&self, message: &Message) -> Result<Arc<Prepared>, String> {
        message.prepare(self.chip)
    }
    pub fn ym2413_status(&self, frame: u64) -> Result<Ym2413Status, String> {
        let s = self.status(frame)?;
        Ok(Ym2413Status {
            ym2413: s.chip,
            frame: s.frame,
            sample_rate: s.sample_rate,
            revision: s.revision,
        })
    }
    pub fn ay8910_status(&self, frame: u64) -> Result<Ay8910Status, String> {
        let s = self.status(frame)?;
        Ok(Ay8910Status {
            ay8910: s.chip,
            frame: s.frame,
            sample_rate: s.sample_rate,
            revision: s.revision,
        })
    }
    pub fn clock(&self) -> u32 {
        self.clock.load(Ordering::Acquire)
    }
    pub fn valid_event(&self, _: &Prepared, sequence: u64, owner: Option<Owner>) -> bool {
        sequence >= self.cancel_before.load(Ordering::Acquire)
            && owner.is_none_or(|o| {
                sequence >= self.owner_cancel_before[(o.owner - 1) as usize].load(Ordering::Acquire)
            })
    }
    pub fn cancel_before(&self, _: &Prepared, sequence: u64, owner: Option<Owner>) {
        if let Some(o) = owner {
            self.owner_cancel_before[(o.owner - 1) as usize].fetch_max(sequence, Ordering::AcqRel);
        } else {
            self.cancel_before.fetch_max(sequence, Ordering::AcqRel);
        }
    }
    fn collect(&self, c: &mut Control) {
        let mut retired = HashSet::new();
        while let Some(cmd) = self.retired.pop() {
            retired.insert(Arc::as_ptr(&cmd) as usize);
        }
        c.prepared.retain(|cmd| {
            let rejected = cmd.id.is_some() && cmd.clock_rejected.load(Ordering::Acquire);
            let observed = cmd.outcome_observed.load(Ordering::Acquire);
            rejected && !observed
                || !retired.contains(&(Arc::as_ptr(cmd) as usize)) && !(rejected && observed)
        });
        if c.prepared.capacity() > 64.max(c.prepared.len().saturating_mul(2)) {
            c.prepared.shrink_to_fit();
        }
    }
    pub fn collect_retired(&self) {
        if let Ok(mut c) = self.control.lock() {
            self.collect(&mut c)
        }
    }
    pub fn retain(&self, commands: &[(Arc<Prepared>, Option<Owner>)]) -> Result<(), String> {
        let mut c = self
            .control
            .lock()
            .map_err(|_| "Native additional simple chip control state unavailable")?;
        self.collect(&mut c);
        if commands.iter().any(|(c, _)| c.chip != self.chip) {
            return Err("Additional command chip type mismatch".into());
        }
        if self.held.load(Ordering::Acquire) != 0
            && commands
                .iter()
                .any(|(v, _)| !v.audition && matches!(v.method, Method::Clock { .. }))
        {
            return Err(self.clock_held_error().into());
        }
        if c.prepared.len() + commands.len() > CAPACITY
            || c.prepared.iter().map(|v| v.bytes()).sum::<usize>()
                + commands.iter().map(|(v, _)| v.bytes()).sum::<usize>()
                > PENDING_BYTES
        {
            return Err(
                "Native additional simple chip pending command limit reached (4 MiB)".into(),
            );
        }
        let mut ids = HashSet::new();
        for id in c
            .prepared
            .iter()
            .filter_map(|v| v.id)
            .chain(commands.iter().filter_map(|(v, _)| v.id))
        {
            if !ids.insert(id) {
                return Err("Duplicate pending Native additional simple chip command ID".into());
            }
        }
        for (cmd, _) in commands {
            if let Some(id) = cmd.id {
                self.highest_id.fetch_max(id, Ordering::AcqRel);
            }
            c.prepared.push(cmd.clone());
        }
        Ok(())
    }
    pub fn wait_applied(&self, id: u64) -> Result<(), String> {
        if id == 0 || id > self.highest_id.load(Ordering::Acquire) {
            return Err("Unknown Native additional simple chip command ID".into());
        }
        let cmd = {
            let mut control = self
                .control
                .lock()
                .map_err(|_| "Native additional simple chip control state unavailable")?;
            if control.rejected_clocks.remove(&id) {
                return Err(self.clock_held_error().into());
            }
            control.prepared.iter().find(|v| v.id == Some(id)).cloned()
        };
        if let Some(cmd) = cmd {
            let start = std::time::Instant::now();
            while !cmd.complete.load(Ordering::Acquire) {
                // A scheduled Worker clock change awaits the same applied
                // command barrier as the authoring client, not an early ACK.
                let deadline = if matches!(cmd.method, Method::Clock { .. } | Method::Reset)
                    || self.chip == Chip::Ym2413
                        && matches!(cmd.method, Method::Write { register: 14, .. })
                {
                    // Applied-layout observations share the Worker lookahead
                    // barrier: the old layout remains authoritative until the
                    // timestamped rhythm-mode command applies or is canceled.
                    5000
                } else {
                    100
                };
                if start.elapsed() > std::time::Duration::from_millis(deadline) {
                    return Err(format!(
                        "Native additional simple chip command was not applied within {deadline} ms"
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            cmd.outcome_observed.store(true, Ordering::Release);
            if cmd.clock_rejected.load(Ordering::Acquire) {
                self.control
                    .lock()
                    .map_err(|_| "Native additional simple chip control state unavailable")?
                    .rejected_clocks
                    .remove(&id);
                return Err(self.clock_held_error().into());
            }
        }
        Ok(())
    }
    /// A read-only long poll for the next applied mode edge or layout-command
    /// completion. Unlike a particular command ID, this observes ASAP keyboard
    /// changes that overtake older timestamped Worker writes.
    pub fn wait_layout(&self, after: u64) -> Result<(), String> {
        if self.chip != Chip::Ym2413 || after > MAX_SAFE_INTEGER {
            return Err("Invalid Native YM2413 layout revision".into());
        }
        let progress = self.layout_progress.load(Ordering::Acquire);
        if self.layout_revision.load(Ordering::Acquire) != after {
            return Ok(());
        }
        {
            let control = self
                .control
                .lock()
                .map_err(|_| "Native additional simple chip control unavailable")?;
            if !control.prepared.iter().any(|command| {
                command.affects_layout() && !command.complete.load(Ordering::Acquire)
            }) {
                return Ok(());
            }
        }
        let start = std::time::Instant::now();
        while self.layout_revision.load(Ordering::Acquire) == after
            && self.layout_progress.load(Ordering::Acquire) == progress
        {
            if start.elapsed() > std::time::Duration::from_millis(5000) {
                return Err(
                    "Native YM2413 layout observation was not applied within 5000 ms".into(),
                );
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        Ok(())
    }
    fn complete(&self, command: &Prepared) {
        if !command.complete.swap(true, Ordering::AcqRel) && command.affects_layout() {
            self.layout_progress.fetch_add(1, Ordering::Release);
        }
    }
    pub fn rollback(&self, cmd: Arc<Prepared>) {
        self.retire(cmd)
    }
    pub fn retire(&self, cmd: Arc<Prepared>) {
        self.complete(&cmd);
        self.retired
            .push(cmd)
            .unwrap_or_else(|_| unreachable!("Bounded OPLL/AY command retirement"));
    }
    fn publish(&self, r: &Renderer, command: Option<&Prepared>) {
        let state = r.state();
        self.revision.fetch_add(1, Ordering::AcqRel);
        if self.chip == Chip::Ym2413
            && (self.registers[14].load(Ordering::Relaxed) ^ state.registers[14]) & 32 != 0
        {
            self.layout_revision.fetch_add(1, Ordering::Relaxed);
        }
        for (out, value) in self.registers.iter().zip(state.registers) {
            out.store(value, Ordering::Relaxed)
        }
        for (out, value) in self.levels.iter().zip(state.levels) {
            out.store(value, Ordering::Relaxed)
        }
        self.clock.store(state.clock, Ordering::Relaxed);
        self.status.store(state.status, Ordering::Relaxed);
        self.irq.store(state.irq, Ordering::Relaxed);
        self.held.store(
            r.owners
                .iter()
                .enumerate()
                .filter(|(_, v)| v.is_some_and(|l| l.audition))
                .fold(0, |mask, (ch, _)| mask | (1 << ch)),
            Ordering::Relaxed,
        );
        self.rate.store(r.rate, Ordering::Relaxed);
        if let Some(c) = command {
            self.complete(c)
        }
        self.revision.fetch_add(1, Ordering::Release);
    }
    fn refresh_running_status(&self, r: &Renderer) {
        let s = r.state();
        if self.status.load(Ordering::Relaxed) != s.status
            || self.irq.load(Ordering::Relaxed) != s.irq
            || self
                .registers
                .iter()
                .zip(s.registers)
                .any(|(out, value)| out.load(Ordering::Relaxed) != value)
        {
            self.publish(r, None)
        }
    }
    pub fn status(&self, frame: u64) -> Result<Status, String> {
        let mut control = self
            .control
            .lock()
            .map_err(|_| "Native additional simple chip control unavailable")?;
        self.collect(&mut control);
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() > std::time::Duration::from_millis(50) {
                return Err(
                    "Native additional simple chip state changed while reading; retry".into(),
                );
            }
            let before = self.revision.load(Ordering::Acquire);
            if before & 1 != 0 {
                std::thread::yield_now();
                continue;
            }
            let held = self.held.load(Ordering::Relaxed);
            let out = Status {
                chip: ChipStatus {
                    clock: self.clock(),
                    registers: self.registers[..self.chip.registers()]
                        .iter()
                        .map(|v| v.load(Ordering::Relaxed))
                        .collect(),
                    levels: if self.chip == Chip::Ay8910 {
                        self.levels
                            .iter()
                            .map(|v| v.load(Ordering::Relaxed))
                            .collect()
                    } else {
                        Vec::new()
                    },
                    gates: if self.chip == Chip::Ay8910 {
                        (8..11)
                            .map(|reg| self.registers[reg].load(Ordering::Relaxed) & 31 != 0)
                            .collect()
                    } else {
                        Vec::new()
                    },
                    status: self.status.load(Ordering::Relaxed),
                    irq: self.irq.load(Ordering::Relaxed) != 0,
                    held: (0..self.chip.channels())
                        .filter(|ch| held & (1 << ch) != 0)
                        .map(|ch| ch as u8)
                        .collect(),
                    pending_commands: control
                        .prepared
                        .iter()
                        .filter(|v| !v.complete.load(Ordering::Relaxed))
                        .filter_map(|v| v.id)
                        .collect(),
                    layout_revision: (self.chip == Chip::Ym2413)
                        .then(|| self.layout_revision.load(Ordering::Relaxed)),
                    layout_pending: (self.chip == Chip::Ym2413).then(|| {
                        control.prepared.iter().any(|command| {
                            command.affects_layout() && !command.complete.load(Ordering::Relaxed)
                        })
                    }),
                },
                frame,
                sample_rate: self.rate.load(Ordering::Relaxed),
                revision: before / 2,
            };
            std::sync::atomic::fence(Ordering::Acquire);
            if self.revision.load(Ordering::Relaxed) == before {
                return Ok(out);
            }
            std::thread::yield_now();
        }
    }
}
#[derive(Clone, Copy)]
struct Lease {
    group: Option<u64>,
    owner: Option<Owner>,
    voice: Option<u64>,
    audition: bool,
}
pub struct Renderer {
    chip: Chip,
    current_group: Option<u64>,
    preview_release: bool,
    ptr: *mut c_void,
    rate: u32,
    owners: [Option<Lease>; VOICES],
    gains: [f32; 2],
    targets: [f32; 2],
    ramp: u32,
}
unsafe impl Send for Renderer {}
impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe {
            if self.chip == Chip::Ym2413 {
                native_ym2413_destroy(self.ptr)
            } else {
                native_ay8910_destroy(self.ptr)
            }
        }
    }
}
impl Renderer {
    pub fn new(rate: u32, chip: Chip) -> Result<Self, String> {
        let ptr = unsafe {
            if chip == Chip::Ym2413 {
                native_ym2413_create(rate)
            } else {
                native_ay8910_create(rate)
            }
        };
        if ptr.is_null() {
            return Err(format!("Could not allocate Native {} core", chip.name()));
        }
        Ok(Self {
            chip,
            current_group: None,
            preview_release: false,
            ptr,
            rate,
            owners: [None; VOICES],
            gains: [1.; 2],
            targets: [1.; 2],
            ramp: 0,
        })
    }
    fn state(&self) -> CoreState {
        unsafe {
            if self.chip == Chip::Ym2413 {
                let mut s: OpllState = std::mem::zeroed();
                native_ym2413_get_state(self.ptr, &mut s);
                CoreState {
                    registers: s.registers,
                    clock: s.clock,
                    levels: [0; 3],
                    status: s.status,
                    irq: s.irq,
                }
            } else {
                let mut s = AyState::default();
                native_ay8910_get_state(self.ptr, &mut s);
                let mut r = [0; 64];
                r[..16].copy_from_slice(&s.registers);
                CoreState {
                    registers: r,
                    clock: s.clock,
                    levels: s.levels,
                    status: 0,
                    irq: 0,
                }
            }
        }
    }
    fn has_manual(&self) -> bool {
        self.owners.iter().flatten().any(|l| l.audition)
    }
    fn protected(&self, ch: usize, voice: Option<u64>, audition: bool) -> bool {
        self.owners[ch]
            .is_some_and(|l| l.audition && (!audition || voice.is_some() && l.voice != voice))
    }
    fn alias_manual(&self, ch: usize, voice: Option<u64>, audition: bool) -> bool {
        (0..self.chip.channels()).any(|other| {
            self.chip.physical(other) == self.chip.physical(ch)
                && self.protected(other, voice, audition)
        })
    }
    fn alias_conflict(&self, ch: usize) -> bool {
        self.chip == Chip::Ym2413
            && (0..14).any(|other| {
                other != ch
                    && (other < 9) != (ch < 9)
                    && self.chip.physical(other) == self.chip.physical(ch)
                    && self.owners[other].is_some_and(|l| l.audition)
            })
    }
    unsafe fn raw_write(&self, reg: u8, value: u8) {
        if self.chip == Chip::Ym2413 {
            native_ym2413_write(self.ptr, reg, value)
        } else {
            native_ay8910_write(self.ptr, reg, value)
        }
    }
    fn release(&mut self, ch: usize) {
        unsafe {
            if self.chip == Chip::Ym2413 {
                native_ym2413_key_off(self.ptr, ch as u8)
            } else {
                native_ay8910_key_off(self.ptr, ch as u8)
            }
        }
        self.owners[ch] = None;
    }
    fn lease(&self, owner: Option<Owner>, voice: Option<u64>, audition: bool) -> Lease {
        Lease {
            group: self.current_group,
            owner,
            voice,
            audition,
        }
    }
    fn foreign(&self, l: Lease, owner: Option<Owner>, voice: Option<u64>, audition: bool) -> bool {
        owner.is_some() && l.owner != owner
            || self.current_group.is_some() && l.group != self.current_group
            || voice.is_some() && l.voice != voice && (audition || l.audition)
            || l.audition && !audition
    }
    fn op(
        &mut self,
        op: &Op,
        owner: Option<Owner>,
        voice: Option<u64>,
        audition: bool,
        preserve: bool,
        declared: Option<usize>,
    ) {
        let state = self.state();
        let channel = declared_channel(self.chip, op);
        // OPLL volume/frequency registers alias the rhythm operators and are
        // filtered below rather than rejected by the inactive FM lease.
        if channel.is_some_and(|ch| self.protected(ch, voice, audition)) {
            return;
        }
        match *op {
            Op::Clock { hz } => {
                if !audition && self.has_manual() {
                    return;
                }
                unsafe {
                    if self.chip == Chip::Ym2413 {
                        native_ym2413_set_clock(self.ptr, hz);
                    } else {
                        native_ay8910_set_clock(self.ptr, hz);
                    }
                }
            }
            Op::SetRetainedLevel { channel, level } => {
                if self.owners[channel as usize]
                    .is_some_and(|l| self.foreign(l, owner, voice, audition))
                {
                    return;
                }
                unsafe { native_ay8910_set_retained_level(self.ptr, channel, level) }
            }
            Op::KeyOff { channel } => {
                let ch = channel as usize;
                if self.owners[ch].is_some_and(|l| self.foreign(l, owner, voice, audition)) {
                    return;
                }
                if self.preview_release {
                    unsafe {
                        if self.chip == Chip::Ym2413 {
                            native_ym2413_preview_key_off(self.ptr, ch as u8)
                        } else {
                            native_ay8910_preview_key_off(self.ptr, ch as u8)
                        }
                    }
                    self.owners[ch] = None;
                } else {
                    self.release(ch)
                }
            }
            Op::AuditionKeyOn { channel } => {
                if self.alias_conflict(channel as usize) {
                    return;
                }
                unsafe {
                    if self.chip == Chip::Ym2413 {
                        native_ym2413_audition_key_on(self.ptr, channel)
                    } else {
                        native_ay8910_audition_key_on(self.ptr, channel)
                    }
                }
                if !preserve {
                    self.owners[channel as usize] = Some(self.lease(owner, voice, audition))
                }
            }
            Op::Write {
                register,
                mut value,
                intent_mask,
            } => {
                if self.chip == Chip::Ay8910 {
                    if register == 6
                        && !audition
                        && (0..3).any(|ch| {
                            self.protected(ch, voice, false)
                                && state.registers[7] & (1 << (ch + 3)) == 0
                        })
                    {
                        return;
                    }
                    if (11..=13).contains(&register)
                        && !audition
                        && (0..3).any(|ch| {
                            self.protected(ch, voice, false) && state.registers[8 + ch] & 16 != 0
                        })
                    {
                        return;
                    }
                    if register == 7 {
                        for ch in 0..3 {
                            let mask = (1 << ch) | (1 << (ch + 3));
                            let old = state.registers[7] & mask;
                            let new = value & mask;
                            if self.protected(ch, voice, audition)
                                || declared.is_some_and(|d| d != ch)
                                || old != mask
                                    && new == mask
                                    && self.owners[ch]
                                        .is_some_and(|l| self.foreign(l, owner, voice, audition))
                            {
                                value = (value & !mask) | old
                            } else if !preserve
                                && old == mask
                                && new != mask
                                && state.registers[8 + ch] & 31 != 0
                            {
                                self.owners[ch] = Some(self.lease(owner, voice, audition))
                            }
                        }
                    }
                    if (8..=10).contains(&register) {
                        let ch = (register - 8) as usize;
                        let on = value & 31 != 0;
                        if !on
                            && self.owners[ch]
                                .is_some_and(|l| self.foreign(l, owner, voice, audition))
                        {
                            return;
                        }
                        if !preserve {
                            self.owners[ch] = if on {
                                Some(self.lease(owner, voice, audition))
                            } else {
                                None
                            }
                        }
                    }
                } else {
                    if register <= 7
                        && !audition
                        && (0..9).any(|ch| {
                            self.protected(ch, voice, false) && state.registers[0x30 + ch] >> 4 == 0
                        })
                    {
                        return;
                    }
                    if (0x10..=0x18).contains(&register) {
                        let ch = (register & 15) as usize;
                        if self.alias_manual(ch, voice, audition) {
                            return;
                        }
                    }
                    if (0x20..=0x28).contains(&register) {
                        let ch = (register & 15) as usize;
                        let old = state.registers[register as usize];
                        if self.alias_manual(ch, voice, audition) {
                            value = (value & !0x2f) | (old & 0x2f)
                        }
                        let on = value & 16 != 0;
                        let mut may_claim = true;
                        if !on
                            && self.owners[ch]
                                .is_some_and(|l| self.foreign(l, owner, voice, audition))
                        {
                            value = (value & !16) | (old & 16);
                            may_claim = false;
                        }
                        if !preserve && may_claim {
                            if value & 16 != 0 {
                                self.owners[ch] = Some(self.lease(owner, voice, audition))
                            } else {
                                self.owners[ch] = None
                            }
                        }
                    }
                    if register == 14 {
                        let old = state.registers[14];
                        let changing = (old ^ value) & 32 != 0;
                        if changing
                            && (6..14).any(|ch| {
                                self.protected(ch, voice, audition)
                                    || self.owners[ch]
                                        .is_some_and(|l| self.foreign(l, owner, voice, audition))
                            })
                        {
                            value = (value & !32) | (old & 32)
                        }
                        for index in 0..5 {
                            let ch = 9 + index;
                            let bit = 16 >> index;
                            let before = old & bit != 0;
                            let after = value & bit != 0;
                            if self.protected(ch, voice, audition)
                                || declared.is_some_and(|d| {
                                    self.chip.physical(d) != self.chip.physical(ch)
                                })
                                || before
                                    && !after
                                    && self.owners[ch]
                                        .is_some_and(|l| self.foreign(l, owner, voice, audition))
                            {
                                value = (value & !bit) | (old & bit)
                            } else if !preserve {
                                if after
                                    && (!before || intent_mask.is_some_and(|mask| mask & bit != 0))
                                {
                                    self.owners[ch] = Some(self.lease(owner, voice, audition))
                                } else if before && !after {
                                    self.owners[ch] = None
                                }
                            }
                        }
                    }
                    if (0x36..=0x38).contains(&register) && state.registers[14] & 32 != 0 {
                        let nibbles: &[(usize, u8)] = match register {
                            0x36 => &[(9, 0x0f)],
                            0x37 => &[(10, 0x0f), (13, 0xf0)],
                            _ => &[(12, 0x0f), (11, 0xf0)],
                        };
                        for &(ch, mask) in nibbles {
                            if self.protected(ch, voice, audition)
                                || declared.is_some_and(|d| d >= 9 && d != ch)
                                || self.owners[ch]
                                    .is_some_and(|l| self.foreign(l, owner, voice, audition))
                            {
                                value =
                                    (value & !mask) | (state.registers[register as usize] & mask)
                            }
                        }
                    }
                }
                unsafe { self.raw_write(register, value) }
            }
        }
    }
    pub fn apply(&mut self, c: &Arc<Prepared>, owner: Option<Owner>, shared: &Shared) {
        if !c.audition
            && self.has_manual()
            && match &c.method {
                Method::Clock { .. } => true,
                Method::Batch { commands, .. } => {
                    commands.iter().any(|op| matches!(op, Op::Clock { .. }))
                }
                _ => false,
            }
        {
            c.clock_rejected.store(true, Ordering::Release);
            shared.publish(self, Some(c));
            return;
        }
        self.current_group = c.group;
        match &c.method {
            Method::Write {
                register,
                value,
                intent_mask,
            } => self.op(
                &Op::Write {
                    register: *register,
                    value: *value,
                    intent_mask: *intent_mask,
                },
                owner,
                None,
                c.audition,
                false,
                None,
            ),
            Method::KeyOff { channel } => self.op(
                &Op::KeyOff { channel: *channel },
                owner,
                None,
                c.audition,
                false,
                None,
            ),
            Method::Clock { hz } => {
                self.op(&Op::Clock { hz: *hz }, owner, None, c.audition, false, None)
            }
            Method::AuditionKeyOn { channel } => self.op(
                &Op::AuditionKeyOn { channel: *channel },
                owner,
                None,
                c.audition,
                false,
                None,
            ),
            Method::SetRetainedLevel { channel, level } => self.op(
                &Op::SetRetainedLevel {
                    channel: *channel,
                    level: *level,
                },
                owner,
                None,
                c.audition,
                false,
                None,
            ),
            Method::Mix { volume, pan } => {
                self.targets = [
                    *volume * if *pan > 0. { 1. - *pan } else { 1. },
                    *volume * if *pan < 0. { 1. + *pan } else { 1. },
                ];
                self.ramp = (self.rate / 200).max(1)
            }
            Method::Reset => {
                if c.audition || !self.has_manual() {
                    unsafe {
                        if self.chip == Chip::Ym2413 {
                            native_ym2413_reset(self.ptr)
                        } else {
                            native_ay8910_reset(self.ptr)
                        }
                    }
                    self.owners.fill(None);
                }
            }
            Method::CancelMainOwner { target_group } => {
                for ch in 0..self.chip.channels() {
                    if self.owners[ch]
                        .is_some_and(|l| !l.audition && l.group == Some(*target_group))
                    {
                        self.release(ch)
                    }
                }
            }
            Method::StopVoices | Method::AllNotesOff => {
                if owner.is_none() && c.group.is_none() && (c.audition || !self.has_manual()) {
                    unsafe {
                        if self.chip == Chip::Ym2413 {
                            native_ym2413_stop(self.ptr)
                        } else {
                            if matches!(c.method, Method::AllNotesOff) {
                                native_ay8910_all_notes_off(self.ptr)
                            } else {
                                native_ay8910_stop(self.ptr)
                            }
                        }
                    }
                    self.owners.fill(None)
                } else {
                    for ch in 0..self.chip.channels() {
                        if self.owners[ch].is_some_and(|l| {
                            (owner.is_none() || l.owner == owner)
                                && (c.group.is_none() || l.group == c.group)
                                && (!l.audition || c.audition)
                        }) {
                            self.release(ch)
                        }
                    }
                }
            }
            Method::Batch {
                commands,
                channel,
                voice,
                action,
            } => {
                let ch = *channel as usize;
                let current = self.owners[ch];
                let allowed = match action {
                    LeaseAction::On => {
                        !self.alias_conflict(ch)
                            && (c.audition || current.is_none_or(|l| !l.audition))
                    }
                    LeaseAction::Off => current.is_some_and(|l| {
                        l.owner == owner
                            && (c.group.is_none() || l.group == c.group)
                            && (voice.is_none() || l.voice == *voice)
                            && (!l.audition || c.audition)
                    }),
                    LeaseAction::Edit => current.is_none_or(|l| {
                        !l.audition || c.audition && (voice.is_none() || l.voice == *voice)
                    }),
                };
                if allowed {
                    self.preview_release = c.audition && *action == LeaseAction::On;
                    if *action == LeaseAction::On {
                        self.owners[ch] = Some(Lease {
                            group: self.current_group,
                            owner,
                            voice: *voice,
                            audition: c.audition,
                        })
                    }
                    for op in commands {
                        self.op(
                            op,
                            owner,
                            *voice,
                            c.audition,
                            *action == LeaseAction::Edit,
                            if *action == LeaseAction::Edit
                                && self.chip == Chip::Ym2413
                                && matches!(
                                    op,
                                    Op::Write {
                                        register: 0..=7 | 14,
                                        ..
                                    }
                                )
                            {
                                None
                            } else {
                                Some(ch)
                            },
                        )
                    }
                    if *action == LeaseAction::On {
                        self.owners[ch] = Some(Lease {
                            group: self.current_group,
                            owner,
                            voice: *voice,
                            audition: c.audition,
                        })
                    } else if *action == LeaseAction::Off {
                        self.owners[ch] = None
                    }
                    self.preview_release = false;
                }
            }
        }
        shared.publish(self, Some(c));
        self.current_group = None;
    }
    pub fn publish(&self, s: &Shared) {
        s.publish(self, None)
    }
    pub fn cancel_invalid(&mut self, valid: impl Fn(Option<Owner>) -> bool, s: &Shared) {
        let mut changed = false;
        for ch in 0..self.chip.channels() {
            if self.owners[ch].is_some_and(|l| !valid(l.owner)) {
                self.release(ch);
                changed = true
            }
        }
        if changed {
            s.publish(self, None)
        } else {
            s.refresh_running_status(self);
        }
    }
    pub fn mix(&mut self, pcm: &mut [f32; 2]) {
        let (mut l, mut r) = (0., 0.);
        unsafe {
            if self.chip == Chip::Ym2413 {
                native_ym2413_sample(self.ptr, &mut l, &mut r)
            } else {
                native_ay8910_sample(self.ptr, &mut l, &mut r)
            }
        }
        if self.ramp > 0 {
            for ch in 0..2 {
                self.gains[ch] += (self.targets[ch] - self.gains[ch]) / self.ramp as f32
            }
            self.ramp -= 1
        }
        pcm[0] += l * self.gains[0];
        pcm[1] += r * self.gains[1];
    }
}
