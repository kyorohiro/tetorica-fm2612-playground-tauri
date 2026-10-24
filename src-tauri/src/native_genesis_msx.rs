//! Callback-owned mono AY-3-8910 + YM2413. All command allocation and reclamation
//! happens on the control thread; snapshots only observe applied core state.
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
pub const AY_CLOCK: u32 = 1_789_773;
const VOICES: usize = 12;
const PENDING_BYTES: usize = 4 * 1024 * 1024;
#[repr(C)]
struct CoreState {
    registers: [u8; 64],
    ay_registers: [u8; 16],
    levels: [u8; 3],
    ay_clock: u32,
    opll_clock: u32,
    ay_mute_mask: u32,
    opll_mute_mask: u32,
    status: u8,
    irq: u8,
}
impl Default for CoreState {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}
extern "C" {
    fn native_msx_create(rate: u32) -> *mut c_void;
    fn native_msx_destroy(ptr: *mut c_void);
    fn native_msx_reset(ptr: *mut c_void, chip: u8);
    fn native_msx_stop(ptr: *mut c_void, chip: u8);
    fn native_msx_key_off(ptr: *mut c_void, voice: u8);
    fn native_msx_audition_key_on(ptr: *mut c_void, voice: u8);
    fn native_msx_write(ptr: *mut c_void, chip: u8, register: u8, value: u8);
    fn native_msx_sample(ptr: *mut c_void, pl: *mut f32, pr: *mut f32, ol: *mut f32, or: *mut f32);
    fn native_msx_set_clock(ptr: *mut c_void, chip: u8, hz: u32) -> i32;
    fn native_msx_set_retained_level(ptr: *mut c_void, channel: u8, level: u8);
    fn native_msx_set_mute_mask(ptr: *mut c_void, chip: u8, mask: u32);
    fn native_msx_get_state(ptr: *mut c_void, state: *mut CoreState);
}
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Chip {
    Ay8910,
    Ym2413,
}
impl Chip {
    fn code(self) -> u8 {
        if self == Self::Ay8910 {
            0
        } else {
            1
        }
    }
    fn channels(self) -> u8 {
        if self == Self::Ay8910 {
            3
        } else {
            14
        }
    }
    fn physical(self, ch: u8) -> usize {
        if self == Self::Ay8910 {
            ch as usize
        } else {
            3 + if ch < 9 {
                ch as usize
            } else {
                [6, 7, 8, 8, 7][(ch - 9) as usize]
            }
        }
    }
    fn voice(self, ch: u8) -> u8 {
        if self == Self::Ay8910 {
            ch
        } else {
            3 + ch
        }
    }
    fn range(self) -> std::ops::Range<usize> {
        if self == Self::Ay8910 {
            0..3
        } else {
            3..12
        }
    }
}
#[derive(Clone, Debug, Deserialize)]
pub struct Message {
    pub chip: Chip,
    #[serde(default)]
    pub id: Option<u64>,
    #[serde(default)]
    pub audition: bool,
    #[serde(flatten)]
    pub method: Method,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "method", rename_all = "kebab-case")]
