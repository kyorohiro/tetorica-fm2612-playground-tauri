//! Standalone additional OPL/OPL2/MSX-Audio cores, with no primary DOS/MSX devices.
//! Control-side ownership retains commands and cores through callback retirement.
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
pub const CLOCK: u32 = 3_579_545;
const VOICES: usize = 23;
const PENDING_BYTES: usize = 4 * 1024 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chip {
    Ym3526,
    Ym3812,
    Ymf262,
    Y8950,
}
impl Chip {
    pub fn name(self) -> &'static str {
        match self {
            Self::Ym3526 => "ym3526",
            Self::Ym3812 => "ym3812",
            Self::Ymf262 => "ymf262",
            Self::Y8950 => "y8950",
        }
    }
    fn code(self) -> u8 {
        match self {
            Self::Ym3526 => 0,
            Self::Ym3812 => 1,
            Self::Ymf262 => 2,
            Self::Y8950 => 3,
        }
    }
    fn clock(self) -> u32 {
        if self == Self::Ymf262 {
            14_318_180
        } else {
            CLOCK
        }
    }
    fn channels(self) -> usize {
        if self == Self::Ymf262 {
            18
        } else {
            9
        }
    }
    fn registers(self) -> usize {
        if self == Self::Ymf262 {
            512
        } else {
            256
        }
    }
    fn valid_voice(self, ch: usize) -> bool {
        ch < self.channels() || (18..23).contains(&ch)
    }
    fn physical(self, ch: usize) -> usize {
        if ch >= 18 {
            [6, 7, 8, 8, 7][ch - 18]
        } else {
            ch
        }
    }
}
#[repr(C)]
struct CoreState {
    registers: [u8; 512],
    clock: u32,
    kind: u8,
    channels: u8,
    status: u8,
    irq: u8,
}
impl Default for CoreState {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}
extern "C" {
    fn native_extra_opl_create(rate: u32, kind: u8) -> *mut c_void;
    fn native_extra_opl_destroy(p: *mut c_void);
    fn native_extra_opl_reset(p: *mut c_void);
    fn native_extra_opl_stop(p: *mut c_void);
    fn native_extra_opl_all_notes_off(p: *mut c_void);
    fn native_extra_opl_write(p: *mut c_void, register: u16, value: u8);
    fn native_extra_opl_key_off_fm(p: *mut c_void, ch: u8);
    fn native_extra_opl_source_preview_key_off(p: *mut c_void, ch: u8);
    fn native_extra_opl_source_audition_key_on(p: *mut c_void, ch: u8);
    fn native_extra_opl_cancel_csm(p: *mut c_void);
    fn native_extra_opl_csm_active(p: *mut c_void) -> u8;
    fn native_extra_opl_cancel_adpcm(p: *mut c_void);
    fn native_extra_opl_sample(p: *mut c_void, l: *mut f32, r: *mut f32);
    fn native_extra_opl_set_clock(p: *mut c_void, hz: u32) -> i32;
    fn native_extra_opl_get_state(p: *mut c_void, s: *mut CoreState);
    fn native_extra_opl_four_op_mask(p: *mut c_void) -> u8;
    fn native_extra_opl_effective_register(p: *mut c_void, reg: u16) -> u8;
    fn native_extra_opl_get_effective_registers(p: *mut c_void, out: *mut u8);
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
        register: u16,
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
    CancelCsm,
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
        register: u16,
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
    Clock {
        hz: u32,
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
    id.is_none_or(|n| n > 0 && n <= MAX_SAFE_INTEGER)
}
const PAIRS: [usize; 6] = [0, 1, 2, 9, 10, 11];
fn paired(a: usize, b: usize) -> bool {
    a == b
        || PAIRS
            .into_iter()
            .any(|p| (a == p && b == p + 3) || (b == p && a == p + 3))
}
fn register_channel(reg: u16) -> Option<usize> {
    let bank = if reg & 256 != 0 { 9 } else { 0 };
    let byte = reg as u8;
    if matches!(byte,0xa0..=0xa8|0xb0..=0xb8|0xc0..=0xc8) {
        return Some(bank + (byte & 15) as usize);
    }
    if matches!(byte & 0xe0, 0x20 | 0x40 | 0x60 | 0x80 | 0xe0) {
        let slot = (byte & 31) as usize;
        return match slot {
            0..=5 => Some(bank + slot % 3),
            8..=13 => Some(bank + 3 + (slot - 8) % 3),
            16..=21 => Some(bank + 6 + (slot - 16) % 3),
            _ => None,
        };
    }
    None
}
fn operator_voice(reg: u16, rhythm: bool) -> Option<usize> {
    if reg < 256 && rhythm && matches!(reg & 0xe0, 0x20 | 0x40 | 0x60 | 0x80 | 0xe0) {
        match reg & 31 {
            16 | 19 => return Some(18),
            20 => return Some(19),
            18 => return Some(20),
            21 => return Some(21),
            17 => return Some(22),
            _ => {}
        }
    }
    register_channel(reg)
}
fn layout_register(register: u16) -> bool {
    matches!(register, 0xbd | 0x1bd | 0x104 | 0x105)
}
fn declared_channel(op: &Op) -> Option<usize> {
    match *op {
        Op::KeyOff { channel } | Op::AuditionKeyOn { channel } => Some(channel as usize),
        Op::Write { register, .. } => register_channel(register),
        _ => None,
    }
}
fn validate_op(chip: Chip, op: &Op) -> Result<(), String> {
    match *op {
        Op::Write { register, .. } if register as usize >= chip.registers() => {
            Err("Native additional OPL register is out of range".into())
        }
        Op::Write {
            register,
            intent_mask: Some(mask),
            ..
        } if register != 0xbd || mask > 31 => Err("Invalid Native OPL rhythm intent mask".into()),
        Op::KeyOff { channel } | Op::AuditionKeyOn { channel }
            if !chip.valid_voice(channel as usize) =>
        {
            Err("Native additional OPL voice is out of range".into())
        }
        Op::Clock { hz } if !(100_000..=100_000_000).contains(&hz) => {
            Err("Native OPL clock must be 100000–100000000 Hz".into())
        }
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
            return Err("Invalid Native additional OPL command ID".into());
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
            Method::Clock { hz } => validate_op(chip, &Op::Clock { hz: *hz })?,
            Method::Mix { volume, pan }
                if !volume.is_finite()
                    || !(0.0..=4.0).contains(volume)
                    || !pan.is_finite()
                    || !(-1.0..=1.0).contains(pan) =>
            {
                return Err("Native additional OPL mixer volume must be0–4,pan−1–1".into())
            }
            Method::CancelMainOwner { target_group } if !valid_id(Some(*target_group)) => {
                return Err("Invalid Native OPL Main owner group".into())
            }
            Method::Batch {
                commands,
                channel,
                voice,
                action,
            } => {
                if !chip.valid_voice(*channel as usize)
                    || commands.is_empty()
                    || commands.len() > 512
                    || !valid_id(*voice)
                    || self.audition && *action != LeaseAction::Edit && voice.is_none()
                {
                    return Err("Invalid Native additional OPL batch or voice lease".into());
                }
                for op in commands {
                    validate_op(chip, op)?;
                    if declared_channel(op).is_some_and(|target| {
                        if chip == Chip::Ymf262 {
                            !paired(chip.physical(target), chip.physical(*channel as usize))
                        } else {
                            chip.physical(target) != chip.physical(*channel as usize)
                        }
                    }) {
                        return Err(
                            "Native additional OPL batch must target its declared physical channel"
                                .into(),
                        );
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
        match &self.method {
            Method::Write { register, .. } => layout_register(*register),
            Method::Reset => true,
            Method::Batch { commands, .. } => commands
                .iter()
                .any(|op| matches!(op, Op::Write { register, .. } if layout_register(*register))),
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
    effective_four_op_mask: AtomicU8,
    effective_registers: [AtomicU8; 512],
    registers: [AtomicU8; 512],
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
            effective_four_op_mask: AtomicU8::new(0),
            effective_registers: std::array::from_fn(|_| AtomicU8::new(0)),
            registers: std::array::from_fn(|_| AtomicU8::new(0)),
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
    pub chip: &'static str,
    pub channels: u8,
    pub clock: u32,
    pub registers: Vec<u8>,
    pub status: u8,
    pub irq: bool,
    pub held: Vec<u8>,
    pub pending_commands: Vec<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout_revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout_pending: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_four_op_mask: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_mode: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_registers: Option<Vec<u8>>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ym3526: Option<ChipStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ym3812: Option<ChipStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y8950: Option<ChipStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ymf262: Option<ChipStatus>,
    pub frame: u64,
    pub sample_rate: u32,
    pub revision: u64,
}
impl Shared {
    fn clock_held_error(&self) -> String {
        format!(
            "Release the manually held additional {} voices before changing the clock",
            self.chip.name().to_uppercase()
        )
    }
    pub fn prepare_message(&self, message: &Message) -> Result<Arc<Prepared>, String> {
        message.prepare(self.chip)
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
                    || matches!(cmd.method, Method::Write { register, .. } if layout_register(register))
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
        if after > MAX_SAFE_INTEGER {
            return Err("Invalid Native additional OPL layout revision".into());
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
                    "Native additional OPL layout observation was not applied within 5000 ms"
                        .into(),
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
            .unwrap_or_else(|_| unreachable!("Bounded additional OPL command retirement"));
    }
    fn publish(&self, r: &Renderer, command: Option<&Prepared>) {
        let state = r.state();
        self.revision.fetch_add(1, Ordering::AcqRel);
        let pair_mask = r.four_op_mask();
        let mut effective = [0; 512];
        if self.chip == Chip::Ymf262 {
            unsafe { native_extra_opl_get_effective_registers(r.ptr, effective.as_mut_ptr()) }
        }
        let old_rhythm = if self.chip == Chip::Ymf262 {
            self.effective_registers[0xbd].load(Ordering::Relaxed)
        } else {
            self.registers[0xbd].load(Ordering::Relaxed)
        };
        let new_rhythm = if self.chip == Chip::Ymf262 {
            effective[0xbd]
        } else {
            state.registers[0xbd]
        };
        if (old_rhythm ^ new_rhythm) & 32 != 0
            || self.chip == Chip::Ymf262
                && (self.effective_four_op_mask.load(Ordering::Relaxed) != pair_mask
                    || (self.registers[0x105].load(Ordering::Relaxed) ^ state.registers[0x105]) & 1
                        != 0)
        {
            self.layout_revision.fetch_add(1, Ordering::Relaxed);
        }
        self.effective_four_op_mask
            .store(pair_mask, Ordering::Relaxed);
        if self.chip == Chip::Ymf262 {
            for (out, value) in self.effective_registers.iter().zip(effective) {
                out.store(value, Ordering::Relaxed)
            }
        }
        for (out, value) in self.registers.iter().zip(state.registers) {
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
                .fold(
                    if r.csm_owner.is_some_and(|l| l.audition)
                        || r.adpcm_owner.is_some_and(|l| l.audition)
                    {
                        (1 << self.chip.channels()) - 1
                    } else {
                        0
                    },
                    |mask, (ch, _)| mask | (1 << ch),
                ),
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
            let chip = ChipStatus {
                chip: self.chip.name(),
                channels: self.chip.channels() as u8,
                clock: self.clock(),
                registers: self.registers[..self.chip.registers()]
                    .iter()
                    .map(|v| v.load(Ordering::Relaxed))
                    .collect(),
                status: self.status.load(Ordering::Relaxed),
                irq: self.irq.load(Ordering::Relaxed) != 0,
                held: (0..VOICES)
                    .filter(|ch| held & (1 << ch) != 0)
                    .map(|ch| ch as u8)
                    .collect(),
                pending_commands: control
                    .prepared
                    .iter()
                    .filter(|v| !v.complete.load(Ordering::Relaxed))
                    .filter_map(|v| v.id)
                    .collect(),
                layout_revision: Some(self.layout_revision.load(Ordering::Relaxed)),
                effective_registers: (self.chip == Chip::Ymf262).then(|| {
                    self.effective_registers
                        .iter()
                        .map(|v| v.load(Ordering::Relaxed))
                        .collect()
                }),
                effective_four_op_mask: (self.chip == Chip::Ymf262)
                    .then(|| self.effective_four_op_mask.load(Ordering::Relaxed)),
                new_mode: (self.chip == Chip::Ymf262)
                    .then(|| self.registers[0x105].load(Ordering::Relaxed) & 1 != 0),
                layout_pending: Some({
                    control.prepared.iter().any(|command| {
                        command.affects_layout() && !command.complete.load(Ordering::Relaxed)
                    })
                }),
            };
            let (ym3526, ym3812, y8950, ymf262) = match self.chip {
                Chip::Ym3526 => (Some(chip), None, None, None),
                Chip::Ym3812 => (None, Some(chip), None, None),
                Chip::Y8950 => (None, None, Some(chip), None),
                Chip::Ymf262 => (None, None, None, Some(chip)),
            };
            let out = Status {
                ym3526,
                ym3812,
                y8950,
                ymf262,
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
    csm_owner: Option<Lease>,
    adpcm_owner: Option<Lease>,
    gains: [f32; 2],
    targets: [f32; 2],
    ramp: u32,
}
unsafe impl Send for Renderer {}
impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe { native_extra_opl_destroy(self.ptr) }
    }
}
impl Renderer {
    pub fn new(rate: u32, chip: Chip) -> Result<Self, String> {
        let ptr = unsafe { native_extra_opl_create(rate, chip.code()) };
        if ptr.is_null() {
            return Err("Could not allocate additional Native OPL core".into());
        }
        Ok(Self {
            chip,
            current_group: None,
            preview_release: false,
            ptr,
            rate,
            owners: [None; VOICES],
            csm_owner: None,
            adpcm_owner: None,
            gains: [1.; 2],
            targets: [1.; 2],
            ramp: 0,
        })
    }
    fn state(&self) -> CoreState {
        let mut state = CoreState::default();
        unsafe { native_extra_opl_get_state(self.ptr, &mut state) }
        state
    }
    fn four_op_mask(&self) -> u8 {
        if self.chip == Chip::Ymf262 {
            unsafe { native_extra_opl_four_op_mask(self.ptr) }
        } else {
            0
        }
    }
    fn effective_register(&self, register: u16, s: &CoreState) -> u16 {
        if self.chip == Chip::Ymf262
            && s.registers[0x105] & 1 == 0
            && register >= 256
            && register != 0x105
        {
            register & 255
        } else {
            register
        }
    }
    fn pair_primary(&self, ch: usize) -> usize {
        let p = self.chip.physical(ch);
        if self.chip == Chip::Ymf262 {
            for (bit, first) in PAIRS.into_iter().enumerate() {
                if self.four_op_mask() & (1 << bit) != 0 && (p == first || p == first + 3) {
                    return first;
                }
            }
        }
        p
    }
    fn has_manual(&self) -> bool {
        self.owners.iter().flatten().any(|l| l.audition)
            || self.csm_owner.is_some_and(|l| l.audition)
            || self.adpcm_owner.is_some_and(|l| l.audition)
    }
    fn protected(&self, ch: usize, voice: Option<u64>, audition: bool) -> bool {
        [self.owners[ch], self.csm_owner]
            .into_iter()
            .flatten()
            .any(|l| l.audition && (!audition || voice.is_some() && l.voice != voice))
    }
    fn alias_manual(&self, ch: usize, voice: Option<u64>, audition: bool) -> bool {
        (0..VOICES).any(|other| {
            self.chip.valid_voice(other)
                && self.pair_primary(other) == self.pair_primary(ch)
                && self.protected(other, voice, audition)
        })
    }
    fn alias_conflict(&self, ch: usize) -> bool {
        (0..VOICES).any(|other| {
            self.chip.valid_voice(other)
                && other != ch
                && (other < 18) != (ch < 18)
                && self.chip.physical(other) == self.chip.physical(ch)
                && self.owners[other].is_some_and(|l| l.audition)
        })
    }
    fn parameter_protected(&self, ch: usize, voice: Option<u64>, audition: bool) -> bool {
        if ch >= 18 {
            return self.protected(ch, voice, audition);
        }
        (0..self.chip.channels()).any(|other| {
            self.pair_primary(other) == self.pair_primary(ch)
                && self.protected(other, voice, audition)
        })
    }
    fn pair_change_allowed(
        &self,
        primary: usize,
        owner: Option<Owner>,
        voice: Option<u64>,
        audition: bool,
    ) -> bool {
        [primary, primary + 3].into_iter().all(|ch| {
            !self.protected(ch, voice, audition)
                && self.owners[ch].is_none_or(|l| !self.foreign(l, owner, voice, audition))
        })
    }
    fn batch_targets_allowed(
        &self,
        commands: &[Op],
        channel: usize,
        action: LeaseAction,
        owner: Option<Owner>,
        voice: Option<u64>,
        audition: bool,
        s: &CoreState,
    ) -> bool {
        if self.chip != Chip::Ymf262 {
            return true;
        }
        if channel >= 9 && channel < 18 && s.registers[0x105] & 1 == 0 && action != LeaseAction::Off
        {
            return false;
        }
        if action != LeaseAction::Off && channel < 18 && self.pair_primary(channel) != channel {
            return false;
        }
        let mut new_mode = s.registers[0x105] & 1 != 0;
        let mut mask = self.four_op_mask();
        let mut transitions = 0u8;
        for op in commands {
            if let Op::Write {
                register, value, ..
            } = *op
            {
                let r = if !new_mode && register >= 256 && register != 0x105 {
                    register & 255
                } else {
                    register
                };
                if r == 0x105 {
                    new_mode = value & 1 != 0;
                }
                if r == 0x104 {
                    let changed = (mask ^ value) & 63;
                    for (bit, p) in PAIRS.into_iter().enumerate() {
                        if changed & (1 << bit) != 0
                            && !self.pair_change_allowed(p, owner, voice, audition)
                        {
                            return false;
                        }
                    }
                    transitions |= changed;
                    mask = value & 63;
                }
            }
        }
        let root = |ch: usize, m: u8| {
            let p = self.chip.physical(ch);
            for (bit, first) in PAIRS.into_iter().enumerate() {
                if m & (1 << bit) != 0 && (p == first || p == first + 3) {
                    return first;
                }
            }
            p
        };
        let mut new_mode = s.registers[0x105] & 1 != 0;
        let mut mask = self.four_op_mask();
        for op in commands {
            let target = match *op {
                Op::Write {
                    register, value, ..
                } => {
                    let r = if !new_mode && register >= 256 && register != 0x105 {
                        register & 255
                    } else {
                        register
                    };
                    let target = register_channel(r);
                    if r == 0x105 {
                        new_mode = value & 1 != 0;
                    }
                    if r == 0x104 {
                        mask = value & 63;
                    }
                    target
                }
                _ => declared_channel(op),
            };
            if let Some(target) = target {
                if root(target, mask) != root(channel, mask) {
                    let transition = PAIRS.into_iter().enumerate().any(|(bit, p)| {
                        transitions & (1 << bit) != 0
                            && channel == p
                            && (target == p || target == p + 3)
                    });
                    if !transition {
                        return false;
                    }
                }
            }
        }
        true
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
    fn release(&mut self, ch: usize) {
        unsafe { native_extra_opl_key_off_fm(self.ptr, ch as u8) }
        self.owners[ch] = None;
    }
    fn cancel_csm(&mut self) {
        unsafe { native_extra_opl_cancel_csm(self.ptr) }
        self.csm_owner = None;
    }
    fn cancel_adpcm(&mut self) {
        unsafe { native_extra_opl_cancel_adpcm(self.ptr) }
        self.adpcm_owner = None;
    }
    fn scoped(&self, l: Lease, owner: Option<Owner>, group: Option<u64>, audition: bool) -> bool {
        (owner.is_none() || l.owner == owner)
            && (group.is_none() || l.group == group)
            && (!l.audition || audition)
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
        let s = self.state();
        let rhythm = if self.chip == Chip::Ymf262 {
            unsafe { native_extra_opl_effective_register(self.ptr, 0xbd) & 32 != 0 }
        } else {
            s.registers[0xbd] & 32 != 0
        };
        let effective = if let Op::Write { register, .. } = *op {
            Some(self.effective_register(register, &s))
        } else {
            None
        };
        let target = if let Some(register) = effective {
            operator_voice(register, rhythm)
        } else {
            declared_channel(op)
        };
        if target.is_some_and(|ch| {
            if effective.is_some_and(|r| !matches!(r as u8, 0xb0..=0xb8)) {
                self.parameter_protected(ch, voice, audition)
            } else {
                self.protected(ch, voice, audition)
            }
        }) {
            return;
        }
        match *op {
            Op::Clock { hz } => {
                if !audition && self.has_manual() {
                    return;
                }
                unsafe {
                    native_extra_opl_set_clock(self.ptr, hz);
                }
            }
            Op::KeyOff { channel } => {
                let ch = channel as usize;
                if self.owners[ch].is_some_and(|l| self.foreign(l, owner, voice, audition)) {
                    return;
                }
                if self.preview_release {
                    unsafe { native_extra_opl_source_preview_key_off(self.ptr, channel) }
                    self.owners[ch] = None;
                } else {
                    self.release(ch)
                }
            }
            Op::AuditionKeyOn { channel } => {
                let ch = channel as usize;
                if self.chip == Chip::Ymf262
                    && (s.registers[0x105] & 1 == 0 && ch >= 9 && ch < 18
                        || self.pair_primary(ch) != self.chip.physical(ch))
                {
                    return;
                }
                if self.alias_conflict(ch) {
                    return;
                }
                unsafe { native_extra_opl_source_audition_key_on(self.ptr, channel) }
                if !preserve {
                    self.owners[ch] = Some(self.lease(owner, voice, audition))
                }
            }
            Op::Write {
                register,
                mut value,
                intent_mask,
            } => {
                let mut post_map_cancel = 0u8;
                let authored_register = register;
                let register = effective.unwrap();
                let old = if self.chip == Chip::Ymf262 {
                    unsafe { native_extra_opl_effective_register(self.ptr, register) }
                } else {
                    s.registers[register as usize]
                };
                if self.chip == Chip::Ymf262
                    && register == 0x105
                    && (old ^ value) & 1 != 0
                    && !audition
                    && self.has_manual()
                {
                    value = (value & !1) | (old & 1);
                }
                if self.chip == Chip::Ymf262 && register == 0x104 {
                    let changed = (old ^ value) & 63;
                    for (bit, primary) in PAIRS.into_iter().enumerate() {
                        if changed & (1 << bit) != 0 {
                            if self.pair_change_allowed(primary, owner, voice, audition) {
                                if self.owners[primary].is_some()
                                    || self.owners[primary + 3].is_some()
                                {
                                    post_map_cancel |= 1 << bit;
                                }
                                for ch in [primary, primary + 3] {
                                    if self.owners[ch].is_some() {
                                        self.release(ch)
                                    }
                                }
                            } else {
                                value = (value & !(1 << bit)) | (old & (1 << bit));
                            }
                        }
                    }
                }
                if self.chip == Chip::Y8950
                    && (7..=0x12).contains(&register)
                    && self.adpcm_owner.is_some_and(|l| l.audition && !audition)
                {
                    return;
                }
                if register == 1 && !audition && self.has_manual() {
                    value = (value & !32) | (old & 32)
                }
                if matches!(register, 2 | 3 | 4 | 8) && !audition && self.has_manual() {
                    return;
                }
                if matches!(register as u8,0xa0..=0xa8|0xb0..=0xb8|0xc0..=0xc8) {
                    let ch = register_channel(register).unwrap();
                    if self.alias_manual(ch, voice, audition) {
                        if (0xb0..=0xb8).contains(&(register as u8)) {
                            value = (value & !0xdf) | (old & 0xdf)
                        } else {
                            return;
                        }
                    }
                }
                if matches!(register & 0xe0, 0x20 | 0x40 | 0x60 | 0x80 | 0xe0) {
                    if let Some(ch) = target {
                        if declared.is_some_and(|d| d >= 18 && ch != d) {
                            return;
                        }
                        if self.owners[self.pair_primary(ch)]
                            .is_some_and(|l| self.foreign(l, owner, voice, audition))
                            && preserve
                        {
                            return;
                        }
                    }
                }
                if (0xb0..=0xb8).contains(&(register as u8)) {
                    let ch = register_channel(register).unwrap();
                    let mut may_claim = true;
                    if value & 32 == 0
                        && self.owners[ch].is_some_and(|l| self.foreign(l, owner, voice, audition))
                    {
                        value = (value & !32) | (old & 32);
                        may_claim = false;
                    }
                    if !preserve && may_claim {
                        self.owners[ch] = if value & 32 != 0 {
                            Some(self.lease(owner, voice, audition))
                        } else {
                            None
                        };
                    }
                }
                let before_csm = unsafe { native_extra_opl_csm_active(self.ptr) != 0 };
                let mut may_claim_csm = true;
                if before_csm && matches!(register, 4 | 8) {
                    let stopping = register == 8 && value & 128 == 0
                        || register == 4 && value & 128 == 0 && value & 1 == 0;
                    if stopping
                        && self
                            .csm_owner
                            .is_some_and(|l| self.foreign(l, owner, voice, audition))
                    {
                        may_claim_csm = false;
                        let bit = if register == 8 { 128 } else { 1 };
                        value = (value & !bit) | (old & bit);
                        // IRQ reset writes do not change effective timer loads.
                        if register == 4 {
                            value = (value & !128) | 1;
                        }
                    }
                }
                if self.chip == Chip::Y8950 && register == 7 {
                    let on = value & 128 != 0 && value & 0x41 == 0;
                    if !on
                        && self
                            .adpcm_owner
                            .is_some_and(|l| self.foreign(l, owner, voice, audition))
                    {
                        return;
                    }
                    if !preserve {
                        if on {
                            self.adpcm_owner = Some(self.lease(owner, voice, audition));
                        } else if value & 1 != 0 {
                            self.adpcm_owner = None;
                        }
                        // Execute-off/recording stop decoding but retain the
                        // interpolated accumulator. Its source lease must live
                        // until reset or owner cancellation clears that DC.
                    }
                }
                if register == 0xbd {
                    if !audition && self.has_manual() {
                        value = (value & !0xc0) | (old & 0xc0)
                    }
                    if (value ^ old) & 32 != 0
                        && [6, 7, 8, 18, 19, 20, 21, 22].into_iter().any(|ch| {
                            self.chip.valid_voice(ch)
                                && (self.protected(ch, voice, audition)
                                    || self.owners[ch]
                                        .is_some_and(|l| self.foreign(l, owner, voice, audition)))
                        })
                    {
                        value = (value & !32) | (old & 32)
                    }
                    for index in 0..5 {
                        let ch = 18 + index;
                        let bit = 16 >> index;
                        let before = old & bit != 0;
                        let after = value & bit != 0;
                        if self.protected(ch, voice, audition)
                            || declared
                                .is_some_and(|d| self.chip.physical(d) != self.chip.physical(ch))
                            || before
                                && !after
                                && self.owners[ch]
                                    .is_some_and(|l| self.foreign(l, owner, voice, audition))
                        {
                            value = (value & !bit) | (old & bit)
                        } else if !preserve {
                            if after && (!before || intent_mask.is_some_and(|mask| mask & bit != 0))
                            {
                                self.owners[ch] = Some(self.lease(owner, voice, audition))
                            } else if before && !after {
                                self.owners[ch] = None;
                            }
                        }
                    }
                }
                unsafe { native_extra_opl_write(self.ptr, authored_register, value) }
                // Reassignment can expose an old secondary envelope after a
                // four-op voice splits. Both physical outputs of an owned
                // changed pair must be silenced under the new map too.
                for (bit, primary) in PAIRS.into_iter().enumerate() {
                    if post_map_cancel & (1 << bit) != 0 {
                        self.release(primary);
                        self.release(primary + 3);
                    }
                }
                if !preserve && matches!(register, 4 | 8) {
                    let active = unsafe { native_extra_opl_csm_active(self.ptr) != 0 };
                    let authored_start = register == 8 && value & 128 != 0
                        || register == 4 && value & 128 == 0 && value & 1 != 0;
                    if active && may_claim_csm && (!before_csm || authored_start) {
                        self.csm_owner = Some(self.lease(owner, voice, audition));
                    } else if before_csm && !active {
                        self.csm_owner = None;
                    }
                }
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
            Method::AuditionKeyOn { channel } => self.op(
                &Op::AuditionKeyOn { channel: *channel },
                owner,
                None,
                c.audition,
                false,
                None,
            ),
            Method::Clock { hz } => {
                self.op(&Op::Clock { hz: *hz }, owner, None, c.audition, false, None)
            }
            Method::Mix { volume, pan } => {
                self.targets = [
                    *volume * if *pan > 0. { 1. - *pan } else { 1. },
                    *volume * if *pan < 0. { 1. + *pan } else { 1. },
                ];
                self.ramp = (self.rate / 200).max(1)
            }
            Method::Reset => {
                if c.audition || !self.has_manual() {
                    unsafe { native_extra_opl_reset(self.ptr) }
                    self.owners.fill(None);
                    self.csm_owner = None;
                    self.adpcm_owner = None;
                }
            }
            Method::CancelCsm => {
                if self
                    .csm_owner
                    .is_some_and(|l| !self.foreign(l, owner, None, c.audition))
                {
                    self.cancel_csm()
                }
            }
            Method::CancelMainOwner { target_group } => {
                for ch in 0..VOICES {
                    if self.owners[ch]
                        .is_some_and(|l| !l.audition && l.group == Some(*target_group))
                    {
                        self.release(ch)
                    }
                }
                if self
                    .csm_owner
                    .is_some_and(|l| !l.audition && l.group == Some(*target_group))
                {
                    self.cancel_csm()
                }
                if self
                    .adpcm_owner
                    .is_some_and(|l| !l.audition && l.group == Some(*target_group))
                {
                    self.cancel_adpcm()
                }
            }
            Method::StopVoices | Method::AllNotesOff => {
                if owner.is_none() && c.group.is_none() && (c.audition || !self.has_manual()) {
                    unsafe {
                        if matches!(c.method, Method::AllNotesOff) {
                            native_extra_opl_all_notes_off(self.ptr)
                        } else {
                            native_extra_opl_stop(self.ptr)
                        }
                    }
                    if self.chip == Chip::Ymf262 && matches!(c.method, Method::AllNotesOff) {
                        // Public allNotesOff uses bus writes. With NEW=0 its
                        // upper writes fold into the lower bank, so still-live
                        // upper physical sources retain their cleanup lease.
                        for ch in 0..18 {
                            let reg = (if ch >= 9 { 256 } else { 0 }) + 0xb0 + (ch % 9) as u16;
                            if unsafe { native_extra_opl_effective_register(self.ptr, reg) } & 32
                                == 0
                            {
                                self.owners[ch] = None;
                            }
                        }
                        let rhythm = unsafe { native_extra_opl_effective_register(self.ptr, 0xbd) };
                        for drum in 18..23 {
                            if rhythm & (16 >> (drum - 18)) == 0 {
                                self.owners[drum] = None;
                            }
                        }
                    } else {
                        self.owners.fill(None);
                    }
                    if matches!(c.method, Method::StopVoices) {
                        self.csm_owner = None;
                        self.adpcm_owner = None;
                    }
                } else {
                    for ch in 0..VOICES {
                        if self.owners[ch]
                            .is_some_and(|l| self.scoped(l, owner, c.group, c.audition))
                        {
                            self.release(ch)
                        }
                    }
                    if matches!(c.method, Method::StopVoices) {
                        if self
                            .csm_owner
                            .is_some_and(|l| self.scoped(l, owner, c.group, c.audition))
                        {
                            self.cancel_csm()
                        }
                        if self
                            .adpcm_owner
                            .is_some_and(|l| self.scoped(l, owner, c.group, c.audition))
                        {
                            self.cancel_adpcm()
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
                if !self.batch_targets_allowed(
                    commands,
                    ch,
                    *action,
                    owner,
                    *voice,
                    c.audition,
                    &self.state(),
                ) {
                    shared.publish(self, Some(c));
                    self.current_group = None;
                    return;
                }
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
                        self.owners[ch] = Some(self.lease(owner, *voice, c.audition))
                    }
                    for op in commands {
                        self.op(
                            op,
                            owner,
                            *voice,
                            c.audition,
                            *action == LeaseAction::Edit,
                            if *action == LeaseAction::Edit
                                && matches!(op, Op::Write { register, .. } if layout_register(*register))
                            {
                                None
                            } else {
                                Some(ch)
                            },
                        )
                    }
                    if *action == LeaseAction::On {
                        self.owners[ch] = Some(self.lease(owner, *voice, c.audition))
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
        for ch in 0..VOICES {
            if self.owners[ch].is_some_and(|l| !valid(l.owner)) {
                self.release(ch);
                changed = true
            }
        }
        if self.csm_owner.is_some_and(|l| !valid(l.owner)) {
            self.cancel_csm();
            changed = true
        }
        if self.adpcm_owner.is_some_and(|l| !valid(l.owner)) {
            self.cancel_adpcm();
            changed = true
        }
        if changed {
            s.publish(self, None)
        } else {
            s.refresh_running_status(self)
        }
    }
    pub fn mix(&mut self, pcm: &mut [f32; 2]) {
        let (mut l, mut r) = (0., 0.);
        unsafe { native_extra_opl_sample(self.ptr, &mut l, &mut r) }
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
