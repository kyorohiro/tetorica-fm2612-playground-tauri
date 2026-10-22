//! Callback-owned DOS PC OPL core. Commands allocate and retire on the control
//! thread; atomic snapshots only observe state already applied to the renderer.
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
const VOICES: usize = 18;
const PENDING_BYTES: usize = 4 * 1024 * 1024;
#[repr(C)]
struct CoreState {
    registers: [u8; 512],
    clock: u32,
    mute_mask: u32,
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
    fn native_opl_create(rate: u32, kind: u8) -> *mut c_void;
    fn native_opl_destroy(p: *mut c_void);
    fn native_opl_reset(p: *mut c_void);
    fn native_opl_stop(p: *mut c_void);
    fn native_opl_write(p: *mut c_void, register: u16, value: u8);
    fn native_opl_sample(p: *mut c_void, left: *mut f32, right: *mut f32);
    fn native_opl_set_clock(p: *mut c_void, hz: u32) -> i32;
    fn native_opl_set_mute_mask(p: *mut c_void, mask: u32);
    fn native_opl_key_off(p: *mut c_void, voice: u8);
    fn native_opl_audition_key_on(p: *mut c_void, voice: u8);
    fn native_opl_read(p: *mut c_void, port: u8) -> u8;
    fn native_opl_get_state(p: *mut c_void, state: *mut CoreState);
}
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Chip {
    Ym3526,
    Ym3812,
    Ymf262,
}
impl Chip {
    pub fn from_mode(mode: &str) -> Option<Self> {
        match mode {
            "ym3526" => Some(Self::Ym3526),
            "ym3812" => Some(Self::Ym3812),
            "ymf262" => Some(Self::Ymf262),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Ym3526 => "ym3526",
            Self::Ym3812 => "ym3812",
            Self::Ymf262 => "ymf262",
        }
    }
    fn code(self) -> u8 {
        match self {
            Self::Ym3526 => 0,
            Self::Ym3812 => 1,
            Self::Ymf262 => 2,
        }
    }
    fn channels(self) -> u8 {
        if self == Self::Ymf262 {
            18
        } else {
            9
        }
    }
    fn clock(self) -> u32 {
        if self == Self::Ymf262 {
            14_318_180
        } else {
            3_579_545
        }
    }
    fn registers(self) -> usize {
        if self == Self::Ymf262 {
            512
        } else {
            256
        }
    }
    fn valid_voice(self, ch: u8) -> bool {
        ch < self.channels() || (18..23).contains(&ch)
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
        register: u16,
        value: u8,
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
    Write { register: u16, value: u8 },
    KeyOff { channel: u8 },
    AuditionKeyOn { channel: u8 },
    Clock { hz: u32 },
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
    clock_rejected: AtomicBool,
    outcome_observed: AtomicBool,
}
fn valid_id(id: Option<u64>) -> bool {
    id.is_none_or(|v| v > 0 && v <= MAX_SAFE_INTEGER)
}
fn static_physical(ch: u8) -> usize {
    if ch >= 18 {
        [6, 7, 8, 8, 7][(ch - 18) as usize]
    } else {
        ch as usize
    }
}
fn register_channel(register: u16) -> Option<u8> {
    let bank = if register & 256 != 0 { 9 } else { 0 };
    let r = register as u8;
    if matches!(r,0xa0..=0xa8|0xb0..=0xb8|0xc0..=0xc8) {
        return Some(bank + (r & 15));
    }
    if matches!(r & 0xe0, 0x20 | 0x40 | 0x60 | 0x80 | 0xe0) {
        let slot = r & 31;
        let channel = match slot {
            0..=5 => slot % 3,
            8..=13 => 3 + (slot - 8) % 3,
            16..=21 => 6 + (slot - 16) % 3,
            _ => return None,
        };
        return Some(bank + channel);
    }
    None
}
fn declared_channel(op: &Op) -> Option<u8> {
    match *op {
        Op::KeyOff { channel } | Op::AuditionKeyOn { channel } => Some(channel),
        Op::Write { register, .. } => register_channel(register),
        _ => None,
    }
}
fn paired(a: usize, b: usize) -> bool {
    a == b
        || [0, 1, 2, 9, 10, 11]
            .into_iter()
            .any(|p| (a == p && b == p + 3) || (b == p && a == p + 3))
}
fn validate_op(op: &Op, chip: Chip) -> Result<(), String> {
    match *op {
        Op::Write { register, .. } if register as usize >= chip.registers() => {
            Err("Invalid Native OPL register".into())
        }
        Op::KeyOff { channel } | Op::AuditionKeyOn { channel } if !chip.valid_voice(channel) => {
            Err("Invalid Native OPL voice".into())
        }
        Op::Clock { hz } if !(100_000..=100_000_000).contains(&hz) => {
            Err("Native OPL clock must be 100000–100000000 Hz".into())
        }
        _ => Ok(()),
    }
}
impl Message {
    pub fn prepare(&self) -> Result<Arc<Prepared>, String> {
        if !valid_id(self.id) {
            return Err("Invalid Native OPL command ID".into());
        }
        match &self.method {
            Method::Write { register, value } => validate_op(
                &Op::Write {
                    register: *register,
                    value: *value,
                },
                self.chip,
            )?,
            Method::KeyOff { channel } | Method::AuditionKeyOn { channel } => {
                validate_op(&Op::KeyOff { channel: *channel }, self.chip)?
            }
            Method::Clock { hz } => validate_op(&Op::Clock { hz: *hz }, self.chip)?,
            Method::Mix { volume, pan }
                if !volume.is_finite()
                    || !(0.0..=4.).contains(volume)
                    || !pan.is_finite()
                    || !(-1.0..=1.).contains(pan) =>
            {
                return Err("Native OPL mixer volume must be 0–4, pan −1–1".into())
            }
            Method::SourceMute { mask } if *mask >= (1 << self.chip.channels()) => {
                return Err("Invalid Native OPL mute mask".into())
            }
            Method::Batch {
                commands,
                channel,
                voice,
                action,
            } => {
                if !self.chip.valid_voice(*channel)
                    || commands.is_empty()
                    || commands.len() > 512
                    || !valid_id(*voice)
                    || self.audition && *action != LeaseAction::Edit && voice.is_none()
                {
                    return Err("Invalid Native OPL batch or voice lease".into());
                }
                for op in commands {
                    validate_op(op, self.chip)?;
                    if declared_channel(op).is_some_and(|ch| {
                        let a = static_physical(ch);
                        let b = static_physical(*channel);
                        if self.chip == Chip::Ymf262 {
                            !paired(a, b)
                        } else {
                            a != b
                        }
                    }) {
                        return Err(
                            "Native OPL batch must target its declared physical voice".into()
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
            clock_rejected: AtomicBool::new(false),
            outcome_observed: AtomicBool::new(false),
        }))
    }
}
impl Prepared {
    pub fn starts(&self) -> bool {
        match &self.method {
            Method::KeyOff { .. }
            | Method::Clock { .. }
            | Method::Mix { .. }
            | Method::SourceMute { .. }
            | Method::Reset
            | Method::StopVoices => false,
            Method::Batch { action, .. } => *action != LeaseAction::Off,
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
    // Failed barriers survive unrelated status reads/control reclamation.
    // Capacity is shared with prepared commands and admission remains bounded.
    rejected_clocks: HashSet<u64>,
}
pub struct Shared {
    chip: Chip,
    revision: AtomicU64,
    registers: [AtomicU8; 512],
    clock: AtomicU32,
    mute: AtomicU32,
    status: AtomicU8,
    irq: AtomicU8,
    ports: [AtomicU8; 4],
    held: AtomicU32,
    rate: AtomicU32,
    control: Mutex<Control>,
    retired: ArrayQueue<Arc<Prepared>>,
    highest_id: AtomicU64,
    cancel_before: AtomicU64,
    owner_cancel_before: [AtomicU64; OWNERS],
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChipStatus {
    pub chip: &'static str,
    pub clock: u32,
    pub registers: Vec<u8>,
    pub channels: u8,
    pub status: u8,
    pub irq: bool,
    pub ports: [u8; 4],
    pub source_mute_mask: u32,
    pub held: Vec<u8>,
    pub pending_commands: Vec<u64>,
    pub sample_rate: u32,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ym3526: Option<ChipStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ym3812: Option<ChipStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ymf262: Option<ChipStatus>,
    pub frame: u64,
    pub sample_rate: u32,
    pub revision: u64,
}
impl Shared {
    pub fn new(chip: Chip) -> Self {
        Self {
            chip,
            revision: AtomicU64::new(0),
            registers: std::array::from_fn(|_| AtomicU8::new(0)),
            clock: AtomicU32::new(chip.clock()),
            mute: AtomicU32::new(0),
            status: AtomicU8::new(0),
            irq: AtomicU8::new(0),
            ports: std::array::from_fn(|_| AtomicU8::new(0)),
            held: AtomicU32::new(0),
            rate: AtomicU32::new(0),
            control: Mutex::new(Control::default()),
            retired: ArrayQueue::new(CAPACITY),
            highest_id: AtomicU64::new(0),
            cancel_before: AtomicU64::new(0),
            owner_cancel_before: std::array::from_fn(|_| AtomicU64::new(0)),
        }
    }
    pub fn mode(&self) -> &'static str {
        self.chip.name()
    }
    pub fn accepts(&self, c: &Prepared) -> bool {
        c.chip == self.chip
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
            if cmd.clock_rejected.load(Ordering::Acquire)
                && !cmd.outcome_observed.load(Ordering::Acquire)
            {
                if let Some(id) = cmd.id {
                    c.rejected_clocks.insert(id);
                }
            }
            retired.insert(Arc::as_ptr(&cmd) as usize);
        }
        c.prepared
            .retain(|cmd| !retired.contains(&(Arc::as_ptr(cmd) as usize)));
    }
    pub fn retain(&self, commands: &[(Arc<Prepared>, Option<Owner>)]) -> Result<(), String> {
        let mut c = self
            .control
            .lock()
            .map_err(|_| "Native OPL control state unavailable")?;
        self.collect(&mut c);
        if commands.iter().any(|(v, _)| !self.accepts(v)) {
            return Err("Native OPL command targets a different selected chip".into());
        }
        if self.held.load(Ordering::Acquire) != 0
            && commands.iter().any(|(v, _)| {
                !v.audition
                    && match &v.method {
                        Method::Clock { .. } => true,
                        Method::Batch { commands, .. } => {
                            commands.iter().any(|op| matches!(op, Op::Clock { .. }))
                        }
                        _ => false,
                    }
            })
        {
            return Err("Release the manually held OPL voices before changing the clock".into());
        }

        if c.prepared.len() + c.rejected_clocks.len() + commands.len() > CAPACITY
            || c.prepared.iter().map(|v| v.bytes()).sum::<usize>()
                + commands.iter().map(|(v, _)| v.bytes()).sum::<usize>()
                > PENDING_BYTES
        {
            return Err("Native OPL pending command limit reached (4 MiB)".into());
        }
        let mut ids = HashSet::new();
        for id in c
            .prepared
            .iter()
            .filter_map(|v| v.id)
            .chain(c.rejected_clocks.iter().copied())
            .chain(commands.iter().filter_map(|(v, _)| v.id))
        {
            if !ids.insert(id) {
                return Err("Duplicate pending Native OPL command ID".into());
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
            return Err("Unknown Native OPL command ID".into());
        }
        let cmd = {
            let mut control = self
                .control
                .lock()
                .map_err(|_| "Native OPL control state unavailable")?;
            if control.rejected_clocks.remove(&id) {
                return Err(
                    "Release the manually held OPL voices before changing the clock".into(),
                );
            }
            control.prepared.iter().find(|v| v.id == Some(id)).cloned()
        };
        if let Some(cmd) = cmd {
            let start = std::time::Instant::now();
            while !cmd.complete.load(Ordering::Acquire) {
                // A scheduled Worker clock change awaits the same applied
                // command barrier as the authoring client, not an early ACK.
                let deadline = if matches!(cmd.method, Method::Clock { .. }) {
                    5000
                } else {
                    100
                };
                if start.elapsed() > std::time::Duration::from_millis(deadline) {
                    return Err(format!(
                        "Native OPL command was not applied within {deadline} ms"
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            cmd.outcome_observed.store(true, Ordering::Release);
            if cmd.clock_rejected.load(Ordering::Acquire) {
                self.control
                    .lock()
                    .map_err(|_| "Native OPL control state unavailable")?
                    .rejected_clocks
                    .remove(&id);
                return Err(
                    "Release the manually held OPL voices before changing the clock".into(),
                );
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
            .unwrap_or_else(|_| unreachable!("Bounded OPL command retirement"));
    }

    fn publish(&self, r: &Renderer, command: Option<&Prepared>) {
        let s = r.state();
        self.revision.fetch_add(1, Ordering::AcqRel);
        for (o, v) in self.registers.iter().zip(s.registers) {
            o.store(v, Ordering::Relaxed)
        }
        self.clock.store(s.clock, Ordering::Relaxed);
        self.mute.store(s.mute_mask, Ordering::Relaxed);
        self.status.store(s.status, Ordering::Relaxed);
        self.irq.store(s.irq, Ordering::Relaxed);
        for (i, o) in self.ports.iter().enumerate() {
            o.store(
                unsafe { native_opl_read(r.ptr, i as u8) },
                Ordering::Relaxed,
            )
        }
        self.held.store(
            r.owners
                .iter()
                .flatten()
                .filter(|l| l.audition)
                .fold(0, |bits, l| bits | (1 << l.channel)),
            Ordering::Relaxed,
        );
        self.rate.store(r.rate, Ordering::Relaxed);
        if let Some(c) = command {
            c.complete.store(true, Ordering::Relaxed)
        }
        self.revision.fetch_add(1, Ordering::Release);
    }
    pub fn status(&self, frame: u64) -> Result<Status, String> {
        let mut control = self
            .control
            .lock()
            .map_err(|_| "Native OPL control unavailable")?;
        self.collect(&mut control);
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() > std::time::Duration::from_millis(50) {
                return Err("Native OPL state changed while reading; retry".into());
            }
            let before = self.revision.load(Ordering::Acquire);
            if before & 1 != 0 {
                std::thread::yield_now();
                continue;
            }
            let held = self.held.load(Ordering::Relaxed);
            let rate = self.rate.load(Ordering::Relaxed);
            let chip = ChipStatus {
                chip: self.mode(),
                clock: self.clock(),
                registers: self.registers[..self.chip.registers()]
                    .iter()
                    .map(|v| v.load(Ordering::Relaxed))
                    .collect(),
                channels: self.chip.channels(),
                status: self.status.load(Ordering::Relaxed),
                irq: self.irq.load(Ordering::Relaxed) != 0,
                ports: std::array::from_fn(|i| self.ports[i].load(Ordering::Relaxed)),
                source_mute_mask: self.mute.load(Ordering::Relaxed),
                held: (0..23).filter(|ch| held & (1 << ch) != 0).collect(),
                pending_commands: control
                    .prepared
                    .iter()
                    .filter(|v| !v.complete.load(Ordering::Relaxed))
                    .filter_map(|v| v.id)
                    .collect(),
                sample_rate: rate,
            };
            let (mut a, mut b, mut c) = (None, None, None);
            match self.chip {
                Chip::Ym3526 => a = Some(chip),
                Chip::Ym3812 => b = Some(chip),
                Chip::Ymf262 => c = Some(chip),
            }
            let out = Status {
                ym3526: a,
                ym3812: b,
                ymf262: c,
                frame,
                sample_rate: rate,
                revision: before / 2,
            };
            std::sync::atomic::fence(Ordering::Acquire);
            if self.revision.load(Ordering::Relaxed) == before {
                return Ok(out);
            }
            std::thread::yield_now()
        }
    }
}
#[derive(Clone, Copy)]
struct Lease {
    owner: Option<Owner>,
    voice: Option<u64>,
    audition: bool,
    channel: u8,
}
pub struct Renderer {
    ptr: *mut c_void,
    chip: Chip,
    rate: u32,
    owners: [Option<Lease>; VOICES],
    gains: [f32; 2],
    targets: [f32; 2],
    ramp: u32,
}
unsafe impl Send for Renderer {}
impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe { native_opl_destroy(self.ptr) }
    }
}
impl Renderer {
    pub fn new(rate: u32, chip: Chip) -> Result<Self, String> {
        let ptr = unsafe { native_opl_create(rate, chip.code()) };
        if ptr.is_null() {
            return Err("Could not allocate Native OPL core".into());
        }
        Ok(Self {
            ptr,
            chip,
            rate,
            owners: [None; VOICES],
            gains: [1.; 2],
            targets: [1.; 2],
            ramp: 0,
        })
    }
    fn state(&self) -> CoreState {
        let mut s = CoreState::default();
        unsafe { native_opl_get_state(self.ptr, &mut s) }
        s
    }
    fn physical_for(&self, ch: u8, s: &CoreState) -> usize {
        let ch = static_physical(ch);
        // Authoring APIs and the Web inspector keep pair ownership from
        // register 0x104, including while OPL3 output mode is disabled.
        if self.chip == Chip::Ymf262 {
            for (i, p) in [0, 1, 2, 9, 10, 11].into_iter().enumerate() {
                if s.registers[0x104] & (1 << i) != 0 && (ch == p || ch == p + 3) {
                    return p;
                }
            }
        }
        ch
    }
    fn protected(&self, p: usize, voice: Option<u64>, audition: bool) -> bool {
        self.owners[p]
            .is_some_and(|l| l.audition && (!audition || voice.is_some() && l.voice != voice))
    }
    fn release(&mut self, p: usize) {
        if let Some(l) = self.owners[p] {
            unsafe {
                native_opl_key_off(self.ptr, l.channel);
                if p >= 6 && p <= 8 {
                    native_opl_key_off(self.ptr, p as u8);
                    for drum in 18..23 {
                        if static_physical(drum) == p {
                            native_opl_key_off(self.ptr, drum)
                        }
                    }
                }
            }
        }
        self.owners[p] = None;
    }
    fn op(
        &mut self,
        op: &Op,
        owner: Option<Owner>,
        voice: Option<u64>,
        audition: bool,
        preserve: bool,
    ) {
        let s = self.state();
        let ch = declared_channel(op);
        let target = ch.map(|ch| self.physical_for(ch, &s));
        if target.is_some_and(|p| self.protected(p, voice, audition)) {
            return;
        }
        let held = self.owners.iter().flatten().any(|l| l.audition);
        if !audition
            && held
            && match *op {
                Op::Clock { .. } => true,
                Op::Write { register, .. } => register < 0x20 || (0x100..0x120).contains(&register),
                _ => false,
            }
        {
            return;
        }
        let mut off = matches!(op, Op::KeyOff { .. });
        let mut on = matches!(op, Op::AuditionKeyOn { .. });
        if let Op::Write { register, value } = *op {
            if matches!(register & 255, 0xb0..=0xb8) {
                off = value & 32 == 0;
                on = !off
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
                    if register == 0xbd {
                        if !audition && held && (value ^ s.registers[0xbd]) & 0xe0 != 0 {
                            return;
                        }
                        for drum in 18..23 {
                            let p = self.physical_for(drum, &s);
                            let bit = [16, 8, 4, 2, 1][(drum - 18) as usize];
                            let stale = self.owners[p].is_some_and(|l| {
                                l.channel == drum && owner.is_some() && l.owner != owner
                            });
                            if self.protected(p, voice, audition) || value & bit == 0 && stale {
                                value = (value & !bit) | (s.registers[0xbd] & bit)
                            }
                        }
                        if audition && (value ^ s.registers[0xbd]) & 32 != 0 {
                            for p in 6..9 {
                                self.release(p)
                            }
                        }
                        native_opl_write(self.ptr, register, value);
                        if !preserve {
                            for drum in 18..23 {
                                let p = self.physical_for(drum, &s);
                                let bit = [16, 8, 4, 2, 1][(drum - 18) as usize];
                                if value & bit != 0 && s.registers[0xbd] & bit == 0 {
                                    self.owners[p] = Some(Lease {
                                        owner,
                                        voice,
                                        audition,
                                        channel: drum,
                                    })
                                } else if value & bit == 0
                                    && s.registers[0xbd] & bit != 0
                                    && self.owners[p].is_some_and(|l| l.channel == drum)
                                {
                                    self.owners[p] = None
                                }
                            }
                        }
                    } else {
                        if register == 0x104 {
                            let changed = (value ^ s.registers[0x104]) & 63;
                            for (bit, primary) in [0, 1, 2, 9, 10, 11].into_iter().enumerate() {
                                if changed & (1 << bit) != 0 {
                                    self.release(primary);
                                    self.release(primary + 3);
                                }
                            }
                        }
                        // Register 0x105 changes the chip's emulation mode;
                        // it does not cancel otherwise unchanged voice leases.
                        native_opl_write(self.ptr, register, value)
                    }
                }
                Op::KeyOff { channel } => native_opl_key_off(self.ptr, channel),
                Op::AuditionKeyOn { channel } => native_opl_audition_key_on(self.ptr, channel),
                Op::Clock { hz } => {
                    native_opl_set_clock(self.ptr, hz);
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
                        channel: ch,
                    })
                }
            }
        }
    }
    pub fn apply(&mut self, c: &Prepared, owner: Option<Owner>, shared: &Shared) {
        if c.chip != self.chip {
            return;
        }
        if !c.audition
            && self.owners.iter().flatten().any(|l| l.audition)
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
        match &c.method {
            Method::Write { register, value } => self.op(
                &Op::Write {
                    register: *register,
                    value: *value,
                },
                owner,
                None,
                c.audition,
                false,
            ),
            Method::KeyOff { channel } => self.op(
                &Op::KeyOff { channel: *channel },
                owner,
                None,
                c.audition,
                false,
            ),
            Method::AuditionKeyOn { channel } => self.op(
                &Op::AuditionKeyOn { channel: *channel },
                owner,
                None,
                c.audition,
                false,
            ),
            Method::Clock { hz } => self.op(&Op::Clock { hz: *hz }, owner, None, c.audition, false),
            Method::Mix { volume, pan } => {
                self.targets = [
                    *volume * if *pan > 0. { 1. - *pan } else { 1. },
                    *volume * if *pan < 0. { 1. + *pan } else { 1. },
                ];
                self.ramp = (self.rate / 200).max(1)
            }
            Method::SourceMute { mask } => unsafe { native_opl_set_mute_mask(self.ptr, *mask) },
            Method::Reset => {
                if c.audition || !self.owners.iter().flatten().any(|l| l.audition) {
                    unsafe { native_opl_reset(self.ptr) }
                    self.owners.fill(None)
                }
            }
            Method::StopVoices => {
                if owner.is_none() {
                    unsafe { native_opl_stop(self.ptr) }
                    self.owners.fill(None)
                } else {
                    for p in 0..VOICES {
                        if self.owners[p]
                            .is_some_and(|l| l.owner == owner && (!l.audition || c.audition))
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
                let s = self.state();
                let p = self.physical_for(*channel, &s);
                // A secondary channel only belongs to this lease while its
                // four-operator pair is actually enabled. Reject the entire
                // batch rather than partially changing a different 2-op voice.
                if commands
                    .iter()
                    .filter_map(declared_channel)
                    .any(|ch| self.physical_for(ch, &s) != p)
                {
                    shared.publish(self, Some(c));
                    return;
                }
                let current = self.owners[p];
                let allowed = match action {
                    LeaseAction::On => c.audition || current.is_none_or(|l| !l.audition),
                    LeaseAction::Off => current.is_some_and(|l| {
                        l.channel == *channel
                            && l.owner == owner
                            && (voice.is_none() || l.voice == *voice)
                            && (!l.audition || c.audition)
                    }),
                    LeaseAction::Edit => current.is_none_or(|l| {
                        !l.audition || c.audition && (voice.is_none() || l.voice == *voice)
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
                            audition: c.audition,
                            channel: *channel,
                        })
                    }
                    for op in commands {
                        self.op(op, owner, *voice, c.audition, *action == LeaseAction::Edit)
                    }
                    if *action == LeaseAction::On {
                        self.owners[p] = Some(Lease {
                            owner,
                            voice: *voice,
                            audition: c.audition,
                            channel: *channel,
                        })
                    } else if *action == LeaseAction::Off {
                        self.owners[p] = None
                    }
                }
            }
        }
        shared.publish(self, Some(c));
    }
    pub fn publish(&self, s: &Shared) {
        s.publish(self, None)
    }
    pub fn cancel_invalid(&mut self, valid: impl Fn(Option<Owner>) -> bool, s: &Shared) {
        let mut changed = false;
        for p in 0..VOICES {
            if self.owners[p].is_some_and(|l| !valid(l.owner)) {
                self.release(p);
                changed = true
            }
        }
        if changed {
            s.publish(self, None)
        }
    }
    pub fn stop(&mut self, s: &Shared) {
        unsafe { native_opl_stop(self.ptr) }
        self.owners.fill(None);
        s.publish(self, None)
    }
    pub fn mix(&mut self, pcm: &mut [f32; 2]) {
        let (mut l, mut r) = (0., 0.);
        unsafe { native_opl_sample(self.ptr, &mut l, &mut r) }
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
    fn setup(chip: Chip) -> (Shared, Renderer) {
        let s = Shared::new(chip);
        let r = Renderer::new(48000, chip).unwrap();
        r.publish(&s);
        (s, r)
    }
    fn apply(r: &mut Renderer, s: &Shared, v: Value, owner: Option<Owner>) {
        let c = prep(v);
        s.retain(&[(c.clone(), owner)]).unwrap();
        r.apply(&c, owner, s);
        s.retire(c)
    }
    fn write(chip: Chip, register: u16, value: u8) -> Value {
        json!({"chip":chip.name(),"method":"write","register":register,"value":value})
    }
    fn lease(chip: Chip, channel: u8, voice: u64, action: &str, commands: Vec<Value>) -> Value {
        json!({"chip":chip.name(),"method":"batch","channel":channel,"voice":voice,"action":action,"audition":true,"commands":commands})
    }
    fn snapshot(s: &Shared) -> ChipStatus {
        let a = s.status(0).unwrap();
        a.ym3526.or(a.ym3812).or(a.ymf262).unwrap()
    }
    fn owner(n: u32) -> Option<Owner> {
        Some(Owner {
            performance: 1,
            owner: n,
        })
    }
    fn note(ch: u8) -> Vec<Value> {
        let bank = if ch >= 9 { 256 } else { 0 };
        let c = ch % 9;
        let slot = [0u16, 1, 2, 8, 9, 10, 16, 17, 18][c as usize];
        let mut out = Vec::new();
        for (op, tl) in [(slot, 63), (slot + 3, 0)] {
            for (reg, value) in [
                (0x20 + op, 0x21),
                (0x40 + op, tl),
                (0x60 + op, 0xf0),
                (0x80 + op, 0x0f),
            ] {
                out.push(json!({"method":"write","register":bank+reg,"value":value}))
            }
        }
        for (reg, value) in [
            (0xa0 + c as u16, 0x98),
            (0xc0 + c as u16, 0x31),
            (0xb0 + c as u16, 0x31),
        ] {
            out.push(json!({"method":"write","register":bank+reg,"value":value}))
        }
        out
    }
    fn start_note(r: &mut Renderer, s: &Shared, ch: u8) {
        for op in note(ch) {
            let mut m = op;
            m["chip"] = json!(s.mode());
            apply(r, s, m, None)
        }
    }
    fn render(r: &mut Renderer, n: usize) -> Vec<[f32; 2]> {
        (0..n)
            .map(|_| {
                let mut p = [0.; 2];
                r.mix(&mut p);
                p
            })
            .collect()
    }
    #[test]
    fn opl_native_validates_profiles_registers_voices_clocks_and_atomic_batch() {
        for chip in [Chip::Ym3526, Chip::Ym3812, Chip::Ymf262] {
            for v in [
                write(chip, chip.registers() as u16, 1),
                json!({"chip":chip.name(),"method":"key-off","channel":23}),
                json!({"chip":chip.name(),"method":"clock","hz":99999}),
                json!({"chip":chip.name(),"method":"source-mute","mask":1u32<<chip.channels()}),
                lease(chip, 0, 1, "on", note(2)),
            ] {
                assert!(serde_json::from_value::<Message>(v)
                    .unwrap()
                    .prepare()
                    .is_err())
            }
            if chip != Chip::Ymf262 {
                assert!(serde_json::from_value::<Message>(
                    json!({"chip":chip.name(),"method":"key-off","channel":9})
                )
                .unwrap()
                .prepare()
                .is_err())
            }
        }
        let s = Shared::new(Chip::Ym3812);
        let good = prep(write(Chip::Ym3812, 0x20, 1));
        let wrong = prep(write(Chip::Ym3526, 0x20, 1));
        assert!(s.retain(&[(good, None), (wrong, None)]).is_err());
        assert!(s.control.lock().unwrap().prepared.is_empty());
    }
    #[test]
    fn opl_native_selected_profiles_clock_snapshot_and_pcm_are_observational() {
        for chip in [Chip::Ym3526, Chip::Ym3812, Chip::Ymf262] {
            let (s, mut r) = setup(chip);
            let s2 = Shared::new(chip);
            let mut reference = Renderer::new(48000, chip).unwrap();
            if chip == Chip::Ymf262 {
                apply(&mut r, &s, write(chip, 0x105, 1), None);
                apply(&mut reference, &s2, write(chip, 0x105, 1), None)
            }
            start_note(&mut r, &s, 0);
            start_note(&mut reference, &s2, 0);
            let before = snapshot(&s);
            assert_eq!(before.clock, chip.clock());
            assert_eq!(before.registers.len(), chip.registers());
            for _ in 0..50 {
                assert_eq!(snapshot(&s).registers, before.registers)
            }
            let pcm = render(&mut r, 4096);
            assert_eq!(pcm, render(&mut reference, 4096));
            assert!(pcm.iter().any(|p| p[0].abs() > 0.001));
            assert_eq!(snapshot(&s).registers, before.registers)
        }
    }
    #[test]
    fn opl_native_manual_voice_blocks_script_operator_pitch_timer_and_clock_but_allows_ui_edit() {
        let chip = Chip::Ym3812;
        let (s, mut r) = setup(chip);
        apply(&mut r, &s, lease(chip, 0, 1, "on", note(0)), None);
        for (reg, val) in [
            (0x20, 15),
            (0x43, 63),
            (0xa0, 1),
            (0xb0, 0),
            (0xc0, 0),
            (2, 255),
            (4, 1),
            (8, 128),
        ] {
            apply(&mut r, &s, write(chip, reg, val), owner(1))
        }
        let clock = prep(json!({"chip":chip.name(),"method":"clock","hz":4_000_000}));
        assert!(s.retain(&[(clock, owner(1))]).unwrap_err().contains("held"));
        let a = snapshot(&s);
        assert_eq!(a.registers[0x20], 0x21);
        assert_eq!(a.registers[0x43], 0);
        assert_eq!(a.registers[0xa0], 0x98);
        assert_eq!(a.registers[0xb0], 0x31);
        assert_eq!(a.registers[2], 0);
        assert_eq!(a.registers[4], 0);
        assert_eq!(a.clock, chip.clock());
        apply(
            &mut r,
            &s,
            lease(
                chip,
                0,
                1,
                "edit",
                vec![json!({"method":"write","register":0x43,"value":12})],
            ),
            None,
        );
        assert_eq!(snapshot(&s).registers[0x43], 12);
        assert_eq!(snapshot(&s).held, [0]);
    }
    #[test]
    fn opl_native_four_operator_pair_secondary_registers_and_stale_release_share_physical_lease() {
        let chip = Chip::Ymf262;
        let (s, mut r) = setup(chip);
        apply(&mut r, &s, write(chip, 0x105, 1), None);
        apply(&mut r, &s, write(chip, 0x104, 1), None);
        let mut ops = note(0);
        ops.extend(note(3));
        apply(&mut r, &s, lease(chip, 0, 1, "on", ops), None);
        for (reg, val) in [
            (0x28, 5),
            (0x4b, 63),
            (0xa3, 4),
            (0xb3, 0),
            (0x104, 0),
            (0x105, 0),
        ] {
            apply(&mut r, &s, write(chip, reg, val), owner(1))
        }
        let a = snapshot(&s);
        assert_eq!(a.registers[0x28], 0x21);
        assert_eq!(a.registers[0x4b], 0);
        assert_eq!(a.registers[0xa3], 0x98);
        assert_eq!(a.registers[0xb3], 0x31);
        assert_eq!(a.registers[0x104], 1);
        assert_eq!(a.registers[0x105], 1);
        apply(&mut r, &s, lease(chip, 3, 2, "on", note(3)), None);
        apply(
            &mut r,
            &s,
            lease(
                chip,
                0,
                1,
                "off",
                vec![json!({"method":"key-off","channel":0})],
            ),
            None,
        );
        assert_eq!(snapshot(&s).held, [3]);
        apply(
            &mut r,
            &s,
            lease(
                chip,
                3,
                2,
                "off",
                vec![json!({"method":"key-off","channel":3})],
            ),
            None,
        );
        assert!(snapshot(&s).held.is_empty());
        assert_eq!(snapshot(&s).registers[0xb0] & 32, 0);
        assert_eq!(snapshot(&s).registers[0xb3] & 32, 0);
    }
    #[test]
    fn opl_native_rhythm_siblings_owner_cancel_and_script_mode_changes_preserve_manual_voice() {
        let chip = Chip::Ym3812;
        let (s, mut r) = setup(chip);
        apply(&mut r, &s, write(chip, 0xbd, 32), None);
        for (ch, voice) in [(19, 1), (22, 2)] {
            apply(
                &mut r,
                &s,
                lease(
                    chip,
                    ch,
                    voice,
                    "on",
                    vec![json!({"method":"audition-key-on","channel":ch})],
                ),
                None,
            );
        }
        render(&mut r, 32);
        apply(
            &mut r,
            &s,
            lease(
                chip,
                19,
                1,
                "off",
                vec![json!({"method":"key-off","channel":19})],
            ),
            None,
        );
        apply(&mut r, &s, write(chip, 0xbd, 0), owner(1));
        assert_eq!(snapshot(&s).held, [22]);
        assert_eq!(snapshot(&s).registers[0xbd] & 33, 33);
        apply(
            &mut r,
            &s,
            lease(
                chip,
                22,
                2,
                "off",
                vec![json!({"method":"key-off","channel":22})],
            ),
            None,
        );
        assert_eq!(snapshot(&s).registers[0xbd] & 9, 0);
        for (ch, n) in [(0, 1), (1, 2)] {
            let mut v = lease(chip, ch, n as u64, "on", note(ch));
            v["audition"] = json!(false);
            apply(&mut r, &s, v, owner(n))
        }
        r.cancel_invalid(|o| o != owner(1), &s);
        assert_eq!(snapshot(&s).registers[0xb0] & 32, 0);
        assert_ne!(snapshot(&s).registers[0xb1] & 32, 0);
    }
    #[test]
    fn opl_native_stop_retains_authored_operators_clock_layout_and_cancels_deferred_on() {
        let chip = Chip::Ymf262;
        let (s, mut r) = setup(chip);
        apply(&mut r, &s, write(chip, 0x105, 1), None);
        apply(&mut r, &s, write(chip, 0x104, 1), None);
        apply(
            &mut r,
            &s,
            json!({"chip":chip.name(),"method":"clock","hz":16_000_000}),
            None,
        );
        start_note(&mut r, &s, 0);
        apply(&mut r, &s, write(chip, 0xbd, 0xe0), None);
        apply(
            &mut r,
            &s,
            lease(
                chip,
                1,
                1,
                "on",
                vec![json!({"method":"audition-key-on","channel":1})],
            ),
            None,
        );
        let old = snapshot(&s);
        r.stop(&s);
        let pcm = render(&mut r, 2048);
        assert!(pcm.iter().all(|v| *v == [0.; 2]));
        let a = snapshot(&s);
        assert_eq!(a.clock, 16_000_000);
        assert_eq!(a.registers[0x104], 1);
        assert_eq!(a.registers[0x105], 1);
        assert_eq!(a.registers[0xbd], 0xe0);
        for (reg, old) in old.registers.iter().enumerate() {
            if !matches!(reg & 255, 0xb0..=0xb8) && reg != 0xbd {
                assert_eq!(a.registers[reg], *old)
            }
        }
        assert!(a.held.is_empty());
        start_note(&mut r, &s, 0);
        assert!(render(&mut r, 2048).iter().any(|p| p[0] != 0.));
    }
    #[test]
    fn opl_native_mix_mute_and_port_status_do_not_advance_other_core() {
        let chip = Chip::Ym3812;
        let (s, mut r) = setup(chip);
        start_note(&mut r, &s, 0);
        apply(
            &mut r,
            &s,
            json!({"chip":chip.name(),"method":"mix","volume":0.5,"pan":1.0}),
            None,
        );
        let pcm = render(&mut r, 2048);
        assert!(pcm[512..].iter().all(|p| p[0].abs() < 1e-20));
        assert!(pcm[512..].iter().any(|p| p[1] != 0.));
        apply(
            &mut r,
            &s,
            json!({"chip":chip.name(),"method":"source-mute","mask":1}),
            None,
        );
        assert!(render(&mut r, 512).iter().all(|p| *p == [0.; 2]));
        apply(&mut r, &s, write(chip, 2, 255), None);
        apply(&mut r, &s, write(chip, 4, 1), None);
        render(&mut r, 4096);
        r.publish(&s);
        let a = snapshot(&s);
        assert_ne!(a.status & 0xc0, 0);
        assert!(a.irq);
        assert_eq!(a.ports[0], a.status);
        assert_eq!(snapshot(&s).ports, a.ports);
    }
    #[test]
    fn opl_native_pending_command_ids_are_unique_and_retired_on_control_thread() {
        let (s, mut r) = setup(Chip::Ym3812);
        let a = prep(json!({"chip":"ym3812","id":1,"method":"write","register":0x20,"value":1}));
        s.retain(&[(a.clone(), None)]).unwrap();
        assert_eq!(snapshot(&s).pending_commands, [1]);
        let duplicate =
            prep(json!({"chip":"ym3812","id":1,"method":"write","register":0x21,"value":2}));
        assert!(s.retain(&[(duplicate, None)]).is_err());
        r.apply(&a, None, &s);
        s.retire(a);
        assert!(snapshot(&s).pending_commands.is_empty());
        s.wait_applied(1).unwrap();
        assert!(s.wait_applied(2).is_err());
        assert!(s.control.lock().unwrap().prepared.is_empty());
    }
    fn queued(v: Value, frame: u64) -> super::super::Command {
        let mut v = json!({"type":"opl","message":v,"frame":frame});
        v["frame"] = json!(frame);
        serde_json::from_value(v).unwrap()
    }
    fn queued_note(chip: Chip, frame: u64) -> Vec<super::super::Command> {
        note(0)
            .into_iter()
            .map(|mut v| {
                v["chip"] = json!(chip.name());
                queued(v, frame)
            })
            .collect()
    }
    fn engine(chip: Chip) -> (super::super::Shared, super::super::Renderer) {
        let s = super::super::Shared::new_modes(48000, 1, false, false, false, Some(chip.name()));
        let r = super::super::Renderer::new_modes(48000, false, false, false, Some(chip.name()))
            .unwrap();
        (s, r)
    }
    fn engine_render(
        r: &mut super::super::Renderer,
        s: &super::super::Shared,
        n: usize,
    ) -> Vec<[f32; 2]> {
        r.prepare(s);
        (0..n).map(|_| r.sample(s)).collect()
    }
    #[test]
    fn opl_native_queue_profile_isolation_timestamp_and_segmentation_are_exact() {
        for chip in [Chip::Ym3526, Chip::Ym3812, Chip::Ymf262] {
            let (s, mut r) = engine(chip);
            let (t, mut q) = engine(chip);
            assert!(r.core.0.is_null());
            assert!(s.msx.is_none() && s.master_system.is_none() && s.ym2608.is_none());
            let mut events = queued_note(chip, 500);
            events.push(queued(write(chip, 0xb0, 0), 2500));
            s.enqueue(1, &events).unwrap();
            t.enqueue(1, &events).unwrap();
            let full = engine_render(&mut r, &s, 4096);
            let mut segmented = Vec::new();
            for n in [37, 463, 777, 17, 802, 2000] {
                segmented.extend(engine_render(&mut q, &t, n));
            }
            assert_eq!(full, segmented);
            assert!(full[..500].iter().all(|p| *p == [0.; 2]));
            assert!(full[600..2500].iter().any(|p| p[0] != 0.));
            assert_eq!(s.status().mode, chip.name());
            assert_eq!(s.status().psg_clock, 0);
            let wrong = if chip == Chip::Ym3812 {
                Chip::Ym3526
            } else {
                Chip::Ym3812
            };
            assert!(s.enqueue(1, &[queued(write(wrong, 0x20, 1), 0)]).is_err());
            assert!(s
                .enqueue(
                    1,
                    &[
                        serde_json::from_value(json!({"type":"psg-write","value":0x90,"frame":0}))
                            .unwrap()
                    ]
                )
                .is_err());
        }
    }
    #[test]
    fn opl_native_queue_stop_fences_future_commands_and_new_performance_can_resume() {
        let chip = Chip::Ym3812;
        let (s, mut r) = engine(chip);
        s.enqueue(1, &queued_note(chip, 0)).unwrap();
        s.enqueue(1, &queued_note(chip, 4000)).unwrap();
        s.enqueue(
            1,
            &[queued(
                json!({"chip":chip.name(),"method":"stop-voices"}),
                500,
            )],
        )
        .unwrap();
        let out = engine_render(&mut r, &s, 5000);
        assert!(out[..500].iter().any(|p| p[0] != 0.));
        assert!(out[600..].iter().all(|p| *p == [0.; 2]));
        assert_eq!(s.pending.load(Ordering::Acquire), 0);
        s.enqueue(1, &queued_note(chip, 0)).unwrap();
        assert!(engine_render(&mut r, &s, 2048).iter().any(|p| p[0] != 0.));
        s.epoch.fetch_add(1, Ordering::Release);
        engine_render(&mut r, &s, 1000);
        assert_eq!(s.stopped_epoch.load(Ordering::Acquire), 1);
        assert!(snapshot(s.opl.as_ref().unwrap()).held.is_empty());
    }
    #[test]
    fn opl_native_all_six_four_operator_pairs_share_voice_lease_and_unpaired_batch_is_atomic() {
        let chip = Chip::Ymf262;
        for (pair, primary) in [0u8, 1, 2, 9, 10, 11].into_iter().enumerate() {
            let (s, mut r) = setup(chip);
            // A 2-op secondary remains independent until layout enables the pair.
            let mut invalid = note(primary);
            invalid.extend(note(primary + 3));
            apply(
                &mut r,
                &s,
                lease(chip, primary, 1, "on", invalid.clone()),
                None,
            );
            assert_eq!(
                snapshot(&s).registers
                    [(if primary >= 9 { 256 } else { 0 }) + 0xa0 + primary as usize % 9],
                0
            );
            assert!(snapshot(&s).held.is_empty());
            apply(&mut r, &s, write(chip, 0x105, 1), None);
            apply(&mut r, &s, write(chip, 0x104, 1 << pair), None);
            apply(&mut r, &s, lease(chip, primary, 2, "on", invalid), None);
            assert_eq!(snapshot(&s).held, [primary]);
            let bank: u16 = if primary >= 9 { 256 } else { 0 };
            let reg = bank + 0xa0 + ((primary + 3) % 9) as u16;
            apply(&mut r, &s, write(chip, reg as u16, 1), owner(1));
            assert_eq!(snapshot(&s).registers[reg as usize], 0x98);
            apply(
                &mut r,
                &s,
                lease(
                    chip,
                    primary,
                    2,
                    "off",
                    vec![json!({"method":"key-off","channel":primary})],
                ),
                None,
            );
            assert!(snapshot(&s).held.is_empty());
        }
    }
    #[test]
    fn opl_native_normal_key_off_releases_envelope_and_forced_off_mutes_immediately() {
        let chip = Chip::Ym3812;
        let (s, mut r) = setup(chip);
        start_note(&mut r, &s, 0);
        apply(&mut r, &s, write(chip, 0x83, 0), None);
        assert!(render(&mut r, 256).iter().any(|p| p[0] != 0.));
        apply(&mut r, &s, write(chip, 0xb0, 0x11), None);
        assert!(render(&mut r, 256).iter().any(|p| p[0] != 0.));
        apply(
            &mut r,
            &s,
            json!({"chip":chip.name(),"method":"key-off","channel":0}),
            None,
        );
        assert!(render(&mut r, 512).iter().all(|p| *p == [0.; 2]));
    }
    #[test]
    fn opl_native_clock_rejection_after_admission_survives_control_collection_and_same_hz() {
        let chip = Chip::Ym3812;
        let (s, mut r) = setup(chip);
        // Queue a clock before an ASAP manual audition wins the physical voice.
        let clock = prep(json!({"chip":chip.name(),"id":77,"method":"clock","hz":chip.clock()}));
        s.retain(&[(clock.clone(), None)]).unwrap();
        apply(&mut r, &s, lease(chip, 0, 1, "on", note(0)), None);
        r.apply(&clock, None, &s);
        s.retire(clock);
        // A separate inspector status read reclaims the completed Arc first.
        assert!(snapshot(&s).pending_commands.is_empty());
        assert!(s.wait_applied(77).unwrap_err().contains("held"));
        assert!(s.control.lock().unwrap().rejected_clocks.is_empty());
    }
    #[test]
    fn opl_native_four_operator_ownership_remains_paired_with_opl3_mode_disabled() {
        let chip = Chip::Ymf262;
        let (s, mut r) = setup(chip);
        apply(&mut r, &s, write(chip, 0x104, 1), None);
        assert_eq!(snapshot(&s).registers[0x105], 0);
        let mut ops = note(0);
        ops.extend(note(3));
        apply(&mut r, &s, lease(chip, 0, 1, "on", ops), None);
        assert_eq!(snapshot(&s).held, [0]);
        apply(&mut r, &s, write(chip, 0xa3, 1), owner(1));
        assert_eq!(snapshot(&s).registers[0xa3], 0x98);
        apply(
            &mut r,
            &s,
            lease(
                chip,
                0,
                1,
                "off",
                vec![json!({"method":"key-off","channel":0})],
            ),
            None,
        );
        assert_eq!(snapshot(&s).registers[0xb0] & 32, 0);
        assert_eq!(snapshot(&s).registers[0xb3] & 32, 0);
        assert!(snapshot(&s).held.is_empty());
    }
    #[test]
    fn opl_native_pair_and_output_mode_edits_preserve_unrelated_script_voice_pcm() {
        let chip = Chip::Ymf262;
        let (s, mut r) = setup(chip);
        let (t, mut reference) = setup(chip);
        for (renderer, shared) in [(&mut r, &s), (&mut reference, &t)] {
            apply(renderer, shared, write(chip, 0x105, 1), None);
            start_note(renderer, shared, 8);
            start_note(renderer, shared, 0);
        }
        assert_eq!(render(&mut r, 512), render(&mut reference, 512));
        // Match the selected core's hardware register edit while the Rust
        // renderer additionally manages ownership on the affected pair only.
        apply(&mut r, &s, write(chip, 0x104, 1), None);
        reference.release(0);
        reference.release(3);
        unsafe { native_opl_write(reference.ptr, 0x104, 1) }
        assert!(r.owners[8].is_some());
        assert_eq!(snapshot(&s).registers[0xb8] & 32, 32);
        let pcm = render(&mut r, 4096);
        assert_eq!(pcm, render(&mut reference, 4096));
        assert!(pcm[512..].iter().any(|p| p[0] != 0.));
        apply(&mut r, &s, write(chip, 0x105, 0), None);
        assert!(r.owners[8].is_some());
        assert_eq!(snapshot(&s).registers[0xb8] & 32, 32);
        assert!(render(&mut r, 1024).iter().any(|p| p[0] != 0.));
        apply(&mut r, &s, write(chip, 0x105, 1), None);
        assert!(r.owners[8].is_some());
    }
}
