//! Callback-owned Neo Geo YM2610/YM2610B. Uploads and command vectors are retained and
//! reclaimed on the control thread, never allocated or freed by the callback.
use super::{Owner, CAPACITY, MAX_SAFE_INTEGER};
use crossbeam_queue::ArrayQueue;
use serde::{Deserialize, Serialize};
use std::{
    ffi::c_void,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering},
        Arc, Mutex,
    },
};
pub const CLOCK: u32 = 8_000_000;
const ROM: usize = 16 * 1024 * 1024;
pub const MAX_BATCH_UPLOAD_BYTES: usize = ROM * 2;
const UPLOAD: usize = 65536;
const BATCH: usize = 512;
const PENDING_BYTES: usize = 40 * 1024 * 1024;
const VOICES: usize = 16;
#[repr(C)]
struct CoreState {
    registers: [u8; 512],
    clock: u32,
    source_mute_mask: u32,
    status: u8,
    status_hi: u8,
    irq: u8,
    variant: u8,
}
impl Default for CoreState {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}
extern "C" {
    fn native_ym2610b_create(rate: u32, variant: u8) -> *mut c_void;
    fn native_ym2610b_destroy(ptr: *mut c_void);
    fn native_ym2610b_reset(ptr: *mut c_void);
    fn native_ym2610b_stop(ptr: *mut c_void);
    fn native_ym2610b_key_off(ptr: *mut c_void, voice: u8);
    fn native_ym2610b_write(ptr: *mut c_void, port: u8, register: u8, value: u8);
    fn native_ym2610b_sample(ptr: *mut c_void, left: *mut f32, right: *mut f32);
    fn native_ym2610b_set_clock(ptr: *mut c_void, hz: u32) -> i32;
    fn native_ym2610b_set_source_mute_mask(ptr: *mut c_void, mask: u32);
    fn native_ym2610b_load_rom(
        ptr: *mut c_void,
        kind: u8,
        data: *const u8,
        offset: u32,
        size: u32,
    ) -> i32;
    fn native_ym2610b_clear_rom(ptr: *mut c_void, kind: u8);
    fn native_ym2610b_get_state(ptr: *mut c_void, out: *mut CoreState);
    #[cfg(test)]
    fn native_ym2610b_copy_rom(
        ptr: *mut c_void,
        kind: u8,
        data: *mut u8,
        offset: u32,
        size: u32,
    ) -> i32;
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
        port: u8,
        register: u8,
        value: u8,
    },
    KeyOff {
        channel: u8,
    },
    Memory {
        kind: u8,
        offset: u32,
        data: Vec<u8>,
    },
    ClearMemory {
        kind: Option<u8>,
    },
    Reset,
    StopVoices,
    Clock {
        hz: u32,
    },
    SourceMute {
        mask: u32,
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
    },
}
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(tag = "method", rename_all = "kebab-case")]
pub enum Op {
    Write { port: u8, register: u8, value: u8 },
    KeyOff { channel: u8 },
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
    Memory(u8, u32, Vec<u8>),
    ClearMemory(Option<u8>),
    Reset,
    Stop,
    Clock(u32),
    Mute(u32),
    Mix(f32, f32),
    Batch(Vec<Op>, u8, Option<u64>, LeaseAction),
}
#[derive(Debug)]
pub struct Prepared {
    pub id: Option<u64>,
    audition: bool,
    edit: Edit,
    complete: AtomicBool,
}
fn valid_id(id: Option<u64>) -> bool {
    id.is_none_or(|id| id > 0 && id <= MAX_SAFE_INTEGER)
}
// OPN key-on register values are physical codes 0,1,2,4,5,6; the
// high-level APIs and leases use six compact voice slots 0..5 instead.
fn fm_key_channel(value: u8) -> Option<usize> {
    let low = value & 3;
    (low < 3).then_some(low as usize + if value & 4 != 0 { 3 } else { 0 })
}
fn op_channel(op: &Op) -> Option<usize> {
    match op {
        Op::KeyOff { channel } => Some(*channel as usize),
        Op::Write {
            port: 0,
            register: 0x28,
            value,
        } => fm_key_channel(*value),
        Op::Write {
            port: 0,
            register: 0..=5,
            ..
        } => match op {
            Op::Write { register, .. } => Some(6 + (*register / 2) as usize),
            _ => unreachable!(),
        },
        Op::Write {
            port: 0,
            register: 8..=10,
            ..
        } => match op {
            Op::Write { register, .. } => Some(6 + (*register - 8) as usize),
            _ => unreachable!(),
        },
        Op::Write {
            port: 1,
            register: 0x08..=0x0d | 0x10..=0x15 | 0x18..=0x1d | 0x20..=0x25 | 0x28..=0x2d,
            ..
        } => match op {
            Op::Write { register, .. } => Some(9 + (*register & 7) as usize),
            _ => unreachable!(),
        },
        Op::Write {
            port: 0,
            register: 0x10..=0x1b,
            ..
        } => Some(15),
        Op::Write {
            port: 0,
            register: 0xa8..=0xae,
            ..
        } if register_low(op) < 3 => Some(2),
        Op::Write {
            port,
            register: 0x30..=0xb6,
            ..
        } if register_low(op) < 3 => Some((*port as usize) * 3 + register_low(op) as usize),
        _ => None,
    }
}
fn register_low(op: &Op) -> u8 {
    match op {
        Op::Write { register, .. } => register & 3,
        _ => 3,
    }
}
fn validate_op(op: &Op) -> Result<(), String> {
    match op {
        Op::Write { port, .. } if *port > 1 => Err("Native YM2610B port must be 0 or 1".into()),
        Op::KeyOff { channel } if *channel >= VOICES as u8 => {
            Err("Native YM2610B voice must be 0–15".into())
        }
        _ => Ok(()),
    }
}
impl Message {
    pub fn prepare(&self) -> Result<Arc<Prepared>, String> {
        if !valid_id(self.id) {
            return Err("Invalid Native YM2610B command ID".into());
        }
        let edit = match &self.method {
            Method::Write {
                port,
                register,
                value,
            } => Edit::Op(Op::Write {
                port: *port,
                register: *register,
                value: *value,
            }),
            Method::KeyOff { channel } => Edit::Op(Op::KeyOff { channel: *channel }),
            Method::Memory { kind, offset, data } => {
                if *kind > 1
                    || data.is_empty()
                    || data.len() > UPLOAD
                    || (*offset as usize)
                        .checked_add(data.len())
                        .is_none_or(|end| end > ROM)
                {
                    return Err("Native YM2610B upload exceeds ROM bounds (16 MiB per bank, 64 KiB per command)".into());
                }
                Edit::Memory(*kind, *offset, data.clone())
            }
            Method::ClearMemory { kind } => {
                if kind.is_some_and(|kind| kind > 1) {
                    return Err("Native YM2610B ROM kind must be 0 (A) or 1 (B)".into());
                }
                Edit::ClearMemory(*kind)
            }
            Method::Reset => Edit::Reset,
            Method::StopVoices => Edit::Stop,
            Method::Clock { hz } => {
                if !(100_000..=100_000_000).contains(hz) {
                    return Err("Native YM2610B clock must be 100000–100000000 Hz".into());
                }
                Edit::Clock(*hz)
            }
            Method::SourceMute { mask } => {
                if *mask > 7 {
                    return Err("Native YM2610B source mask must be 0–7".into());
                }
                Edit::Mute(*mask)
            }
            Method::Mix { volume, pan } => {
                if !volume.is_finite()
                    || !(0.0..=4.0).contains(volume)
                    || !pan.is_finite()
                    || !(-1.0..=1.0).contains(pan)
                {
                    return Err("Native YM2610B mixer volume must be 0–4, pan −1–1".into());
                }
                Edit::Mix(*volume, *pan)
            }
            Method::Batch {
                commands,
                channel,
                voice,
                action,
            } => {
                if *channel >= VOICES as u8
                    || commands.is_empty()
                    || commands.len() > BATCH
                    || !valid_id(*voice)
                    || self.audition && *action != LeaseAction::Edit && voice.is_none()
                {
                    return Err("Invalid Native YM2610B batch or voice lease".into());
                }
                for op in commands {
                    validate_op(op)?;
                    if op_channel(op).is_some_and(|target| target != *channel as usize) {
                        return Err("Native YM2610B batch must target its declared voice".into());
                    }
                    if let Op::Write {
                        port: 1,
                        register: 0,
                        value,
                    } = op
                    {
                        if !(9..15).contains(channel) || value & 0x3f & !(1 << (*channel - 9)) != 0
                        {
                            return Err(
                                "Native YM2610B ADPCM-A batch must target its declared voice"
                                    .into(),
                            );
                        }
                    }
                    if matches!(
                        op,
                        Op::Write {
                            port: 0,
                            register: 7,
                            ..
                        }
                    ) && !(6..9).contains(channel)
                    {
                        return Err("Native YM2610B mixer batch must target an SSG voice".into());
                    }
                }
                Edit::Batch(commands.clone(), *channel, *voice, *action)
            }
        };
        if let Edit::Op(op) = &edit {
            validate_op(op)?;
        }
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
            match op {
                Op::Write {
                    port: 0,
                    register: 0x28,
                    value,
                } => value & 0xf0 != 0,
                Op::Write {
                    port: 0,
                    register: 8..=10,
                    value,
                } => value & 31 != 0,
                Op::Write {
                    port: 1,
                    register: 0,
                    value,
                } => value & 0x80 == 0 && value & 0x3f != 0,
                Op::Write {
                    port: 0,
                    register: 0x10,
                    value,
                } => value & 0x80 != 0 && value & 1 == 0,
                _ => false,
            }
        }
        match &self.edit {
            Edit::Op(op) => starts(op),
            Edit::Batch(ops, _, _, action) => *action == LeaseAction::On || ops.iter().any(starts),
            _ => false,
        }
    }
    pub fn stops(&self) -> bool {
        matches!(self.edit, Edit::Stop)
    }
    fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + match &self.edit {
                Edit::Memory(_, _, data) => data.len(),
                Edit::Batch(ops, ..) => ops.len() * std::mem::size_of::<Op>(),
                _ => 0,
            }
    }
}
pub struct Shared {
    revision: AtomicU64,
    registers: [AtomicU8; 512],
    clock: AtomicU32,
    mute: AtomicU32,
    status: AtomicU8,
    status_hi: AtomicU8,
    irq: AtomicU8,
    held: AtomicU32,
    rate: AtomicU32,
    variant: bool,
    rom_bytes: [AtomicU32; 2],
    prepared: Mutex<Vec<Arc<Prepared>>>,
    retired: ArrayQueue<Arc<Prepared>>,
    highest_id: AtomicU64,
}
impl Shared {
    pub fn new(variant: bool) -> Self {
        Self {
            revision: AtomicU64::new(0),
            registers: std::array::from_fn(|_| AtomicU8::new(0)),
            clock: AtomicU32::new(CLOCK),
            mute: AtomicU32::new(0),
            status: AtomicU8::new(0),
            status_hi: AtomicU8::new(0),
            irq: AtomicU8::new(0),
            held: AtomicU32::new(0),
            rate: AtomicU32::new(0),
            variant,
            rom_bytes: std::array::from_fn(|_| AtomicU32::new(0)),
            prepared: Mutex::new(Vec::new()),
            retired: ArrayQueue::new(CAPACITY),
            highest_id: AtomicU64::new(0),
        }
    }
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub clock: u32,
    pub sample_rate: u32,
    pub source_mute_mask: u32,
    pub registers: Vec<Vec<u8>>,
    pub status: u8,
    pub status_hi: u8,
    pub irq: bool,
    pub held: Vec<u8>,
    pub variant: &'static str,
    pub adpcm_a_rom_bytes: u32,
    pub adpcm_b_rom_bytes: u32,
    pub frame: u64,
    pub revision: u64,
    pub pending_commands: Vec<u64>,
}
impl Shared {
    pub fn variant(&self) -> &'static str {
        if self.variant {
            "ym2610b"
        } else {
            "ym2610"
        }
    }
    pub fn mode(&self) -> &'static str {
        if self.variant {
            "neogeo-b"
        } else {
            "neogeo"
        }
    }
    pub fn clock(&self) -> u32 {
        self.clock.load(Ordering::Acquire)
    }
    fn collect(&self, registry: &mut Vec<Arc<Prepared>>) {
        let mut retired = std::collections::HashSet::new();
        while let Some(command) = self.retired.pop() {
            retired.insert(Arc::as_ptr(&command) as usize);
        }
        registry.retain(|command| !retired.contains(&(Arc::as_ptr(command) as usize)));
    }
    pub fn retain(&self, commands: &[Arc<Prepared>]) -> Result<(), String> {
        if !self.variant
            && commands
                .iter()
                .any(|command| matches!(&command.edit, Edit::Batch(_, 0 | 3, _, _)))
        {
            return Err("Native YM2610 has four FM voices (physical channels 1, 2, 4, 5)".into());
        }
        let mut registry = self
            .prepared
            .lock()
            .map_err(|_| "Native YM2610B control state unavailable")?;
        self.collect(&mut registry);
        if registry.len() + commands.len() > CAPACITY
            || registry
                .iter()
                .chain(commands)
                .map(|v| v.bytes())
                .sum::<usize>()
                > PENDING_BYTES
        {
            return Err("Native YM2610B pending command limit reached (40 MiB)".into());
        }
        let mut ids = std::collections::HashSet::new();
        for command in registry.iter().chain(commands) {
            if command.id.is_some_and(|id| !ids.insert(id)) {
                return Err("Duplicate pending Native YM2610B command ID".into());
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
        if !valid_id(Some(id)) || id > self.highest_id.load(Ordering::Acquire) {
            return Err("Unknown Native YM2610B command ID".into());
        }
        let command = self
            .prepared
            .lock()
            .map_err(|_| "Native YM2610B control state unavailable")?
            .iter()
            .find(|v| v.id == Some(id))
            .cloned();
        if let Some(command) = command {
            let start = std::time::Instant::now();
            while !command.complete.load(Ordering::Acquire) {
                if start.elapsed() >= std::time::Duration::from_millis(100) {
                    return Err("Native YM2610B command was not applied within 100 ms".into());
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
            .unwrap_or_else(|_| unreachable!("Bounded YM2610B command retirement"));
    }
    fn publish(&self, renderer: &Renderer, command: Option<&Prepared>) {
        let state = renderer.state();
        self.revision.fetch_add(1, Ordering::AcqRel);
        for (out, value) in self.registers.iter().zip(state.registers) {
            out.store(value, Ordering::Relaxed);
        }
        self.clock.store(state.clock, Ordering::Relaxed);
        self.mute.store(state.source_mute_mask, Ordering::Relaxed);
        self.status.store(state.status, Ordering::Relaxed);
        self.status_hi.store(state.status_hi, Ordering::Relaxed);
        self.irq.store(state.irq, Ordering::Relaxed);
        let held = renderer
            .owners
            .iter()
            .enumerate()
            .fold(0, |mask, (ch, lease)| {
                mask | if lease.is_some_and(|v| v.audition) {
                    1 << ch
                } else {
                    0
                }
            });
        self.held.store(held, Ordering::Relaxed);
        self.rate.store(renderer.rate, Ordering::Relaxed);
        for (out, count) in self.rom_bytes.iter().zip(renderer.rom_bytes) {
            out.store(count, Ordering::Relaxed);
        }
        if let Some(command) = command {
            command.complete.store(true, Ordering::Relaxed);
        }
        self.revision.fetch_add(1, Ordering::Release);
    }
    pub fn status(&self, frame: u64) -> Result<Status, String> {
        let mut registry = self
            .prepared
            .lock()
            .map_err(|_| "Native YM2610B control state unavailable")?;
        self.collect(&mut registry);
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() >= std::time::Duration::from_millis(50) {
                return Err("Native YM2610B state changed while reading; retry".into());
            }
            let before = self.revision.load(Ordering::Acquire);
            if before & 1 != 0 {
                std::thread::yield_now();
                continue;
            }
            let held = self.held.load(Ordering::Relaxed);
            let status = Status {
                clock: self.clock.load(Ordering::Relaxed),
                sample_rate: self.rate.load(Ordering::Relaxed),
                source_mute_mask: self.mute.load(Ordering::Relaxed),
                registers: self
                    .registers
                    .chunks(256)
                    .map(|r| r.iter().map(|v| v.load(Ordering::Relaxed)).collect())
                    .collect(),
                status: self.status.load(Ordering::Relaxed),
                status_hi: self.status_hi.load(Ordering::Relaxed),
                irq: self.irq.load(Ordering::Relaxed) != 0,
                held: (0..16).filter(|ch| held & (1 << ch) != 0).collect(),
                variant: self.variant(),
                adpcm_a_rom_bytes: self.rom_bytes[0].load(Ordering::Relaxed),
                adpcm_b_rom_bytes: self.rom_bytes[1].load(Ordering::Relaxed),
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
    owners: [Option<Lease>; VOICES],
    gains: [f32; 2],
    targets: [f32; 2],
    ramp: u32,
    rate: u32,
    variant: bool,
    rom_bytes: [u32; 2],
}
unsafe impl Send for Renderer {}
impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe {
            native_ym2610b_destroy(self.ptr);
        }
    }
}
impl Renderer {
    pub fn new(rate: u32, variant: bool) -> Result<Self, String> {
        let ptr = unsafe { native_ym2610b_create(rate, variant as u8) };
        if ptr.is_null() {
            return Err("Could not allocate Native YM2610B core".into());
        }
        unsafe {
            native_ym2610b_set_clock(ptr, CLOCK);
        }
        Ok(Self {
            ptr,
            owners: [None; VOICES],
            gains: [1.0; 2],
            targets: [1.0; 2],
            ramp: 0,
            rate,
            variant,
            rom_bytes: [0; 2],
        })
    }
    fn state(&self) -> CoreState {
        let mut state = CoreState::default();
        unsafe {
            native_ym2610b_get_state(self.ptr, &mut state);
        }
        state
    }
    fn release(&mut self, ch: usize) {
        unsafe {
            native_ym2610b_key_off(self.ptr, ch as u8);
        }
        self.owners[ch] = None;
    }
    fn protected(&self, ch: usize, voice: Option<u64>, audition: bool) -> bool {
        self.owners[ch].is_some_and(|lease| lease.audition && (!audition || lease.voice != voice))
    }
    fn write(
        &mut self,
        op: &Op,
        owner: Option<Owner>,
        voice: Option<u64>,
        audition: bool,
        preserve: bool,
        declared: Option<usize>,
    ) {
        let channel = op_channel(op);
        if !self.variant && channel.is_some_and(|ch| ch == 0 || ch == 3) {
            return;
        }
        // Shared generators cannot be isolated per voice. Preserve manual
        // auditions against sequence changes; explicit UI edits remain global.
        let shared_group = match op {
            Op::Write {
                port: 0,
                register: 6 | 11..=13,
                ..
            } => Some(6..9),
            Op::Write {
                port: 1,
                register: 1,
                ..
            } => Some(9..15),
            Op::Write {
                port: 0,
                register: 0x22,
                ..
            } => Some(0..6),
            Op::Write {
                port: 0,
                register: 0x1c,
                ..
            } => Some(9..16),
            _ => None,
        };
        if !audition
            && shared_group
                .is_some_and(|mut group| group.any(|ch| self.protected(ch, voice, false)))
        {
            return;
        }
        if channel.is_some_and(|ch| self.protected(ch, voice, audition)) {
            return;
        }
        let off = match op {
            Op::KeyOff { .. } => true,
            Op::Write {
                port: 0,
                register: 0x28,
                value,
            } => value & 0xf0 == 0,
            Op::Write {
                port: 0,
                register: 8..=10,
                value,
            } => value & 31 == 0,
            Op::Write {
                port: 0,
                register: 0x10,
                value,
            } => value & 1 != 0,
            _ => false,
        };
        if off
            && channel.is_some_and(|ch| {
                self.owners[ch].is_some_and(|lease| {
                    owner.is_some() && lease.owner != owner
                        || voice.is_some() && lease.voice != voice
                })
            })
        {
            return;
        }
        match *op {
            Op::Write {
                port: 0,
                register: 0x27,
                mut value,
            } if !audition && self.protected(2, voice, false) => {
                let previous = self.state().registers[0x27];
                value = (value & 0x3f) | (previous & 0xc0);
                unsafe {
                    native_ym2610b_write(self.ptr, 0, 0x27, value);
                }
            }
            Op::KeyOff { channel } => unsafe {
                native_ym2610b_key_off(self.ptr, channel);
            },
            Op::Write {
                port: 0,
                register: 7,
                mut value,
            } => {
                let previous = self.state().registers[7];
                for ch in 6..9 {
                    if self.protected(ch, voice, audition)
                        || declared.is_some_and(|target| target != ch)
                    {
                        let bits = (1 << (ch - 6)) | (1 << (ch - 3));
                        value = (value & !bits) | (previous & bits);
                    }
                }
                unsafe {
                    native_ym2610b_write(self.ptr, 0, 7, value);
                }
            }
            Op::Write {
                port: 1,
                register: 0,
                mut value,
            } => {
                let stop = value & 0x80 != 0;
                for ch in 9..15 {
                    let current = self.owners[ch];
                    let stale = stop
                        && current.is_some_and(|lease| {
                            owner.is_some() && lease.owner != owner
                                || voice.is_some() && lease.voice != voice
                        });
                    if self.protected(ch, voice, audition)
                        || declared.is_some_and(|target| target != ch)
                        || stale
                    {
                        value &= !(1 << (ch - 9));
                    }
                }
                unsafe {
                    native_ym2610b_write(self.ptr, 1, 0, value);
                }
                if !preserve {
                    for ch in 9..15 {
                        if value & (1 << (ch - 9)) != 0 {
                            self.owners[ch] = if stop {
                                None
                            } else {
                                Some(Lease {
                                    owner,
                                    voice,
                                    audition,
                                })
                            };
                        }
                    }
                }
            }
            Op::Write {
                port,
                register,
                value,
            } => unsafe {
                native_ym2610b_write(self.ptr, port, register, value);
            },
        }
        if !preserve {
            if let Some(ch) = channel {
                if off {
                    self.owners[ch] = None;
                } else if matches!(
                    op,
                    Op::Write {
                        port: 0,
                        register: 0x28 | 8..=10,
                        ..
                    } | Op::Write {
                        port: 0,
                        register: 0x10,
                        ..
                    }
                ) {
                    self.owners[ch] = Some(Lease {
                        owner,
                        voice,
                        audition,
                    });
                }
            }
        }
    }
    pub fn apply(&mut self, command: &Prepared, owner: Option<Owner>, shared: &Shared) {
        match &command.edit {
            Edit::Op(op) => self.write(op, owner, None, command.audition, false, None),
            Edit::Memory(kind, offset, data) => unsafe {
                native_ym2610b_load_rom(self.ptr, *kind, data.as_ptr(), *offset, data.len() as u32);
                self.rom_bytes[*kind as usize] =
                    self.rom_bytes[*kind as usize].max(*offset + data.len() as u32);
            },
            Edit::ClearMemory(kind) => {
                for bank in 0..2 {
                    if kind.is_none_or(|kind| kind == bank) {
                        unsafe {
                            native_ym2610b_clear_rom(self.ptr, bank);
                        }
                        self.rom_bytes[bank as usize] = 0;
                    }
                }
            }
            Edit::Reset => {
                if command.audition || !self.owners.iter().flatten().any(|lease| lease.audition) {
                    unsafe {
                        native_ym2610b_reset(self.ptr);
                    }
                    self.owners.fill(None);
                }
            }
            Edit::Stop => {
                if owner.is_none() {
                    unsafe {
                        native_ym2610b_stop(self.ptr);
                    }
                    self.owners.fill(None);
                } else {
                    for ch in 0..VOICES {
                        if self.owners[ch].is_some_and(|lease| {
                            lease.owner == owner && (!lease.audition || command.audition)
                        }) {
                            self.release(ch);
                        }
                    }
                }
            }
            Edit::Clock(hz) => unsafe {
                native_ym2610b_set_clock(self.ptr, *hz);
            },
            Edit::Mute(mask) => unsafe {
                native_ym2610b_set_source_mute_mask(self.ptr, *mask);
            },
            Edit::Mix(volume, pan) => {
                self.targets = [
                    *volume * if *pan > 0.0 { 1.0 - *pan } else { 1.0 },
                    *volume * if *pan < 0.0 { 1.0 + *pan } else { 1.0 },
                ];
                self.ramp = (self.rate / 200).max(1);
            }
            Edit::Batch(ops, channel, voice, action) => {
                let ch = *channel as usize;
                let current = self.owners[ch];
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
                        self.owners[ch] = Some(Lease {
                            owner,
                            voice: *voice,
                            audition: command.audition,
                        });
                    }
                    for op in ops {
                        self.write(
                            op,
                            owner,
                            *voice,
                            command.audition,
                            *action == LeaseAction::Edit,
                            Some(ch),
                        );
                    }
                    if *action == LeaseAction::On {
                        self.owners[ch] = Some(Lease {
                            owner,
                            voice: *voice,
                            audition: command.audition,
                        });
                    }
                    if *action == LeaseAction::Off {
                        self.owners[ch] = None;
                    }
                }
            }
        }
        shared.publish(self, Some(command));
    }
    pub fn publish(&self, shared: &Shared) {
        shared.publish(self, None);
    }
    pub fn cancel_invalid(&mut self, valid: impl Fn(Option<Owner>) -> bool, shared: &Shared) {
        let mut changed = false;
        for ch in 0..VOICES {
            if self.owners[ch].is_some_and(|lease| !valid(lease.owner)) {
                self.release(ch);
                changed = true;
            }
        }
        if changed {
            shared.publish(self, None);
        }
    }
    pub fn stop(&mut self, shared: &Shared) {
        unsafe {
            native_ym2610b_stop(self.ptr);
        }
        self.owners.fill(None);
        shared.publish(self, None);
    }
    pub fn mix(&mut self, pcm: &mut [f32; 2]) {
        let mut sample = [0.0; 2];
        unsafe {
            native_ym2610b_sample(self.ptr, &mut sample[0], &mut sample[1]);
        }
        if self.ramp > 0 {
            for ch in 0..2 {
                self.gains[ch] += (self.targets[ch] - self.gains[ch]) / self.ramp as f32;
            }
            self.ramp -= 1;
        }
        for ch in 0..2 {
            pcm[ch] += sample[ch] * self.gains[ch];
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    fn prepare(value: Value) -> Arc<Prepared> {
        serde_json::from_value::<Message>(value)
            .unwrap()
            .prepare()
            .unwrap()
    }
    fn apply(renderer: &mut Renderer, shared: &Shared, value: Value, owner: Option<Owner>) {
        let command = prepare(value);
        shared.retain(&[command.clone()]).unwrap();
        renderer.apply(&command, owner, shared);
        shared.retire(command);
    }
    fn state(shared: &Shared) -> Status {
        shared.status(0).unwrap()
    }
    fn write(port: u8, register: u8, value: u8) -> Value {
        json!({"method":"write","port":port,"register":register,"value":value})
    }
    fn manual(channel: u8, voice: u64, commands: Vec<Value>) -> Value {
        json!({"method":"batch","channel":channel,"voice":voice,"audition":true,"action":"on","commands":commands})
    }
    fn off(channel: u8, voice: u64) -> Value {
        json!({"method":"batch","channel":channel,"voice":voice,"audition":true,"action":"off","commands":[{"method":"key-off","channel":channel}]})
    }
    fn owner() -> Option<Owner> {
        Some(Owner {
            performance: 1,
            owner: 1,
        })
    }
    #[test]
    fn ym2610b_native_validates_both_rom_banks_batches_and_leases_before_mutation() {
        for value in [
            write(2, 0, 1),
            json!({"method":"key-off","channel":16}),
            json!({"method":"clock","hz":99999}),
            json!({"method":"source-mute","mask":8}),
            json!({"method":"mix","volume":5,"pan":0}),
            json!({"method":"memory","kind":0,"offset":ROM,"data":[1]}),
            json!({"method":"memory","kind":1,"offset":0,"data":vec![1;UPLOAD+1]}),
            json!({"method":"memory","kind":2,"offset":0,"data":[1]}),
            json!({"method":"clear-memory","kind":2}),
            manual(0, 1, vec![write(1, 0xa0, 1)]),
            manual(6, 1, vec![write(0, 2, 1)]),
            manual(9, 1, vec![write(1, 0, 3)]),
            manual(9, 1, vec![write(1, 9, 0xdf)]),
            manual(15, 1, vec![write(0, 0x28, 0xf0)]),
            json!({"method":"batch","channel":0,"audition":true,"action":"on","commands":[write(0,0x28,0xf0)]}),
        ] {
            assert!(serde_json::from_value::<Message>(value)
                .unwrap()
                .prepare()
                .is_err());
        }
        assert!(prepare(manual(5, MAX_SAFE_INTEGER, vec![write(0, 0x28, 0xf6)])).starts());
        assert!(prepare(manual(9, 1, vec![write(1, 0, 1)])).starts());
        assert!(prepare(manual(15, 1, vec![write(0, 0x10, 0xa0)])).starts());
        assert_eq!(fm_key_channel(0xf4), Some(3));
        assert_eq!(fm_key_channel(0xf6), Some(5));
        assert_eq!(fm_key_channel(0xf7), None);
    }
    #[test]
    fn ym2610b_native_variant_masks_physical_fm_voices_and_has_independent_empty_roms() {
        for variant in [false, true] {
            let shared = Shared::new(variant);
            let mut renderer = Renderer::new(48000, variant).unwrap();
            renderer.publish(&shared);
            let s = state(&shared);
            assert_eq!(s.variant, if variant { "ym2610b" } else { "ym2610" });
            assert_eq!(s.clock, CLOCK);
            assert_eq!(s.sample_rate, 48000);
            assert_eq!(s.adpcm_a_rom_bytes, 0);
            assert_eq!(s.adpcm_b_rom_bytes, 0);
            assert_eq!(s.registers.len(), 2);
            assert!(s.held.is_empty());
            assert_eq!(renderer.state().variant, variant as u8);
            for channel in 0..6 {
                let key = channel + if channel >= 3 { 1 } else { 0 };
                let command = prepare(manual(
                    channel,
                    channel as u64 + 1,
                    vec![write(0, 0x28, 0xf0 | key)],
                ));
                if variant || ![0, 3].contains(&channel) {
                    shared.retain(&[command.clone()]).unwrap();
                    renderer.apply(&command, None, &shared);
                    shared.retire(command);
                } else {
                    assert!(shared.retain(&[command]).is_err());
                }
            }
            assert_eq!(
                state(&shared).held,
                if variant {
                    vec![0, 1, 2, 3, 4, 5]
                } else {
                    vec![1, 2, 4, 5]
                }
            );
        }
    }
    #[test]
    fn ym2610b_native_manual_fm_leases_preserve_patch_lfo_ch3_mode_and_ignore_stale_off() {
        let shared = Shared::new(true);
        let mut renderer = Renderer::new(48000, true).unwrap();
        apply(
            &mut renderer,
            &shared,
            manual(
                2,
                41,
                vec![
                    write(0, 0x22, 9),
                    write(0, 0x27, 0x40),
                    write(0, 0x42, 9),
                    write(0, 0x28, 0xf2),
                ],
            ),
            None,
        );
        for op in [
            write(0, 0x42, 99),
            write(0, 0x28, 2),
            write(0, 0x22, 0),
            write(0, 0x27, 0),
        ] {
            apply(&mut renderer, &shared, op, owner());
        }
        apply(&mut renderer, &shared, write(1, 0x42, 33), owner());
        let s = state(&shared);
        assert_eq!(s.registers[0][0x42], 9);
        assert_eq!(s.registers[1][0x42], 33);
        assert_eq!(s.registers[0][0x22], 9);
        assert_eq!(s.registers[0][0x27] & 0xc0, 0x40);
        apply(
            &mut renderer,
            &shared,
            manual(2, 42, vec![write(0, 0x42, 12), write(0, 0x28, 0xf2)]),
            None,
        );
        apply(&mut renderer, &shared, off(2, 41), None);
        assert_eq!(state(&shared).held, vec![2]);
        apply(&mut renderer, &shared, off(2, 42), None);
        assert!(state(&shared).held.is_empty());
        assert_eq!(state(&shared).registers[0][0x28], 2);
    }
    #[test]
    fn ym2610b_native_ssg_and_adpcm_a_shared_gates_preserve_other_manual_channels() {
        let shared = Shared::new(true);
        let mut renderer = Renderer::new(48000, true).unwrap();
        apply(
            &mut renderer,
            &shared,
            manual(
                6,
                1,
                vec![write(0, 0, 80), write(0, 7, 0x3e), write(0, 8, 12)],
            ),
            None,
        );
        for op in [write(0, 7, 0x3f), write(0, 0, 5), write(0, 6, 25)] {
            apply(&mut renderer, &shared, op, owner());
        }
        let s = state(&shared);
        assert_eq!(s.registers[0][7] & 9, 8);
        assert_eq!(s.registers[0][0], 80);
        assert_eq!(s.registers[0][6], 0);
        apply(
            &mut renderer,
            &shared,
            manual(7, 2, vec![write(0, 7, 0x3d), write(0, 9, 8)]),
            None,
        );
        assert_eq!(state(&shared).registers[0][7] & 3, 0);
        apply(
            &mut renderer,
            &shared,
            manual(
                9,
                3,
                vec![
                    write(1, 1, 12),
                    write(1, 8, 0xdf),
                    write(1, 0x10, 20),
                    write(1, 0, 1),
                ],
            ),
            None,
        );
        for op in [
            write(1, 0, 0x83),
            write(1, 1, 30),
            write(1, 8, 0),
            write(1, 0x10, 0),
            write(0, 0x1c, 0xbf),
        ] {
            apply(&mut renderer, &shared, op, owner());
        }
        let s = state(&shared);
        assert_eq!(s.registers[1][1], 12);
        assert_eq!(s.registers[1][0], 0x82);
        assert_eq!(s.registers[1][8], 0xdf);
        assert_eq!(s.registers[1][0x10], 20);
        assert_eq!(s.registers[0][0x1c], 0);
        assert_eq!(s.held, vec![6, 7, 9]);
        apply(&mut renderer, &shared, off(9, 2), None);
        assert!(state(&shared).held.contains(&9));
        apply(&mut renderer, &shared, off(9, 3), None);
        assert!(!state(&shared).held.contains(&9));
    }
    #[test]
    fn ym2610b_native_adpcm_b_lease_and_upload_retirement_preserve_both_rom_banks() {
        let shared = Shared::new(true);
        let mut renderer = Renderer::new(48000, true).unwrap();
        apply(
            &mut renderer,
            &shared,
            manual(15, 99, vec![write(0, 0x11, 0xc0), write(0, 0x10, 0xa0)]),
            None,
        );
        for op in [write(0, 0x11, 0), write(0, 0x10, 1), write(0, 0x1c, 0xff)] {
            apply(&mut renderer, &shared, op, owner());
        }
        let s = state(&shared);
        assert_eq!(s.registers[0][0x11], 0xc0);
        assert_eq!(s.registers[0][0x10], 0xa0);
        assert_eq!(s.registers[0][0x1c], 0);
        for kind in 0..2 {
            let command = prepare(
                json!({"id":50+kind,"method":"memory","kind":kind,"offset":ROM-4,"data":[7+kind,8,9,10]}),
            );
            let weak = Arc::downgrade(&command);
            shared.retain(&[command.clone()]).unwrap();
            assert_eq!(state(&shared).pending_commands, vec![50 + kind as u64]);
            renderer.apply(&command, None, &shared);
            shared.retire(command);
            assert!(
                weak.upgrade().is_some(),
                "callback must retain upload allocation"
            );
            let s = state(&shared);
            assert!(s.pending_commands.is_empty());
            assert!(weak.upgrade().is_none());
            assert_eq!(
                if kind == 0 {
                    s.adpcm_a_rom_bytes
                } else {
                    s.adpcm_b_rom_bytes
                },
                ROM as u32
            );
            let mut bytes = [0; 4];
            assert_eq!(
                unsafe {
                    native_ym2610b_copy_rom(
                        renderer.ptr,
                        kind,
                        bytes.as_mut_ptr(),
                        ROM as u32 - 4,
                        4,
                    )
                },
                1
            );
            assert_eq!(bytes, [7 + kind, 8, 9, 10]);
        }
        apply(
            &mut renderer,
            &shared,
            json!({"method":"clear-memory","kind":0}),
            None,
        );
        let s = state(&shared);
        assert_eq!(s.adpcm_a_rom_bytes, 0);
        assert_eq!(s.adpcm_b_rom_bytes, ROM as u32);
        let mut bytes = [0; 4];
        unsafe {
            native_ym2610b_copy_rom(renderer.ptr, 1, bytes.as_mut_ptr(), ROM as u32 - 4, 4);
        };
        assert_eq!(bytes, [8, 8, 9, 10]);
        apply(
            &mut renderer,
            &shared,
            json!({"method":"clear-memory"}),
            None,
        );
        assert_eq!(state(&shared).adpcm_b_rom_bytes, 0);
    }
    #[test]
    fn ym2610b_native_stop_and_reset_retain_rom_clock_mix_source_mask_and_authored_ranges() {
        let shared = Shared::new(true);
        let mut renderer = Renderer::new(48000, true).unwrap();
        for value in [
            json!({"method":"memory","kind":0,"offset":10,"data":[23,42,99]}),
            json!({"method":"memory","kind":1,"offset":20,"data":[54,66]}),
            json!({"method":"clock","hz":7987200}),
            json!({"method":"source-mute","mask":5}),
            json!({"method":"mix","volume":0.7,"pan":0.5}),
            write(0, 0, 80),
            write(0, 7, 0x3e),
            write(0, 8, 12),
            write(0, 0x40, 77),
            write(1, 0x10, 0x34),
            write(0, 0x19, 0x12),
        ] {
            apply(&mut renderer, &shared, value, None);
        }
        renderer.stop(&shared);
        let s = state(&shared);
        assert_eq!(s.clock, 7987200);
        assert_eq!(s.source_mute_mask, 5);
        assert_eq!(s.registers[0][0x40], 77);
        assert_eq!(s.registers[0][0], 80);
        assert_eq!(s.registers[0][8], 0);
        assert_eq!(s.registers[1][0x10], 0x34);
        assert_eq!(s.registers[0][0x19], 0x12);
        assert_eq!(renderer.targets, [0.35, 0.7]);
        apply(&mut renderer, &shared, json!({"method":"reset"}), None);
        let s = state(&shared);
        assert_eq!(s.clock, 7987200);
        assert_eq!(s.source_mute_mask, 5);
        assert_eq!(s.registers[0][0x40], 0);
        assert_eq!(s.adpcm_a_rom_bytes, 13);
        assert_eq!(s.adpcm_b_rom_bytes, 22);
        for (kind, offset, expected) in [(0, 10, vec![23, 42, 99]), (1, 20, vec![54, 66])] {
            let mut bytes = vec![0; expected.len()];
            unsafe {
                native_ym2610b_copy_rom(
                    renderer.ptr,
                    kind,
                    bytes.as_mut_ptr(),
                    offset,
                    bytes.len() as u32,
                );
            };
            assert_eq!(bytes, expected);
        }
    }
    #[test]
    fn ym2610b_native_retained_upload_admission_is_atomic_bounded_and_deduplicates_ids() {
        let shared = Shared::new(true);
        let command = prepare(json!({"id":1,"method":"write","port":0,"register":0,"value":20}));
        assert!(shared.retain(&[command.clone(), command]).is_err());
        assert!(state(&shared).pending_commands.is_empty());
        let upload = prepare(json!({"method":"memory","kind":0,"offset":0,"data":vec![1;UPLOAD]}));
        assert!(shared
            .retain(&vec![upload; PENDING_BYTES / UPLOAD + 1])
            .is_err());
        assert!(state(&shared).pending_commands.is_empty());
    }
}

#[cfg(test)]
mod pipeline_tests {

    use super::super::*;
    use serde_json::{json, Value};
    fn command(frame: u64, message: Value) -> Command {
        serde_json::from_value(json!({"frame":frame,"type":"ym2610b","message":message})).unwrap()
    }
    fn tagged(frame: u64, message: Value, owner: u32) -> Command {
        let mut c = command(frame, message);
        c.performance = Some(1);
        c.owner = Some(owner);
        c
    }
    fn setup() -> (Shared, Renderer) {
        let shared = Shared::new_modes(48000, 1, false, false, false, Some("ym2610b"));
        let renderer = Renderer::new_modes(48000, false, false, false, Some("ym2610b")).unwrap();
        (shared, renderer)
    }
    fn render(shared: &Shared, renderer: &mut Renderer, frames: usize) -> Vec<[f32; 2]> {
        renderer.prepare(shared);
        let output = (0..frames).map(|_| renderer.sample(shared)).collect();
        shared.frame.store(renderer.frame, Ordering::Release);
        renderer.publish_primary(shared);
        output
    }
    fn snapshot(shared: &Shared) -> ym2610b::Status {
        shared
            .ym2610b
            .as_ref()
            .unwrap()
            .status(shared.frame.load(Ordering::Acquire))
            .unwrap()
    }
    fn note(ch: u8, voice: u64, audition: bool) -> Value {
        json!({"method":"batch","channel":ch+6,"voice":voice,"audition":audition,"action":"on","commands":[
            {"method":"write","port":0,"register":ch*2,"value":80},
            {"method":"write","port":0,"register":ch*2+1,"value":0},
            {"method":"write","port":0,"register":7,"value":0x38},
            {"method":"write","port":0,"register":ch+8,"value":12}]})
    }
    fn off(ch: u8, voice: u64) -> Value {
        json!({"method":"batch","channel":ch+6,"voice":voice,"action":"off","commands":[{"method":"key-off","channel":ch+6}]})
    }
    #[test]
    fn ym2610b_native_neogeo_owns_only_ym2610b_and_rejects_foreign_batches_atomically() {
        for (chip, mode) in [("ym2610", "neogeo"), ("ym2610b", "neogeo-b")] {
            let shared = Shared::new_modes(48000, 1, false, false, false, Some(chip));
            let renderer = Renderer::new_modes(48000, false, false, false, Some(chip)).unwrap();
            renderer.publish_primary(&shared);
            assert!(renderer.core.0.is_null());
            assert!(renderer.ym2608.is_none());
            assert_eq!(shared.status().mode, mode);
            assert_eq!(shared.status().ym2610b_clock, Some(8_000_000));
            assert_eq!(shared.status().ym_clock, 8_000_000);
            assert_eq!(shared.status().psg_clock, 0);
            assert_eq!(snapshot(&shared).variant, chip);
        }
        let (shared, mut renderer) = setup();
        assert!(renderer.core.0.is_null());
        assert!(renderer.ym2610b.is_some());
        assert!(renderer.ym2608.is_none());
        assert!(
            renderer.dx7.is_none()
                && renderer.nes.is_none()
                && renderer.rf5c164.is_none()
                && renderer.pwm.is_none()
        );
        assert_eq!(shared.status().mode, "neogeo-b");
        let mut oversized = command(0, note(0, 1, false));
        oversized.action = CommandAction::Ym2610b {
            message: ym2610b::Message {
                id: None,
                audition: false,
                method: ym2610b::Method::Memory {
                    kind: 0,
                    offset: 0,
                    data: vec![0; ym2610b::MAX_BATCH_UPLOAD_BYTES + 1],
                },
            },
        };
        assert!(shared
            .enqueue(1, &[oversized])
            .unwrap_err()
            .contains("batch exceeds"));
        assert_eq!(shared.pending.load(Ordering::Acquire), 0);
        for value in [
            json!({"frame":0,"type":"write","register":40,"value":0}),
            json!({"frame":0,"type":"psg-write","value":128}),
            json!({"frame":0,"type":"rf5c164-reset"}),
        ] {
            let invalid: Command = serde_json::from_value(value).unwrap();
            assert!(shared
                .enqueue(1, &[command(0, note(0, 1, false)), invalid])
                .is_err());
            assert_eq!(shared.pending.load(Ordering::Acquire), 0);
        }
        assert!(shared.enqueue(2, &[command(0, note(0, 1, false))]).is_err());
        assert!(render(&shared, &mut renderer, 128)
            .iter()
            .all(|s| *s == [0.0; 2]));
        let genesis = Shared::new(48000, 2);
        assert!(genesis
            .enqueue(2, &[command(0, note(0, 1, false))])
            .is_err());
    }
    #[test]
    fn ym2610b_native_due_frames_and_callback_segmentation_preserve_pcm() {
        let (a, mut left) = setup();
        let (b, mut right) = setup();
        a.enqueue(
            1,
            &[command(200, note(0, 1, false)), command(6000, off(0, 1))],
        )
        .unwrap();
        b.enqueue(
            1,
            &[command(200, note(0, 1, false)), command(6000, off(0, 1))],
        )
        .unwrap();
        let expected = render(&a, &mut left, 8192);
        let mut actual = Vec::new();
        for frames in [1, 127, 53, 512, 31, 1024, 4096, 2348] {
            actual.extend(render(&b, &mut right, frames));
        }
        assert_eq!(expected, actual);
        assert!(expected[..199].iter().all(|s| *s == [0.0; 2]));
        assert!(expected[512..4000].iter().any(|s| s[0].abs() > 0.001));
        assert_eq!(snapshot(&a).registers[0][8], 0);
    }
    #[test]
    fn ym2610b_native_scoped_stop_cancels_older_future_and_leaves_other_owner() {
        let (shared, mut renderer) = setup();
        shared.performance("start", 1, None).unwrap();
        shared
            .enqueue(
                1,
                &[
                    tagged(0, note(0, 1, false), 1),
                    tagged(0, note(1, 2, false), 2),
                    tagged(1000, note(0, 3, false), 1),
                    tagged(1200, note(1, 4, false), 2),
                ],
            )
            .unwrap();
        render(&shared, &mut renderer, 256);
        assert_eq!(&snapshot(&shared).registers[0][8..10], &[12, 12]);
        shared
            .enqueue(1, &[tagged(0, json!({"method":"stop-voices"}), 1)])
            .unwrap();
        render(&shared, &mut renderer, 1500);
        assert_eq!(&snapshot(&shared).registers[0][8..10], &[0, 12]);
        assert!(snapshot(&shared).pending_commands.is_empty());
        shared.performance("cancel-owner", 1, Some(1)).unwrap();
        render(&shared, &mut renderer, 1);
        assert_eq!(snapshot(&shared).registers[0][9], 12);
        shared.performance("cancel-owner", 1, Some(2)).unwrap();
        render(&shared, &mut renderer, 1);
        assert_eq!(snapshot(&shared).registers[0][9], 0);
    }
    #[test]
    fn ym2610b_native_epoch_stop_retires_uploads_and_future_notes_preserves_ram_and_restarts() {
        let (shared, mut renderer) = setup();
        let mut future = note(1, 2, false);
        future["id"] = json!(42);
        shared
            .enqueue(
                1,
                &[
                    command(
                        0,
                        json!({"method":"memory","kind":1,"offset":10,"data":[1,2,3]}),
                    ),
                    command(0, note(0, 1, true)),
                    command(5000, future),
                ],
            )
            .unwrap();
        render(&shared, &mut renderer, 512);
        assert_eq!(snapshot(&shared).held, vec![6]);
        shared.epoch.fetch_add(1, Ordering::AcqRel);
        render(&shared, &mut renderer, 1024);
        let s = snapshot(&shared);
        assert!(s.held.is_empty());
        assert!(s.pending_commands.is_empty());
        assert_eq!(s.adpcm_b_rom_bytes, 13);
        assert_eq!(s.registers[0][8], 0);
        assert_eq!(s.registers[0][9], 0);
        assert!(render(&shared, &mut renderer, 5000)
            .iter()
            .all(|s| *s == [0.0; 2]));
        assert_eq!(shared.pending.load(Ordering::Acquire), 0);
        shared.enqueue(1, &[command(0, note(0, 3, false))]).unwrap();
        assert!(render(&shared, &mut renderer, 2048)
            .iter()
            .any(|s| s[0].abs() > 0.001));
        assert_eq!(shared.session, 1);
    }
    #[test]
    fn ym2610b_native_chip_stop_keeps_common_sample_bus_and_master_applies() {
        let (shared, mut renderer) = setup();
        shared
            .load_media(
                media::Kind::Sample,
                "common".into(),
                48000.0,
                vec![vec![0.25; 4096]],
            )
            .unwrap();
        let sample: Command = serde_json::from_value(
            json!({"frame":1500,"type":"sample-play","name":"common","voice":1}),
        )
        .unwrap();
        shared
            .enqueue(
                1,
                &[
                    command(0, note(0, 1, true)),
                    command(1000, note(1, 2, false)),
                    sample,
                ],
            )
            .unwrap();
        render(&shared, &mut renderer, 256);
        shared
            .enqueue(1, &[command(0, json!({"method":"stop-voices"}))])
            .unwrap();
        render(&shared, &mut renderer, 1200);
        assert!(render(&shared, &mut renderer, 900)
            .iter()
            .any(|s| s[0] > 0.1));
        let master:Command=serde_json::from_value(json!({"frame":0,"type":"mix","fmLeft":1,"fmRight":1,"psgLeft":1,"psgRight":1,"master":0})).unwrap();
        shared.enqueue(1, &[master]).unwrap();
        assert!(render(&shared, &mut renderer, 512)[300..]
            .iter()
            .all(|s| *s == [0.0; 2]));
    }
    #[test]
    fn ym2610b_native_source_mix_fx_and_recording_share_the_final_signal() {
        let (dry, mut reference) = setup();
        let (wet, mut renderer) = setup();
        let fx:FxMessage=serde_json::from_value(json!({"op":"chain","children":[{"type":"gain","slot":0}],"units":[{"type":"gain","slot":0,"values":{"gain":0.5}}],"beatSeconds":0.5})).unwrap();
        let graph = wet.fx.lock().unwrap().command(fx, 48000).unwrap().unwrap();
        wet.enqueue_special(Action::Fx(graph)).unwrap();
        wet.enqueue(
            1,
            &[
                command(0, json!({"method":"mix","volume":0.5,"pan":0})),
                command(0, note(0, 1, false)),
            ],
        )
        .unwrap();
        dry.enqueue(1, &[command(0, note(0, 1, false))]).unwrap();
        wet.record_start(0.02).unwrap();
        let expected = render(&dry, &mut reference, 2048);
        let actual = render(&wet, &mut renderer, 2048);
        assert!(expected[512..].iter().any(|s| s[0].abs() > 0.001));
        for (a, b) in expected[512..].iter().zip(&actual[512..]) {
            for ch in 0..2 {
                assert!((a[ch] * 0.25 - b[ch]).abs() < 1e-7);
            }
        }
        let captured = wet.record_stop().unwrap();
        assert_eq!(captured.frames, 960);
        assert_eq!(
            captured.channels[0],
            actual[..960].iter().map(|s| s[0]).collect::<Vec<_>>()
        );
        assert_eq!(
            captured.channels[1],
            actual[..960].iter().map(|s| s[1]).collect::<Vec<_>>()
        );
    }
}
