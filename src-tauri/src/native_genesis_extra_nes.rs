//! Independently allocated NES APU + optional FDS core. Control retains all variable-sized commands;
//! the render callback accesses its own core and atomic status only.
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

pub const CLOCK: u32 = 1_789_773;
pub const RAM: usize = 65_536;
const COVERAGE: usize = RAM / 64;
const PENDING_BYTES: usize = 8 * 1024 * 1024;
const BATCH: usize = 512;
#[repr(C)]
struct CoreState {
    registers: [u8; 0x90],
    wave: [u8; 64],
    modulation: [u8; 32],
    periods: [u32; 5],
    lengths: [u32; 4],
    volumes: [u32; 4],
    dmc_address: u32,
    dmc_remaining: u32,
    dmc_output: u32,
    dmc_frequency: u32,
    fds_frequency: u32,
    fds_mod_frequency: u32,
    fds_mod_index: u32,
    fds_mod_clock: u32,
    fds_mod_counter: i32,
    fds_gain: u32,
    fds_sweep_gain: u32,
    fds_master: u32,
    fds_accumulator: u32,
    fds_output: u32,
    enable_mask: u32,
    mute_mask: u32,
    fds_enabled: u32,
    fds_io: u32,
    fds_active: u32,
    fds_write_enabled: u32,
    fds_mod_enabled: u32,
    frame_irq: u32,
    dmc_irq: u32,
    clock: f64,
    base_clock: f64,
}
impl Default for CoreState {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}
extern "C" {
    fn native_nes_create(rate: u32, base_clock: f64, fds: bool) -> *mut c_void;
    fn native_nes_destroy(ptr: *mut c_void);
    fn native_nes_reset(ptr: *mut c_void);
    fn native_nes_stop_channel(ptr: *mut c_void, channel: u8);
    fn native_nes_active_mask(ptr: *mut c_void) -> u32;
    #[cfg(test)]
    pub fn native_nes_core_size() -> u32;
    fn native_nes_set_mute(ptr: *mut c_void, mask: u8);
    fn native_nes_write(ptr: *mut c_void, address: u16, value: u8);
    fn native_nes_write_enable(ptr: *mut c_void, value: u8, preserve: u8);
    fn native_nes_enable_channel(ptr: *mut c_void, channel: u8, enabled: bool);
    fn native_nes_load(ptr: *mut c_void, bytes: *const u8, offset: u32, size: u32) -> i32;
    fn native_nes_set_clock(ptr: *mut c_void, hz: u32) -> i32;
    fn native_nes_sample(ptr: *mut c_void, left: *mut f32, right: *mut f32);
    fn native_nes_get_state(ptr: *mut c_void, out: *mut CoreState);
    #[cfg(test)]
    fn native_nes_read_memory(ptr: *mut c_void, address: u16) -> u8;
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
        address: u16,
        value: u8,
    },
    Enable {
        channel: u8,
        enabled: bool,
    },
    Memory {
        offset: u32,
        data: Vec<u8>,
    },
    LoadMemory {
        address: u32,
        data: Vec<u8>,
    },
    Reset,
    StopVoices,
    AllNotesOff,
    Clock {
        hz: u32,
    },
    Mix {
        volume: f32,
        pan: f32,
    },
    SourceMute {
        mask: u8,
    },
    CancelMainOwner {
        #[serde(rename = "targetGroup")]
        target_group: u64,
    },
    Batch {
        commands: Vec<Op>,
        channel: u8,
        voice: Option<u64>,
        action: LeaseAction,
        #[serde(rename = "fdsGain")]
        fds_gain: Option<u8>,
        #[serde(rename = "fdsMaster")]
        fds_master: Option<u8>,
    },
}
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(tag = "method", rename_all = "kebab-case")]
pub enum Op {
    Write { address: u16, value: u8 },
    Enable { channel: u8, enabled: bool },
}
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum LeaseAction {
    On,
    Off,
    Edit,
}
#[derive(Debug)]
enum Edit {
    Op(Op),
    Memory(u32, Vec<u8>),
    Reset,
    Stop,
    Clock(u32),
    Mix(f32, f32),
    SourceMute(u8),
    CancelMainOwner(u64),
    Batch(
        Vec<Op>,
        u8,
        Option<u64>,
        LeaseAction,
        Option<u8>,
        Option<u8>,
    ),
}
#[derive(Debug)]
pub struct Prepared {
    pub id: Option<u64>,
    audition: bool,
    group: Option<u64>,
    edit: Edit,
    complete: AtomicBool,
    retired: AtomicBool,
    cancelled: AtomicBool,
    clock_rejected: AtomicBool,
    memory_rejected: AtomicBool,
}
const CLOCK_HELD: &str =
    "Release the manually held additional NES voices before changing the clock";
