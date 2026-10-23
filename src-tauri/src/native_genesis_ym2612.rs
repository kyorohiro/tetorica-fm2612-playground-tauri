//! Independent six-channel YM2612. The callback exclusively owns the core.
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
pub const CLOCK: u32 = 7_670_454;
const VOICES: usize = 6;
const PENDING_BYTES: usize = 4 * 1024 * 1024;
#[repr(C)]
struct CoreState {
    registers: [u8; 512],
    clock: u32,
    channel_mute_mask: u32,
    keys: [u8; 6],
    status: u8,
    irq: u8,
}
impl Default for CoreState {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}
extern "C" {
    fn native_ym2612_create(rate: u32) -> *mut c_void;
    fn native_ym2612_destroy(p: *mut c_void);
    fn native_ym2612_reset(p: *mut c_void);
    fn native_ym2612_stop(p: *mut c_void);
    fn native_ym2612_write(p: *mut c_void, port: u8, register: u8, value: u8);
    fn native_ym2612_key_off_fm(p: *mut c_void, voice: u8);
    fn native_ym2612_cancel_csm(p: *mut c_void);
    fn native_ym2612_audition_fm_key_on(p: *mut c_void, channel: u8, mask: u8);
    fn native_ym2612_sample(p: *mut c_void, l: *mut f32, r: *mut f32);
    fn native_ym2612_set_clock(p: *mut c_void, hz: u32) -> i32;
    fn native_ym2612_set_mute_mask(p: *mut c_void, mask: u32);
    fn native_ym2612_get_state(p: *mut c_void, s: *mut CoreState);
}
#[derive(Clone, Debug, Deserialize)]
pub struct Message {
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
        port: u8,
        register: u8,
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
    ChannelMute {
        mask: u32,
    },
    Reset,
    StopVoices,
    CancelCsm,
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
    audition: bool,
    method: Method,
    complete: AtomicBool,
    clock_rejected: AtomicBool,
    outcome_observed: AtomicBool,
}
fn valid_id(id: Option<u64>) -> bool {
    id.is_none_or(|v| v > 0 && v <= MAX_SAFE_INTEGER)
}
fn declared_channel(op: &Op) -> Option<usize> {
    match *op {
        Op::KeyOff { channel } | Op::AuditionKeyOn { channel } => Some(channel as usize),
        Op::Write {
            port: 0,
            register: 0x28,
            value,
        } => {
            let code = value & 7;
            (code & 3 < 3).then_some(((code & 3) + if code & 4 != 0 { 3 } else { 0 }) as usize)
        }
        Op::Write {
            port: 0,
            register: 0xa8..=0xae,
            ..
        } => {
            if let Op::Write { register, .. } = *op {
                (register & 3 < 3).then_some(2)
            } else {
                None
            }
        }
        Op::Write {
            port,
            register: 0x30..=0xb6,
            ..
        } => {
            if let Op::Write { register, .. } = *op {
                (register & 3 < 3).then_some((port * 3 + (register & 3)) as usize)
            } else {
                None
            }
        }
        Op::Write {
            port: 0,
            register: 0x2a..=0x2b,
            ..
        } => Some(5),
        // Timer A can key CH3 implicitly through CSM.
        Op::Write {
            port: 0,
            register: 0x24..=0x27,
            ..
        } => Some(2),
        _ => None,
    }
}
fn validate_op(op: &Op) -> Result<(), String> {
    match *op {
        Op::Write { port, .. } if port > 1 => Err("Native YM2612 port must be 0 or 1".into()),
        Op::KeyOff { channel } | Op::AuditionKeyOn { channel } if channel >= 6 => {
            Err("Native YM2612 voice must be 0–5".into())
        }
        Op::Clock { hz } if !(100_000..=100_000_000).contains(&hz) => {
            Err("Native YM2612 clock must be 100000–100000000 Hz".into())
        }
        _ => Ok(()),
    }
}
impl Message {
    pub fn prepare(&self) -> Result<Arc<Prepared>, String> {
        if !valid_id(self.id) {
            return Err("Invalid Native YM2612 command ID".into());
        }
        match &self.method {
            Method::Write {
                port,
                register,
                value,
            } => validate_op(&Op::Write {
                port: *port,
                register: *register,
                value: *value,
            })?,
            Method::KeyOff { channel } | Method::AuditionKeyOn { channel } => {
                validate_op(&Op::KeyOff { channel: *channel })?
            }
            Method::Clock { hz } => validate_op(&Op::Clock { hz: *hz })?,
            Method::Mix { volume, pan }
                if !volume.is_finite()
                    || !(0.0..=4.).contains(volume)
                    || !pan.is_finite()
                    || !(-1.0..=1.).contains(pan) =>
            {
                return Err("Native YM2612 mixer volume must be 0–4, pan −1–1".into())
            }
            Method::SourceMute { mask } if *mask > 1 => {
                return Err("Native YM2612 source mute mask must be 0–1".into())
            }
            Method::ChannelMute { mask } if *mask > 63 => {
                return Err("Native YM2612 FM mute mask must be 0–63".into())
            }
            Method::Batch {
                commands,
                channel,
                voice,
                action,
            } => {
                if *channel >= 6
                    || commands.is_empty()
                    || commands.len() > 512
                    || !valid_id(*voice)
                    || self.audition && *action != LeaseAction::Edit && voice.is_none()
                {
                    return Err("Invalid Native YM2612 batch or voice lease".into());
                }
                for op in commands {
                    validate_op(op)?;
                    if declared_channel(op).is_some_and(|target| target != *channel as usize) {
                        return Err(
                            "Native YM2612 batch must target its declared physical voice".into(),
                        );
                    }
                }
            }
            _ => {}
        }
        Ok(Arc::new(Prepared {
            id: self.id,
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
            | Method::ChannelMute { .. }
            | Method::Reset
            | Method::StopVoices
            | Method::CancelCsm => false,
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
    rejected_clocks: HashSet<u64>,
}
pub struct Shared {
    revision: AtomicU64,
    registers: [AtomicU8; 512],
    keys: [AtomicU8; 6],
    clock: AtomicU32,
    source_mute: AtomicU32,
    channel_mute: AtomicU32,
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
impl Default for Shared {
    fn default() -> Self {
        Self {
            revision: AtomicU64::new(0),
            registers: std::array::from_fn(|_| AtomicU8::new(0)),
            keys: std::array::from_fn(|_| AtomicU8::new(0)),
            clock: AtomicU32::new(CLOCK),
            source_mute: AtomicU32::new(0),
            channel_mute: AtomicU32::new(0),
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
    pub registers: Vec<Vec<u8>>,
    pub key_masks: Vec<u8>,
    pub source_mute_mask: u32,
    pub channel_mute_mask: u32,
    pub status: u8,
    pub irq: bool,
    pub held: Vec<u8>,
    pub pending_commands: Vec<u64>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub ym2612: ChipStatus,
    pub frame: u64,
    pub sample_rate: u32,
    pub revision: u64,
}
impl Shared {
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
            .map_err(|_| "Native YM2612 control state unavailable")?;
        self.collect(&mut c);
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
            return Err(
                "Release the manually held additional YM2612 voices before changing the clock"
                    .into(),
            );
        }

        if c.prepared.len() + c.rejected_clocks.len() + commands.len() > CAPACITY
            || c.prepared.iter().map(|v| v.bytes()).sum::<usize>()
                + commands.iter().map(|(v, _)| v.bytes()).sum::<usize>()
                > PENDING_BYTES
        {
            return Err("Native YM2612 pending command limit reached (4 MiB)".into());
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
                return Err("Duplicate pending Native YM2612 command ID".into());
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
            return Err("Unknown Native YM2612 command ID".into());
        }
        let cmd = {
            let mut control = self
                .control
                .lock()
                .map_err(|_| "Native YM2612 control state unavailable")?;
            if control.rejected_clocks.remove(&id) {
                return Err(
                    "Release the manually held additional YM2612 voices before changing the clock"
                        .into(),
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
                        "Native YM2612 command was not applied within {deadline} ms"
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            cmd.outcome_observed.store(true, Ordering::Release);
            if cmd.clock_rejected.load(Ordering::Acquire) {
                self.control
                    .lock()
                    .map_err(|_| "Native YM2612 control state unavailable")?
                    .rejected_clocks
                    .remove(&id);
                return Err(
                    "Release the manually held additional YM2612 voices before changing the clock"
                        .into(),
                );
            }
        }
        Ok(())
    }
    pub fn retire(&self, cmd: Arc<Prepared>) {
        // Invalid events can retire on control while callback publishes a
        // different event. Only callback writes the DSP snapshot seqlock.
        cmd.complete.store(true, Ordering::Release);
        self.retired
            .push(cmd)
            .unwrap_or_else(|_| unreachable!("Bounded YM2612 command retirement"));
    }

    fn publish(&self, r: &Renderer, command: Option<&Prepared>) {
        let state = r.state();
        self.revision.fetch_add(1, Ordering::AcqRel);
        for (out, value) in self.registers.iter().zip(state.registers) {
            out.store(value, Ordering::Relaxed)
        }
        for (out, value) in self.keys.iter().zip(state.keys) {
            out.store(value, Ordering::Relaxed)
        }
        self.clock.store(state.clock, Ordering::Relaxed);
        self.source_mute.store(r.source_mute, Ordering::Relaxed);
        self.channel_mute
            .store(state.channel_mute_mask, Ordering::Relaxed);
        self.status.store(state.status, Ordering::Relaxed);
        self.irq.store(state.irq, Ordering::Relaxed);
        self.held.store(
            r.owners
                .iter()
                .enumerate()
                .filter(|(_, v)| v.is_some_and(|l| l.audition))
                .fold(0, |bits, (ch, _)| bits | (1 << ch)),
            Ordering::Relaxed,
        );
        self.rate.store(r.rate, Ordering::Relaxed);
        if let Some(c) = command {
            c.complete.store(true, Ordering::Release)
        }
        self.revision.fetch_add(1, Ordering::Release);
    }
    fn refresh_running_status(&self, renderer: &Renderer) {
        let state = renderer.state();
        // Timer/busy flags and an audition's delayed key edge can change
        // between commands. Avoid rewriting 512 register atomics or dirtying
        // the inspector revision when no applied state changed.
        if self.status.load(Ordering::Relaxed) == state.status
            && self.irq.load(Ordering::Relaxed) == state.irq
            && self
                .keys
                .iter()
                .zip(state.keys)
                .all(|(out, value)| out.load(Ordering::Relaxed) == value)
        {
            return;
        }
        self.revision.fetch_add(1, Ordering::AcqRel);
        self.status.store(state.status, Ordering::Relaxed);
        self.irq.store(state.irq, Ordering::Relaxed);
        for (out, value) in self.keys.iter().zip(state.keys) {
            out.store(value, Ordering::Relaxed);
        }
        self.revision.fetch_add(1, Ordering::Release);
    }
    pub fn status(&self, frame: u64) -> Result<Status, String> {
        let mut control = self
            .control
            .lock()
            .map_err(|_| "Native YM2612 control unavailable")?;
        self.collect(&mut control);
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() > std::time::Duration::from_millis(50) {
                return Err("Native YM2612 state changed while reading; retry".into());
            }
            let before = self.revision.load(Ordering::Acquire);
            if before & 1 != 0 {
                std::thread::yield_now();
                continue;
            }
            let held = self.held.load(Ordering::Relaxed);
            let out = ChipStatus {
                clock: self.clock(),
                registers: self
                    .registers
                    .chunks(256)
                    .map(|bank| bank.iter().map(|v| v.load(Ordering::Relaxed)).collect())
                    .collect(),
                key_masks: self
                    .keys
                    .iter()
                    .map(|v| v.load(Ordering::Relaxed))
                    .collect(),
                source_mute_mask: self.source_mute.load(Ordering::Relaxed),
                channel_mute_mask: self.channel_mute.load(Ordering::Relaxed),
                status: self.status.load(Ordering::Relaxed),
                irq: self.irq.load(Ordering::Relaxed) != 0,
                held: (0..6).filter(|ch| held & (1 << ch) != 0).collect(),
                pending_commands: control
                    .prepared
                    .iter()
                    .filter(|v| !v.complete.load(Ordering::Relaxed))
                    .filter_map(|v| v.id)
                    .collect(),
            };
            std::sync::atomic::fence(Ordering::Acquire);
            if self.revision.load(Ordering::Relaxed) == before {
                return Ok(Status {
                    ym2612: out,
                    frame,
                    sample_rate: self.rate.load(Ordering::Relaxed),
                    revision: before / 2,
                });
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
    rate: u32,
    owners: [Option<Lease>; VOICES],
    csm_owner: Option<Lease>,
    gains: [f32; 2],
    targets: [f32; 2],
    ramp: u32,
    source_mute: u32,
}
unsafe impl Send for Renderer {}
impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe { native_ym2612_destroy(self.ptr) }
    }
}
impl Renderer {
    pub fn new(rate: u32) -> Result<Self, String> {
        let ptr = unsafe { native_ym2612_create(rate) };
        if ptr.is_null() {
            return Err("Could not allocate Native YM2612 core".into());
        }
        Ok(Self {
            ptr,
            rate,
            owners: [None; VOICES],
            csm_owner: None,
            gains: [1.; 2],
            targets: [1.; 2],
            ramp: 0,
            source_mute: 0,
        })
    }
    fn state(&self) -> CoreState {
        let mut state = CoreState::default();
        unsafe { native_ym2612_get_state(self.ptr, &mut state) }
        state
    }
    fn protected(&self, ch: usize, voice: Option<u64>, audition: bool) -> bool {
        self.owners[ch]
            .is_some_and(|l| l.audition && (!audition || voice.is_some() && l.voice != voice))
    }
    fn release(&mut self, ch: usize) {
        unsafe { native_ym2612_key_off_fm(self.ptr, ch as u8) }
        self.owners[ch] = None;
    }
    fn op(
        &mut self,
        op: &Op,
        owner: Option<Owner>,
        voice: Option<u64>,
        audition: bool,
        preserve: bool,
        _declared: Option<usize>,
    ) {
        let state = self.state();
        let csm_before = state.registers[0x27] & 0x81 == 0x81;
        let csm_write = if let Op::Write {
            port: 0,
            register: 0x27,
            value,
        } = *op
        {
            Some(value & 0x81 == 0x81)
        } else {
            None
        };
        let channel = declared_channel(op);
        if channel.is_some_and(|ch| self.protected(ch, voice, audition)) {
            return;
        }
        if !audition {
            match *op {
                Op::Clock { .. } if self.owners.iter().flatten().any(|l| l.audition) => return,
                Op::Write {
                    port: 0,
                    register: 0x22,
                    ..
                } if self.owners.iter().flatten().any(|l| l.audition) => return,
                _ => {}
            }
        }
        if let Some(csm_on) = csm_write {
            let releasing = csm_before && !csm_on;
            if releasing
                && self.csm_owner.is_some_and(|lease| {
                    owner.is_some() && lease.owner != owner
                        || voice.is_some() && lease.voice != voice
                        || lease.audition && !audition
                })
            {
                return;
            }
            if let Op::Write {
                port,
                register,
                value,
            } = *op
            {
                unsafe {
                    native_ym2612_write(self.ptr, port, register, value);
                }
            }
            if !csm_on {
                self.csm_owner = None;
            } else if !preserve {
                self.csm_owner = Some(Lease {
                    owner,
                    voice,
                    audition,
                });
            }
            return;
        }
        let mut off = matches!(op, Op::KeyOff { .. });
        let mut on = matches!(op, Op::AuditionKeyOn { .. });
        if let Op::Write {
            port,
            register,
            value,
        } = *op
        {
            if port == 0 && register == 0x28 {
                off = value & 0xf0 == 0;
                on = !off
            } else if port == 0 && register == 0x2b {
                off = value & 0x80 == 0;
                on = !off;
            } else if port == 0 && register == 0x2a {
                on = state.registers[0x2b] & 0x80 != 0;
            }
        }
        if off
            && channel.is_some_and(|ch| {
                self.owners[ch].is_some_and(|l| {
                    owner.is_some() && l.owner != owner || voice.is_some() && l.voice != voice
                })
            })
        {
            return;
        }
        unsafe {
            match *op {
                Op::Write {
                    port,
                    register,
                    value,
                } => {
                    if audition
                        && port == 0
                        && register == 0x28
                        && value & 0xf0 != 0
                        && value & 3 < 3
                    {
                        let code = value & 7;
                        native_ym2612_audition_fm_key_on(
                            self.ptr,
                            (code & 3) + if code & 4 != 0 { 3 } else { 0 },
                            value >> 4,
                        );
                    } else {
                        native_ym2612_write(self.ptr, port, register, value)
                    }
                }
                Op::KeyOff { channel } => native_ym2612_key_off_fm(self.ptr, channel),
                Op::AuditionKeyOn { channel } => {
                    native_ym2612_audition_fm_key_on(self.ptr, channel, 15)
                }
                Op::Clock { hz } => {
                    native_ym2612_set_clock(self.ptr, hz);
                }
            }
        }
        if !preserve {
            if let Some(ch) = channel {
                if off {
                    self.owners[ch] = None
                } else if on {
                    self.owners[ch] = Some(Lease {
                        owner,
                        voice,
                        audition,
                    })
                }
            }
        }
    }
    pub fn apply(&mut self, c: &Prepared, owner: Option<Owner>, shared: &Shared) {
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
            Method::Write {
                port,
                register,
                value,
            } => self.op(
                &Op::Write {
                    port: *port,
                    register: *register,
                    value: *value,
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
            Method::SourceMute { mask } => self.source_mute = *mask,
            Method::ChannelMute { mask } => unsafe { native_ym2612_set_mute_mask(self.ptr, *mask) },
            Method::Reset => {
                if c.audition || !self.owners.iter().flatten().any(|l| l.audition) {
                    unsafe { native_ym2612_reset(self.ptr) }
                    self.owners.fill(None);
                    self.csm_owner = None;
                }
            }
            Method::CancelCsm => {
                // CSM and normal CH3 are independent key sources. A matched
                // program CSM lease can end while a manual normal key stays
                // held: the C++ helper preserves that normal key source.
                // Guard manual CSM leases themselves, and untracked CSM that
                // may have been enabled by an inspector edit during the hold.
                let allowed = self.csm_owner.map_or_else(
                    || !self.protected(2, None, c.audition),
                    |lease| {
                        (owner.is_none() || lease.owner == owner) && (!lease.audition || c.audition)
                    },
                );
                if allowed {
                    unsafe {
                        native_ym2612_cancel_csm(self.ptr);
                    }
                    self.csm_owner = None;
                }
            }
            Method::StopVoices => {
                if owner.is_none()
                    && (c.audition || !self.owners.iter().flatten().any(|l| l.audition))
                {
                    unsafe { native_ym2612_stop(self.ptr) }
                    self.owners.fill(None);
                    self.csm_owner = None;
                } else {
                    if self.csm_owner.is_some_and(|lease| {
                        (owner.is_none() || lease.owner == owner) && (!lease.audition || c.audition)
                    }) {
                        unsafe {
                            native_ym2612_cancel_csm(self.ptr);
                        }
                        self.csm_owner = None;
                    }
                    for ch in 0..6 {
                        if if owner.is_none() {
                            self.owners[ch].is_none_or(|l| !l.audition || c.audition)
                        } else {
                            self.owners[ch]
                                .is_some_and(|l| l.owner == owner && (!l.audition || c.audition))
                        } {
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
                    LeaseAction::On => c.audition || current.is_none_or(|l| !l.audition),
                    LeaseAction::Off => current.is_some_and(|l| {
                        l.owner == owner
                            && (voice.is_none() || l.voice == *voice)
                            && (!l.audition || c.audition)
                    }),
                    LeaseAction::Edit => current.is_none_or(|l| {
                        !l.audition || c.audition && (voice.is_none() || l.voice == *voice)
                    }),
                };
                if allowed {
                    if *action == LeaseAction::On {
                        self.owners[ch] = Some(Lease {
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
                            Some(ch),
                        )
                    }
                    if *action == LeaseAction::On {
                        self.owners[ch] = Some(Lease {
                            owner,
                            voice: *voice,
                            audition: c.audition,
                        })
                    } else if *action == LeaseAction::Off {
                        self.owners[ch] = None
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
        if self.csm_owner.is_some_and(|lease| !valid(lease.owner)) {
            unsafe {
                native_ym2612_cancel_csm(self.ptr);
            }
            self.csm_owner = None;
            changed = true;
        }
        for ch in 0..6 {
            if self.owners[ch].is_some_and(|l| !valid(l.owner)) {
                self.release(ch);
                changed = true;
            }
        }
        if changed {
            s.publish(self, None);
        } else {
            s.refresh_running_status(self);
        }
    }
    pub fn mix(&mut self, pcm: &mut [f32; 2]) {
        let (mut l, mut r) = (0., 0.);
        unsafe { native_ym2612_sample(self.ptr, &mut l, &mut r) }
        if self.ramp > 0 {
            for ch in 0..2 {
                self.gains[ch] += (self.targets[ch] - self.gains[ch]) / self.ramp as f32
            }
            self.ramp -= 1
        }
        if self.source_mute == 0 {
            pcm[0] += l * self.gains[0];
            pcm[1] += r * self.gains[1];
        }
    }
}
