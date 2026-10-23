//! Independent eight-channel OPM; no X68000 OKI device or sample RAM.
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
pub const CLOCK: u32 = 3_579_545;
const VOICES: usize = 8;
const PENDING_BYTES: usize = 4 * 1024 * 1024;
#[repr(C)]
struct CoreState {
    registers: [u8; 256],
    clock: u32,
    keys: [u8; 8],
    status: u8,
    irq: u8,
    amd: u8,
    pmd: u8,
}
impl Default for CoreState {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}
extern "C" {
    fn native_ym2151_create(rate: u32) -> *mut c_void;
    fn native_ym2151_destroy(p: *mut c_void);
    fn native_ym2151_reset(p: *mut c_void);
    fn native_ym2151_stop(p: *mut c_void);
    fn native_ym2151_all_notes_off(p: *mut c_void);
    fn native_ym2151_write(p: *mut c_void, register: u8, value: u8);
    fn native_ym2151_key_off_fm(p: *mut c_void, channel: u8);
    fn native_ym2151_cancel_csm(p: *mut c_void);
    fn native_ym2151_audition_key_on(p: *mut c_void, channel: u8, mask: u8);
    fn native_ym2151_sample(p: *mut c_void, left: *mut f32, right: *mut f32);
    fn native_ym2151_set_clock(p: *mut c_void, hz: u32) -> i32;
    fn native_ym2151_get_state(p: *mut c_void, state: *mut CoreState);
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
    },
    KeyOff {
        channel: u8,
    },
    AuditionKeyOn {
        channel: u8,
        mask: u8,
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
    Write { register: u8, value: u8 },
    KeyOff { channel: u8 },
    Clock { hz: u32 },
    AuditionKeyOn { channel: u8, mask: u8 },
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
    group: Option<u64>,
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
        Op::KeyOff { channel } | Op::AuditionKeyOn { channel, .. } => Some(channel as usize),
        Op::Write { register: 8, value } => Some((value & 7) as usize),
        Op::Write { register: 15, .. } => Some(7),
        Op::Write {
            register: 0x20..=0xff,
            ..
        } => {
            if let Op::Write { register, .. } = *op {
                Some((register & 7) as usize)
            } else {
                None
            }
        }
        _ => None,
    }
}
fn validate_op(op: &Op) -> Result<(), String> {
    match *op {
        Op::KeyOff { channel } | Op::AuditionKeyOn { channel, .. } if channel >= 8 => {
            Err("Native YM2151 channel must be 0–7".into())
        }
        Op::AuditionKeyOn { mask, .. } if mask > 15 => {
            Err("Native YM2151 operator mask must be 0–15".into())
        }
        Op::Clock { hz } if !(100_000..=100_000_000).contains(&hz) => {
            Err("Native YM2151 clock must be 100000–100000000 Hz".into())
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
    pub fn prepare(&self) -> Result<Arc<Prepared>, String> {
        if !valid_id(self.id) || !valid_id(self.group) {
            return Err("Invalid Native YM2151 command ID".into());
        }
        match &self.method {
            Method::Write { register, value } => validate_op(&Op::Write {
                register: *register,
                value: *value,
            })?,
            Method::KeyOff { channel } => validate_op(&Op::KeyOff { channel: *channel })?,
            Method::AuditionKeyOn { channel, mask } => validate_op(&Op::AuditionKeyOn {
                channel: *channel,
                mask: *mask,
            })?,
            Method::Clock { hz } => validate_op(&Op::Clock { hz: *hz })?,
            Method::Mix { volume, pan }
                if !volume.is_finite()
                    || !(0.0..=4.).contains(volume)
                    || !pan.is_finite()
                    || !(-1.0..=1.).contains(pan) =>
            {
                return Err("Native YM2151 mixer volume must be 0–4, pan −1–1".into())
            }
            Method::CancelMainOwner { target_group } if !valid_id(Some(*target_group)) => {
                return Err("Invalid Native YM2151 Main owner group".into())
            }
            Method::Batch {
                commands,
                channel,
                voice,
                action,
            } => {
                if *channel >= 8
                    || commands.is_empty()
                    || commands.len() > 512
                    || !valid_id(*voice)
                    || self.audition && *action != LeaseAction::Edit && voice.is_none()
                {
                    return Err("Invalid Native YM2151 batch or voice lease".into());
                }
                for op in commands {
                    validate_op(op)?;
                    if declared_channel(op).is_some_and(|target| target != *channel as usize) {
                        return Err(
                            "Native YM2151 batch must target its declared physical channel".into(),
                        );
                    }
                }
            }
            _ => {}
        }
        Ok(Arc::new(Prepared {
            id: self.id,
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
            Method::Write { .. } => true,
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
}
#[derive(Default)]
struct Control {
    prepared: Vec<Arc<Prepared>>,
    rejected_clocks: HashSet<u64>,
}
pub struct Shared {
    additional: bool,
    revision: AtomicU64,
    registers: [AtomicU8; 256],
    keys: [AtomicU8; 8],
    am_depth: AtomicU8,
    pm_depth: AtomicU8,
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
impl Default for Shared {
    fn default() -> Self {
        Self {
            additional: true,
            revision: AtomicU64::new(0),
            registers: std::array::from_fn(|_| AtomicU8::new(0)),
            keys: std::array::from_fn(|_| AtomicU8::new(0)),
            am_depth: AtomicU8::new(0),
            pm_depth: AtomicU8::new(0),
            clock: AtomicU32::new(CLOCK),
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
pub struct Status {
    pub clock: u32,
    pub registers: Vec<u8>,
    pub key_masks: Vec<u8>,
    pub am_depth: u8,
    pub pm_depth: u8,
    pub status: u8,
    pub irq: bool,
    pub held: Vec<u8>,
    pub pending_commands: Vec<u64>,
    pub frame: u64,
    pub sample_rate: u32,
    pub revision: u64,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdditionalStatus {
    pub ym2151: AdditionalChipStatus,
    pub frame: u64,
    pub sample_rate: u32,
    pub revision: u64,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdditionalChipStatus {
    pub clock: u32,
    pub registers: Vec<u8>,
    pub key_masks: Vec<u8>,
    pub am_depth: u8,
    pub pm_depth: u8,
    pub status: u8,
    pub irq: bool,
    pub held: Vec<u8>,
    pub pending_commands: Vec<u64>,
}
impl Shared {
    fn clock_held_error(&self) -> &'static str {
        if self.additional {
            "Release the manually held additional YM2151 voices before changing the clock"
        } else {
            "Release the manually held YM2151 voices before changing the clock"
        }
    }
    pub fn additional_status(&self, frame: u64) -> Result<AdditionalStatus, String> {
        let value = self.status(frame)?;
        Ok(AdditionalStatus {
            frame: value.frame,
            sample_rate: value.sample_rate,
            revision: value.revision,
            ym2151: AdditionalChipStatus {
                clock: value.clock,
                registers: value.registers,
                key_masks: value.key_masks,
                am_depth: value.am_depth,
                pm_depth: value.pm_depth,
                status: value.status,
                irq: value.irq,
                held: value.held,
                pending_commands: value.pending_commands,
            },
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
            .map_err(|_| "Native YM2151 control state unavailable")?;
        self.collect(&mut c);
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
            return Err("Native YM2151 pending command limit reached (4 MiB)".into());
        }
        let mut ids = HashSet::new();
        for id in c
            .prepared
            .iter()
            .filter_map(|v| v.id)
            .chain(commands.iter().filter_map(|(v, _)| v.id))
        {
            if !ids.insert(id) {
                return Err("Duplicate pending Native YM2151 command ID".into());
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
            return Err("Unknown Native YM2151 command ID".into());
        }
        let cmd = {
            let mut control = self
                .control
                .lock()
                .map_err(|_| "Native YM2151 control state unavailable")?;
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
                let deadline = if matches!(cmd.method, Method::Clock { .. } | Method::Reset) {
                    5000
                } else {
                    100
                };
                if start.elapsed() > std::time::Duration::from_millis(deadline) {
                    return Err(format!(
                        "Native YM2151 command was not applied within {deadline} ms"
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            cmd.outcome_observed.store(true, Ordering::Release);
            if cmd.clock_rejected.load(Ordering::Acquire) {
                self.control
                    .lock()
                    .map_err(|_| "Native YM2151 control state unavailable")?
                    .rejected_clocks
                    .remove(&id);
                return Err(self.clock_held_error().into());
            }
        }
        Ok(())
    }
    pub fn rollback(&self, cmd: Arc<Prepared>) {
        self.retire(cmd)
    }
    pub fn retire(&self, cmd: Arc<Prepared>) {
        cmd.complete.store(true, Ordering::Release);
        self.retired
            .push(cmd)
            .unwrap_or_else(|_| unreachable!("Bounded YM2151 command retirement"));
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
        self.am_depth.store(state.amd, Ordering::Relaxed);
        self.pm_depth.store(state.pmd, Ordering::Relaxed);
        self.clock.store(state.clock, Ordering::Relaxed);
        self.status.store(state.status, Ordering::Relaxed);
        self.irq.store(state.irq, Ordering::Relaxed);
        self.held.store(
            r.owners
                .iter()
                .enumerate()
                .filter(|(_, v)| v.is_some_and(|l| l.audition))
                .fold(
                    if r.csm_owner.is_some_and(|l| l.audition) {
                        255
                    } else {
                        0
                    },
                    |bits, (ch, _)| bits | (1 << ch),
                ),
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
            .map_err(|_| "Native YM2151 control unavailable")?;
        self.collect(&mut control);
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() > std::time::Duration::from_millis(50) {
                return Err("Native YM2151 state changed while reading; retry".into());
            }
            let before = self.revision.load(Ordering::Acquire);
            if before & 1 != 0 {
                std::thread::yield_now();
                continue;
            }
            let held = self.held.load(Ordering::Relaxed);
            let out = Status {
                clock: self.clock(),
                registers: self
                    .registers
                    .iter()
                    .map(|v| v.load(Ordering::Relaxed))
                    .collect(),
                key_masks: self
                    .keys
                    .iter()
                    .map(|v| v.load(Ordering::Relaxed))
                    .collect(),
                am_depth: self.am_depth.load(Ordering::Relaxed),
                pm_depth: self.pm_depth.load(Ordering::Relaxed),
                status: self.status.load(Ordering::Relaxed),
                irq: self.irq.load(Ordering::Relaxed) != 0,
                held: (0..8).filter(|ch| held & (1 << ch) != 0).collect(),
                pending_commands: control
                    .prepared
                    .iter()
                    .filter(|v| !v.complete.load(Ordering::Relaxed))
                    .filter_map(|v| v.id)
                    .collect(),
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
    additional: bool,
    current_group: Option<u64>,
    csm_owner: Option<Lease>,
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
        unsafe { native_ym2151_destroy(self.ptr) }
    }
}
impl Renderer {
    pub fn new_additional(rate: u32) -> Result<Self, String> {
        Self::new_flavor(rate, true)
    }
    fn new_flavor(rate: u32, additional: bool) -> Result<Self, String> {
        let ptr = unsafe { native_ym2151_create(rate) };
        if ptr.is_null() {
            return Err("Could not allocate Native YM2151 core".into());
        }
        Ok(Self {
            additional,
            current_group: None,
            csm_owner: None,
            ptr,
            rate,
            owners: [None; VOICES],
            gains: [1.; 2],
            targets: [1.; 2],
            ramp: 0,
        })
    }
    fn state(&self) -> CoreState {
        let mut state = CoreState::default();
        unsafe { native_ym2151_get_state(self.ptr, &mut state) }
        state
    }
    fn has_manual(&self) -> bool {
        self.owners.iter().flatten().any(|l| l.audition)
            || self.csm_owner.is_some_and(|l| l.audition)
    }
    fn protected(&self, ch: usize, voice: Option<u64>, audition: bool) -> bool {
        self.owners[ch].is_some_and(|lease| {
            lease.audition && (!audition || voice.is_some() && lease.voice != voice)
        }) || !audition && self.csm_owner.is_some_and(|lease| lease.audition)
    }
    fn release(&mut self, ch: usize) {
        unsafe { native_ym2151_key_off_fm(self.ptr, ch as u8) }
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
    ) {
        let channel = declared_channel(op);
        if channel.is_some_and(|ch| self.protected(ch, voice, audition)) {
            return;
        }
        if !audition
            && matches!(
                op,
                Op::Clock { .. }
                    | Op::Write {
                        register: 1 | 0x10..=0x14 | 0x18 | 0x19 | 0x1b,
                        ..
                    }
            )
            && self.has_manual()
        {
            return;
        }
        if let Op::Write {
            register: 0x14,
            value,
        } = *op
        {
            let on = value & 0x81 == 0x81;
            if !on
                && self
                    .csm_owner
                    .is_some_and(|l| self.foreign(l, owner, voice, audition))
            {
                return;
            }
            unsafe { native_ym2151_write(self.ptr, 0x14, value) }
            if !on {
                self.csm_owner = None
            } else if !preserve {
                self.csm_owner = Some(self.lease(owner, voice, audition))
            }
            return;
        }
        let off = match *op {
            Op::KeyOff { .. } => true,
            Op::Write { register: 8, value } => value & 0x78 == 0,
            Op::AuditionKeyOn { mask, .. } => mask == 0,
            _ => false,
        };
        if off
            && channel.is_some_and(|ch| {
                self.owners[ch].is_some_and(|l| self.foreign(l, owner, voice, audition))
            })
        {
            return;
        }
        let on = matches!(op,Op::AuditionKeyOn{mask,..}if *mask!=0)
            || matches!(op,Op::Write{register:8,value}if value&0x78!=0);
        unsafe {
            match *op {
                Op::Write { register: 8, value } if audition && value & 0x78 != 0 => {
                    native_ym2151_audition_key_on(self.ptr, value & 7, (value >> 3) & 15)
                }
                Op::Write { register, value } => native_ym2151_write(self.ptr, register, value),
                Op::KeyOff { channel } => native_ym2151_key_off_fm(self.ptr, channel),
                Op::Clock { hz } => {
                    native_ym2151_set_clock(self.ptr, hz);
                }
                Op::AuditionKeyOn { channel, mask } => {
                    native_ym2151_audition_key_on(self.ptr, channel, mask)
                }
            }
        }
        if !preserve {
            if let Some(ch) = channel {
                if off {
                    self.owners[ch] = None
                } else if on {
                    self.owners[ch] = Some(self.lease(owner, voice, audition))
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
        self.current_group = if self.additional { c.group } else { None };
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
            Method::Clock { hz } => self.op(&Op::Clock { hz: *hz }, owner, None, c.audition, false),
            Method::AuditionKeyOn { channel, mask } => self.op(
                &Op::AuditionKeyOn {
                    channel: *channel,
                    mask: *mask,
                },
                owner,
                None,
                c.audition,
                false,
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
                    unsafe { native_ym2151_reset(self.ptr) }
                    self.owners.fill(None);
                    self.csm_owner = None;
                }
            }
            Method::CancelCsm => {
                let allowed = self.csm_owner.map_or_else(
                    || !self.has_manual() || c.audition,
                    |lease| {
                        (owner.is_none() || lease.owner == owner)
                            && (c.group.is_none() || lease.group == c.group)
                            && (!lease.audition || c.audition)
                    },
                );
                if allowed {
                    unsafe {
                        native_ym2151_cancel_csm(self.ptr);
                    }
                    self.csm_owner = None;
                }
            }
            Method::CancelMainOwner { target_group } => {
                let matches = |lease: Lease| !lease.audition && lease.group == Some(*target_group);
                if self.csm_owner.is_some_and(matches) {
                    unsafe {
                        native_ym2151_cancel_csm(self.ptr);
                    }
                    self.csm_owner = None;
                }
                for channel in 0..8 {
                    if self.owners[channel].is_some_and(matches) {
                        self.release(channel);
                    }
                }
            }
            Method::StopVoices | Method::AllNotesOff => {
                if owner.is_none() && c.group.is_none() && (c.audition || !self.has_manual()) {
                    if matches!(c.method, Method::AllNotesOff) {
                        unsafe { native_ym2151_all_notes_off(self.ptr) }
                        // Public allNotesOff gates the host output and retains
                        // Timer/CSM settings for a later ordinary key-on.
                        self.owners.fill(None);
                    } else {
                        unsafe { native_ym2151_stop(self.ptr) }
                        self.owners.fill(None);
                        self.csm_owner = None;
                    }
                } else {
                    if self.csm_owner.is_some_and(|lease| {
                        (owner.is_none() || lease.owner == owner)
                            && (c.group.is_none() || lease.group == c.group)
                            && (!lease.audition || c.audition)
                    }) {
                        unsafe {
                            native_ym2151_cancel_csm(self.ptr);
                        }
                        self.csm_owner = None;
                    }
                    for ch in 0..8 {
                        if if owner.is_none() {
                            self.owners[ch].is_none_or(|l| {
                                (c.group.is_none() || l.group == c.group)
                                    && (!l.audition || c.audition)
                            })
                        } else {
                            self.owners[ch].is_some_and(|l| {
                                l.owner == owner
                                    && (c.group.is_none() || l.group == c.group)
                                    && (!l.audition || c.audition)
                            })
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
                            && (c.group.is_none() || l.group == c.group)
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
                            group: self.current_group,
                            owner,
                            voice: *voice,
                            audition: c.audition,
                        })
                    }
                    for op in commands {
                        self.op(op, owner, *voice, c.audition, *action == LeaseAction::Edit)
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
        if self.additional && self.csm_owner.is_some_and(|lease| !valid(lease.owner)) {
            unsafe {
                native_ym2151_cancel_csm(self.ptr);
            }
            self.csm_owner = None;
            changed = true;
        }
        for ch in 0..8 {
            if self.owners[ch].is_some_and(|l| !valid(l.owner)) {
                self.release(ch);
                changed = true
            }
        }
        if changed {
            s.publish(self, None)
        } else if self.additional {
            s.refresh_running_status(self);
        }
    }
    pub fn mix(&mut self, pcm: &mut [f32; 2]) {
        let (mut l, mut r) = (0., 0.);
        unsafe { native_ym2151_sample(self.ptr, &mut l, &mut r) }
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
