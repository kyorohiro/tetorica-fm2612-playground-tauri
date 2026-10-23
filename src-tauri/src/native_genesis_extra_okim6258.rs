//! Independent OKIM6258. Immutable sample payloads and their budget guards are
//! retained by control; the callback only swaps Arc handles and binds pointers.
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
pub const CLOCK: u32 = 8_000_000;
pub const MAX_SAMPLE: usize = 16 * 1024 * 1024;
const CLOCK_HELD: &str =
    "Release the manually held additional OKIM6258 voices before changing the clock";
const MEMORY_HELD: &str =
    "Release the manually held additional OKIM6258 voices before replacing their sample";
#[repr(C)]
#[derive(Default)]
pub(super) struct CoreState {
    clock: u32,
    clock_buffer: u32,
    divider: u32,
    sample_bytes: u32,
    position: u32,
    fed_bytes: u32,
    pan: u8,
    playing: u8,
    streaming: u8,
    repeat: u8,
}
extern "C" {
    fn native_extra_okim6258_create(rate: u32) -> *mut c_void;
    fn native_extra_okim6258_destroy(p: *mut c_void);
    fn native_extra_okim6258_reset(p: *mut c_void);
    fn native_extra_okim6258_stop(p: *mut c_void);
    fn native_extra_okim6258_write(p: *mut c_void, reg: u8, value: u8) -> i32;
    fn native_extra_okim6258_set_clock(p: *mut c_void, hz: u32) -> i32;
    fn native_extra_okim6258_load_sample(p: *mut c_void, data: *const u8, len: u32) -> i32;
    fn native_extra_okim6258_play(p: *mut c_void, repeat: u8) -> i32;
    fn native_extra_okim6258_set_repeat(p: *mut c_void, repeat: u8);
    fn native_extra_okim6258_sample(p: *mut c_void, l: *mut f32, r: *mut f32);
    fn native_extra_okim6258_get_state(p: *mut c_void, s: *mut CoreState);
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
    Play {
        #[serde(default)]
        repeat: bool,
    },
    Stop,
    SetRepeat {
        repeat: bool,
    },
    LoadMemory {
        #[serde(rename = "sampleId")]
        sample_id: u64,
        data: Vec<u8>,
        metadata: Metadata,
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
    },
    Play {
        #[serde(default)]
        repeat: bool,
    },
    Stop,
    SetRepeat {
        repeat: bool,
    },
}
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum LeaseAction {
    On,
    Off,
    Edit,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Metadata {
    pub name: Option<String>,
    pub frames: usize,
    pub sample_rate: f64,
    #[serde(default)]
    pub waveform: Vec<f32>,
    pub source: String,
}
impl Metadata {
    fn bytes(&self) -> usize {
        self.name.as_ref().map_or(0, |s| s.capacity())
            + self.source.capacity()
            + self.waveform.capacity() * 4
    }
    fn validate(&self, size: usize) -> Result<(), String> {
        if self.frames != size * 2 && self.frames != size * 2 - 1
            || !self.sample_rate.is_finite()
            || self.sample_rate <= 0.
            || self.sample_rate > 1_000_000.
            || self.waveform.len() > 512
            || self
                .waveform
                .iter()
                .any(|v| !v.is_finite() || !(-1.0..=1.0).contains(v))
            || !["raw", "sample"].contains(&self.source.as_str())
            || self.name.as_ref().is_some_and(|s| s.len() > 4096)
        {
            return Err("Invalid Native additional OKIM6258 sample metadata".into());
        }
        Ok(())
    }
}
#[derive(Debug)]
pub struct Prepared {
    pub id: Option<u64>,
    audition: bool,
    group: Option<u64>,
    method: Method,
    complete: AtomicBool,
    retired: AtomicBool,
    cancelled: AtomicBool,
    clock_rejected: AtomicBool,
    memory_rejected: AtomicBool,
}
pub fn validate_clock(hz: u32, rate: u32) -> Result<(), String> {
    if !(100_000..=100_000_000).contains(&hz) || hz as u64 > 14 * 512 * rate as u64 {
        Err("Native OKIM6258 clock exceeds the output rate FIFO limit".into())
    } else {
        Ok(())
    }
}
fn valid_id(v: Option<u64>) -> bool {
    v.is_none_or(|v| v > 0 && v <= MAX_SAFE_INTEGER)
}
fn validate_op(v: &Op) -> Result<(), String> {
    if matches!(v,Op::Write{register,..} if *register>12) {
        Err("Native OKIM6258 register must be0–12".into())
    } else {
        Ok(())
    }
}
impl Message {
    pub fn prepared_bytes(&self) -> usize {
        std::mem::size_of::<Prepared>()
            + match &self.method {
                Method::Batch { commands, .. } => commands.len() * std::mem::size_of::<Op>(),
                Method::LoadMemory { data, metadata, .. } => {
                    data.len()
                        + metadata.name.as_ref().map_or(0, |s| s.len())
                        + metadata.source.len()
                        + metadata.waveform.len() * 4
                }
                _ => 0,
            }
    }
    pub fn prepare(&self) -> Result<Arc<Prepared>, String> {
        if !valid_id(self.id) || !valid_id(self.group) {
            return Err("Invalid Native additional OKIM6258 command ID".into());
        }
        match &self.method {
            Method::Write { register, value } => validate_op(&Op::Write {
                register: *register,
                value: *value,
            })?,
            Method::LoadMemory {
                sample_id,
                data,
                metadata,
            } => {
                if !valid_id(Some(*sample_id)) || data.is_empty() || data.len() > MAX_SAMPLE {
                    return Err("Native OKIM6258 sample must contain1–16MiB".into());
                }
                metadata.validate(data.len())?;
            }
            Method::Clock { hz } if !(100_000..=100_000_000).contains(hz) => {
                return Err("Native OKIM6258 clock must be100k–100MHz".into())
            }
            Method::Mix { volume, pan }
                if !volume.is_finite()
                    || !(0.0..=4.0).contains(volume)
                    || !pan.is_finite()
                    || !(-1.0..=1.0).contains(pan) =>
            {
                return Err("Invalid Native OKIM6258 mixer settings".into())
            }
            Method::CancelMainOwner { target_group } if !valid_id(Some(*target_group)) => {
                return Err("Invalid Native OKIM6258 Main owner".into())
            }
            Method::Batch {
                commands,
                channel,
                voice,
                action,
            } => {
                if *channel != 0
                    || commands.is_empty()
                    || commands.len() > 512
                    || !valid_id(*voice)
                    || self.audition && *action != LeaseAction::Edit && voice.is_none()
                {
                    return Err("Invalid Native OKIM6258 batch or voice lease".into());
                }
                for v in commands {
                    validate_op(v)?;
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
        matches!(self.method, Method::CancelMainOwner { .. })
    }
    pub fn starts(&self) -> bool {
        matches!(
            self.method,
            Method::Write { .. } | Method::Play { .. } | Method::LoadMemory { .. }
        ) || matches!(self.method,Method::Batch{action,..} if action!=LeaseAction::Off)
    }
    pub fn stops(&self) -> bool {
        matches!(self.method, Method::StopVoices) && self.group.is_none()
    }
    pub fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + match &self.method {
                Method::LoadMemory { data, metadata, .. } => data.capacity() + metadata.bytes(),
                Method::Batch { commands, .. } => commands.capacity() * std::mem::size_of::<Op>(),
                _ => 0,
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
    // Store outcomes without retaining cancelled PCM. The bounded ring lives
    // within the fixed instance management allowance, including empty slots.
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
        Err("Native OKIM6258 command was cancelled before it applied".into())
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
    clock: AtomicU32,
    rate: AtomicU32,
    divider: AtomicU32,
    pan: AtomicU8,
    flags: AtomicU8,
    position: AtomicU32,
    fed: AtomicU32,
    held: AtomicBool,
    sample_id: AtomicU64,
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
            clock: AtomicU32::new(CLOCK),
            rate: AtomicU32::new(0),
            divider: AtomicU32::new(512),
            pan: AtomicU8::new(0),
            flags: AtomicU8::new(0),
            position: AtomicU32::new(0),
            fed: AtomicU32::new(0),
            held: AtomicBool::new(false),
            sample_id: AtomicU64::new(0),
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
pub struct Sample {
    pub bytes: usize,
    pub frames: usize,
    pub sample_rate: f64,
    pub duration_seconds: f64,
    pub waveform: Vec<f32>,
    pub source: String,
    pub name: Option<String>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChipStatus {
    pub clock: u32,
    pub divider: u32,
    pub pan: u8,
    pub playing: bool,
    pub streaming: bool,
    pub repeat: bool,
    pub position_frames: u32,
    pub fed_bytes: u32,
    pub memory_revision: u64,
    pub sample_id: u64,
    pub sample: Option<Sample>,
    pub held: Vec<u8>,
    pub pending_commands: Vec<u64>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub sample_rate: u32,
    pub frame: u64,
    pub revision: u64,
    pub okim6258: ChipStatus,
}
impl Shared {
    pub fn prepare_message(&self, m: &Message) -> Result<Arc<Prepared>, String> {
        if let Method::Clock { hz } = m.method {
            validate_clock(hz, self.rate.load(Ordering::Acquire))?;
        }
        m.prepare()
    }
    pub fn clock(&self) -> u32 {
        self.clock.load(Ordering::Acquire)
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
        let sample = self.sample_id.load(Ordering::Acquire);
        let Control {
            prepared,
            outcomes,
            cursor,
        } = c;
        prepared.retain(|v| {
            let loaded =
                matches!(v.method, Method::LoadMemory { sample_id, .. } if sample_id == sample);
            if loaded || !v.retired.load(Ordering::Acquire) {
                return true;
            }
            if let Some(id) = v.id {
                outcomes[*cursor] = Completion {
                    id,
                    flags: v.outcome(),
                };
                *cursor = (*cursor + 1) % outcomes.len();
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
            .map_err(|_| "Native OKIM6258 control unavailable")?;
        self.collect(&mut c);
        if self.held.load(Ordering::Acquire)
            && commands.iter().any(|(v, _)| {
                !v.audition && matches!(v.method, Method::Clock { .. } | Method::LoadMemory { .. })
            })
        {
            return Err(if commands
                .iter()
                .any(|(v, _)| matches!(v.method, Method::Clock { .. }))
            {
                CLOCK_HELD
            } else {
                MEMORY_HELD
            }
            .into());
        }
        if c.prepared.len() + commands.len() > CAPACITY {
            return Err("Native OKIM6258 pending command limit reached".into());
        }
        let mut ids = HashSet::new();
        let mut sample_ids = HashSet::new();
        for v in c.prepared.iter().chain(commands.iter().map(|(v, _)| v)) {
            if v.id.is_some_and(|id| !ids.insert(id)) {
                return Err("Duplicate pending Native OKIM6258 command ID".into());
            }
            if let Method::LoadMemory { sample_id, .. } = v.method {
                if !sample_ids.insert(sample_id) {
                    return Err("Duplicate Native OKIM6258 sample ID".into());
                }
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
    pub fn commit_cancel(&self, command: &Prepared) -> bool {
        let Method::CancelMainOwner { target_group } = command.method else {
            return false;
        };
        let c = self
            .control
            .lock()
            .expect("OKI control checked by transactional retain");
        for v in &c.prepared {
            if v.group == Some(target_group) && !v.audition && !v.complete.load(Ordering::Acquire) {
                v.cancelled.store(true, Ordering::Release);
            }
        }
        true
    }
    pub fn wait_applied(&self, id: u64) -> Result<(), String> {
        if id == 0 || id > self.highest_id.load(Ordering::Acquire) {
            return Err("Unknown Native OKIM6258 command ID".into());
        }
        let cmd = {
            let c = self
                .control
                .lock()
                .map_err(|_| "Native OKIM6258 control unavailable")?;
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
                return Err("Native OKIM6258 command outcome has expired".into());
            }
        };
        let start = std::time::Instant::now();
        while !cmd.complete.load(Ordering::Acquire) {
            if start.elapsed() > std::time::Duration::from_millis(5000) {
                return Err("Native OKIM6258 command was not applied within5000ms".into());
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
            .unwrap_or_else(|_| unreachable!("Bounded OKIM6258 retirement"))
    }
    fn publish(&self, r: &Renderer, c: Option<&Prepared>) {
        let s = r.state();
        self.revision.fetch_add(1, Ordering::AcqRel);
        self.sample_id.store(
            r.sample
                .as_ref()
                .and_then(|v| {
                    if let Method::LoadMemory { sample_id, .. } = v.method {
                        Some(sample_id)
                    } else {
                        None
                    }
                })
                .unwrap_or(0),
            Ordering::Relaxed,
        );
        self.memory_revision
            .store(r.memory_revision, Ordering::Relaxed);
        self.clock.store(s.clock, Ordering::Relaxed);
        self.rate.store(r.rate, Ordering::Relaxed);
        self.divider.store(s.divider, Ordering::Relaxed);
        self.pan.store(s.pan, Ordering::Relaxed);
        self.flags.store(
            s.playing | s.streaming << 1 | s.repeat << 2,
            Ordering::Relaxed,
        );
        self.position.store(s.position, Ordering::Relaxed);
        self.fed.store(s.fed_bytes, Ordering::Relaxed);
        self.held
            .store(r.lease.is_some_and(|l| l.audition), Ordering::Relaxed);
        if let Some(c) = c {
            c.complete.store(true, Ordering::Release)
        }
        self.revision.fetch_add(1, Ordering::Release);
    }
    pub fn status(&self, frame: u64) -> Result<Status, String> {
        let mut c = self
            .control
            .lock()
            .map_err(|_| "Native OKIM6258 control unavailable")?;
        self.collect(&mut c);
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() > std::time::Duration::from_millis(50) {
                return Err("Native OKIM6258 snapshot changing;retry".into());
            }
            let before = self.revision.load(Ordering::Acquire);
            if before & 1 != 0 {
                std::thread::yield_now();
                continue;
            }
            let sample_id = self.sample_id.load(Ordering::Relaxed);
            let sample = c.prepared.iter().find_map(|v| {
                if let Method::LoadMemory {
                    sample_id: id,
                    data,
                    metadata: m,
                } = &v.method
                {
                    (*id == sample_id).then(|| Sample {
                        bytes: data.len(),
                        frames: m.frames,
                        sample_rate: m.sample_rate,
                        duration_seconds: m.frames as f64 / m.sample_rate,
                        waveform: m.waveform.clone(),
                        source: m.source.clone(),
                        name: m.name.clone(),
                    })
                } else {
                    None
                }
            });
            let flags = self.flags.load(Ordering::Relaxed);
            let out = Status {
                sample_rate: self.rate.load(Ordering::Relaxed),
                frame,
                revision: before / 2,
                okim6258: ChipStatus {
                    clock: self.clock(),
                    divider: self.divider.load(Ordering::Relaxed),
                    pan: self.pan.load(Ordering::Relaxed),
                    playing: flags & 1 != 0,
                    streaming: flags & 2 != 0,
                    repeat: flags & 4 != 0,
                    position_frames: self.position.load(Ordering::Relaxed),
                    fed_bytes: self.fed.load(Ordering::Relaxed),
                    memory_revision: self.memory_revision.load(Ordering::Relaxed),
                    sample_id,
                    sample,
                    held: if self.held.load(Ordering::Relaxed) {
                        vec![0]
                    } else {
                        vec![]
                    },
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
    owner: Option<Owner>,
    group: Option<u64>,
    voice: Option<u64>,
    audition: bool,
}
pub struct Renderer {
    ptr: *mut c_void,
    rate: u32,
    lease: Option<Lease>,
    current_group: Option<u64>,
    sample: Option<Arc<Prepared>>,
    memory_revision: u64,
    gains: [f32; 2],
    targets: [f32; 2],
    ramp: u32,
}
unsafe impl Send for Renderer {}
impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe { native_extra_okim6258_destroy(self.ptr) }
    }
}
impl Renderer {
    pub fn new(rate: u32) -> Result<Self, String> {
        let ptr = unsafe { native_extra_okim6258_create(rate) };
        if ptr.is_null() {
            return Err("Could not allocate Native additional OKIM6258 core".into());
        }
        Ok(Self {
            ptr,
            rate,
            lease: None,
            current_group: None,
            sample: None,
            memory_revision: 0,
            gains: [1.; 2],
            targets: [1.; 2],
            ramp: 0,
        })
    }
    fn state(&self) -> CoreState {
        let mut s = CoreState::default();
        unsafe { native_extra_okim6258_get_state(self.ptr, &mut s) }
        s
    }
    fn manual(&self) -> bool {
        self.lease.is_some_and(|l| l.audition)
    }
    fn protected(&self, voice: Option<u64>, audition: bool) -> bool {
        self.lease
            .is_some_and(|l| l.audition && (!audition || voice.is_some() && voice != l.voice))
    }
    fn matching(&self, owner: Option<Owner>, voice: Option<u64>, audition: bool) -> bool {
        self.lease.is_some_and(|l| {
            l.owner == owner
                && (self.current_group.is_none() || l.group == self.current_group)
                && (voice.is_none() || voice == l.voice)
                && (!l.audition || audition)
        })
    }
    fn release(&mut self) {
        unsafe { native_extra_okim6258_stop(self.ptr) }
        self.lease = None
    }
    fn op(&mut self, op: Op, owner: Option<Owner>, voice: Option<u64>, audition: bool) {
        match op {
            Op::Stop => {
                if self.matching(owner, voice, audition) || self.lease.is_none() {
                    self.release()
                }
            }
            _ => {
                if self.protected(voice, audition) {
                    return;
                }
                unsafe {
                    match op {
                        Op::Write { register, value } => {
                            if register == 0
                                && (value & 1 != 0 || value & 2 == 0)
                                && self.lease.is_some()
                                && !self.matching(owner, voice, audition)
                            {
                                return;
                            }
                            native_extra_okim6258_write(self.ptr, register, value);
                            if register == 0 {
                                if value & 1 != 0 || value & 2 == 0 {
                                    self.lease = None
                                } else if value & 2 != 0 {
                                    self.lease = Some(Lease {
                                        owner,
                                        group: self.current_group,
                                        voice,
                                        audition,
                                    })
                                }
                            }
                        }
                        Op::Play { repeat } => {
                            if native_extra_okim6258_play(self.ptr, repeat as u8) != 0 {
                                self.lease = Some(Lease {
                                    owner,
                                    group: self.current_group,
                                    voice,
                                    audition,
                                })
                            }
                        }
                        Op::SetRepeat { repeat } => {
                            native_extra_okim6258_set_repeat(self.ptr, repeat as u8)
                        }
                        Op::Stop => unreachable!(),
                    }
                }
            }
        }
    }
    pub fn apply(&mut self, c: &Arc<Prepared>, owner: Option<Owner>, s: &Shared) {
        self.current_group = c.group;
        match &c.method {
            Method::Write { register, value } => self.op(
                Op::Write {
                    register: *register,
                    value: *value,
                },
                owner,
                None,
                c.audition,
            ),
            Method::Play { repeat } => {
                self.op(Op::Play { repeat: *repeat }, owner, None, c.audition)
            }
            Method::Stop => self.op(Op::Stop, owner, None, c.audition),
            Method::SetRepeat { repeat } => {
                self.op(Op::SetRepeat { repeat: *repeat }, owner, None, c.audition)
            }
            Method::LoadMemory {
                sample_id, data, ..
            } => {
                if !c.audition && self.manual() {
                    c.memory_rejected.store(true, Ordering::Release)
                } else {
                    self.release();
                    unsafe {
                        native_extra_okim6258_load_sample(
                            self.ptr,
                            data.as_ptr(),
                            data.len() as u32,
                        );
                    }
                    self.sample = Some(c.clone());
                    let _ = sample_id;
                    self.memory_revision += 1;
                }
            }
            Method::Clock { hz } => {
                if !c.audition && self.manual() {
                    c.clock_rejected.store(true, Ordering::Release)
                } else {
                    unsafe {
                        native_extra_okim6258_set_clock(self.ptr, *hz);
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
                    unsafe { native_extra_okim6258_reset(self.ptr) }
                    self.lease = None
                }
            }
            Method::CancelMainOwner { target_group } => {
                if self
                    .lease
                    .is_some_and(|l| !l.audition && l.group == Some(*target_group))
                {
                    self.release()
                }
            }
            Method::StopVoices | Method::AllNotesOff => {
                if self.lease.is_some_and(|l| {
                    (!l.audition || c.audition)
                        && (owner.is_none() || l.owner == owner)
                        && (c.group.is_none() || l.group == c.group)
                }) {
                    self.release()
                }
                if owner.is_none()
                    && c.group.is_none()
                    && !self.manual()
                    && matches!(c.method, Method::StopVoices)
                {
                    self.release()
                }
            }
            Method::Batch {
                commands,
                voice,
                action,
                ..
            } => {
                let allowed = match action {
                    LeaseAction::On => c.audition || !self.protected(*voice, false),
                    LeaseAction::Off => self.matching(owner, *voice, c.audition),
                    LeaseAction::Edit => !self.protected(*voice, c.audition),
                };
                if allowed {
                    if *action == LeaseAction::On {
                        self.lease = Some(Lease {
                            owner,
                            group: c.group,
                            voice: *voice,
                            audition: c.audition,
                        })
                    }
                    for v in commands {
                        self.op(*v, owner, *voice, c.audition)
                    }
                    if *action == LeaseAction::Off {
                        self.lease = None
                    }
                }
            }
        }
        self.current_group = None;
        s.publish(self, Some(c));
    }
    pub fn publish(&self, s: &Shared) {
        s.publish(self, None)
    }
    pub fn cancel_invalid(&mut self, valid: impl Fn(Option<Owner>) -> bool, s: &Shared) {
        if self.lease.is_some_and(|l| !valid(l.owner)) {
            self.release()
        }
        // A finite preview releases its lease when the physical stream ends.
        // This block-boundary check avoids another FFI call for every sample.
        if self.state().playing == 0 {
            self.lease = None;
        }
        s.publish(self, None)
    }
    pub fn mix(&mut self, pcm: &mut [f32; 2]) {
        let (mut l, mut r) = (0., 0.);
        unsafe { native_extra_okim6258_sample(self.ptr, &mut l, &mut r) }
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
