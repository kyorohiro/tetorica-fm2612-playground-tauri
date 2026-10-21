//! Callback-owned NES APU + FDS. Prepared uploads/batches are retained by the
//! control registry, so applying or cancelling an event never frees a Vec.
use super::{Owner, CAPACITY, MAX_SAFE_INTEGER};
use crossbeam_queue::ArrayQueue;
use serde::{Deserialize, Serialize};
use std::{
    ffi::c_void,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering},
        Arc, Mutex,
    },
};

pub const CLOCK: u32 = 1_789_773;
const RAM: usize = 65_536;
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
    fn native_nes_stop(ptr: *mut c_void);
    fn native_nes_write(ptr: *mut c_void, address: u16, value: u8);
    fn native_nes_enable(ptr: *mut c_void, channel: u8, enabled: bool);
    fn native_nes_load(ptr: *mut c_void, bytes: *const u8, offset: u32, size: u32) -> i32;
    fn native_nes_set_clock(ptr: *mut c_void, hz: u32) -> i32;
    fn native_nes_sample(ptr: *mut c_void, left: *mut f32, right: *mut f32);
    fn native_nes_get_state(ptr: *mut c_void, out: *mut CoreState);
    #[cfg(test)]
    fn native_nes_read_memory(ptr: *mut c_void, address: u16) -> u8;
}
#[derive(Debug, Deserialize)]
pub struct Message {
    #[serde(default)]
    pub id: Option<u64>,
    #[serde(default)]
    pub audition: bool,
    #[serde(flatten)]
    pub method: Method,
}
#[derive(Debug, Deserialize)]
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
    LoadMemory {
        address: u32,
        data: Vec<u8>,
    },
    Reset,
    StopVoices,
    Clock {
        hz: u32,
    },
    Mix {
        volume: f32,
        pan: f32,
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
pub enum Edit {
    Op(Op),
    Load(u32, Vec<u8>),
    Reset,
    Stop,
    Clock(u32),
    Mix(f32, f32),
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
    edit: Edit,
    complete: AtomicBool,
}
fn valid_lease(value: Option<u64>) -> bool {
    value.is_none_or(|v| v > 0 && v <= MAX_SAFE_INTEGER)
}
fn address_channel(address: u16) -> Option<u8> {
    match address {
        0x4000..=0x4013 => Some(((address - 0x4000) / 4) as u8),
        0x4023 | 0x4040..=0x408a => Some(5),
        _ => None,
    }
}
fn validate_op(op: Op) -> Result<(), String> {
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
    pub fn prepare(&self) -> Result<Arc<Prepared>, String> {
        if !valid_lease(self.id) {
            return Err("Invalid Native NES command ID".into());
        }
        let edit = match &self.method {
            Method::Write { address, value } => {
                let op = Op::Write {
                    address: *address,
                    value: *value,
                };
                validate_op(op)?;
                Edit::Op(op)
            }
            Method::Enable { channel, enabled } => {
                let op = Op::Enable {
                    channel: *channel,
                    enabled: *enabled,
                };
                validate_op(op)?;
                Edit::Op(op)
            }
            Method::LoadMemory { address, data } => {
                if *address as usize > RAM || data.len() > RAM - *address as usize {
                    return Err("Native NES upload is outside 64 KiB RAM".into());
                }
                Edit::Load(*address, data.to_vec())
            }
            Method::Reset => Edit::Reset,
            Method::StopVoices => Edit::Stop,
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
            Method::Batch {
                commands,
                channel,
                voice,
                action,
                fds_gain,
                fds_master,
            } => {
                if *channel > 5
                    || commands.len() > BATCH
                    || (commands.is_empty() && *action != LeaseAction::Edit)
                    || !valid_lease(*voice)
                    || (self.audition && *action != LeaseAction::Edit && voice.is_none())
                {
                    return Err("Invalid Native NES channel batch or voice lease".into());
                }
                if fds_gain.is_some_and(|v| v > 32)
                    || fds_master.is_some_and(|v| v > 3)
                    || *channel != 5 && (fds_gain.is_some() || fds_master.is_some())
                {
                    return Err("Invalid Native FDS configured gain or master".into());
                }
                for op in commands {
                    validate_op(*op)?;
                    let target = match op {
                        Op::Write { address, .. } => address_channel(*address),
                        Op::Enable { channel, .. } => Some(*channel),
                    };
                    if target.is_some_and(|target| target != *channel)
                        || matches!(
                            op,
                            Op::Write {
                                address: 0x4017,
                                ..
                            }
                        )
                    {
                        return Err("Native NES batch must target its declared channel".into());
                    }
                }
                Edit::Batch(
                    commands.to_vec(),
                    *channel,
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
            edit,
            complete: AtomicBool::new(false),
        }))
    }
}
impl Prepared {
    pub fn starts(&self) -> bool {
        fn starts(op: &Op) -> bool {
            matches!(
                op,
                Op::Enable { enabled: true, .. }
                    | Op::Write {
                        address: 0x4003 | 0x4007 | 0x400b | 0x400f,
                        ..
                    }
            ) || matches!(op,Op::Write{address:0x4015,value} if value&31!=0)
                || matches!(op,Op::Write{address:0x4083,value} if value&128==0)
        }
        match &self.edit {
            Edit::Op(op) => starts(op),
            Edit::Batch(ops, _, _, action, ..) => {
                *action == LeaseAction::On || ops.iter().any(starts)
            }
            _ => false,
        }
    }
    pub fn stops(&self) -> bool {
        matches!(self.edit, Edit::Stop)
    }
    fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + match &self.edit {
                Edit::Load(_, data) => data.len(),
                Edit::Batch(ops, ..) => ops.len() * std::mem::size_of::<Op>(),
                _ => 0,
            }
    }
}
pub struct Shared {
    revision: AtomicU64,
    registers: [AtomicU8; 0x90],
    modulation: [AtomicU8; 32],
    coverage: [AtomicU64; COVERAGE],
    clock: AtomicU64,
    enabled: AtomicU8,
    fds_gain: AtomicU8,
    fds_master: AtomicU8,
    dmc_active: AtomicBool,
    fds_active: AtomicBool,
    prepared: Mutex<Vec<Arc<Prepared>>>,
    retired: ArrayQueue<Arc<Prepared>>,
    highest_id: AtomicU64,
}
impl Default for Shared {
    fn default() -> Self {
        Self {
            revision: AtomicU64::new(0),
            registers: std::array::from_fn(|_| AtomicU8::new(0)),
            modulation: std::array::from_fn(|_| AtomicU8::new(0)),
            coverage: std::array::from_fn(|_| AtomicU64::new(0)),
            clock: AtomicU64::new((CLOCK as f64).to_bits()),
            enabled: AtomicU8::new(0),
            fds_gain: AtomicU8::new(32),
            fds_master: AtomicU8::new(0),
            dmc_active: AtomicBool::new(false),
            fds_active: AtomicBool::new(false),
            prepared: Mutex::new(Vec::new()),
            retired: ArrayQueue::new(CAPACITY),
            highest_id: AtomicU64::new(0),
        }
    }
}
#[derive(Debug, Serialize)]
pub struct MemoryRange {
    address: u32,
    end: u32,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub clock: f64,
    pub registers: Vec<u8>,
    pub enabled: u8,
    pub fds_gain: u8,
    pub fds_master: u8,
    pub modulation: Vec<u8>,
    pub memory_ranges: Vec<MemoryRange>,
    pub dmc_active: bool,
    pub fds_active: bool,
    pub frame: u64,
    pub revision: u64,
    pub pending_commands: Vec<u64>,
}
impl Shared {
    pub fn clock(&self) -> u32 {
        f64::from_bits(self.clock.load(Ordering::Acquire)) as u32
    }
    fn collect(&self, registry: &mut Vec<Arc<Prepared>>) {
        let mut retired = std::collections::HashSet::new();
        while let Some(command) = self.retired.pop() {
            retired.insert(Arc::as_ptr(&command) as usize);
        }
        registry.retain(|command| !retired.contains(&(Arc::as_ptr(command) as usize)));
    }
    pub fn retain(&self, commands: &[Arc<Prepared>]) -> Result<(), String> {
        let mut registry = self
            .prepared
            .lock()
            .map_err(|_| "Native NES control state is unavailable")?;
        self.collect(&mut registry);
        let uploads: usize = commands
            .iter()
            .map(|command| {
                if let Edit::Load(_, data) = &command.edit {
                    data.len()
                } else {
                    0
                }
            })
            .sum();
        if commands.len() + registry.len() > CAPACITY
            || uploads > RAM
            || registry
                .iter()
                .chain(commands)
                .map(|command| command.bytes())
                .sum::<usize>()
                > PENDING_BYTES
        {
            return Err(
                "Native NES pending command limit reached (64 KiB uploads/batch, 8 MiB pending)"
                    .into(),
            );
        }
        let mut ids = std::collections::HashSet::new();
        for command in registry.iter().chain(commands) {
            if command.id.is_some_and(|id| !ids.insert(id)) {
                return Err("Duplicate pending Native NES command ID".into());
            }
        }
        registry.extend(commands.iter().cloned());
        for command in commands {
            if let Some(id) = command.id {
                self.highest_id.fetch_max(id, Ordering::Release);
            }
        }
        Ok(())
    }
    pub fn wait_applied(&self, id: u64) -> Result<(), String> {
        if !valid_lease(Some(id)) || id > self.highest_id.load(Ordering::Acquire) {
            return Err("Unknown Native NES applied command ID".into());
        }
        let command = {
            let registry = self
                .prepared
                .lock()
                .map_err(|_| "Native NES control state is unavailable")?;
            registry
                .iter()
                .find(|command| command.id == Some(id))
                .cloned()
        };
        // A command no longer retained was already completed and reclaimed on
        // the control side. Frontend command IDs are monotonic within a session.
        if let Some(command) = command {
            let start = std::time::Instant::now();
            while !command.complete.load(Ordering::Acquire) {
                if start.elapsed() >= std::time::Duration::from_millis(100) {
                    return Err("Native NES command was not applied within 100 ms".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        Ok(())
    }
    pub fn retire(&self, command: Arc<Prepared>) {
        if !command.complete.load(Ordering::Relaxed) {
            self.revision.fetch_add(1, Ordering::AcqRel);
            command.complete.store(true, Ordering::Relaxed);
            self.revision.fetch_add(1, Ordering::Release);
        }
        self.retired
            .push(command)
            .unwrap_or_else(|_| unreachable!("Bounded NES command retirement"));
    }
    fn publish(&self, renderer: &Renderer, command: Option<&Prepared>, coverage_changed: bool) {
        let mut state = CoreState::default();
        unsafe {
            native_nes_get_state(renderer.ptr, &mut state);
        }
        self.revision.fetch_add(1, Ordering::AcqRel);
        for (index, out) in self.registers.iter().enumerate() {
            let value = if (0x40..0x80).contains(&index) {
                state.wave[index - 0x40]
            } else {
                state.registers[index]
            };
            out.store(value, Ordering::Relaxed);
        }
        for (out, value) in self.modulation.iter().zip(state.modulation) {
            out.store(value, Ordering::Relaxed);
        }
        if coverage_changed {
            for (out, value) in self.coverage.iter().zip(renderer.coverage) {
                out.store(value, Ordering::Relaxed);
            }
        }
        self.clock.store(state.clock.to_bits(), Ordering::Relaxed);
        self.enabled
            .store(state.enable_mask as u8, Ordering::Relaxed);
        self.fds_gain.store(renderer.fds_gain, Ordering::Relaxed);
        self.fds_master
            .store(renderer.fds_master, Ordering::Relaxed);
        self.dmc_active
            .store(state.dmc_remaining != 0, Ordering::Relaxed);
        self.fds_active
            .store(state.fds_active != 0, Ordering::Relaxed);
        if let Some(command) = command {
            command.complete.store(true, Ordering::Relaxed);
        }
        self.revision.fetch_add(1, Ordering::Release);
    }
    pub fn status(&self, frame: u64) -> Result<Status, String> {
        let mut registry = self
            .prepared
            .lock()
            .map_err(|_| "Native NES control state is unavailable")?;
        self.collect(&mut registry);
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() >= std::time::Duration::from_millis(50) {
                return Err("Native NES state changed while reading; retry".into());
            }
            let before = self.revision.load(Ordering::Acquire);
            if before & 1 != 0 {
                std::thread::yield_now();
                continue;
            }
            let registers = self
                .registers
                .iter()
                .map(|v| v.load(Ordering::Relaxed))
                .collect();
            let modulation = self
                .modulation
                .iter()
                .map(|v| v.load(Ordering::Relaxed))
                .collect();
            let coverage: [u64; COVERAGE] =
                std::array::from_fn(|i| self.coverage[i].load(Ordering::Relaxed));
            let mut memory_ranges = Vec::new();
            let mut address = 0;
            while address < RAM {
                if coverage[address / 64] & (1 << (address % 64)) == 0 {
                    address += 1;
                    continue;
                }
                let begin = address;
                while address < RAM && coverage[address / 64] & (1 << (address % 64)) != 0 {
                    address += 1;
                }
                memory_ranges.push(MemoryRange {
                    address: begin as u32,
                    end: address as u32,
                });
            }
            let status = Status {
                clock: f64::from_bits(self.clock.load(Ordering::Relaxed)),
                registers,
                enabled: self.enabled.load(Ordering::Relaxed),
                fds_gain: self.fds_gain.load(Ordering::Relaxed),
                fds_master: self.fds_master.load(Ordering::Relaxed),
                modulation,
                memory_ranges,
                dmc_active: self.dmc_active.load(Ordering::Relaxed),
                fds_active: self.fds_active.load(Ordering::Relaxed),
                frame,
                revision: before / 2,
                pending_commands: registry
                    .iter()
                    .filter(|v| !v.complete.load(Ordering::Relaxed))
                    .filter_map(|v| v.id)
                    .collect(),
            };
            std::sync::atomic::fence(Ordering::Acquire);
            if self.revision.load(Ordering::Relaxed) == before {
                return Ok(status);
            }
            std::thread::yield_now();
        }
    }
}
#[derive(Clone, Copy)]
struct Lease {
    owner: Option<Owner>,
    voice: Option<u64>,
    audition: bool,
}
pub struct Renderer {
    ptr: *mut c_void,
    coverage: [u64; COVERAGE],
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
    pub fn new(rate: u32) -> Result<Self, String> {
        let ptr = unsafe { native_nes_create(rate, CLOCK as f64, true) };
        if ptr.is_null() {
            return Err("Could not allocate Native NES/FDS core".into());
        }
        Ok(Self {
            ptr,
            coverage: [0; COVERAGE],
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
        match op {
            Op::Enable { channel, enabled } => {
                if !enabled
                    && self.owners[channel as usize].is_some_and(|lease| {
                        owner.is_some() && lease.owner != owner
                            || voice.is_some() && lease.voice != voice
                    })
                {
                    return;
                }
                unsafe {
                    native_nes_enable(self.ptr, channel, enabled);
                }
                if !preserve {
                    self.owners[channel as usize] = enabled.then_some(Lease {
                        owner,
                        voice,
                        audition,
                    });
                }
            }
            Op::Write { address, mut value } => {
                if address == 0x4017 && manual != 0 {
                    return;
                }
                if address == 0x4015 {
                    let state = self.state();
                    let current = state.enable_mask as u8;
                    let mut protect = manual & 31;
                    for channel in 0..5 {
                        if current & (1 << channel) != 0
                            && value & (1 << channel) == 0
                            && self.owners[channel].is_some_and(|lease| {
                                owner.is_some() && lease.owner != owner
                                    || voice.is_some() && lease.voice != voice
                            })
                        {
                            protect |= 1 << channel;
                        }
                    }
                    value = (value & !protect) | (current & protect);
                    // A protected completed DMC must not be restarted merely
                    // because another channel writes the global enable mask.
                    if protect & 16 != 0 && state.dmc_remaining == 0 {
                        value &= !16;
                    }
                    if !preserve {
                        for channel in 0..5 {
                            if protect & (1 << channel) == 0 {
                                self.owners[channel] = if value & (1 << channel) != 0 {
                                    if current & (1 << channel) != 0 {
                                        self.owners[channel]
                                    } else {
                                        Some(Lease {
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
                            || voice.is_some() && lease.voice != voice
                    })
                {
                    return;
                }
                unsafe {
                    native_nes_write(self.ptr, address, value);
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
    fn release(&mut self, channel: usize) {
        unsafe {
            if channel < 5 {
                native_nes_enable(self.ptr, channel as u8, false);
            } else {
                let control = self.state().registers[0x83];
                native_nes_write(self.ptr, 0x4083, control | 128);
                native_nes_write(self.ptr, 0x4080, 128);
            }
        }
        self.owners[channel] = None;
    }
    pub fn apply(&mut self, command: &Prepared, owner: Option<Owner>, shared: &Shared) {
        match &command.edit {
            Edit::Op(op) => self.write(*op, owner, None, command.audition, false),
            Edit::Load(address, data) => {
                unsafe {
                    native_nes_load(self.ptr, data.as_ptr(), *address, data.len() as u32);
                }
                for address in *address as usize..*address as usize + data.len() {
                    self.coverage[address / 64] |= 1 << (address % 64);
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
            Edit::Stop => {
                if owner.is_none() {
                    unsafe {
                        native_nes_stop(self.ptr);
                    }
                    self.owners.fill(None);
                } else {
                    for channel in 0..6 {
                        if self.owners[channel].is_some_and(|lease| {
                            (!lease.audition || command.audition)
                                && (owner.is_none() || lease.owner == owner)
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
                            owner,
                            voice: *voice,
                            audition: command.audition,
                        });
                    }
                    if *action == LeaseAction::Off {
                        self.owners[index] = None;
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
        shared.publish(
            self,
            Some(command),
            matches!(command.edit, Edit::Load(..) | Edit::Reset),
        );
    }
    pub fn publish(&self, shared: &Shared) {
        shared.publish(self, None, false);
    }
    pub fn cancel_invalid(&mut self, valid: impl Fn(Option<Owner>) -> bool, shared: &Shared) {
        let mut changed = false;
        for channel in 0..6 {
            if self.owners[channel].is_some_and(|lease| !valid(lease.owner)) {
                self.release(channel);
                changed = true;
            }
        }
        if changed {
            shared.publish(self, None, false);
        }
    }
    pub fn stop(&mut self, shared: &Shared) {
        unsafe {
            native_nes_stop(self.ptr);
        }
        self.owners.fill(None);
        shared.publish(self, None, false);
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