const MEMORY_HELD: &str =
    "Release the manually held additional NES DMC voice before replacing sample memory";
fn valid_id(v: Option<u64>) -> bool {
    v.is_none_or(|v| v > 0 && v <= MAX_SAFE_INTEGER)
}
fn address_channel(address: u16) -> Option<u8> {
    match address {
        0x4000..=0x4013 => Some(((address - 0x4000) / 4) as u8),
        0x4023 | 0x4040..=0x408a => Some(5),
        _ => None,
    }
}
fn validate(op: Op) -> Result<(), String> {
    match op {
        Op::Write { address, .. } if !(matches!(address,0x4000..=0x4017 | 0x4023 | 0x4040..=0x408a)) => {
            Err("Unsupported Native NES/FDS address".into())
        }
        Op::Enable { channel, .. } if channel > 4 => {
            Err("Native NES enable channel must be 0–4".into())
        }
        _ => Ok(()),
    }
}
impl Message {
    pub fn prepared_bytes(&self) -> usize {
        std::mem::size_of::<Prepared>()
            + if let Method::Batch { commands, .. } = &self.method {
                commands.len() * std::mem::size_of::<Op>()
            } else if let Method::Memory { data, .. } | Method::LoadMemory { data, .. } =
                &self.method
            {
                data.len()
            } else {
                0
            }
    }
    pub fn prepare(&self) -> Result<Arc<Prepared>, String> {
        if !valid_id(self.id) || !valid_id(self.group) {
            return Err("Invalid Native additional NES command ID".into());
        }
        let edit = match &self.method {
            Method::Write { address, value } => {
                let op = Op::Write {
                    address: *address,
                    value: *value,
                };
                validate(op)?;
                Edit::Op(op)
            }
            Method::Enable { channel, enabled } => {
                let op = Op::Enable {
                    channel: *channel,
                    enabled: *enabled,
                };
                validate(op)?;
                Edit::Op(op)
            }
            Method::Memory { offset, data }
            | Method::LoadMemory {
                address: offset,
                data,
            } => {
                if *offset as usize > RAM || data.len() > RAM - *offset as usize {
                    return Err("Native additional NES upload is outside 64 KiB RAM".into());
                }
                Edit::Memory(*offset, data.clone())
            }
            Method::Reset => Edit::Reset,
            Method::StopVoices | Method::AllNotesOff => Edit::Stop,
            Method::Clock { hz } => {
                if !(100_000..=100_000_000).contains(hz) {
                    return Err("Native NES clock must be 100000–100000000 Hz".into());
                }
                Edit::Clock(*hz)
            }
            Method::Mix { volume, pan } => {
                if !volume.is_finite()
                    || !(0.0..=4.0).contains(volume)
                    || !pan.is_finite()
                    || !(-1.0..=1.0).contains(pan)
                {
                    return Err("Native NES mixer volume must be 0–4, pan −1–1".into());
                }
                Edit::Mix(*volume, *pan)
            }
            Method::SourceMute { mask } => {
                if *mask > 63 {
                    return Err("Native NES source mute mask must be 0–63".into());
                }
                Edit::SourceMute(*mask)
            }
            Method::CancelMainOwner { target_group } => {
                if !valid_id(Some(*target_group)) {
                    return Err("Invalid Native NES Main owner group".into());
                }
                Edit::CancelMainOwner(*target_group)
            }
            Method::Batch {
                commands,
                channel: target,
                voice,
                action,
                fds_gain,
                fds_master,
            } => {
                if *target > 5
                    || (commands.is_empty() && *action != LeaseAction::Edit)
                    || commands.len() > BATCH
                    || !valid_id(*voice)
                    || self.audition && *action != LeaseAction::Edit && voice.is_none()
                {
                    return Err("Invalid Native NES channel batch or voice lease".into());
                }
                if fds_gain.is_some_and(|v| v > 32)
                    || fds_master.is_some_and(|v| v > 3)
                    || *target != 5 && (fds_gain.is_some() || fds_master.is_some())
                {
                    return Err("Invalid Native FDS configured gain or master".into());
                }
                for op in commands {
                    validate(*op)?;
                    let actual = match op {
                        Op::Write { address, .. } => address_channel(*address),
                        Op::Enable { channel, .. } => Some(*channel),
                    };
                    if actual.is_some_and(|ch| ch != *target)
                        || matches!(
                            op,
                            Op::Write {
                                address: 0x4015 | 0x4017,
                                ..
                            }
                        )
                    {
                        return Err("Native NES batch must target its declared channel; enable/frame sequencer are global".into());
                    }
                }
                Edit::Batch(
                    commands.clone(),
                    *target,
                    *voice,
                    *action,
                    *fds_gain,
                    *fds_master,
                )
            }
        };
        Ok(Arc::new(Prepared {
            id: self.id,
            audition: self.audition,
            group: self.group,
            edit,
            complete: AtomicBool::new(false),
            retired: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
            clock_rejected: AtomicBool::new(false),
            memory_rejected: AtomicBool::new(false),
        }))
    }
}
impl Prepared {
    fn outcome(&self) -> u8 {
        self.cancelled.load(Ordering::Acquire) as u8
            | ((self.clock_rejected.load(Ordering::Acquire) as u8) << 1)
            | ((self.memory_rejected.load(Ordering::Acquire) as u8) << 2)
    }
    pub fn is_owner_cancel(&self) -> bool {
        matches!(self.edit, Edit::CancelMainOwner(_))
    }
    pub fn upload_bytes(&self) -> usize {
        if let Edit::Memory(_, data) = &self.edit {
            data.len()
        } else {
            0
        }
    }
    pub fn starts(&self) -> bool {
        matches!(self.edit, Edit::Op(_))
            || matches!(self.edit,Edit::Batch(_,_,_,action,..)if action!=LeaseAction::Off)
    }
    pub fn stops(&self) -> bool {
        matches!(self.edit, Edit::Stop) && self.group.is_none()
    }
    pub fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + if let Edit::Batch(ops, ..) = &self.edit {
                ops.capacity() * std::mem::size_of::<Op>()
            } else if let Edit::Memory(_, data) = &self.edit {
                data.capacity()
            } else {
                0
            }
    }
}
#[derive(Clone, Copy, Default)]
struct Completion {
    id: u64,
    flags: u8,
}
struct Control {
    prepared: Vec<Arc<Prepared>>,
    outcomes: [Completion; 512],
    cursor: usize,
}
impl Default for Control {
    fn default() -> Self {
        Self {
            prepared: Vec::new(),
            outcomes: [Completion::default(); 512],
            cursor: 0,
        }
    }
}
fn outcome(flags: u8) -> Result<(), String> {
    if flags & 1 != 0 {
        Err("Native NES command was cancelled before it applied".into())
    } else if flags & 2 != 0 {
        Err(CLOCK_HELD.into())
    } else if flags & 4 != 0 {
        Err(MEMORY_HELD.into())
    } else {
        Ok(())
    }
}
pub struct Shared {
    revision: AtomicU64,
    registers: [AtomicU8; 144],
    modulation: [AtomicU8; 32],
    coverage: Box<[AtomicU64; COVERAGE]>,
    enabled: AtomicU8,
    fds_enabled: AtomicBool,
    base_clock: AtomicU64,
    fds_gain: AtomicU8,
    fds_master: AtomicU8,
    active: AtomicU8,
    clock: AtomicU64,
    rate: AtomicU32,
    mute: AtomicU8,
    held: AtomicU8,
    control: Mutex<Control>,
    retired: ArrayQueue<Arc<Prepared>>,
    highest_id: AtomicU64,
    cancel_before: AtomicU64,
    owner_cancel_before: [AtomicU64; OWNERS],
}
impl Default for Shared {
    fn default() -> Self {
        Self {
            revision: AtomicU64::new(0),
            registers: std::array::from_fn(|_| AtomicU8::new(0)),
            modulation: std::array::from_fn(|_| AtomicU8::new(0)),
            coverage: Box::new(std::array::from_fn(|_| AtomicU64::new(0))),
            enabled: AtomicU8::new(0),
            fds_enabled: AtomicBool::new(false),
            base_clock: AtomicU64::new((CLOCK as f64).to_bits()),
            fds_gain: AtomicU8::new(32),
            fds_master: AtomicU8::new(0),
            active: AtomicU8::new(0),
            clock: AtomicU64::new((CLOCK as f64).to_bits()),
            rate: AtomicU32::new(0),
            mute: AtomicU8::new(0),
            held: AtomicU8::new(0),
            control: Mutex::new(Control::default()),
            retired: ArrayQueue::new(CAPACITY),
            highest_id: AtomicU64::new(0),
            cancel_before: AtomicU64::new(0),
            owner_cancel_before: std::array::from_fn(|_| AtomicU64::new(0)),
        }
    }
}
#[derive(Debug, Serialize)]
pub struct MemoryRange {
    pub address: u32,
    pub end: u32,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChipStatus {
    pub clock: f64,
    pub registers: Vec<u8>,
    pub modulation: Vec<u8>,
    pub enabled: u8,
    pub fds_enabled: bool,
    pub base_clock: f64,
    pub fds_gain: u8,
    pub fds_master: u8,
    pub memory_ranges: Vec<MemoryRange>,
    pub dmc_active: bool,
    pub fds_active: bool,
    pub gates: [bool; 6],
    pub held: Vec<u8>,
    pub source_mute_mask: u8,
    pub pending_commands: Vec<u64>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub nes: ChipStatus,
    pub frame: u64,
    pub sample_rate: u32,
    pub revision: u64,
}
impl Shared {
    pub fn prepare_message(&self, m: &Message) -> Result<Arc<Prepared>, String> {
        m.prepare()
    }
    pub fn valid_event(&self, c: &Prepared, seq: u64, owner: Option<Owner>) -> bool {
        !c.cancelled.load(Ordering::Acquire)
            && seq >= self.cancel_before.load(Ordering::Acquire)
            && owner.is_none_or(|o| {
                seq >= self.owner_cancel_before[(o.owner - 1) as usize].load(Ordering::Acquire)
            })
    }
    pub fn cancel_before(&self, _: &Prepared, seq: u64, owner: Option<Owner>) {
        if let Some(o) = owner {
            self.owner_cancel_before[(o.owner - 1) as usize].fetch_max(seq, Ordering::AcqRel);
        } else {
            self.cancel_before.fetch_max(seq, Ordering::AcqRel);
        }
    }
    fn collect(&self, c: &mut Control) {
        while self.retired.pop().is_some() {}
        let Control {
            prepared,
            outcomes,
            cursor,
        } = c;
        prepared.retain(|v| {
            if !v.retired.load(Ordering::Acquire) {
                return true;
            }
            if let Some(id) = v.id {
                outcomes[*cursor] = Completion {
                    id,
                    flags: v.outcome(),
                };
                *cursor = (*cursor + 1) % outcomes.len()
            }
            false
        });
        if prepared.capacity() > 64.max(prepared.len() * 2) {
            prepared.shrink_to_fit()
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
            .map_err(|_| "Native NES control unavailable")?;
        self.collect(&mut c);
        if self.held.load(Ordering::Acquire) != 0
            && commands
                .iter()
                .any(|(v, _)| !v.audition && matches!(v.edit, Edit::Clock(_)))
        {
            return Err(CLOCK_HELD.into());
        }
        if self.held.load(Ordering::Acquire) & 16 != 0
            && commands
                .iter()
                .any(|(v, _)| !v.audition && matches!(v.edit, Edit::Memory(..)))
        {
            return Err(MEMORY_HELD.into());
        }
        if c.prepared.len() + commands.len() > CAPACITY
            || c.prepared.iter().map(|v| v.retained_bytes()).sum::<usize>()
                + commands
                    .iter()
                    .map(|(v, _)| v.retained_bytes())
                    .sum::<usize>()
                > PENDING_BYTES
        {
            return Err("Native NES pending command limit reached (8 MiB)".into());
        }
        let mut ids = HashSet::new();
        for v in c.prepared.iter().chain(commands.iter().map(|(v, _)| v)) {
            if v.id.is_some_and(|id| !ids.insert(id)) {
                return Err("Duplicate pending Native NES command ID".into());
            }
        }
        for (v, _) in commands {
            if let Some(id) = v.id {
                self.highest_id.fetch_max(id, Ordering::AcqRel);
            }
            c.prepared.push(v.clone())
        }
        Ok(())
    }
    pub fn commit_cancel(&self, c: &Prepared) -> bool {
        let Edit::CancelMainOwner(target) = c.edit else {
            return false;
        };
        let control = self
            .control
            .lock()
            .expect("NES control checked by transactional retain");
        for v in &control.prepared {
            if v.group == Some(target) && !v.audition && !v.complete.load(Ordering::Acquire) {
                v.cancelled.store(true, Ordering::Release);
            }
        }
        true
    }
    pub fn wait_applied(&self, id: u64) -> Result<(), String> {
        if id == 0 || id > self.highest_id.load(Ordering::Acquire) {
            return Err("Unknown Native NES command ID".into());
        }
        let cmd = {
            let c = self
                .control
                .lock()
                .map_err(|_| "Native NES control unavailable")?;
            if let Some(v) = c.prepared.iter().find(|v| v.id == Some(id)) {
                v.clone()
            } else {
                for offset in 0..c.outcomes.len() {
                    let result =
                        c.outcomes[(c.cursor + c.outcomes.len() - 1 - offset) % c.outcomes.len()];
                    if result.id == id {
                        return outcome(result.flags);
                    }
                }
                return Err("Native NES command outcome has expired".into());
            }
        };
        let start = std::time::Instant::now();
        while !cmd.complete.load(Ordering::Acquire) {
            if start.elapsed() > std::time::Duration::from_millis(5000) {
                return Err("Native NES command was not applied within 5000 ms".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        outcome(cmd.outcome())
    }
    pub fn rollback(&self, v: Arc<Prepared>) {
        self.retire(v)
    }
    pub fn retire(&self, v: Arc<Prepared>) {
        if !v.complete.load(Ordering::Acquire) {
            v.cancelled.store(true, Ordering::Release)
        }
        v.complete.store(true, Ordering::Release);
        v.retired.store(true, Ordering::Release);
        self.retired
            .push(v)
            .unwrap_or_else(|_| unreachable!("Bounded NES retirement"))
    }
    fn publish(&self, r: &Renderer, c: Option<&Prepared>) {
        let s = r.state();
        self.revision.fetch_add(1, Ordering::AcqRel);
        for (index, out) in self.registers.iter().enumerate() {
            let value = if (0x40..0x80).contains(&index) {
                s.wave[index - 0x40]
            } else {
                s.registers[index]
            };
            out.store(value, Ordering::Relaxed);
        }
        for (out, value) in self.modulation.iter().zip(s.modulation) {
            out.store(value, Ordering::Relaxed);
        }
        for (out, value) in self.coverage.iter().zip(r.coverage.iter().copied()) {
            out.store(value, Ordering::Relaxed);
        }
        self.enabled.store(s.enable_mask as u8, Ordering::Relaxed);
        self.base_clock
            .store(s.base_clock.to_bits(), Ordering::Relaxed);
        self.fds_enabled
            .store(s.fds_enabled != 0, Ordering::Relaxed);
        self.fds_gain.store(r.fds_gain, Ordering::Relaxed);
        self.fds_master.store(r.fds_master, Ordering::Relaxed);
        self.active.store(r.active(), Ordering::Relaxed);
        self.clock.store(s.clock.to_bits(), Ordering::Relaxed);
        self.mute.store(s.mute_mask as u8, Ordering::Relaxed);
        self.rate.store(r.rate, Ordering::Relaxed);
        self.held.store(
            r.owners
                .iter()
                .enumerate()
                .filter(|(_, v)| v.is_some_and(|l| l.audition))
                .fold(0, |bits, (ch, _)| bits | (1 << ch)),
            Ordering::Relaxed,
        );
        if let Some(c) = c {
            c.complete.store(true, Ordering::Release)
        }
        self.revision.fetch_add(1, Ordering::Release);
    }
    fn memory_ranges(&self) -> Vec<MemoryRange> {
        let mut ranges = Vec::new();
        let mut address = 0;
        while address < RAM {
            if self.coverage[address / 64].load(Ordering::Relaxed) & (1 << (address % 64)) == 0 {
                address += 1;
                continue;
            }
            let start = address;
            while address < RAM
                && self.coverage[address / 64].load(Ordering::Relaxed) & (1 << (address % 64)) != 0
            {
                address += 1;
            }
            ranges.push(MemoryRange {
                address: start as u32,
                end: address as u32,
            });
        }
        ranges
    }
    pub fn status(&self, frame: u64) -> Result<Status, String> {
        let mut c = self
            .control
            .lock()
            .map_err(|_| "Native NES control unavailable")?;
        self.collect(&mut c);
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() > std::time::Duration::from_millis(50) {
                return Err("Native NES snapshot changing; retry".into());
            }
            let before = self.revision.load(Ordering::Acquire);
            if before & 1 != 0 {
                std::thread::yield_now();
                continue;
            }
            let active = self.active.load(Ordering::Relaxed);
            let held = self.held.load(Ordering::Relaxed);
            let out = Status {
                frame,
                sample_rate: self.rate.load(Ordering::Relaxed),
                revision: before / 2,
                nes: ChipStatus {
                    clock: f64::from_bits(self.clock.load(Ordering::Relaxed)),
                    registers: self
                        .registers
                        .iter()
                        .map(|v| v.load(Ordering::Relaxed))
                        .collect(),
                    modulation: self
                        .modulation
                        .iter()
                        .map(|v| v.load(Ordering::Relaxed))
                        .collect(),
                    enabled: self.enabled.load(Ordering::Relaxed),
                    base_clock: f64::from_bits(self.base_clock.load(Ordering::Relaxed)),
                    fds_enabled: self.fds_enabled.load(Ordering::Relaxed),
                    fds_gain: self.fds_gain.load(Ordering::Relaxed),
                    fds_master: self.fds_master.load(Ordering::Relaxed),
                    memory_ranges: self.memory_ranges(),
                    dmc_active: active & 16 != 0,
                    fds_active: active & 32 != 0,
                    gates: std::array::from_fn(|ch| active & (1 << ch) != 0),
                    held: (0..6).filter(|ch| held & (1 << ch) != 0).collect(),
                    source_mute_mask: self.mute.load(Ordering::Relaxed),
                    pending_commands: c
                        .prepared
                        .iter()
                        .filter(|v| !v.complete.load(Ordering::Relaxed))
                        .filter_map(|v| v.id)
                        .collect(),
                },
            };
            std::sync::atomic::fence(Ordering::Acquire);
            if self.revision.load(Ordering::Relaxed) == before {
                return Ok(out);
            }
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
    current_group: Option<u64>,
    ptr: *mut c_void,
    coverage: Box<[u64; COVERAGE]>,
    owners: [Option<Lease>; 6],
    fds_gain: u8,
    fds_master: u8,
    gains: [f32; 2],
    targets: [f32; 2],
    ramp: u32,
    rate: u32,
}
unsafe impl Send for Renderer {}
impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe {
            native_nes_destroy(self.ptr);
        }
    }
}
impl Renderer {
    pub fn new(rate: u32, clock: f64, fds: bool) -> Result<Self, String> {
        if !clock.is_finite() || !(1_600_000.0..=1_900_000.0).contains(&clock) {
            return Err("Native NES constructor clock must be 1600000–1900000 Hz".into());
        }
        let ptr = unsafe { native_nes_create(rate, clock, fds) };
        if ptr.is_null() {
            return Err("Could not allocate Native NES/FDS core".into());
        }
        Ok(Self {
            current_group: None,
            ptr,
            coverage: Box::new([0; COVERAGE]),
            owners: [None; 6],
            fds_gain: 32,
            fds_master: 0,
            gains: [1.0; 2],
            targets: [1.0; 2],
            ramp: 0,
            rate,
        })
    }
    fn state(&self) -> CoreState {
        let mut state = CoreState::default();
        unsafe {
            native_nes_get_state(self.ptr, &mut state);
        }
        state
    }
    fn manual_mask(&self, voice: Option<u64>, audition: bool) -> u8 {
        let mut mask = 0;
        for (channel, lease) in self.owners.iter().enumerate() {
            if lease.is_some_and(|lease| lease.audition && (!audition || lease.voice != voice)) {
                mask |= 1 << channel;
            }
        }
        mask
    }
    fn write(
        &mut self,
        op: Op,
        owner: Option<Owner>,
        voice: Option<u64>,
        audition: bool,
        preserve: bool,
    ) {
        let manual = self.manual_mask(voice, audition);
        let channel = match op {
            Op::Write { address, .. } => address_channel(address),
            Op::Enable { channel, .. } => Some(channel),
        };
        if channel.is_some_and(|channel| manual & (1 << channel) != 0) {
            return;
        }
        if channel.is_some_and(|ch| ch == 5 && self.state().fds_enabled == 0) {
            return;
        }
        match op {
            Op::Enable { channel, enabled } => {
                if !enabled
                    && self.owners[channel as usize].is_some_and(|lease| {
                        owner.is_some() && lease.owner != owner
                            || self.current_group.is_some() && lease.group != self.current_group
                            || voice.is_some() && lease.voice != voice
                    })
                {
                    return;
                }
                unsafe {
                    // An explicit DMC trigger may replace a nonmanual owner's
                    // stream. Its earlier off was correctly ignored as foreign;
                    // reproduce that off now that ownership is being taken.
                    if channel == 4 && enabled && !preserve && self.foreign(4, owner, voice) {
                        native_nes_enable_channel(self.ptr, 4, false);
                    }
                    native_nes_enable_channel(self.ptr, channel, enabled);
                }
                if !preserve {
                    self.owners[channel as usize] = enabled.then_some(Lease {
                        group: self.current_group,
                        owner,
                        voice,
                        audition,
                    });
                }
            }
            Op::Write { address, mut value } => {
                if address == 0x4017
                    && (manual != 0 || (0..6).any(|ch| self.foreign(ch, owner, voice)))
                {
                    return;
                }
                if address_channel(address)
                    .is_some_and(|ch| self.foreign(ch as usize, owner, voice))
                    && (address == 0x4011 && value == 0
                        || address == 0x4023 && value & 2 == 0
                        || address == 0x4080 && value & 63 == 0
                        || address == 0x4083 && value & 128 != 0
                        || address == 0x4089 && value & 128 != 0
                        || address == 0x4000 && value & 15 == 0
                        || address == 0x4004 && value & 15 == 0
                        || address == 0x4008 && value & 127 == 0
                        || address == 0x400c && value & 15 == 0)
                {
                    return;
                }
                let mut enable_protect = 0;
                if address == 0x4015 {
                    let state = self.state();
                    let current = state.enable_mask as u8;
                    let mut protect = manual & 31;
                    for channel in 0..5 {
                        if current & (1 << channel) != 0
                            && (value & (1 << channel) == 0 || channel == 4)
                            && self.owners[channel].is_some_and(|lease| {
                                owner.is_some() && lease.owner != owner
                                    || self.current_group.is_some()
                                        && lease.group != self.current_group
                                    || voice.is_some() && lease.voice != voice
                            })
                        {
                            protect |= 1 << channel;
                        }
                    }
                    value = (value & !protect) | (current & protect);
                    enable_protect = protect;
                    if !preserve {
                        for channel in 0..5 {
                            if protect & (1 << channel) == 0 {
                                self.owners[channel] = if value & (1 << channel) != 0 {
                                    if current & (1 << channel) != 0 {
                                        self.owners[channel]
                                    } else {
                                        Some(Lease {
                                            group: self.current_group,
                                            owner,
                                            voice,
                                            audition,
                                        })
                                    }
                                } else {
                                    None
                                };
                            }
                        }
                    }
                }
                if address == 0x4083
                    && value & 128 != 0
                    && self.owners[5].is_some_and(|lease| {
                        owner.is_some() && lease.owner != owner
                            || self.current_group.is_some() && lease.group != self.current_group
                            || voice.is_some() && lease.voice != voice
                    })
                {
                    return;
                }
                unsafe {
                    if address == 0x4015 {
                        native_nes_write_enable(self.ptr, value, enable_protect);
                    } else {
                        native_nes_write(self.ptr, address, value);
                    }
                }
                if !preserve && matches!(address, 0x4003 | 0x4007 | 0x400b | 0x400f) {
                    let index = ((address - 0x4000) / 4) as usize;
                    if self.active() & (1 << index) != 0 {
                        self.owners[index] = Some(Lease {
                            group: self.current_group,
                            owner,
                            voice,
                            audition,
                        });
                    }
                }
                if address == 0x4080 && value & 128 != 0 && value & 63 != 0 {
                    self.fds_gain = (value & 63).min(32);
                }
                if address == 0x4089 {
                    self.fds_master = value & 3;
                }
                if address == 0x4083 && !preserve {
                    self.owners[5] = if value & 128 == 0 {
                        Some(Lease {
                            group: self.current_group,
                            owner,
                            voice,
                            audition,
                        })
                    } else {
                        None
                    };
                }
            }
        }
    }
    fn active(&self) -> u8 {
        unsafe { native_nes_active_mask(self.ptr) as u8 }
    }
    fn foreign(&self, index: usize, owner: Option<Owner>, voice: Option<u64>) -> bool {
        self.owners[index].is_some_and(|l| {
            owner.is_some() && l.owner != owner
                || self.current_group.is_some() && l.group != self.current_group
                || voice.is_some() && l.voice != voice
        })
    }
    fn release(&mut self, channel: usize) {
        unsafe {
            native_nes_stop_channel(self.ptr, channel as u8);
        }
        self.owners[channel] = None;
    }
    fn reconcile(&mut self) -> bool {
        let active = self.active();
        let mut changed = false;
        for (ch, lease) in self.owners.iter_mut().enumerate() {
            if active & (1 << ch) == 0 && lease.is_some() {
                *lease = None;
                changed = true;
            }
        }
        changed
    }
    pub fn apply(&mut self, command: &Prepared, owner: Option<Owner>, shared: &Shared) {
        self.reconcile();
        if !command.audition
            && matches!(command.edit, Edit::Clock(_))
            && self.manual_mask(None, false) != 0
        {
            command.clock_rejected.store(true, Ordering::Release);
            shared.publish(self, Some(command));
            return;
        }
        if !command.audition
            && matches!(command.edit, Edit::Memory(..))
            && self.manual_mask(None, false) & 16 != 0
        {
            command.memory_rejected.store(true, Ordering::Release);
            shared.publish(self, Some(command));
            return;
        }
        self.current_group = command.group;
        match &command.edit {
            Edit::Op(op) => self.write(*op, owner, None, command.audition, false),
            Edit::Memory(address, data) => {
                unsafe {
                    native_nes_load(self.ptr, data.as_ptr(), *address, data.len() as u32);
                }
                let mut cursor = *address as usize;
                let end = cursor + data.len();
                while cursor < end {
                    let bits = (64 - cursor % 64).min(end - cursor);
                    let mask = if bits == 64 {
                        u64::MAX
                    } else {
                        ((1u64 << bits) - 1) << (cursor % 64)
                    };
                    self.coverage[cursor / 64] |= mask;
                    cursor += bits;
                }
            }
            Edit::Reset if self.manual_mask(None, command.audition) == 0 => {
                unsafe {
                    native_nes_reset(self.ptr);
                }
                self.coverage.fill(0);
                self.owners.fill(None);
                self.fds_gain = 32;
                self.fds_master = 0;
            }
            Edit::Reset => {}
            Edit::SourceMute(mask) => unsafe {
                native_nes_set_mute(self.ptr, *mask);
            },
            Edit::CancelMainOwner(target) => {
                for ch in 0..6 {
                    if self.owners[ch].is_some_and(|l| !l.audition && l.group == Some(*target)) {
                        self.release(ch);
                    }
                }
            }
            Edit::Stop => {
                if owner.is_none()
                    && command.group.is_none()
                    && (command.audition || self.manual_mask(None, false) == 0)
                {
                    for channel in 0..6 {
                        self.release(channel);
                    }
                } else {
                    for channel in 0..6 {
                        if self.owners[channel].is_some_and(|lease| {
                            (!lease.audition || command.audition)
                                && (owner.is_none() || lease.owner == owner)
                                && (command.group.is_none() || lease.group == command.group)
                        }) {
                            self.release(channel);
                        }
                    }
                }
            }
            Edit::Clock(hz) => unsafe {
                native_nes_set_clock(self.ptr, *hz);
            },
            Edit::Mix(volume, pan) => {
                self.targets = [
                    *volume * if *pan > 0.0 { 1.0 - *pan } else { 1.0 },
                    *volume * if *pan < 0.0 { 1.0 + *pan } else { 1.0 },
                ];
                self.ramp = (self.rate / 200).max(1);
            }
            Edit::Batch(ops, channel, voice, action, fds_gain, fds_master) => {
                let index = *channel as usize;
                let current = self.owners[index];
                let allowed = match action {
                    LeaseAction::On => {
                        command.audition || current.is_none_or(|lease| !lease.audition)
                    }
                    LeaseAction::Off => current.is_some_and(|lease| {
                        lease.owner == owner
                            && (command.group.is_none() || lease.group == command.group)
                            && (voice.is_none() || lease.voice == *voice)
                            && (!lease.audition || command.audition)
                    }),
                    LeaseAction::Edit => current.is_none_or(|lease| {
                        !lease.audition || command.audition && lease.voice == *voice
                    }),
                };
                if allowed {
                    if *action == LeaseAction::On {
                        self.owners[index] = Some(Lease {
                            group: self.current_group,
                            owner,
                            voice: *voice,
                            audition: command.audition,
                        });
                    }
                    for op in ops {
                        self.write(
                            *op,
                            owner,
                            *voice,
                            command.audition,
                            *action == LeaseAction::Edit,
                        );
                    }
                    if *action == LeaseAction::On {
                        self.owners[index] = Some(Lease {
                            group: self.current_group,
                            owner,
                            voice: *voice,
                            audition: command.audition,
                        });
                    }
                    if *action == LeaseAction::Off {
                        self.release(index);
                    }
                    if let Some(gain) = fds_gain {
                        self.fds_gain = *gain;
                    }
                    if let Some(master) = fds_master {
                        self.fds_master = *master;
                    }
                }
            }
        }
        self.reconcile();
        shared.publish(self, Some(command));
        self.current_group = None;
    }
    pub fn publish(&self, shared: &Shared) {
        shared.publish(self, None);
    }
    pub fn cancel_invalid(&mut self, valid: impl Fn(Option<Owner>) -> bool, shared: &Shared) {
        let mut changed = self.reconcile();
        for channel in 0..6 {
            if self.owners[channel].is_some_and(|lease| !valid(lease.owner)) {
                self.release(channel);
                changed = true;
            }
        }
        if changed || self.active() != shared.active.load(Ordering::Acquire) {
            shared.publish(self, None);
        }
    }
    #[cfg(test)]
    pub fn dmc_status(&self) -> (u32, bool) {
        (self.state().dmc_remaining, self.active() & 16 != 0)
    }
    #[cfg(test)]
    pub fn memory_byte(&self, address: u16) -> u8 {
        unsafe { native_nes_read_memory(self.ptr, address) }
    }
    pub fn mix(&mut self, pcm: &mut [f32; 2]) {
        let mut sample = [0.0; 2];
        unsafe {
            native_nes_sample(self.ptr, &mut sample[0], &mut sample[1]);
        }
        if self.ramp > 0 {
            for channel in 0..2 {
                self.gains[channel] +=
                    (self.targets[channel] - self.gains[channel]) / self.ramp as f32;
            }
            self.ramp -= 1;
        }
        for channel in 0..2 {
            pcm[channel] += sample[channel] * self.gains[channel];
        }
    }
}