pub enum Method {
    Write {
        #[serde(default)]
        register: u8,
        value: u8,
    },
    SetRetainedLevel {
        channel: u8,
        level: u8,
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
    SourceMute {
        mask: u32,
    },
    Reset,
    StopVoices,
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
        #[serde(default)]
        register: u8,
        value: u8,
    },
    SetRetainedLevel {
        channel: u8,
        level: u8,
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
    method: Method,
    complete: AtomicBool,
}
fn valid_id(id: Option<u64>) -> bool {
    id.is_none_or(|v| v > 0 && v <= MAX_SAFE_INTEGER)
}
fn declared_channel(chip: Chip, op: &Op) -> Option<u8> {
    match *op {
        Op::KeyOff { channel }
        | Op::AuditionKeyOn { channel }
        | Op::SetRetainedLevel { channel, .. } => Some(channel),
        Op::Write {
            register: 0..=5, ..
        } if chip == Chip::Ay8910 => {
            if let Op::Write { register, .. } = *op {
                Some(register / 2)
            } else {
                None
            }
        }
        Op::Write {
            register: 8..=10, ..
        } if chip == Chip::Ay8910 => {
            if let Op::Write { register, .. } = *op {
                Some(register - 8)
            } else {
                None
            }
        }
        Op::Write { register, .. }
            if chip == Chip::Ym2413 && matches!(register,0x10..=0x18|0x20..=0x28|0x30..=0x38) =>
        {
            Some(register & 15)
        }
        _ => None,
    }
}
fn validate_op(op: &Op, chip: Chip) -> Result<(), String> {
    match *op {
        Op::KeyOff { channel } | Op::AuditionKeyOn { channel } if channel >= chip.channels() => {
            Err("Invalid Native MSX channel".into())
        }
        Op::SetRetainedLevel { channel, level }
            if chip != Chip::Ay8910 || channel >= 3 || level > 31 =>
        {
            Err("Invalid Native AY retained level".into())
        }
        Op::Clock { hz } if !(100_000..=100_000_000).contains(&hz) => {
            Err("Native MSX clock must be 100000–100000000 Hz".into())
        }
        Op::Write { register, .. } if register >= if chip == Chip::Ay8910 { 16 } else { 64 } => {
            Err("Invalid Native MSX register".into())
        }
        _ => Ok(()),
    }
}
impl Message {
    pub fn prepare(&self) -> Result<Arc<Prepared>, String> {
        if !valid_id(self.id) {
            return Err("Invalid Native MSX command ID".into());
        }
        match &self.method {
            Method::Write { register, value } => validate_op(
                &Op::Write {
                    register: *register,
                    value: *value,
                },
                self.chip,
            )?,
            Method::SetRetainedLevel { channel, level } => validate_op(
                &Op::SetRetainedLevel {
                    channel: *channel,
                    level: *level,
                },
                self.chip,
            )?,
            Method::KeyOff { channel } | Method::AuditionKeyOn { channel } => {
                validate_op(&Op::KeyOff { channel: *channel }, self.chip)?
            }
            Method::Clock { hz } => validate_op(&Op::Clock { hz: *hz }, self.chip)?,
            Method::Mix { volume, pan }
                if !volume.is_finite()
                    || !(0.0..=4.0).contains(volume)
                    || !pan.is_finite()
                    || !(-1.0..=1.0).contains(pan) =>
            {
                return Err("Native MSX mixer volume must be 0–4, pan −1–1".into())
            }
            Method::SourceMute { mask }
                if *mask > if self.chip == Chip::Ay8910 { 7 } else { 511 } =>
            {
                return Err("Invalid Native MSX source mute mask".into())
            }
            Method::Batch {
                commands,
                channel,
                voice,
                action,
            } => {
                if *channel >= self.chip.channels()
                    || commands.is_empty()
                    || commands.len() > 512
                    || !valid_id(*voice)
                    || self.audition && *action != LeaseAction::Edit && voice.is_none()
                {
                    return Err("Invalid Native MSX batch or voice lease".into());
                }
                for op in commands {
                    validate_op(op, self.chip)?;
                    let target = declared_channel(self.chip, op);
                    if target
                        .is_some_and(|ch| self.chip.physical(ch) != self.chip.physical(*channel))
                    {
                        return Err(
                            "Native MSX batch must target its declared physical voice".into()
                        );
                    }
                }
            }
            _ => {}
        }
        Ok(Arc::new(Prepared {
            id: self.id,
            chip: self.chip,
            audition: self.audition,
            method: self.method.clone(),
            complete: AtomicBool::new(false),
        }))
    }
}
impl Prepared {
    pub fn starts(&self) -> bool {
        match &self.method {
            Method::SetRetainedLevel { .. }
            | Method::KeyOff { .. }
            | Method::Clock { .. }
            | Method::Mix { .. }
            | Method::SourceMute { .. }
            | Method::Reset
            | Method::StopVoices => false,
            Method::Batch { action, .. } => {
                *action == LeaseAction::On || *action == LeaseAction::Edit
            }
            _ => true,
        }
    }
    pub fn stops(&self) -> bool {
        matches!(self.method, Method::StopVoices)
    }
    fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + if let Method::Batch { commands, .. } = &self.method {
                commands.len() * std::mem::size_of::<Op>()
            } else {
                0
            }
    }
}
#[derive(Default)]
struct Control {
    prepared: Vec<Arc<Prepared>>,
}
pub struct Shared {
    revision: AtomicU64,
    registers: [AtomicU8; 64],
    ay_registers: [AtomicU8; 16],
    levels: [AtomicU8; 3],
    clock: [AtomicU32; 2],
    mute: [AtomicU32; 2],
    status: AtomicU8,
    irq: AtomicU8,
    held: AtomicU32,
    rate: AtomicU32,
    control: Mutex<Control>,
    retired: ArrayQueue<Arc<Prepared>>,
    highest_id: AtomicU64,
    cancel_before: [AtomicU64; 2],
    owner_cancel_before: [[AtomicU64; OWNERS]; 2],
}
impl Default for Shared {
    fn default() -> Self {
        Self {
            revision: AtomicU64::new(0),
            registers: std::array::from_fn(|_| AtomicU8::new(0)),
            ay_registers: std::array::from_fn(|_| AtomicU8::new(0)),
            levels: std::array::from_fn(|_| AtomicU8::new(9)),
            clock: [AtomicU32::new(AY_CLOCK), AtomicU32::new(CLOCK)],
            mute: std::array::from_fn(|_| AtomicU32::new(0)),
            status: AtomicU8::new(0),
            irq: AtomicU8::new(0),
            held: AtomicU32::new(0),
            rate: AtomicU32::new(0),
            control: Mutex::new(Control::default()),
            retired: ArrayQueue::new(CAPACITY),
            highest_id: AtomicU64::new(0),
            cancel_before: std::array::from_fn(|_| AtomicU64::new(0)),
            owner_cancel_before: std::array::from_fn(|_| {
                std::array::from_fn(|_| AtomicU64::new(0))
            }),
        }
    }
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AyStatus {
    pub clock: u32,
    pub registers: Vec<u8>,
    pub levels: Vec<u8>,
    pub gates: Vec<bool>,
    pub source_mute_mask: u32,
    pub held: Vec<u8>,
    pub pending_commands: Vec<u64>,
    pub sample_rate: u32,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpllStatus {
    pub clock: u32,
    pub registers: Vec<u8>,
    pub status: u8,
    pub irq: bool,
    pub source_mute_mask: u32,
    pub held: Vec<u8>,
    pub pending_commands: Vec<u64>,
    pub sample_rate: u32,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub ay8910: AyStatus,
    pub ym2413: OpllStatus,
    pub sample_rate: u32,
    pub frame: u64,
    pub revision: u64,
}
impl Shared {
    pub fn clock(&self) -> u32 {
        self.clock[0].load(Ordering::Acquire)
    }
    pub fn opll_clock(&self) -> u32 {
        self.clock[1].load(Ordering::Acquire)
    }
    pub fn valid_event(&self, command: &Prepared, sequence: u64, owner: Option<Owner>) -> bool {
        let chip = command.chip.code() as usize;
        sequence >= self.cancel_before[chip].load(Ordering::Acquire)
            && owner.is_none_or(|o| {
                sequence
                    >= self.owner_cancel_before[chip][(o.owner - 1) as usize]
                        .load(Ordering::Acquire)
            })
    }
    pub fn cancel_before(&self, command: &Prepared, sequence: u64, owner: Option<Owner>) {
        let chip = command.chip.code() as usize;
        if let Some(o) = owner {
            self.owner_cancel_before[chip][(o.owner - 1) as usize]
                .fetch_max(sequence, Ordering::AcqRel);
        } else {
            self.cancel_before[chip].fetch_max(sequence, Ordering::AcqRel);
        }
    }
    fn collect(&self, c: &mut Control) {
        let mut retired = HashSet::new();
        while let Some(cmd) = self.retired.pop() {
            retired.insert(Arc::as_ptr(&cmd) as usize);
        }
        c.prepared
            .retain(|cmd| !retired.contains(&(Arc::as_ptr(cmd) as usize)));
    }
    pub fn retain(&self, commands: &[(Arc<Prepared>, Option<Owner>)]) -> Result<(), String> {
        self.retain_then(commands, || Ok(()))
    }
    pub fn retain_then(
        &self,
        commands: &[(Arc<Prepared>, Option<Owner>)],
        next: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        let mut c = self
            .control
            .lock()
            .map_err(|_| "Native MSX control state unavailable")?;
        self.collect(&mut c);
        if c.prepared.len() + commands.len() > CAPACITY
            || c.prepared.iter().map(|v| v.bytes()).sum::<usize>()
                + commands.iter().map(|(v, _)| v.bytes()).sum::<usize>()
                > PENDING_BYTES
        {
            return Err("Native MSX pending command limit reached (4 MiB)".into());
        }
        let mut ids = HashSet::new();
        for id in c
            .prepared
            .iter()
            .filter_map(|v| v.id)
            .chain(commands.iter().filter_map(|(v, _)| v.id))
        {
            if !ids.insert(id) {
                return Err("Duplicate pending Native MSX command ID".into());
            }
        }
        // With MSX Audio, OPL admission can still fail (for example a held
        // clock change). Commit neither module until both validations pass.
        next()?;
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
            return Err("Unknown Native MSX command ID".into());
        }
        let cmd = self
            .control
            .lock()
            .map_err(|_| "Native MSX control state unavailable")?
            .prepared
            .iter()
            .find(|v| v.id == Some(id))
            .cloned();
        if let Some(cmd) = cmd {
            let start = std::time::Instant::now();
            while !cmd.complete.load(Ordering::Acquire) {
                if start.elapsed() > std::time::Duration::from_millis(100) {
                    return Err("Native MSX command was not applied within 100 ms".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        Ok(())
    }
    pub fn retire(&self, cmd: Arc<Prepared>) {
        if !cmd.complete.load(Ordering::Relaxed) {
            self.revision.fetch_add(1, Ordering::AcqRel);
            cmd.complete.store(true, Ordering::Relaxed);
            self.revision.fetch_add(1, Ordering::Release);
        }
        self.retired
            .push(cmd)
            .unwrap_or_else(|_| unreachable!("Bounded MSX command retirement"));
    }
    fn publish(&self, r: &Renderer, command: Option<&Prepared>) {
        let s = r.state();
        self.revision.fetch_add(1, Ordering::AcqRel);
        for (o, v) in self.registers.iter().zip(s.registers) {
            o.store(v, Ordering::Relaxed)
        }
        for (o, v) in self.ay_registers.iter().zip(s.ay_registers) {
            o.store(v, Ordering::Relaxed)
        }
        for (o, v) in self.levels.iter().zip(s.levels) {
            o.store(v, Ordering::Relaxed)
        }
        self.clock[0].store(s.ay_clock, Ordering::Relaxed);
        self.clock[1].store(s.opll_clock, Ordering::Relaxed);
        self.mute[0].store(s.ay_mute_mask, Ordering::Relaxed);
        self.mute[1].store(s.opll_mute_mask, Ordering::Relaxed);
        self.status.store(s.status, Ordering::Relaxed);
        self.irq.store(s.irq, Ordering::Relaxed);
        self.held.store(
            r.owners
                .iter()
                .flatten()
                .filter(|l| l.audition)
                .fold(0, |bits, l| bits | (1 << l.wire_voice())),
            Ordering::Relaxed,
        );
        self.rate.store(r.rate, Ordering::Relaxed);
        if let Some(cmd) = command {
            cmd.complete.store(true, Ordering::Relaxed)
        }
        self.revision.fetch_add(1, Ordering::Release);
    }
    pub fn status(&self, frame: u64) -> Result<Status, String> {
        let mut c = self
            .control
            .lock()
            .map_err(|_| "Native MSX control state unavailable")?;
        self.collect(&mut c);
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() > std::time::Duration::from_millis(50) {
                return Err("Native MSX state changed while reading; retry".into());
            }
            let before = self.revision.load(Ordering::Acquire);
            if before & 1 != 0 {
                std::thread::yield_now();
                continue;
            }
            let held = self.held.load(Ordering::Relaxed);
            let rate = self.rate.load(Ordering::Relaxed);
            let pending = |chip| {
                c.prepared
                    .iter()
                    .filter(|v| v.chip == chip && !v.complete.load(Ordering::Relaxed))
                    .filter_map(|v| v.id)
                    .collect()
            };
            let out = Status {
                ay8910: AyStatus {
                    clock: self.clock(),
                    registers: self
                        .ay_registers
                        .iter()
                        .map(|v| v.load(Ordering::Relaxed))
                        .collect(),
                    levels: self
                        .levels
                        .iter()
                        .map(|v| v.load(Ordering::Relaxed))
                        .collect(),
                    gates: (8..=10)
                        .map(|reg| self.ay_registers[reg].load(Ordering::Relaxed) & 31 != 0)
                        .collect(),
                    source_mute_mask: self.mute[0].load(Ordering::Relaxed),
                    held: (0..3).filter(|ch| held & (1 << ch) != 0).collect(),
                    pending_commands: pending(Chip::Ay8910),
                    sample_rate: rate,
                },
                ym2413: OpllStatus {
                    clock: self.opll_clock(),
                    registers: self
                        .registers
                        .iter()
                        .map(|v| v.load(Ordering::Relaxed))
                        .collect(),
                    status: self.status.load(Ordering::Relaxed),
                    irq: self.irq.load(Ordering::Relaxed) != 0,
                    source_mute_mask: self.mute[1].load(Ordering::Relaxed),
                    held: (0..14).filter(|ch| held & (1 << (3 + ch)) != 0).collect(),
                    pending_commands: pending(Chip::Ym2413),
                    sample_rate: rate,
                },
                sample_rate: rate,
                frame,
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
    owner: Option<Owner>,
    voice: Option<u64>,
    audition: bool,
    chip: Chip,
    channel: u8,
}
impl Lease {
    fn wire_voice(self) -> u8 {
        self.chip.voice(self.channel)
    }
}
pub struct Renderer {
    ptr: *mut c_void,
    rate: u32,
    owners: [Option<Lease>; VOICES],
    gains: [[f32; 2]; 2],
    targets: [[f32; 2]; 2],
    ramps: [u32; 2],
}
unsafe impl Send for Renderer {}
impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe { native_msx_destroy(self.ptr) }
    }
}
impl Renderer {
    pub fn new(rate: u32) -> Result<Self, String> {
        let ptr = unsafe { native_msx_create(rate) };
        if ptr.is_null() {
            return Err("Could not allocate Native MSX core".into());
        }
        Ok(Self {
            ptr,
            rate,
            owners: [None; VOICES],
            gains: [[1.; 2]; 2],
            targets: [[1.; 2]; 2],
            ramps: [0; 2],
        })
    }
    fn state(&self) -> CoreState {
        let mut s = CoreState::default();
        unsafe { native_msx_get_state(self.ptr, &mut s) }
        s
    }
    fn protected(&self, ch: usize, voice: Option<u64>, audition: bool) -> bool {
        self.owners[ch]
            .is_some_and(|l| l.audition && (!audition || voice.is_some() && l.voice != voice))
    }
    fn release(&mut self, ch: usize) {
        if let Some(l) = self.owners[ch] {
            unsafe { native_msx_key_off(self.ptr, l.wire_voice()) }
            if ch >= 9 {
                // Rhythm siblings share a physical OPLL channel. Cancellation
                // releases the whole leased oscillator, including pending gates.
                let melodic = (ch - 3) as u8;
                unsafe {
                    native_msx_key_off(self.ptr, 3 + melodic);
                    for drum in 0..5 {
                        if Chip::Ym2413.physical(9 + drum) == ch {
                            native_msx_key_off(self.ptr, 12 + drum);
                        }
                    }
                }
            }
        }
        self.owners[ch] = None;
    }
    fn op(
        &mut self,
        chip: Chip,
        op: &Op,
        owner: Option<Owner>,
        voice: Option<u64>,
        audition: bool,
        preserve: bool,
    ) {
        let state = self.state();
        let ch = declared_channel(chip, op);
        let target = ch.map(|ch| chip.physical(ch));
        if target.is_some_and(|p| self.protected(p, voice, audition)) {
            return;
        }
        if !audition {
            if matches!(op, Op::Clock { .. })
                && chip.range().any(|p| self.protected(p, voice, false))
            {
                return;
            }
            if chip == Chip::Ay8910 {
                let noise_held = (0..3).any(|p| {
                    self.protected(p, voice, false) && state.ay_registers[7] & (1 << (3 + p)) == 0
                });
                let envelope_held = (0..3).any(|p| {
                    self.protected(p, voice, false) && state.ay_registers[8 + p] & 16 != 0
                });
                if matches!(op, Op::Write { register: 6, .. }) && noise_held
                    || matches!(
                        op,
                        Op::Write {
                            register: 11..=13,
                            ..
                        }
                    ) && envelope_held
                {
                    return;
                }
            }
            if chip == Chip::Ym2413
                && matches!(
                    op,
                    Op::Write {
                        register: 0..=7,
                        ..
                    }
                )
                && (3..12).any(|p| self.protected(p, voice, false))
            {
                return;
            }
        }
        let mut off = matches!(op, Op::KeyOff { .. });
        let mut on = matches!(op, Op::AuditionKeyOn { .. });
        if let Op::Write {
            register, value, ..
        } = *op
        {
            if chip == Chip::Ay8910 {
                if (8..=10).contains(&register) {
                    off = value & 31 == 0;
                    on = !off;
                }
            } else if (0x20..=0x28).contains(&register) {
                off = value & 16 == 0;
                on = !off;
            }
        }
        if off
            && target.is_some_and(|p| {
                self.owners[p].is_some_and(|l| {
                    owner.is_some() && l.owner != owner || voice.is_some() && l.voice != voice
                })
            })
        {
            return;
        }
        unsafe {
            match *op {
                Op::Write {
                    register,
                    mut value,
                } => {
                    if chip == Chip::Ay8910 {
                        if register == 7 && !audition {
                            for p in 0..3 {
                                if self.protected(p, voice, false) {
                                    let bits = (1 << p) | (1 << (p + 3));
                                    value = (value & !bits) | (state.ay_registers[7] & bits);
                                }
                            }
                        }
                        native_msx_write(self.ptr, 0, register, value);
                    } else {
                        if register == 14 {
                            if !audition
                                && (value ^ state.registers[14]) & 32 != 0
                                && (3..12).any(|p| self.protected(p, voice, false))
                            {
                                return;
                            }
                            // The five drums share three physical channels; never alter a held sibling gate.
                            for drum in 0..5 {
                                let logical = 9 + drum;
                                let p = chip.physical(logical);
                                let bit = [16, 8, 4, 2, 1][drum as usize];
                                let stale = self.owners[p].is_some_and(|l| {
                                    l.channel == logical && owner.is_some() && l.owner != owner
                                });
                                if self.protected(p, voice, audition) || value & bit == 0 && stale {
                                    value = (value & !bit) | (state.registers[14] & bit);
                                }
                            }
                            native_msx_write(self.ptr, 1, register, value);
                            if !preserve {
                                for drum in 0..5 {
                                    let logical = 9 + drum;
                                    let p = chip.physical(logical);
                                    let bit = [16, 8, 4, 2, 1][drum as usize];
                                    if value & bit != 0 && state.registers[14] & bit == 0 {
                                        self.owners[p] = Some(Lease {
                                            owner,
                                            voice,
                                            audition,
                                            chip,
                                            channel: logical,
                                        })
                                    } else if value & bit == 0
                                        && state.registers[14] & bit != 0
                                        && self.owners[p].is_some_and(|l| l.channel == logical)
                                    {
                                        self.owners[p] = None;
                                    }
                                }
                            }
                        } else {
                            native_msx_write(self.ptr, 1, register, value);
                        }
                    }
                }
                Op::SetRetainedLevel { channel, level } => {
                    native_msx_set_retained_level(self.ptr, channel, level)
                }
                Op::KeyOff { channel } => native_msx_key_off(self.ptr, chip.voice(channel)),
                Op::AuditionKeyOn { channel } => {
                    native_msx_audition_key_on(self.ptr, chip.voice(channel))
                }
                Op::Clock { hz } => {
                    native_msx_set_clock(self.ptr, chip.code(), hz);
                }
            }
        }
        if !preserve {
            if let (Some(ch), Some(p)) = (ch, target) {
                if off {
                    self.owners[p] = None
                } else if on {
                    self.owners[p] = Some(Lease {
                        owner,
                        voice,
                        audition,
                        chip,
                        channel: ch,
                    })
                }
            }
        }
    }
    pub fn apply(&mut self, cmd: &Prepared, owner: Option<Owner>, shared: &Shared) {
        let chip = cmd.chip;
        match &cmd.method {
            Method::Write { register, value } => self.op(
                chip,
                &Op::Write {
                    register: *register,
                    value: *value,
                },
                owner,
                None,
                cmd.audition,
                false,
            ),
            Method::SetRetainedLevel { channel, level } => self.op(
                chip,
                &Op::SetRetainedLevel {
                    channel: *channel,
                    level: *level,
                },
                owner,
                None,
                cmd.audition,
                false,
            ),
            Method::KeyOff { channel } => self.op(
                chip,
                &Op::KeyOff { channel: *channel },
                owner,
                None,
                cmd.audition,
                false,
            ),
            Method::AuditionKeyOn { channel } => self.op(
                chip,
                &Op::AuditionKeyOn { channel: *channel },
                owner,
                None,
                cmd.audition,
                false,
            ),
            Method::Clock { hz } => self.op(
                chip,
                &Op::Clock { hz: *hz },
                owner,
                None,
                cmd.audition,
                false,
            ),
            Method::Mix { volume, pan } => {
                let i = chip.code() as usize;
                self.targets[i] = [
                    *volume * if *pan > 0. { 1. - *pan } else { 1. },
                    *volume * if *pan < 0. { 1. + *pan } else { 1. },
                ];
                self.ramps[i] = (self.rate / 200).max(1);
            }
            Method::SourceMute { mask } => unsafe {
                native_msx_set_mute_mask(self.ptr, chip.code(), *mask)
            },
            Method::Reset => {
                if cmd.audition || !chip.range().any(|p| self.protected(p, None, false)) {
                    unsafe { native_msx_reset(self.ptr, chip.code()) }
                    for p in chip.range() {
                        self.owners[p] = None
                    }
                }
            }
            Method::StopVoices => {
                if owner.is_none() {
                    unsafe { native_msx_stop(self.ptr, chip.code()) }
                    for p in chip.range() {
                        self.owners[p] = None
                    }
                } else {
                    for p in chip.range() {
                        if self.owners[p]
                            .is_some_and(|l| l.owner == owner && (!l.audition || cmd.audition))
                        {
                            self.release(p)
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
                let p = chip.physical(*channel);
                let current = self.owners[p];
                let allowed = match action {
                    LeaseAction::On => cmd.audition || current.is_none_or(|l| !l.audition),
                    LeaseAction::Off => current.is_some_and(|l| {
                        l.channel == *channel
                            && l.owner == owner
                            && (voice.is_none() || l.voice == *voice)
                            && (!l.audition || cmd.audition)
                    }),
                    LeaseAction::Edit => current.is_none_or(|l| {
                        !l.audition || cmd.audition && (voice.is_none() || l.voice == *voice)
                    }),
                };
                if allowed {
                    if *action == LeaseAction::On {
                        if current.is_some_and(|l| l.channel != *channel) {
                            self.release(p)
                        }
                        self.owners[p] = Some(Lease {
                            owner,
                            voice: *voice,
                            audition: cmd.audition,
                            chip,
                            channel: *channel,
                        });
                    }
                    for op in commands {
                        self.op(
                            chip,
                            op,
                            owner,
                            *voice,
                            cmd.audition,
                            *action == LeaseAction::Edit,
                        )
                    }
                    if *action == LeaseAction::On {
                        self.owners[p] = Some(Lease {
                            owner,
                            voice: *voice,
                            audition: cmd.audition,
                            chip,
                            channel: *channel,
                        })
                    } else if *action == LeaseAction::Off {
                        self.owners[p] = None;
                    }
                }
            }
        }
        shared.publish(self, Some(cmd));
    }
    pub fn publish(&self, shared: &Shared) {
        shared.publish(self, None)
    }
    pub fn cancel_invalid(&mut self, valid: impl Fn(Option<Owner>) -> bool, shared: &Shared) {
        let mut changed = false;
        for p in 0..VOICES {
            if self.owners[p].is_some_and(|l| !valid(l.owner)) {
                self.release(p);
                changed = true
            }
        }
        if changed {
            shared.publish(self, None)
        }
    }
    pub fn stop(&mut self, shared: &Shared) {
        unsafe { native_msx_stop(self.ptr, 2) }
        self.owners.fill(None);
        shared.publish(self, None)
    }
    pub fn mix(&mut self, pcm: &mut [f32; 2]) {
        let (mut pl, mut pr, mut ol, mut or) = (0., 0., 0., 0.);
        unsafe { native_msx_sample(self.ptr, &mut pl, &mut pr, &mut ol, &mut or) }
        for i in 0..2 {
            if self.ramps[i] > 0 {
                for ch in 0..2 {
                    self.gains[i][ch] +=
                        (self.targets[i][ch] - self.gains[i][ch]) / self.ramps[i] as f32;
                }
                self.ramps[i] -= 1;
            }
        }
        pcm[0] += pl * self.gains[0][0] + ol * self.gains[1][0];
        pcm[1] += pr * self.gains[0][1] + or * self.gains[1][1];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    fn prep(v: Value) -> Arc<Prepared> {
        serde_json::from_value::<Message>(v)
            .unwrap()
            .prepare()
            .unwrap()
    }
    fn apply(r: &mut Renderer, s: &Shared, v: Value, owner: Option<Owner>) {
        let c = prep(v);
        s.retain(&[(c.clone(), owner)]).unwrap();
        r.apply(&c, owner, s);
        s.retire(c)
    }
    fn write(chip: &str, register: u8, value: u8) -> Value {
        json!({"chip":chip,"method":"write","register":register,"value":value})
    }
    fn lease(chip: &str, channel: u8, voice: u64, action: &str, commands: Vec<Value>) -> Value {
        json!({"chip":chip,"method":"batch","channel":channel,"voice":voice,"action":action,"audition":true,"commands":commands})
    }
    fn owner(n: u32) -> Option<Owner> {
        Some(Owner {
            performance: 1,
            owner: n,
        })
    }
    fn setup() -> (Shared, Renderer) {
        let s = Shared::default();
        let r = Renderer::new(48000).unwrap();
        r.publish(&s);
        (s, r)
    }
    fn snapshot(s: &Shared) -> Status {
        s.status(0).unwrap()
    }
    fn ay(ch: u8, level: u8) -> Vec<Value> {
        vec![
            json!({"method":"write","register":ch*2,"value":127}),
            json!({"method":"write","register":ch*2+1,"value":1}),
            json!({"method":"write","register":8+ch,"value":level}),
        ]
    }
    fn note(ch: u8) -> Vec<Value> {
        vec![
            json!({"method":"write","register":0x30+ch,"value":0x15}),
            json!({"method":"write","register":0x10+ch,"value":0x90}),
            json!({"method":"write","register":0x20+ch,"value":0x17}),
        ]
    }
    #[test]
    fn msx_native_validates_channels_registers_levels_clocks_and_atomic_batches() {
        for v in [
            json!({"chip":"ay8910","method":"write","register":16,"value":0}),
            json!({"chip":"ym2413","method":"write","register":64,"value":0}),
            json!({"chip":"ay8910","method":"key-off","channel":3}),
            json!({"chip":"ym2413","method":"audition-key-on","channel":14}),
            json!({"chip":"ay8910","method":"set-retained-level","channel":0,"level":32}),
            json!({"chip":"ym2413","method":"set-retained-level","channel":0,"level":1}),
            json!({"chip":"ay8910","method":"clock","hz":99999}),
            json!({"chip":"ay8910","method":"source-mute","mask":8}),
            lease("ay8910", 0, 1, "on", ay(1, 9)),
            lease("ym2413", 0, 1, "on", note(1)),
        ] {
            assert!(serde_json::from_value::<Message>(v)
                .unwrap()
                .prepare()
                .is_err());
        }
    }
    #[test]
    fn msx_native_snapshot_observes_applied_state_without_advancing_clock() {
        let (s, mut r) = setup();
        let a = snapshot(&s);
        assert_eq!(a.ay8910.clock, AY_CLOCK);
        assert_eq!(a.ym2413.clock, CLOCK);
        assert_eq!(a.ay8910.levels, [9; 3]);
        assert_eq!(a.ay8910.registers.len(), 16);
        assert_eq!(a.ym2413.registers.len(), 64);
        for _ in 0..256 {
            let mut p = [0.; 2];
            r.mix(&mut p);
            assert_eq!(p[0], p[1]);
        }
        assert_eq!(snapshot(&s).revision, a.revision);
    }
    #[test]
    fn msx_native_manual_ay_blocks_affected_shared_noise_envelope_and_preserves_mixer_bits() {
        let (s, mut r) = setup();
        apply(&mut r, &s, write("ay8910", 7, 0), None);
        apply(&mut r, &s, write("ay8910", 6, 5), None);
        apply(&mut r, &s, write("ay8910", 11, 7), None);
        apply(&mut r, &s, write("ay8910", 12, 8), None);
        apply(&mut r, &s, write("ay8910", 13, 10), None);
        apply(&mut r, &s, lease("ay8910", 0, 1, "on", ay(0, 16)), None);
        for (reg, v) in [
            (0, 1),
            (1, 2),
            (6, 31),
            (7, 63),
            (8, 1),
            (11, 31),
            (12, 32),
            (13, 4),
        ] {
            apply(&mut r, &s, write("ay8910", reg, v), owner(1));
        }
        apply(
            &mut r,
            &s,
            json!({"chip":"ay8910","method":"set-retained-level","channel":0,"level":0}),
            owner(1),
        );
        apply(
            &mut r,
            &s,
            json!({"chip":"ay8910","method":"clock","hz":4000000}),
            owner(1),
        );
        let out = snapshot(&s);
        assert_eq!(out.ay8910.registers[0], 127);
        assert_eq!(out.ay8910.registers[1], 1);
        assert_eq!(out.ay8910.registers[6], 5);
        assert_eq!(out.ay8910.registers[7], 54);
        assert_eq!(&out.ay8910.registers[11..14], &[7, 8, 10]);
        assert_eq!(out.ay8910.levels[0], 16);
        assert_eq!(out.ay8910.held, [0]);
        assert_eq!(out.ay8910.clock, AY_CLOCK);
        apply(
            &mut r,
            &s,
            lease(
                "ay8910",
                0,
                1,
                "edit",
                vec![json!({"method":"write","register":13,"value":4})],
            ),
            None,
        );
        assert_eq!(snapshot(&s).ay8910.registers[13], 4);
    }
    #[test]
    fn msx_native_unaffected_shared_ay_parameters_and_other_voices_remain_writable() {
        let (s, mut r) = setup();
        apply(&mut r, &s, write("ay8910", 7, 0x38), None);
        apply(&mut r, &s, lease("ay8910", 0, 1, "on", ay(0, 9)), None);
        for (reg, v) in [(6, 31), (11, 19), (12, 32), (13, 4), (2, 33), (9, 6)] {
            apply(&mut r, &s, write("ay8910", reg, v), owner(1));
        }
        let a = snapshot(&s);
        assert_eq!(a.ay8910.registers[6], 31);
        assert_eq!(&a.ay8910.registers[11..14], &[19, 32, 4]);
        assert_eq!(a.ay8910.registers[2], 33);
        assert_eq!(a.ay8910.registers[9], 6);
    }
    #[test]
    fn msx_native_stop_retains_ay_authored_levels_including_explicit_zero_and_all_shared_parameters(
    ) {
        let (s, mut r) = setup();
        for (reg, v) in [
            (0, 33),
            (1, 2),
            (6, 7),
            (7, 0x38),
            (8, 12),
            (9, 16),
            (10, 5),
            (11, 1),
            (12, 2),
            (13, 14),
            (14, 0x55),
            (15, 0xaa),
        ] {
            apply(&mut r, &s, write("ay8910", reg, v), None);
        }
        apply(
            &mut r,
            &s,
            json!({"chip":"ay8910","method":"set-retained-level","channel":2,"level":0}),
            None,
        );
        apply(
            &mut r,
            &s,
            json!({"chip":"ay8910","method":"clock","hz":2000000}),
            None,
        );
        r.stop(&s);
        let out = snapshot(&s);
        assert_eq!(&out.ay8910.registers[8..11], &[0; 3]);
        assert_eq!(out.ay8910.levels, [12, 16, 0]);
        assert_eq!(out.ay8910.registers[7], 0x38);
        assert_eq!(&out.ay8910.registers[11..], &[1, 2, 14, 0x55, 0xaa]);
        assert_eq!(out.ay8910.clock, 2000000);
        for ch in 0..3 {
            apply(
                &mut r,
                &s,
                json!({"chip":"ay8910","method":"audition-key-on","channel":ch}),
                None,
            );
        }
        assert_eq!(&snapshot(&s).ay8910.registers[8..11], &[12, 16, 0]);
    }
    #[test]
    fn msx_native_stale_ay_lease_release_and_owner_cancel_do_not_stop_replacement() {
        let (s, mut r) = setup();
        apply(&mut r, &s, lease("ay8910", 0, 1, "on", ay(0, 9)), None);
        apply(&mut r, &s, lease("ay8910", 0, 2, "on", ay(0, 12)), None);
        apply(
            &mut r,
            &s,
            lease(
                "ay8910",
                0,
                1,
                "off",
                vec![json!({"method":"key-off","channel":0})],
            ),
            None,
        );
        apply(
            &mut r,
            &s,
            json!({"chip":"ay8910","method":"batch","channel":1,"voice":3,"action":"on","commands":ay(1,8)}),
            owner(1),
        );
        apply(
            &mut r,
            &s,
            json!({"chip":"ay8910","method":"batch","channel":2,"voice":4,"action":"on","commands":ay(2,7)}),
            owner(2),
        );
        r.cancel_invalid(|o| o != owner(1), &s);
        let a = snapshot(&s);
        assert_eq!(&a.ay8910.registers[8..11], &[12, 0, 7]);
        assert_eq!(a.ay8910.held, [0]);
    }
    #[test]
    fn msx_native_opll_patch_rhythm_and_physical_drum_leases_match_master_system() {
        let (s, mut r) = setup();
        apply(&mut r, &s, lease("ym2413", 0, 1, "on", note(0)), None);
        apply(&mut r, &s, write("ym2413", 0, 0x71), owner(1));
        apply(&mut r, &s, write("ym2413", 14, 0x20), owner(1));
        assert_eq!(snapshot(&s).ym2413.registers[0], 0);
        assert_eq!(snapshot(&s).ym2413.registers[14], 0);
        apply(
            &mut r,
            &s,
            lease(
                "ym2413",
                0,
                1,
                "off",
                vec![json!({"method":"key-off","channel":0})],
            ),
            None,
        );
        apply(&mut r, &s, write("ym2413", 14, 0x20), None);
        for (channel, voice) in [(10, 2), (13, 3)] {
            apply(
                &mut r,
                &s,
                lease(
                    "ym2413",
                    channel,
                    voice,
                    "on",
                    vec![json!({"method":"audition-key-on","channel":channel})],
                ),
                None,
            );
        }
        apply(
            &mut r,
            &s,
            lease(
                "ym2413",
                10,
                2,
                "off",
                vec![json!({"method":"key-off","channel":10})],
            ),
            None,
        );
        for _ in 0..32 {
            r.mix(&mut [0.; 2]);
        }
        r.publish(&s);
        let a = snapshot(&s);
        assert_eq!(a.ym2413.held, [13]);
        assert_eq!(a.ym2413.registers[14] & 9, 1);
    }
    #[test]
    fn msx_native_admission_is_bounded_atomic_and_reclaimed_off_callback() {
        let (s, mut r) = setup();
        let c = prep(json!({"chip":"ay8910","method":"write","register":8,"value":9,"id":1}));
        s.retain(&[(c.clone(), None)]).unwrap();
        assert!(s.retain(&[(c.clone(), None)]).is_err());
        assert_eq!(snapshot(&s).ay8910.pending_commands, [1]);
        r.apply(&c, None, &s);
        s.retire(c);
        assert!(snapshot(&s).ay8910.pending_commands.is_empty());
        assert!(s.control.lock().unwrap().prepared.is_empty());
        assert!(s.wait_applied(1).is_ok());
        assert!(s.wait_applied(2).is_err());
    }
}
#[cfg(test)]
mod pipeline_tests {
    use super::super::*;
    use serde_json::{json, Value};
    fn command(frame: u64, message: Value) -> Command {
        serde_json::from_value(json!({"frame":frame,"type":"msx","message":message})).unwrap()
    }
    fn tagged(frame: u64, message: Value, owner: u32) -> Command {
        let mut c = command(frame, message);
        c.performance = Some(1);
        c.owner = Some(owner);
        c
    }
    fn setup() -> (Shared, Renderer) {
        let s = Shared::new_modes(48000, 1, false, false, false, Some("msx"));
        let r = Renderer::new_modes(48000, false, false, false, Some("msx")).unwrap();
        r.publish_primary(&s);
        (s, r)
    }
    fn render(s: &Shared, r: &mut Renderer, frames: usize) -> Vec<[f32; 2]> {
        r.prepare(s);
        let out = (0..frames).map(|_| r.sample(s)).collect();
        s.frame.store(r.frame, Ordering::Release);
        r.publish_primary(s);
        out
    }
    fn note(ch: u8) -> Value {
        json!({"chip":"ym2413","method":"batch","channel":ch,"action":"on","voice":ch+1,"commands":[{"method":"write","register":0x30+ch,"value":0x15},{"method":"write","register":0x10+ch,"value":0x90},{"method":"write","register":0x20+ch,"value":0x17}]})
    }
    fn ay() -> Value {
        json!({"chip":"ay8910","method":"batch","channel":0,"action":"on","voice":10,"commands":[{"method":"write","register":0,"value":5},{"method":"write","register":1,"value":1},{"method":"write","register":7,"value":56},{"method":"write","register":8,"value":9}]})
    }
    fn snapshot(s: &Shared) -> msx::Status {
        s.msx
            .as_ref()
            .unwrap()
            .status(s.frame.load(Ordering::Acquire))
            .unwrap()
    }
    #[test]
    fn msx_native_primary_has_no_hidden_genesis_and_rejects_foreign_commands_atomically() {
        let (s, mut r) = setup();
        assert!(r.core.0.is_null());
        assert!(r.msx.is_some());
        assert!(r.dx7.is_none() && r.x68000.is_none() && r.rf5c164.is_none());
        assert_eq!(s.status().mode, "msx");
        assert_eq!(s.status().ym2413_clock, Some(msx::CLOCK));
        assert_eq!(s.status().psg_clock, 0);
        assert_eq!(s.status().ay8910_clock, Some(msx::AY_CLOCK));
        for value in [
            json!({"frame":0,"type":"write","register":40,"value":0}),
            json!({"frame":0,"type":"psg-write","value":128}),
            json!({"frame":0,"type":"rf5c164-reset"}),
        ] {
            let invalid: Command = serde_json::from_value(value).unwrap();
            assert!(s.enqueue(1, &[command(0, note(0)), invalid]).is_err());
            assert_eq!(s.pending.load(Ordering::Acquire), 0)
        }
        assert!(render(&s, &mut r, 128).iter().all(|v| *v == [0.; 2]));
        let genesis = Shared::new(48000, 2);
        assert!(genesis.enqueue(2, &[command(0, note(0))]).is_err());
    }
    #[test]
    fn msx_native_callback_segmentation_and_scheduled_frames_are_exact() {
        let (a, mut left) = setup();
        let (b, mut right) = setup();
        let cmds = [command(100, note(0)), command(175, ay())];
        a.enqueue(1, &cmds).unwrap();
        b.enqueue(1, &cmds).unwrap();
        let all = render(&a, &mut left, 4096);
        let mut split = Vec::new();
        for n in [1, 31, 128, 384, 1024, 2528] {
            split.extend(render(&b, &mut right, n));
        }
        assert_eq!(all, split);
        assert!(all[..100].iter().all(|v| *v == [0.; 2]));
        assert!(all[500..].iter().any(|v| v[0].abs() > 0.0001));
        assert!(all.iter().all(|v| v[0] == v[1]));
    }
    #[test]
    fn msx_native_chip_scoped_stop_cancels_only_matching_chip_and_owner_future() {
        let (s, mut r) = setup();
        s.performance("start", 1, None).unwrap();
        s.enqueue(
            1,
            &[
                tagged(0, note(0), 1),
                tagged(0, note(1), 2),
                tagged(1000, note(0), 1),
                tagged(1200, note(1), 2),
                tagged(1100, ay(), 1),
            ],
        )
        .unwrap();
        render(&s, &mut r, 256);
        s.enqueue(
            1,
            &[tagged(
                0,
                json!({"chip":"ym2413","method":"stop-voices"}),
                1,
            )],
        )
        .unwrap();
        render(&s, &mut r, 1500);
        let out = snapshot(&s);
        assert_eq!(out.ym2413.registers[0x20] & 16, 0);
        assert_ne!(out.ym2413.registers[0x21] & 16, 0);
        assert_eq!(out.ay8910.registers[8], 9);
        assert!(out.ym2413.pending_commands.is_empty());
        assert!(out.ay8910.pending_commands.is_empty());
    }
    #[test]
    fn msx_native_epoch_stop_cancels_future_and_releases_both_chips_preserving_authored_data() {
        let (s, mut r) = setup();
        s.enqueue(
            1,
            &[
                command(0, note(0)),
                command(0, ay()),
                command(5000, note(1)),
            ],
        )
        .unwrap();
        render(&s, &mut r, 256);
        s.epoch.fetch_add(1, Ordering::AcqRel);
        render(&s, &mut r, 6000);
        let out = snapshot(&s);
        assert_eq!(&out.ay8910.registers[8..11], &[0; 3]);
        assert_eq!(out.ay8910.registers[0], 5);
        assert_eq!(out.ym2413.registers[0x30], 0x15);
        assert_eq!(out.ym2413.registers[0x20] & 16, 0);
        assert_eq!(out.ym2413.registers[0x21] & 16, 0);
        assert_eq!(s.pending.load(Ordering::Acquire), 0);
    }
    #[test]
    fn msx_native_independent_ay_opll_mixers_reach_the_same_final_pcm() {
        let (s, mut r) = setup();
        s.enqueue(
            1,
            &[
                command(0, note(0)),
                command(0, ay()),
                command(
                    0,
                    json!({"chip":"ay8910","method":"mix","volume":1,"pan":-1}),
                ),
                command(
                    0,
                    json!({"chip":"ym2413","method":"mix","volume":1,"pan":1}),
                ),
            ],
        )
        .unwrap();
        let out = render(&s, &mut r, 4096);
        assert!(out[512..].iter().any(|v| v[0].abs() > 0.0001));
        assert!(out[512..].iter().any(|v| v[1].abs() > 0.0001));
        assert!(out[512..].iter().any(|v| v[0] != v[1]));
        s.enqueue(
            1,
            &[command(
                0,
                json!({"chip":"ay8910","method":"mix","volume":0,"pan":0}),
            )],
        )
        .unwrap();
        let out = render(&s, &mut r, 1024);
        assert!(out[512..].iter().all(|v| v[0] == 0.));
        assert!(out[512..].iter().any(|v| v[1].abs() > 0.0001));
    }
}

#[cfg(test)]
mod msx_audio_pipeline_tests {
    use super::super::*;
    use serde_json::{json, Value};
    fn command(frame: u64, message: Value) -> Command {
        let kind = if ["y8950", "ym3526", "ym3812", "ymf262"]
            .iter()
            .any(|chip| message["chip"] == *chip)
        {
            "opl"
        } else {
            "msx"
        };
        serde_json::from_value(json!({"frame":frame,"type":kind,"message":message})).unwrap()
    }
    fn setup() -> (Shared, Renderer) {
        let s = Shared::new_modes(48000, 1, false, false, false, Some("msx-audio"));
        let r = Renderer::new_modes(48000, false, false, false, Some("msx-audio")).unwrap();
        r.publish_primary(&s);
        (s, r)
    }
    fn render(s: &Shared, r: &mut Renderer, frames: usize) -> Vec<[f32; 2]> {
        r.prepare(s);
        let out = (0..frames).map(|_| r.sample(s)).collect();
        s.frame.store(r.frame, Ordering::Release);
        r.publish_primary(s);
        out
    }
    fn snapshot(s: &Shared) -> MsxAudioStatus {
        s.msx_audio_status().unwrap()
    }
    fn write(chip: &str, register: u16, value: u8) -> Value {
        json!({"chip":chip,"method":"write","register":register,"value":value})
    }
    fn ay_note() -> Value {
        json!({"chip":"ay8910","method":"batch","channel":0,"action":"on","voice":1,"commands":[{"method":"write","register":0,"value":127},{"method":"write","register":1,"value":1},{"method":"write","register":7,"value":62},{"method":"write","register":8,"value":9}]})
    }
    fn opll_note(ch: u8) -> Value {
        json!({"chip":"ym2413","method":"batch","channel":ch,"action":"on","voice":ch+1,"commands":[{"method":"write","register":0x30+ch,"value":0x15},{"method":"write","register":0x10+ch,"value":0x90},{"method":"write","register":0x20+ch,"value":0x17}]})
    }
    fn y_note(ch: u8) -> Value {
        let slot = [0u16, 1, 2, 8, 9, 10, 16, 17, 18][ch as usize];
        let mut ops = Vec::new();
        for (op, tl) in [(slot, 63), (slot + 3, 0)] {
            for (register, value) in [
                (0x20 + op, 0x21),
                (0x40 + op, tl),
                (0x60 + op, 0xf0),
                (0x80 + op, 0x0f),
            ] {
                ops.push(json!({"method":"write","register":register,"value":value}))
            }
        }
        for (register, value) in [
            (0xa0 + ch as u16, 0x98),
            (0xc0 + ch as u16, 1),
            (0xb0 + ch as u16, 0x31),
        ] {
            ops.push(json!({"method":"write","register":register,"value":value}))
        }
        json!({"chip":"y8950","method":"batch","channel":ch,"voice":ch+1,"action":"on","commands":ops})
    }
    fn notes(frame: u64) -> Vec<Command> {
        vec![
            command(frame, ay_note()),
            command(frame, opll_note(0)),
            command(frame, y_note(0)),
        ]
    }
    fn tagged(mut command: Command, owner: u32) -> Command {
        command.performance = Some(1);
        command.owner = Some(owner);
        command
    }
    #[test]
    fn msx_audio_native_selected_three_chips_status_clocks_and_profile_guards() {
        let (s, r) = setup();
        assert!(r.core.0.is_null());
        assert!(r.master_system.is_none() && r.ym2608.is_none() && r.ym2610b.is_none());
        assert!(r.msx.is_some() && r.opl.is_some());
        let status = s.status();
        assert_eq!(status.mode, "msx-audio");
        assert_eq!(status.psg_clock, 0);
        assert_eq!(status.y8950_clock, Some(3_579_545));
        assert_eq!(status.ym2413_clock, Some(3_579_545));
        assert_eq!(status.ay8910_clock, Some(1_789_773));
        let state = snapshot(&s);
        assert_eq!(state.y8950.chip, "y8950");
        assert_eq!(state.y8950.channels, 9);
        assert_eq!(state.y8950.registers.len(), 256);
        assert_eq!(state.ym2413.registers.len(), 64);
        assert_eq!(state.ay8910.registers.len(), 16);
        for mode in [
            "msx",
            "ym3526",
            "ym3812",
            "ymf262",
            "master-system",
            "game-gear",
        ] {
            let other = Shared::new_modes(48000, 1, false, false, false, Some(mode));
            assert!(other.enqueue(1, &[command(0, y_note(0))]).is_err());
            assert!(other.msx_audio_status().is_err())
        }
        let foreign =
            serde_json::from_value(json!({"type":"psg-write","frame":0,"value":0x90})).unwrap();
        assert!(s.enqueue(1, &[foreign]).is_err());
        assert!(s
            .enqueue(1, &[command(0, write("ym3812", 0x20, 1))])
            .is_err());
        assert_eq!(s.pending.load(Ordering::Acquire), 0);
        assert!(opl::Chip::from_mode("y8950").is_none());
    }
    #[test]
    fn msx_audio_native_real_mixed_pcm_equals_sum_of_three_independent_same_cores() {
        let (all, mut ar) = setup();
        let mut singles = Vec::new();
        for source in 0..3 {
            let (s, mut r) = setup();
            let selected = notes(0).remove(source);
            let chip = ["ay8910", "ym2413", "y8950"][source];
            s.enqueue(
                1,
                &[
                    command(0, json!({"chip":chip,"method":"mix","volume":0.25,"pan":0})),
                    selected,
                ],
            )
            .unwrap();
            singles.push(render(&s, &mut r, 4096))
        }
        let mut commands = notes(0);
        for chip in ["ay8910", "ym2413", "y8950"] {
            commands.push(command(
                0,
                json!({"chip":chip,"method":"mix","volume":0.25,"pan":0}),
            ))
        }
        all.enqueue(1, &commands).unwrap();
        let mixed = render(&all, &mut ar, 4096);
        for source in &singles {
            assert!(source[512..].iter().any(|p| p[0].abs() > 0.0001))
        }
        for i in 512..4096 {
            for ch in 0..2 {
                let expected = (singles[0][i][ch] + singles[1][i][ch]) + singles[2][i][ch];
                assert!((mixed[i][ch] - expected).abs() < 1e-7)
            }
        }
    }
    #[test]
    fn msx_audio_native_timestamp_order_and_callback_segmentation_are_exact() {
        let (s, mut r) = setup();
        let (t, mut q) = setup();
        let mut commands = notes(100);
        commands.push(command(1000, write("y8950", 0xb0, 0x11)));
        commands.push(command(1500, write("ym2413", 0x20, 7)));
        commands.push(command(2000, write("ay8910", 8, 0)));
        s.enqueue(1, &commands).unwrap();
        t.enqueue(1, &commands).unwrap();
        let full = render(&s, &mut r, 4096);
        let mut pieces = Vec::new();
        for n in [33, 467, 1, 999, 2596] {
            pieces.extend(render(&t, &mut q, n))
        }
        assert_eq!(full, pieces);
        assert!(full[..100].iter().all(|p| *p == [0.; 2]));
        let a = snapshot(&s);
        assert_eq!(a.y8950.registers[0xb0] & 32, 0);
        assert_eq!(a.ym2413.registers[0x20] & 16, 0);
        assert_eq!(a.ay8910.registers[8], 0);
        assert!(
            a.y8950.pending_commands.is_empty()
                && a.ym2413.pending_commands.is_empty()
                && a.ay8910.pending_commands.is_empty()
        );
    }
    #[test]
    fn msx_audio_native_independent_mixers_clock_sources_and_master_fx_recording() {
        let (dry, mut dr) = setup();
        let (wet, mut wr) = setup();
        let mut source_commands = notes(0);
        for chip in ["ay8910", "ym2413", "y8950"] {
            source_commands.push(command(
                0,
                json!({"chip":chip,"method":"mix","volume":0.25,"pan":0}),
            ));
        }
        dry.enqueue(1, &source_commands).unwrap();
        wet.enqueue(1, &source_commands).unwrap();
        let fx:FxMessage=serde_json::from_value(json!({"op":"chain","children":[{"type":"gain","slot":0}],"units":[{"type":"gain","slot":0,"values":{"gain":0.5}}],"beatSeconds":0.5})).unwrap();
        let graph = wet.fx.lock().unwrap().command(fx, 48000).unwrap().unwrap();
        wet.enqueue_special(Action::Fx(graph)).unwrap();
        wet.enqueue(
            1,
            &[serde_json::from_value(
                json!({"type":"mix","frame":0,"fmLeft":1,"fmRight":1,"psgLeft":1,"psgRight":1,"master":0.5}),
            )
            .unwrap()],
        )
        .unwrap();
        wet.record_start(0.02).unwrap();
        let reference = render(&dry, &mut dr, 2048);
        let pcm = render(&wet, &mut wr, 2048);
        assert!(reference[512..].iter().any(|p| p[0].abs() > 0.001));
        for i in 512..2048 {
            for ch in 0..2 {
                assert!((pcm[i][ch] - reference[i][ch] * 0.25).abs() < 1e-7)
            }
        }
        let captured = wet.record_stop().unwrap();
        for ch in 0..2 {
            assert_eq!(
                captured.channels[ch],
                pcm[..960].iter().map(|p| p[ch]).collect::<Vec<_>>()
            )
        }
        wet.enqueue(
            1,
            &[
                command(0, json!({"chip":"ay8910","method":"clock","hz":2_000_000})),
                command(0, json!({"chip":"ym2413","method":"clock","hz":4_000_000})),
                command(0, json!({"chip":"y8950","method":"clock","hz":5_000_000})),
                command(
                    0,
                    json!({"chip":"ay8910","method":"mix","volume":0,"pan":0}),
                ),
                command(
                    0,
                    json!({"chip":"ym2413","method":"mix","volume":1,"pan":-1}),
                ),
                command(0, json!({"chip":"y8950","method":"mix","volume":1,"pan":1})),
            ],
        )
        .unwrap();
        let pcm = render(&wet, &mut wr, 1024);
        assert!(pcm[512..]
            .iter()
            .any(|p| p[0] != 0. && p[1] != 0. && p[0] != p[1]));
        let a = snapshot(&wet);
        assert_eq!(
            (a.ay8910.clock, a.ym2413.clock, a.y8950.clock),
            (2_000_000, 4_000_000, 5_000_000)
        );
        wet.enqueue(
            1,
            &[command(
                0,
                json!({"chip":"y8950","method":"mix","volume":0,"pan":0}),
            )],
        )
        .unwrap();
        let pcm = render(&wet, &mut wr, 1024);
        assert!(pcm[512..].iter().all(|p| p[1] == 0.));
        assert!(pcm[512..].iter().any(|p| p[0] != 0.));
    }
    #[test]
    fn msx_audio_native_cross_module_admission_failure_does_not_retain_or_publish_any_prefix() {
        let (s, mut r) = setup();
        let mut manual = y_note(0);
        manual["audition"] = json!(true);
        s.enqueue(1, &[command(0, manual)]).unwrap();
        render(&s, &mut r, 128);
        let before = snapshot(&s);
        let failed = vec![
            command(
                0,
                json!({"chip":"ay8910","id":123,"method":"write","register":0,"value":1}),
            ),
            command(
                0,
                json!({"chip":"y8950","id":123,"method":"clock","hz":4_000_000}),
            ),
        ];
        assert!(s.enqueue(1, &failed).unwrap_err().contains("held"));
        assert_eq!(s.pending.load(Ordering::Acquire), 0);
        assert!(s.queue.is_empty());
        let after = snapshot(&s);
        assert_eq!(after.ay8910.registers, before.ay8910.registers);
        assert!(after.ay8910.pending_commands.is_empty());
        assert!(s
            .msx
            .as_ref()
            .unwrap()
            .wait_applied(123)
            .unwrap_err()
            .contains("Unknown"));
        let failed = vec![
            command(
                0,
                json!({"chip":"ay8910","id":5,"method":"write","register":0,"value":1}),
            ),
            command(
                0,
                json!({"chip":"ym2413","id":5,"method":"write","register":0,"value":1}),
            ),
            command(
                0,
                json!({"chip":"y8950","id":8,"method":"write","register":0x20,"value":3}),
            ),
        ];
        assert!(s.enqueue(1, &failed).is_err());
        assert!(s
            .opl
            .as_ref()
            .unwrap()
            .status(0)
            .unwrap()
            .y8950
            .unwrap()
            .pending_commands
            .is_empty());
        assert!(s
            .opl
            .as_ref()
            .unwrap()
            .wait_applied(8)
            .unwrap_err()
            .contains("Unknown"));
    }
    #[test]
    fn msx_audio_native_owner_cancellation_and_chip_stop_do_not_suppress_other_sources() {
        let (s, mut r) = setup();
        s.active_performance.store(1, Ordering::Release);
        let original = vec![
            tagged(command(0, y_note(0)), 1),
            tagged(command(0, opll_note(0)), 2),
            tagged(command(0, ay_note()), 3),
        ];
        s.enqueue(1, &original).unwrap();
        render(&s, &mut r, 512);
        s.cancelled_owners[0].store(true, Ordering::Release);
        s.invalidation.fetch_add(1, Ordering::Release);
        render(&s, &mut r, 512);
        let a = snapshot(&s);
        assert_eq!(a.y8950.registers[0xb0] & 32, 0);
        assert_ne!(a.ym2413.registers[0x20] & 16, 0);
        assert_eq!(a.ay8910.registers[8], 9);
        s.enqueue(
            1,
            &[
                tagged(command(5000, y_note(1)), 2),
                tagged(command(5000, opll_note(1)), 2),
                tagged(command(5000, ay_note()), 3),
            ],
        )
        .unwrap();
        s.enqueue(
            1,
            &[tagged(
                command(0, json!({"chip":"y8950","method":"stop-voices"})),
                2,
            )],
        )
        .unwrap();
        render(&s, &mut r, 5000);
        let a = snapshot(&s);
        assert_eq!(a.y8950.registers[0xb1] & 32, 0);
        assert_ne!(a.ym2413.registers[0x21] & 16, 0);
        assert_eq!(a.ay8910.registers[8], 9);
        assert!(a.y8950.pending_commands.is_empty());
    }
    #[test]
    fn msx_audio_native_epoch_stop_retains_all_three_authored_sources_and_clocks() {
        let (s, mut r) = setup();
        let mut commands = notes(0);
        commands.extend(notes(5000));
        for (chip, hz) in [
            ("ay8910", 2_000_000),
            ("ym2413", 4_000_000),
            ("y8950", 5_000_000),
        ] {
            commands.push(command(0, json!({"chip":chip,"method":"clock","hz":hz})))
        }
        s.enqueue(1, &commands).unwrap();
        render(&s, &mut r, 512);
        let before = snapshot(&s);
        s.epoch.fetch_add(1, Ordering::Release);
        let pcm = render(&s, &mut r, 6000);
        assert!(pcm[1000..].iter().all(|p| *p == [0.; 2]));
        let a = snapshot(&s);
        assert_eq!(
            (a.ay8910.clock, a.ym2413.clock, a.y8950.clock),
            (2_000_000, 4_000_000, 5_000_000)
        );
        assert_eq!(a.y8950.registers[0x43], before.y8950.registers[0x43]);
        assert_eq!(a.ym2413.registers[0x30], before.ym2413.registers[0x30]);
        assert_eq!(a.ay8910.levels, before.ay8910.levels);
        assert_eq!(a.y8950.registers[0xb0] & 32, 0);
        assert_eq!(a.ym2413.registers[0x20] & 16, 0);
        assert_eq!(a.ay8910.registers[8], 0);
        assert_eq!(s.pending.load(Ordering::Acquire), 0);
        s.enqueue(1, &notes(0)).unwrap();
        assert!(render(&s, &mut r, 1024).iter().any(|p| p[0] != 0.));
    }
    #[test]
    fn msx_audio_native_y8950_raw_delta_t_register_and_port_state_match_existing_empty_memory_scope(
    ) {
        let (s, mut r) = setup();
        let registers = [
            (0x07, 0x80),
            (0x08, 0xc0),
            (0x09, 0x40),
            (0x0a, 0),
            (0x0b, 0),
            (0x0c, 0xff),
            (0x0d, 0x10),
            (0x0e, 0),
            (0x0f, 0xff),
            (0x10, 0x20),
            (0x19, 0x55),
            (0x1a, 0xaa),
        ];
        let commands: Vec<_> = registers
            .into_iter()
            .map(|(reg, value)| command(0, write("y8950", reg, value)))
            .collect();
        s.enqueue(1, &commands).unwrap();
        render(&s, &mut r, 1024);
        let a = snapshot(&s);
        for (reg, value) in registers {
            assert_eq!(a.y8950.registers[reg as usize], value)
        }
        assert_eq!(a.y8950.ports[0], a.y8950.status);
        assert_eq!(a.y8950.ports[1], 0);
        let before = a.y8950.registers;
        for _ in 0..20 {
            assert_eq!(snapshot(&s).y8950.registers, before)
        }
        assert_eq!(snapshot(&s).ay8910.registers, [0; 16]);
    }
    #[test]
    fn msx_audio_native_y8950_manual_fm_protects_shared_bits_and_allows_independent_adpcm() {
        let (s, mut r) = setup();
        let mut manual = y_note(0);
        manual["audition"] = json!(true);
        s.enqueue(1, &[command(0, manual)]).unwrap();
        render(&s, &mut r, 256);
        s.enqueue(
            1,
            &[
                command(0, write("y8950", 0x43, 63)),
                command(0, write("y8950", 7, 0x80)),
                command(0, write("y8950", 8, 0x80)),
                command(0, opll_note(1)),
                command(0, ay_note()),
            ],
        )
        .unwrap();
        render(&s, &mut r, 256);
        let state = snapshot(&s);
        assert_eq!(state.y8950.held, [0]);
        assert_eq!(state.y8950.registers[0x43], 0);
        assert_eq!(state.y8950.registers[7], 0x80);
        assert_eq!(state.y8950.registers[8], 0);
        assert_ne!(state.ym2413.registers[0x21] & 16, 0);
        assert_eq!(state.ay8910.registers[8], 9);
        assert_eq!(
            s.enqueue(
                1,
                &[command(
                    0,
                    json!({"chip":"y8950","method":"clock","hz":4_000_000})
                )]
            )
            .unwrap_err(),
            "Release the manually held OPL voices before changing the clock"
        );
        s.enqueue(1, &[command(0, write("ay8910", 0, 0x40))])
            .unwrap();
        render(&s, &mut r, 256);
        assert_eq!(snapshot(&s).ay8910.registers[0], 0x40);
    }
}
