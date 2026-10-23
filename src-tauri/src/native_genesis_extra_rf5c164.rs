//! Independent RF5C164 core. Uploads and final reclamation are prepared on
//! control; the callback applies bounded immutable payloads without allocations.
use super::{Owner, CAPACITY, MAX_SAFE_INTEGER, OWNERS};
use crossbeam_queue::ArrayQueue;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    ffi::c_void,
    sync::{
        atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicU64, AtomicU8, Ordering},
        Arc, Mutex,
    },
};
pub const CLOCK: u32 = 12_500_000;
pub const RAM: usize = 65_536;
const PENDING_BYTES: usize = 4 * 1024 * 1024;
const CLOCK_HELD: &str =
    "Release the manually held additional RF5C164 voices before changing the clock";
#[repr(C)]
pub(super) struct CoreState {
    registers: [[u8; 7]; 8],
    control: u8,
    selected: u8,
    mask: u8,
    resampling: u8,
    clock: u32,
    addresses: [u32; 8],
    bank: u16,
    reserved: u16,
}
impl Default for CoreState {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}
extern "C" {
    fn native_extra_rf5c164_create(rate: u32) -> *mut c_void;
    fn native_extra_rf5c164_destroy(p: *mut c_void);
    fn native_extra_rf5c164_reset(p: *mut c_void);
    fn native_extra_rf5c164_stop(p: *mut c_void);
    fn native_extra_rf5c164_write(p: *mut c_void, reg: u8, value: u8);
    fn native_extra_rf5c164_key_off(p: *mut c_void, ch: u8);
    fn native_extra_rf5c164_load(p: *mut c_void, data: *const u8, offset: u32, size: u32) -> i32;
    fn native_extra_rf5c164_clear_memory(p: *mut c_void);
    fn native_extra_rf5c164_sample(p: *mut c_void, l: *mut f32, r: *mut f32);
    fn native_extra_rf5c164_set_clock(p: *mut c_void, hz: u32) -> i32;
    fn native_extra_rf5c164_get_state(p: *mut c_void, s: *mut CoreState);
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
    SetChannel {
        channel: u8,
        options: Options,
    },
    KeyOn {
        channel: u8,
        #[serde(default)]
        voice: Option<u64>,
    },
    KeyOff {
        channel: u8,
        #[serde(default)]
        voice: Option<u64>,
    },
    LoadMemory {
        offset: u32,
        data: Vec<u8>,
    },
    WriteMemory {
        offset: u16,
        value: u8,
    },
    ClearMemory,
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
    },
    SetChannel {
        channel: u8,
        options: Options,
    },
    KeyOn {
        channel: u8,
        #[serde(default)]
        voice: Option<u64>,
    },
    KeyOff {
        channel: u8,
        #[serde(default)]
        voice: Option<u64>,
    },
}
#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Options {
    pub start: Option<u32>,
    pub loop_start: Option<u16>,
    pub step: Option<u16>,
    pub volume: Option<u8>,
    pub pan: Option<Pan>,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct Pan {
    pub left: u8,
    pub right: u8,
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
    cancelled: AtomicBool,
    clock_rejected: AtomicBool,
    memory_rejected: AtomicBool,
    outcome_observed: AtomicBool,
}
fn valid_id(id: Option<u64>) -> bool {
    id.is_none_or(|id| id > 0 && id <= MAX_SAFE_INTEGER)
}
fn validate_options(o: &Options) -> Result<(), String> {
    if o.start.is_some_and(|v| v > 0xff00 || v & 255 != 0)
        || o.pan.is_some_and(|p| p.left > 15 || p.right > 15)
    {
        Err("Invalid Native additional RF5C164 channel settings".into())
    } else {
        Ok(())
    }
}
fn validate_op(op: &Op) -> Result<(), String> {
    match op {
        Op::Write { register, .. } if *register > 8 => {
            Err("Native RF5C164 register must be0–8".into())
        }
        Op::SetChannel { channel, options } => {
            if *channel > 7 {
                return Err("Native RF5C164 channel must be0–7".into());
            }
            validate_options(options)
        }
        Op::KeyOn { channel, voice } | Op::KeyOff { channel, voice }
            if *channel > 7 || !valid_id(*voice) =>
        {
            Err("Invalid Native RF5C164 voice lease".into())
        }
        _ => Ok(()),
    }
}
impl Message {
    pub fn prepared_bytes(&self) -> usize {
        std::mem::size_of::<Prepared>()
            + match &self.method {
                Method::Batch { commands, .. } => commands.len() * std::mem::size_of::<Op>(),
                Method::LoadMemory { data, .. } => data.len(),
                _ => 0,
            }
    }
    pub fn prepare(&self) -> Result<Arc<Prepared>, String> {
        if !valid_id(self.id) || !valid_id(self.group) {
            return Err("Invalid Native additional RF5C164 command ID".into());
        }
        match &self.method {
            Method::Write { register, value } => validate_op(&Op::Write {
                register: *register,
                value: *value,
            })?,
            Method::SetChannel { channel, options } => validate_op(&Op::SetChannel {
                channel: *channel,
                options: *options,
            })?,
            Method::KeyOn { channel, voice } => validate_op(&Op::KeyOn {
                channel: *channel,
                voice: *voice,
            })?,
            Method::KeyOff { channel, voice } => validate_op(&Op::KeyOff {
                channel: *channel,
                voice: *voice,
            })?,
            Method::WriteMemory { offset, .. } if *offset > 4095 => {
                return Err("Native RF5C164 banked offset must be0–4095".into())
            }
            Method::LoadMemory { offset, data }
                if *offset as usize > RAM || data.len() > RAM - *offset as usize =>
            {
                return Err("Native RF5C164 upload is outside64KiBRAM".into())
            }
            Method::Clock { hz } if !(100_000..=100_000_000).contains(hz) => {
                return Err("Native RF5C164 clock must be100k–100MHz".into())
            }
            Method::Mix { volume, pan }
                if !volume.is_finite()
                    || !(0.0..=4.0).contains(volume)
                    || !pan.is_finite()
                    || !(-1.0..=1.0).contains(pan) =>
            {
                return Err("Invalid Native RF5C164 mixer settings".into())
            }
            Method::CancelMainOwner { target_group } if !valid_id(Some(*target_group)) => {
                return Err("Invalid Native RF5C164 Main owner".into())
            }
            Method::Batch {
                commands,
                channel,
                voice,
                action,
            } => {
                if *channel > 7
                    || commands.is_empty()
                    || commands.len() > 512
                    || !valid_id(*voice)
                    || self.audition && *action != LeaseAction::Edit && voice.is_none()
                {
                    return Err("Invalid Native RF5C164 batch or voice lease".into());
                }
                let mut selected = *channel;
                for op in commands {
                    validate_op(op)?;
                    match op {
                        Op::Write { register: 7, value } => {
                            if value & 64 != 0 {
                                selected = value & 7
                            }
                            if value & 128 == 0 {
                                return Err(
                                    "RF5C164 voice batch cannot disable the global output".into()
                                );
                            }
                        }
                        Op::Write {
                            register: 0..=6, ..
                        } if selected != *channel => {
                            return Err("RF5C164 batch must target its declared channel".into())
                        }
                        Op::Write { register: 8, .. } => {
                            return Err(
                                "RF5C164 voice batch must use key-on/off, not a global mask".into(),
                            )
                        }
                        Op::SetChannel { channel: ch, .. }
                        | Op::KeyOn { channel: ch, .. }
                        | Op::KeyOff { channel: ch, .. }
                            if ch != channel =>
                        {
                            return Err("RF5C164 batch must target its declared channel".into())
                        }
                        _ => {}
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
            cancelled: AtomicBool::new(false),
            clock_rejected: AtomicBool::new(false),
            memory_rejected: AtomicBool::new(false),
            outcome_observed: AtomicBool::new(false),
        }))
    }
}
impl Prepared {
    pub fn is_owner_cancel(&self) -> bool {
        matches!(self.method, Method::CancelMainOwner { .. })
    }
    pub fn starts(&self) -> bool {
        matches!(
            &self.method,
            Method::Write { .. } | Method::KeyOn { .. } | Method::SetChannel { .. }
        ) || matches!(&self.method,Method::Batch{action,..}if *action!=LeaseAction::Off)
    }
    pub fn stops(&self) -> bool {
        matches!(self.method, Method::StopVoices) && self.group.is_none()
    }
    pub fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + match &self.method {
                Method::Batch { commands, .. } => commands.capacity() * std::mem::size_of::<Op>(),
                Method::LoadMemory { data, .. } => data.capacity(),
                _ => 0,
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
    registers: [[AtomicU8; 7]; 8],
    configured: [AtomicU16; 8],
    control_reg: AtomicU8,
    selected: AtomicU8,
    mask: AtomicU8,
    bank: AtomicU16,
    clock: AtomicU32,
    rate: AtomicU32,
    held: AtomicU8,
    known: [AtomicU64; 1024],
    memory_revision: AtomicU64,
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
            registers: std::array::from_fn(|_| std::array::from_fn(|_| AtomicU8::new(0))),
            configured: std::array::from_fn(|_| AtomicU16::new(0)),
            control_reg: AtomicU8::new(0),
            selected: AtomicU8::new(0),
            mask: AtomicU8::new(255),
            bank: AtomicU16::new(0),
            clock: AtomicU32::new(CLOCK),
            rate: AtomicU32::new(0),
            held: AtomicU8::new(0),
            known: std::array::from_fn(|_| AtomicU64::new(0)),
            memory_revision: AtomicU64::new(0),
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
pub struct Channel {
    pub channel: u8,
    pub start: u32,
    pub loop_start: u16,
    pub step: u16,
    pub configured_step: u16,
    pub volume: u8,
    pub pan: Pan,
    pub active: bool,
}
#[derive(Debug, Serialize)]
pub struct Range {
    pub start: u32,
    pub end: u32,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChipStatus {
    pub clock: u32,
    pub channels: Vec<Channel>,
    pub enabled: bool,
    pub mask: u8,
    pub selected: u8,
    pub bank: u16,
    pub held: Vec<u8>,
    pub pending_commands: Vec<u64>,
    pub memory_revision: u64,
    pub known_memory_ranges: Vec<Range>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub sample_rate: u32,
    pub frame: u64,
    pub revision: u64,
    pub rf5c164: ChipStatus,
}
impl Shared {
    pub fn prepare_message(&self, m: &Message) -> Result<Arc<Prepared>, String> {
        m.prepare()
    }
    pub fn clock(&self) -> u32 {
        self.clock.load(Ordering::Acquire)
    }
    pub fn valid_event(&self, command: &Prepared, sequence: u64, owner: Option<Owner>) -> bool {
        !command.cancelled.load(Ordering::Acquire)
            && sequence >= self.cancel_before.load(Ordering::Acquire)
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
            let rejected = cmd.id.is_some()
                && (cmd.clock_rejected.load(Ordering::Acquire)
                    || cmd.memory_rejected.load(Ordering::Acquire)
                    || cmd.cancelled.load(Ordering::Acquire));
            let observed = cmd.outcome_observed.load(Ordering::Acquire);
            rejected && !observed
                || !retired.contains(&(Arc::as_ptr(cmd) as usize)) && !(rejected && observed)
        });
        if c.prepared.capacity() > 64.max(c.prepared.len() * 2) {
            c.prepared.shrink_to_fit()
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
            .map_err(|_| "Native RF5C164 control unavailable")?;
        self.collect(&mut c);
        if self.held.load(Ordering::Acquire) != 0
            && commands
                .iter()
                .any(|(v, _)| !v.audition && matches!(v.method, Method::Clock { .. }))
        {
            return Err(CLOCK_HELD.into());
        }
        if c.prepared.len() + commands.len() > CAPACITY
            || c.prepared.iter().map(|v| v.retained_bytes()).sum::<usize>()
                + commands
                    .iter()
                    .map(|(v, _)| v.retained_bytes())
                    .sum::<usize>()
                > PENDING_BYTES
        {
            return Err("Native RF5C164 pending command limit reached(4MiB)".into());
        }
        let mut ids = HashSet::new();
        for id in c
            .prepared
            .iter()
            .filter_map(|v| v.id)
            .chain(commands.iter().filter_map(|(v, _)| v.id))
        {
            if !ids.insert(id) {
                return Err("Duplicate pending Native RF5C164 command ID".into());
            }
        }
        for (cmd, _) in commands {
            if let Some(id) = cmd.id {
                self.highest_id.fetch_max(id, Ordering::AcqRel);
            }
            c.prepared.push(cmd.clone())
        }
        Ok(())
    }
    /// Commit only after every instance/family has passed transactional retain.
    /// This invalidates queued lookahead without leaking a rejected batch's
    /// cancellation into an older, independently admitted performance.
    pub fn commit_cancel(&self, command: &Prepared) -> bool {
        let Method::CancelMainOwner { target_group } = command.method else {
            return false;
        };
        let control = self
            .control
            .lock()
            .expect("RF control checked by transactional retain");
        for pending in &control.prepared {
            if pending.group == Some(target_group) && !pending.audition {
                pending.cancelled.store(true, Ordering::Release);
            }
        }
        true
    }
    pub fn wait_applied(&self, id: u64) -> Result<(), String> {
        if id == 0 || id > self.highest_id.load(Ordering::Acquire) {
            return Err("Unknown Native RF5C164 command ID".into());
        }
        let cmd = {
            let mut c = self
                .control
                .lock()
                .map_err(|_| "Native RF5C164 control unavailable")?;
            if c.rejected_clocks.remove(&id) {
                return Err(CLOCK_HELD.into());
            }
            c.prepared.iter().find(|v| v.id == Some(id)).cloned()
        };
        if let Some(cmd) = cmd {
            let start = std::time::Instant::now();
            while !cmd.complete.load(Ordering::Acquire) {
                if start.elapsed() > std::time::Duration::from_millis(5000) {
                    return Err("Native RF5C164 command was not applied within5000ms".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(1))
            }
            cmd.outcome_observed.store(true, Ordering::Release);
            if cmd.cancelled.load(Ordering::Acquire) {
                return Err("Native RF5C164 command was cancelled before it applied".into());
            }
            if cmd.clock_rejected.load(Ordering::Acquire) {
                return Err(CLOCK_HELD.into());
            }
            if cmd.memory_rejected.load(Ordering::Acquire) {
                return Err(
                    "Release manually held RF5C164 voices before overwriting their sample RAM"
                        .into(),
                );
            }
        }
        Ok(())
    }
    pub fn rollback(&self, c: Arc<Prepared>) {
        self.retire(c)
    }
    pub fn retire(&self, c: Arc<Prepared>) {
        if !c.complete.load(Ordering::Acquire) {
            c.cancelled.store(true, Ordering::Release);
        }
        c.complete.store(true, Ordering::Release);
        self.retired
            .push(c)
            .unwrap_or_else(|_| unreachable!("Bounded RF5C164 retirement"))
    }
    fn publish(&self, r: &Renderer, c: Option<&Prepared>) {
        let state = r.state();
        self.revision.fetch_add(1, Ordering::AcqRel);
        for (out, row) in self.registers.iter().zip(state.registers) {
            for (out, value) in out.iter().zip(row) {
                out.store(value, Ordering::Relaxed)
            }
        }
        for (out, v) in self.configured.iter().zip(r.configured) {
            out.store(v, Ordering::Relaxed)
        }
        self.control_reg.store(state.control, Ordering::Relaxed);
        self.selected.store(state.selected, Ordering::Relaxed);
        self.mask.store(state.mask, Ordering::Relaxed);
        self.bank.store(state.bank, Ordering::Relaxed);
        self.clock.store(state.clock, Ordering::Relaxed);
        self.rate.store(r.rate, Ordering::Relaxed);
        self.held.store(
            r.owners
                .iter()
                .enumerate()
                .filter(|(_, l)| l.is_some_and(|l| l.audition))
                .fold(0, |m, (ch, _)| m | 1 << ch),
            Ordering::Relaxed,
        );
        if let Some(c) = c {
            c.complete.store(true, Ordering::Release)
        }
        self.revision.fetch_add(1, Ordering::Release);
    }
    pub fn status(&self, frame: u64) -> Result<Status, String> {
        let mut c = self
            .control
            .lock()
            .map_err(|_| "Native RF5C164 control unavailable")?;
        self.collect(&mut c);
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() > std::time::Duration::from_millis(50) {
                return Err("Native RF5C164 snapshot changing;retry".into());
            }
            let before = self.revision.load(Ordering::Acquire);
            if before & 1 != 0 {
                std::thread::yield_now();
                continue;
            }
            let enabled = self.control_reg.load(Ordering::Relaxed) & 128 != 0;
            let mask = self.mask.load(Ordering::Relaxed);
            let held = self.held.load(Ordering::Relaxed);
            let mut ranges = Vec::new();
            let mut range = None;
            for i in 0..RAM {
                let known = self.known[i / 64].load(Ordering::Relaxed) & (1 << (i % 64)) != 0;
                if known && range.is_none() {
                    range = Some(i as u32)
                } else if !known {
                    if let Some(start) = range.take() {
                        ranges.push(Range {
                            start,
                            end: i as u32,
                        })
                    }
                }
            }
            if let Some(start) = range {
                ranges.push(Range {
                    start,
                    end: RAM as u32,
                })
            }
            let out = Status {
                sample_rate: self.rate.load(Ordering::Relaxed),
                frame,
                revision: before / 2,
                rf5c164: ChipStatus {
                    clock: self.clock(),
                    enabled,
                    mask,
                    selected: self.selected.load(Ordering::Relaxed),
                    bank: self.bank.load(Ordering::Relaxed),
                    held: (0..8).filter(|ch| held & (1 << ch) != 0).collect(),
                    pending_commands: c
                        .prepared
                        .iter()
                        .filter(|p| !p.complete.load(Ordering::Relaxed))
                        .filter_map(|p| p.id)
                        .collect(),
                    memory_revision: self.memory_revision.load(Ordering::Relaxed),
                    known_memory_ranges: ranges,
                    channels: (0..8)
                        .map(|ch| {
                            let row: [u8; 7] = std::array::from_fn(|i| {
                                self.registers[ch][i].load(Ordering::Relaxed)
                            });
                            Channel {
                                channel: ch as u8,
                                start: (row[6] as u32) << 8,
                                loop_start: u16::from_le_bytes([row[4], row[5]]),
                                step: u16::from_le_bytes([row[2], row[3]]),
                                configured_step: self.configured[ch].load(Ordering::Relaxed),
                                volume: row[0],
                                pan: Pan {
                                    left: row[1] & 15,
                                    right: row[1] >> 4,
                                },
                                active: enabled && mask & (1 << ch) == 0,
                            }
                        })
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
#[derive(Clone, Copy, Debug)]
struct Lease {
    owner: Option<Owner>,
    group: Option<u64>,
    voice: Option<u64>,
    audition: bool,
}
#[derive(Clone, Copy, Default)]
struct Span {
    start: u32,
    end: u32,
}
#[derive(Clone, Copy, Default)]
struct Guard {
    spans: [Span; 6],
    len: usize,
}
impl Guard {
    fn intersects(&self, offset: usize, size: usize) -> bool {
        self.spans[..self.len]
            .iter()
            .any(|s| offset < s.end as usize && offset + size > s.start as usize)
    }
    fn add(&mut self, start: usize, end: usize) {
        if end - start >= RAM {
            self.spans = [Span::default(); 6];
            self.spans[0] = Span {
                start: 0,
                end: RAM as u32,
            };
            self.len = 1;
            return;
        }
        if end <= RAM {
            self.spans[self.len] = Span {
                start: start as u32,
                end: end as u32,
            };
            self.len += 1
        } else {
            self.add(start, RAM);
            self.add(0, end - RAM)
        }
    }
}
pub struct Renderer {
    ptr: *mut c_void,
    rate: u32,
    owners: [Option<Lease>; 8],
    configured: [u16; 8],
    markers: [u64; 1024],
    guards: [Guard; 8],
    guard_dirty: [bool; 8],
    current_group: Option<u64>,
    gains: [f32; 2],
    targets: [f32; 2],
    ramp: u32,
}
unsafe impl Send for Renderer {}
impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe { native_extra_rf5c164_destroy(self.ptr) }
    }
}
impl Renderer {
    pub fn new(rate: u32) -> Result<Self, String> {
        let ptr = unsafe { native_extra_rf5c164_create(rate) };
        if ptr.is_null() {
            return Err("Could not allocate Native additional RF5C164 core".into());
        }
        Ok(Self {
            ptr,
            rate,
            owners: [None; 8],
            configured: [0; 8],
            markers: [0; 1024],
            guards: [Guard::default(); 8],
            guard_dirty: [true; 8],
            current_group: None,
            gains: [1.; 2],
            targets: [1.; 2],
            ramp: 0,
        })
    }
    fn state(&self) -> CoreState {
        let mut s = CoreState::default();
        unsafe { native_extra_rf5c164_get_state(self.ptr, &mut s) }
        s
    }
    fn manual(&self) -> bool {
        self.owners.iter().any(|l| l.is_some_and(|l| l.audition))
    }
    fn protected(&self, ch: usize, voice: Option<u64>, audition: bool) -> bool {
        self.owners[ch]
            .is_some_and(|l| l.audition && (!audition || voice.is_some() && voice != l.voice))
    }
    fn matching(
        &self,
        ch: usize,
        owner: Option<Owner>,
        voice: Option<u64>,
        audition: bool,
    ) -> bool {
        self.owners[ch].is_some_and(|l| {
            l.owner == owner
                && (self.current_group.is_none() || l.group == self.current_group)
                && (voice.is_none() || voice == l.voice)
                && (!l.audition || audition)
        })
    }
    fn lease(&self, owner: Option<Owner>, voice: Option<u64>, audition: bool) -> Lease {
        Lease {
            owner,
            voice,
            audition,
            group: self.current_group,
        }
    }
    // Find a guaranteed end-marker run using 64-bit words. ceil(step/2048)
    // consecutive markers cannot be skipped by a forward PCM step. At most
    // 1024 words ×5 bit intersections per path; never per-sample or per-write.
    fn guarded_end(&self, start: usize, run: usize) -> usize {
        let first = start / 64;
        for index in first..=first + 1024 {
            let word = index % 1024;
            let original =
                self.markers[word] as u128 | ((self.markers[(word + 1) % 1024] as u128) << 64);
            let mut candidates = original;
            let mut power = 1;
            while power * 2 <= run {
                candidates &= candidates >> power;
                power *= 2
            }
            if power < run {
                candidates &= candidates >> (run - power)
            }
            let mut bits = candidates as u64;
            if index == first {
                bits &= u64::MAX << (start % 64)
            }
            if index == first + 1024 {
                bits &= if start % 64 == 0 {
                    0
                } else {
                    (1u64 << (start % 64)) - 1
                }
            }
            if bits != 0 {
                return (index * 64 + bits.trailing_zeros() as usize + run).min(start + RAM);
            }
        }
        start + RAM
    }
    fn update_guard(&mut self, ch: usize) {
        let s = self.state();
        let r = s.registers[ch];
        let starts = [
            (r[6] as usize) << 8,
            u16::from_le_bytes([r[4], r[5]]) as usize,
            ((s.addresses[ch] >> 11) & 65535) as usize,
        ];
        let step = u16::from_le_bytes([r[2], r[3]]) as usize;
        let run = step.div_ceil(2048).max(1);
        let mut g = Guard::default();
        for start in starts {
            let end = self.guarded_end(start, run);
            if end - start >= RAM {
                g = Guard::default();
                g.add(0, RAM);
                break;
            }
            g.add(start, end);
        }
        self.guards[ch] = g;
        self.guard_dirty[ch] = false;
    }
    fn release(&mut self, ch: usize, strong: bool) {
        if strong {
            unsafe { native_extra_rf5c164_key_off(self.ptr, ch as u8) }
        } else {
            let mask = self.state().mask | (1 << ch);
            unsafe { native_extra_rf5c164_write(self.ptr, 8, mask) }
        }
        self.owners[ch] = None;
    }
    fn raw(
        &mut self,
        reg: u8,
        value: u8,
        owner: Option<Owner>,
        voice: Option<u64>,
        audition: bool,
    ) {
        let s = self.state();
        if reg <= 6 {
            let ch = s.selected as usize;
            if self.protected(ch, voice, audition) {
                return;
            }
            unsafe { native_extra_rf5c164_write(self.ptr, reg, value) }
            if reg == 2 || reg == 3 {
                self.configured[ch] =
                    u16::from_le_bytes(self.state().registers[ch][2..4].try_into().unwrap())
            }
            if self.owners[ch].is_some_and(|l| l.audition) {
                self.guard_dirty[ch] = true
            }
        } else if reg == 7 {
            let mut merged = value;
            if value & 128 == 0
                && !audition
                && (0..8).any(|ch| {
                    self.owners[ch].is_some() && !self.matching(ch, owner, voice, audition)
                })
            {
                merged |= s.control & 128
            }
            unsafe { native_extra_rf5c164_write(self.ptr, reg, merged) }
        } else {
            let mut merged = value;
            for ch in 0..8 {
                let bit = 1 << ch;
                if self.protected(ch, voice, audition) {
                    merged = (merged & !bit) | (s.mask & bit);
                    continue;
                }
                if value & bit == 0 {
                    if s.mask & bit != 0
                        || self.owners[ch].is_none()
                        || self.matching(ch, owner, voice, audition)
                    {
                        self.owners[ch] = Some(self.lease(owner, voice, audition));
                        if audition {
                            self.guard_dirty[ch] = true
                        }
                    }
                } else if !audition
                    && self.owners[ch].is_some()
                    && !self.matching(ch, owner, voice, audition)
                {
                    merged = (merged & !bit) | (s.mask & bit)
                } else {
                    self.owners[ch] = None
                }
            }
            unsafe { native_extra_rf5c164_write(self.ptr, 8, merged) }
        }
    }
    fn set_channel(
        &mut self,
        ch: usize,
        o: Options,
        owner: Option<Owner>,
        voice: Option<u64>,
        audition: bool,
        retain_pitch: bool,
    ) {
        if self.protected(ch, voice, audition) {
            return;
        }
        unsafe { native_extra_rf5c164_write(self.ptr, 7, 0xc0 | ch as u8) }
        let pairs = [
            (0, o.volume),
            (1, o.pan.map(|p| p.left | (p.right << 4))),
            (2, o.step.map(|v| v as u8)),
            (3, o.step.map(|v| (v >> 8) as u8)),
            (4, o.loop_start.map(|v| v as u8)),
            (5, o.loop_start.map(|v| (v >> 8) as u8)),
            (6, o.start.map(|v| (v >> 8) as u8)),
        ];
        for (reg, val) in pairs {
            if let Some(val) = val {
                unsafe { native_extra_rf5c164_write(self.ptr, reg, val) }
            }
        }
        if retain_pitch {
            if let Some(step) = o.step {
                self.configured[ch] = step
            }
        }
        if self.owners[ch].is_some_and(|l| l.audition) {
            self.guard_dirty[ch] = true
        }
        let _ = owner;
    }
    fn key(
        &mut self,
        ch: usize,
        on: bool,
        owner: Option<Owner>,
        voice: Option<u64>,
        audition: bool,
    ) {
        if on {
            if self.protected(ch, voice, audition) {
                return;
            }
            let s = self.state();
            unsafe {
                native_extra_rf5c164_write(self.ptr, 7, 0xc0 | ch as u8);
                native_extra_rf5c164_write(self.ptr, 8, s.mask | (1 << ch));
                native_extra_rf5c164_write(self.ptr, 8, s.mask & !(1 << ch))
            }
            self.owners[ch] = Some(self.lease(owner, voice, audition));
            if audition {
                self.guard_dirty[ch] = true
            }
        } else if self.matching(ch, owner, voice, audition)
            || self.owners[ch].is_none() && !self.protected(ch, voice, audition)
        {
            self.release(ch, false)
        }
    }
    fn op(
        &mut self,
        op: Op,
        owner: Option<Owner>,
        voice: Option<u64>,
        audition: bool,
        retain_pitch: bool,
    ) {
        match op {
            Op::Write { register, value } => self.raw(register, value, owner, voice, audition),
            Op::SetChannel { channel, options } => self.set_channel(
                channel as usize,
                options,
                owner,
                voice,
                audition,
                retain_pitch,
            ),
            Op::KeyOn { channel, voice: v } => {
                self.key(channel as usize, true, owner, v.or(voice), audition)
            }
            Op::KeyOff { channel, voice: v } => {
                self.key(channel as usize, false, owner, v.or(voice), audition)
            }
        }
    }
    fn memory_allowed(&mut self, offset: usize, size: usize, audition: bool) -> bool {
        if audition || size == 0 {
            return true;
        }
        for ch in 0..8 {
            if self.owners[ch].is_some_and(|l| l.audition) {
                if self.guard_dirty[ch] {
                    self.update_guard(ch)
                }
                if self.guards[ch].intersects(offset, size) {
                    return false;
                }
            }
        }
        true
    }
    fn load(&mut self, offset: usize, data: &[u8], audition: bool, s: &Shared) -> bool {
        if !self.memory_allowed(offset, data.len(), audition) {
            return false;
        }
        unsafe {
            native_extra_rf5c164_load(self.ptr, data.as_ptr(), offset as u32, data.len() as u32);
        }
        for (i, value) in data.iter().enumerate() {
            let addr = offset + i;
            let mask = 1u64 << (addr % 64);
            if *value == 255 {
                self.markers[addr / 64] |= mask
            } else {
                self.markers[addr / 64] &= !mask
            }
        }
        s.revision.fetch_add(1, Ordering::AcqRel);
        if !data.is_empty() {
            let end = offset + data.len();
            for word in offset / 64..=(end - 1) / 64 {
                let lo = offset.saturating_sub(word * 64).min(64);
                let hi = (end - word * 64).min(64);
                let mask = (u64::MAX << lo) & if hi == 64 { u64::MAX } else { (1u64 << hi) - 1 };
                s.known[word].fetch_or(mask, Ordering::Relaxed);
            }
        }
        s.memory_revision.fetch_add(1, Ordering::Relaxed);
        s.revision.fetch_add(1, Ordering::Release);
        if audition {
            for ch in 0..8 {
                if self.owners[ch].is_some_and(|l| l.audition) {
                    self.guard_dirty[ch] = true
                }
            }
        }
        true
    }
    pub fn apply(&mut self, c: &Arc<Prepared>, owner: Option<Owner>, s: &Shared) {
        self.current_group = c.group;
        match &c.method {
            Method::Write { register, value } => {
                self.raw(*register, *value, owner, None, c.audition)
            }
            Method::SetChannel { channel, options } => {
                self.set_channel(*channel as usize, *options, owner, None, c.audition, true)
            }
            Method::KeyOn { channel, voice } => {
                self.key(*channel as usize, true, owner, *voice, c.audition)
            }
            Method::KeyOff { channel, voice } => {
                self.key(*channel as usize, false, owner, *voice, c.audition)
            }
            Method::LoadMemory { offset, data } => {
                if !self.load(*offset as usize, data, c.audition, s) {
                    c.memory_rejected.store(true, Ordering::Release)
                }
            }
            Method::WriteMemory { offset, value } => {
                if !self.load(
                    (self.state().bank | *offset) as usize,
                    &[*value],
                    c.audition,
                    s,
                ) {
                    c.memory_rejected.store(true, Ordering::Release)
                }
            }
            Method::ClearMemory => {
                if c.audition || !self.manual() {
                    unsafe { native_extra_rf5c164_clear_memory(self.ptr) }
                    self.markers.fill(0);
                    s.revision.fetch_add(1, Ordering::AcqRel);
                    for word in &s.known {
                        word.store(u64::MAX, Ordering::Relaxed)
                    }
                    s.memory_revision.fetch_add(1, Ordering::Relaxed);
                    s.revision.fetch_add(1, Ordering::Release);
                } else {
                    c.memory_rejected.store(true, Ordering::Release)
                }
            }
            Method::Clock { hz } => {
                if !c.audition && self.manual() {
                    c.clock_rejected.store(true, Ordering::Release)
                } else {
                    unsafe {
                        native_extra_rf5c164_set_clock(self.ptr, *hz);
                    }
                }
            }
            Method::Mix { volume, pan } => {
                self.targets = [
                    *volume * if *pan > 0. { 1. - *pan } else { 1. },
                    *volume * if *pan < 0. { 1. + *pan } else { 1. },
                ];
                self.ramp = (self.rate / 200).max(1)
            }
            Method::Reset => {
                if c.audition || !self.manual() {
                    unsafe { native_extra_rf5c164_reset(self.ptr) }
                    self.owners.fill(None);
                    self.configured.fill(0)
                }
            }
            Method::CancelMainOwner { target_group } => {
                for ch in 0..8 {
                    if self.owners[ch]
                        .is_some_and(|l| !l.audition && l.group == Some(*target_group))
                    {
                        self.release(ch, true)
                    }
                }
            }
            Method::StopVoices | Method::AllNotesOff => {
                for ch in 0..8 {
                    if self.owners[ch].is_some_and(|l| {
                        (!l.audition || c.audition)
                            && (owner.is_none() || l.owner == owner)
                            && (c.group.is_none() || l.group == c.group)
                    }) {
                        self.release(ch, matches!(c.method, Method::StopVoices))
                    }
                }
                if owner.is_none()
                    && c.group.is_none()
                    && !self.manual()
                    && matches!(c.method, Method::StopVoices)
                {
                    unsafe { native_extra_rf5c164_stop(self.ptr) }
                }
            }
            Method::Batch {
                commands,
                channel,
                voice,
                action,
            } => {
                let ch = *channel as usize;
                let mut selected = self.state().selected;
                let target_safe = commands.iter().all(|op| match op {
                    Op::Write { register: 7, value } => {
                        if value & 64 != 0 {
                            selected = value & 7
                        }
                        true
                    }
                    Op::Write {
                        register: 0..=6, ..
                    } => selected == *channel,
                    Op::SetChannel { channel, .. } => {
                        selected = *channel;
                        true
                    }
                    Op::KeyOn { channel, .. } => {
                        selected = *channel;
                        true
                    }
                    _ => true,
                });
                let allowed = target_safe
                    && match action {
                        LeaseAction::On => c.audition || !self.protected(ch, *voice, false),
                        LeaseAction::Off => self.matching(ch, owner, *voice, c.audition),
                        LeaseAction::Edit => !self.protected(ch, *voice, c.audition),
                    };
                if allowed {
                    if *action == LeaseAction::On {
                        self.owners[ch] = Some(self.lease(owner, *voice, c.audition))
                    }
                    for op in commands {
                        self.op(*op, owner, *voice, c.audition, *action == LeaseAction::Edit)
                    }
                    if *action == LeaseAction::On {
                        self.owners[ch] = Some(self.lease(owner, *voice, c.audition));
                        if c.audition {
                            self.guard_dirty[ch] = true
                        }
                    } else if *action == LeaseAction::Off {
                        self.owners[ch] = None
                    }
                }
            }
        };
        self.current_group = None;
        s.publish(self, Some(c));
    }
    pub fn publish(&self, s: &Shared) {
        s.publish(self, None)
    }
    pub fn cancel_invalid(&mut self, valid: impl Fn(Option<Owner>) -> bool, s: &Shared) {
        let mut changed = false;
        for ch in 0..8 {
            if self.owners[ch].is_some_and(|l| !valid(l.owner)) {
                self.release(ch, true);
                changed = true
            }
        }
        if changed {
            s.publish(self, None)
        }
    }
    pub fn mix(&mut self, pcm: &mut [f32; 2]) {
        let (mut l, mut r) = (0., 0.);
        unsafe { native_extra_rf5c164_sample(self.ptr, &mut l, &mut r) }
        if self.ramp > 0 {
            for ch in 0..2 {
                self.gains[ch] += (self.targets[ch] - self.gains[ch]) / self.ramp as f32
            }
            self.ramp -= 1
        }
        pcm[0] += l * self.gains[0];
        pcm[1] += r * self.gains[1]
    }
}
