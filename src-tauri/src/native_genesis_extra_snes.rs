//! Accurate callback-owned CPU-free S-DSP. Control-side registries retain all
//! variable data until callback retirement; rendering never allocates or locks.
use super::{Owner, CAPACITY, MAX_SAFE_INTEGER, OWNERS};
use crossbeam_queue::ArrayQueue;
use serde::{Deserialize, Serialize};
use std::{
    cell::UnsafeCell,
    ffi::c_void,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering},
        Arc, Mutex,
    },
};
pub const CLOCK: u32 = 1_024_000;
pub const RAM: usize = 65_536;
const MAX_EVENTS: usize = 1_000_000;
const MAX_PENDING: usize = 32 * 1024 * 1024;
pub const INSTANCE_BYTES: usize = super::extra_budget::INSTANCE_BYTES + 1024 * 1024 + 128 * 1024;
pub const RAM_HELD: &str =
    "Release the manually held additional SNES voices before replacing sample memory";
pub const STATE_HELD: &str =
    "Release the manually held additional SNES voices before replacing the DSP state";
const TIMELINE_EXCLUSIVE: &str =
    "SNES timeline mode is exclusive; reset() before editing instruments";
#[repr(C)]
struct CoreState {
    registers: [u8; 128],
    native_frames: u64,
    timeline_loaded: u32,
    timeline_playing: u32,
    timeline_position: u32,
    timeline_total: u32,
}
impl Default for CoreState {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}
extern "C" {
    fn native_snes_create(rate: u32) -> *mut c_void;
    fn native_snes_destroy(ptr: *mut c_void);
    fn native_snes_reset(ptr: *mut c_void);
    fn native_snes_stop(ptr: *mut c_void);
    fn native_snes_write_protected(ptr: *mut c_void, register: u8, value: u8, mask: u8);
    fn native_snes_key_on(ptr: *mut c_void, channel: u8);
    fn native_snes_key_off(ptr: *mut c_void, channel: u8);
    fn native_snes_load(ptr: *mut c_void, data: *const u8, address: u32, size: u32) -> i32;
    fn native_snes_sample(ptr: *mut c_void, left: *mut f32, right: *mut f32);
    fn native_snes_get_state(ptr: *mut c_void, out: *mut CoreState);
    fn native_snes_ram_data(ptr: *const c_void) -> *const u8;
    fn native_snes_prepare_timeline(
        ptr: *mut c_void,
        ram: *const u8,
        dsp: *const u8,
        events: *const u32,
        event_count: u32,
        duration: u32,
    );
    fn native_snes_play_timeline(ptr: *mut c_void);
    fn native_snes_stop_timeline(ptr: *mut c_void);
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
    KeyOn {
        channel: u8,
    },
    KeyOff {
        channel: u8,
    },
    LoadMemory {
        address: u32,
        data: Vec<u8>,
    },
    Reset,
    ResetState {
        dsp: Vec<u8>,
        uploads: Vec<Upload>,
    },
    LoadMemoryBatch {
        uploads: Vec<Upload>,
    },
    Barrier {
        #[serde(default)]
        ram: bool,
    },
    CancelMainOwner {
        #[serde(rename = "targetGroup")]
        target_group: u64,
    },
    StopVoices,
    AllNotesOff,
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
    PrepareTimeline {
        ram: Vec<u8>,
        dsp: Vec<u8>,
        events: Vec<u32>,
        #[serde(rename = "durationClocks")]
        duration_clocks: u32,
    },
    PlayTimeline,
    StopTimeline,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Upload {
    pub address: u32,
    pub data: Vec<u8>,
}
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(tag = "method", rename_all = "kebab-case")]
pub enum Op {
    Write { register: u8, value: u8 },
    KeyOn { channel: u8 },
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
struct Timeline {
    ram: Vec<u8>,
    dsp: Vec<u8>,
    events: Vec<u32>,
    duration: u32,
}
#[derive(Debug)]
enum Edit {
    Op(Op),
    Load(u32, Vec<u8>),
    Reset,
    ResetState(Vec<u8>, Vec<Upload>),
    Uploads(Vec<Upload>),
    Barrier,
    CancelMainOwner(u64),
    Stop,
    Mix(f32, f32),
    Batch(Vec<Op>, u8, Option<u64>, LeaseAction),
    PrepareTimeline(Arc<Timeline>),
    PlayTimeline,
    StopTimeline,
}
#[derive(Debug)]
pub struct Prepared {
    pub id: Option<u64>,
    audition: bool,
    edit: Edit,
    complete: AtomicBool,
    retired: AtomicBool,
    cancelled: AtomicBool,
    rejected: AtomicU8,
    group: Option<u64>,
    capture: Option<Arc<Capture>>,
}
fn valid_id(value: Option<u64>) -> bool {
    value.is_none_or(|v| v > 0 && v <= MAX_SAFE_INTEGER)
}
fn validate_op(op: Op) -> Result<(), String> {
    match op {
        Op::Write { register, .. } if register > 127 => {
            Err("Native SNES DSP register must be 0–127".into())
        }
        Op::KeyOn { channel } | Op::KeyOff { channel } if channel > 7 => {
            Err("Native SNES channel must be 0–7".into())
        }
        _ => Ok(()),
    }
}
fn local_channel(register: u8) -> Option<u8> {
    ((register & 15) <= 9).then_some(register >> 4)
}
fn validate_uploads(uploads: &[Upload]) -> Result<(), String> {
    if uploads.len() > 512
        || uploads.iter().map(|u| u.data.len()).sum::<usize>() > RAM
        || uploads
            .iter()
            .any(|u| u.address as usize > RAM || u.data.len() > RAM - u.address as usize)
    {
        return Err("Native additional SNES upload batch exceeds 64 KiB RAM".into());
    }
    Ok(())
}
impl Message {
    pub fn prepared_bytes(&self) -> usize {
        std::mem::size_of::<Prepared>()
            + match &self.method {
                Method::LoadMemory { data, .. } => data.len(),
                Method::LoadMemoryBatch { uploads } => {
                    uploads.len() * std::mem::size_of::<Upload>()
                        + uploads.iter().map(|v| v.data.len()).sum::<usize>()
                }
                Method::ResetState { dsp, uploads } => {
                    dsp.len()
                        + uploads.len() * std::mem::size_of::<Upload>()
                        + uploads.iter().map(|v| v.data.len()).sum::<usize>()
                }
                Method::Batch { commands, .. } => commands.len() * std::mem::size_of::<Op>(),
                Method::PrepareTimeline {
                    ram, dsp, events, ..
                } => {
                    std::mem::size_of::<Timeline>() + 16 + ram.len() + dsp.len() + events.len() * 4
                }
                Method::Barrier { ram: true } => std::mem::size_of::<Capture>() + 16 + RAM,
                _ => 0,
            }
    }
    pub fn upload_bytes(&self) -> usize {
        match &self.method {
            Method::LoadMemory { data, .. } => data.len(),
            Method::LoadMemoryBatch { uploads } | Method::ResetState { uploads, .. } => {
                uploads.iter().map(|u| u.data.len()).sum()
            }
            _ => 0,
        }
    }
    pub fn prepare(&self) -> Result<Arc<Prepared>, String> {
        if !valid_id(self.id) || !valid_id(self.group) {
            return Err("Invalid Native SNES command ID".into());
        }
        let edit = match &self.method {
            Method::Write { register, value } => {
                let op = Op::Write {
                    register: *register,
                    value: *value,
                };
                validate_op(op)?;
                Edit::Op(op)
            }
            Method::KeyOn { channel } => {
                let op = Op::KeyOn { channel: *channel };
                validate_op(op)?;
                Edit::Op(op)
            }
            Method::KeyOff { channel } => {
                let op = Op::KeyOff { channel: *channel };
                validate_op(op)?;
                Edit::Op(op)
            }
            Method::LoadMemory { address, data } => {
                if *address as usize > RAM || data.len() > RAM - *address as usize {
                    return Err("Native SNES upload is outside 64 KiB RAM".into());
                }
                Edit::Load(*address, data.clone())
            }
            Method::Reset => Edit::Reset,
            Method::ResetState { dsp, uploads } => {
                if dsp.len() != 128 {
                    return Err("Native additional SNES reset needs 128 DSP registers".into());
                }
                validate_uploads(uploads)?;
                Edit::ResetState(dsp.clone(), uploads.clone())
            }
            Method::LoadMemoryBatch { uploads } => {
                validate_uploads(uploads)?;
                Edit::Uploads(uploads.clone())
            }
            Method::Barrier { ram } => {
                if *ram && self.id.is_none() {
                    return Err("Native additional SNES RAM capture requires a command ID".into());
                }
                Edit::Barrier
            }
            Method::CancelMainOwner { target_group } => {
                if !valid_id(Some(*target_group)) {
                    return Err("Invalid Native additional SNES Main owner group".into());
                }
                Edit::CancelMainOwner(*target_group)
            }
            Method::StopVoices | Method::AllNotesOff => Edit::Stop,
            Method::Mix { volume, pan } => {
                if !volume.is_finite()
                    || !(0.0..=4.0).contains(volume)
                    || !pan.is_finite()
                    || !(-1.0..=1.0).contains(pan)
                {
                    return Err("Invalid Native SNES mixer gain/pan".into());
                }
                Edit::Mix(*volume, *pan)
            }
            Method::Batch {
                commands,
                channel,
                voice,
                action,
            } => {
                if *channel > 7
                    || commands.len() > 512
                    || !valid_id(*voice)
                    || (self.audition && *action != LeaseAction::Edit && voice.is_none())
                {
                    return Err("Invalid Native SNES channel batch or voice lease".into());
                }
                for op in commands {
                    validate_op(*op)?;
                    match *op {
                        Op::KeyOn { channel: target } | Op::KeyOff { channel: target }
                            if target != *channel =>
                        {
                            return Err("Native SNES batch must target its declared channel".into())
                        }
                        Op::Write { register, .. }
                            if local_channel(register).is_some_and(|target| target != *channel)
                                || local_channel(register).is_none()
                                    && !matches!(register, 0x4c | 0x5c | 0x2d | 0x3d | 0x4d) =>
                        {
                            return Err(
                                "Native SNES batch contains unrelated global/register edits".into(),
                            )
                        }
                        _ => {}
                    }
                }
                Edit::Batch(commands.clone(), *channel, *voice, *action)
            }
            Method::PrepareTimeline {
                ram,
                dsp,
                events,
                duration_clocks,
            } => {
                if ram.len() != RAM
                    || dsp.len() != 128
                    || events.len() % 4 != 0
                    || events.len() / 4 > MAX_EVENTS
                    || *duration_clocks < 32
                    || *duration_clocks > 600 * CLOCK
                    || duration_clocks % 32 != 0
                {
                    return Err("Invalid Native SNES timeline bounds".into());
                }
                let mut previous = 0;
                for e in events.chunks_exact(4) {
                    if e[0] < previous
                        || e[0] > *duration_clocks
                        || e[1] > 1
                        || e[2] > if e[1] == 0 { 127 } else { 65535 }
                        || e[3] > 255
                    {
                        return Err("Invalid Native SNES timeline event ordering or value".into());
                    }
                    previous = e[0];
                }
                Edit::PrepareTimeline(Arc::new(Timeline {
                    ram: ram.clone(),
                    dsp: dsp.clone(),
                    events: events.clone(),
                    duration: *duration_clocks,
                }))
            }
            Method::PlayTimeline => Edit::PlayTimeline,
            Method::StopTimeline => Edit::StopTimeline,
        };
        Ok(Arc::new(Prepared {
            id: self.id,
            audition: self.audition,
            edit,
            complete: AtomicBool::new(false),
            retired: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
            rejected: AtomicU8::new(0),
            group: self.group,
            capture: matches!(self.method, Method::Barrier { ram: true })
                .then(|| Arc::new(Capture::new())),
        }))
    }
}
impl Prepared {
    pub fn starts(&self) -> bool {
        fn starts(op: &Op) -> bool {
            matches!(op, Op::KeyOn { .. })
                || matches!(op,Op::Write{register:0x4c,value}if *value!=0)
        }
        match &self.edit {
            Edit::Op(op) => starts(op),
            Edit::Batch(ops, _, _, action) => *action == LeaseAction::On || ops.iter().any(starts),
            Edit::PlayTimeline => true,
            _ => false,
        }
    }
    pub fn stops(&self) -> bool {
        matches!(self.edit, Edit::Stop) && self.group.is_none()
    }
    pub fn is_owner_cancel(&self) -> bool {
        matches!(self.edit, Edit::CancelMainOwner(_))
    }
    pub fn capture_bytes(&self) -> usize {
        if self.capture.is_some() {
            RAM
        } else {
            0
        }
    }
    pub fn upload_bytes(&self) -> usize {
        match &self.edit {
            Edit::Load(_, data) => data.len(),
            Edit::Uploads(uploads) | Edit::ResetState(_, uploads) => {
                uploads.iter().map(|u| u.data.len()).sum()
            }
            _ => 0,
        }
    }
    pub fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + match &self.edit {
                Edit::Load(_, data) => data.capacity(),
                Edit::Uploads(v) => {
                    v.capacity() * std::mem::size_of::<Upload>()
                        + v.iter().map(|u| u.data.capacity()).sum::<usize>()
                }
                Edit::ResetState(dsp, v) => {
                    dsp.capacity()
                        + v.capacity() * std::mem::size_of::<Upload>()
                        + v.iter().map(|u| u.data.capacity()).sum::<usize>()
                }
                Edit::Batch(v, ..) => v.capacity() * std::mem::size_of::<Op>(),
                Edit::PrepareTimeline(t) => {
                    std::mem::size_of::<Timeline>()
                        + 16
                        + t.ram.capacity()
                        + t.dsp.capacity()
                        + t.events.capacity() * 4
                }
                _ => 0,
            }
            + self
                .capture
                .as_ref()
                .map_or(0, |_| std::mem::size_of::<Capture>() + 16 + RAM)
    }
    fn result(&self) -> u8 {
        self.cancelled.load(Ordering::Acquire) as u8 | self.rejected.load(Ordering::Acquire)
    }
}
#[derive(Debug)]
struct Capture {
    ram: UnsafeCell<Box<[u8]>>,
    registers: [AtomicU8; 128],
    native_frames: AtomicU64,
    enabled: AtomicU8,
    held: AtomicU8,
    frame: AtomicU64,
    revision: AtomicU64,
    loaded: AtomicBool,
    playing: AtomicBool,
    position: AtomicU64,
    total: AtomicU64,
    generation: AtomicU64,
    play_id: AtomicU64,
    delivered: AtomicBool,
    owner_performance: AtomicU64,
    owner_id: AtomicU32,
    created: std::time::Instant,
}
// Capture RAM has one callback writer, followed by immutable control reads.
// Prepared completion Release/Acquire and control lifetime guards enforce this
// handoff. The callback never accesses a capture after retiring its command.
unsafe impl Sync for Capture {}
impl Capture {
    fn new() -> Self {
        Self {
            ram: UnsafeCell::new(vec![0u8; RAM].into_boxed_slice()),
            registers: std::array::from_fn(|_| AtomicU8::new(0)),
            native_frames: AtomicU64::new(0),
            enabled: AtomicU8::new(0),
            held: AtomicU8::new(0),
            frame: AtomicU64::new(0),
            revision: AtomicU64::new(0),
            loaded: AtomicBool::new(false),
            playing: AtomicBool::new(false),
            position: AtomicU64::new(0),
            total: AtomicU64::new(0),
            generation: AtomicU64::new(0),
            play_id: AtomicU64::new(0),
            delivered: AtomicBool::new(false),
            owner_performance: AtomicU64::new(0),
            owner_id: AtomicU32::new(0),
            created: std::time::Instant::now(),
        }
    }
    fn record(&self, r: &Renderer, revision: u64) {
        let s = r.state();
        // Exactly one callback applies a Prepared capture. Control reads these
        // bytes only after acquiring its completion Release, then they remain
        // immutable until the final control-side retirement.
        unsafe {
            std::ptr::copy_nonoverlapping(r.ram().as_ptr(), (*self.ram.get()).as_mut_ptr(), RAM);
        }
        for (o, v) in self.registers.iter().zip(s.registers) {
            o.store(v, Ordering::Relaxed)
        }
        self.native_frames.store(s.native_frames, Ordering::Relaxed);
        self.enabled.store(r.gates, Ordering::Relaxed);
        self.held
            .store(r.manual_mask(None, false), Ordering::Relaxed);
        self.frame.store(r.frame, Ordering::Relaxed);
        self.revision.store(revision, Ordering::Relaxed);
        self.loaded.store(s.timeline_loaded != 0, Ordering::Relaxed);
        self.playing
            .store(s.timeline_playing != 0, Ordering::Relaxed);
        self.position
            .store(s.timeline_position as u64, Ordering::Relaxed);
        self.total.store(s.timeline_total as u64, Ordering::Relaxed);
        self.generation
            .store(r.timeline_generation, Ordering::Relaxed);
        self.play_id.store(r.play_command_id, Ordering::Relaxed);
    }
    fn status(&self) -> ChipStatus {
        let loaded = self.loaded.load(Ordering::Relaxed);
        let position = self.position.load(Ordering::Relaxed);
        let total = self.total.load(Ordering::Relaxed);
        ChipStatus {
            clock: CLOCK,
            registers: self
                .registers
                .iter()
                .map(|v| v.load(Ordering::Relaxed))
                .collect(),
            ram: Some(unsafe { (&*self.ram.get()).to_vec() }),
            native_frames: self.native_frames.load(Ordering::Relaxed),
            enabled: self.enabled.load(Ordering::Relaxed),
            held: (0..8)
                .filter(|ch| self.held.load(Ordering::Relaxed) & (1 << ch) != 0)
                .collect(),
            frame: self.frame.load(Ordering::Relaxed),
            revision: self.revision.load(Ordering::Relaxed),
            timeline: TimelineState {
                loaded,
                playing: self.playing.load(Ordering::Relaxed),
                position_frames: position,
                total_frames: total,
                ended: loaded && position >= total,
                generation: self.generation.load(Ordering::Relaxed),
                play_command_id: self.play_id.load(Ordering::Relaxed),
            },
            pending_commands: Vec::new(),
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
    outcomes: Box<[Completion]>,
    cursor: usize,
}
impl Default for Control {
    fn default() -> Self {
        Self {
            prepared: Vec::new(),
            outcomes: vec![Completion::default(); 512].into_boxed_slice(),
            cursor: 0,
        }
    }
}
fn outcome(flags: u8) -> Result<(), String> {
    if flags & 1 != 0 {
        Err("Native additional SNES command was cancelled before it applied".into())
    } else if flags & 2 != 0 {
        Err(RAM_HELD.into())
    } else if flags & 4 != 0 {
        Err(STATE_HELD.into())
    } else if flags & 8 != 0 {
        Err(TIMELINE_EXCLUSIVE.into())
    } else {
        Ok(())
    }
}
pub struct Shared {
    revision: AtomicU64,
    registers: [AtomicU8; 128],
    rate: AtomicU32,
    native_frames: AtomicU64,
    enabled: AtomicU8,
    held: AtomicU8,
    timeline_loaded: AtomicBool,
    timeline_playing: AtomicBool,
    timeline_position: AtomicU64,
    timeline_total: AtomicU64,
    timeline_generation: AtomicU64,
    timeline_play_id: AtomicU64,
    control: Mutex<Control>,
    retired: ArrayQueue<Arc<Prepared>>,
    retired_timelines: ArrayQueue<Arc<Timeline>>,
    highest_id: AtomicU64,
    cancel_before: AtomicU64,
    owner_cancel_before: [AtomicU64; OWNERS],
}
impl Default for Shared {
    fn default() -> Self {
        Self {
            revision: AtomicU64::new(0),
            registers: std::array::from_fn(|_| AtomicU8::new(0)),
            rate: AtomicU32::new(48000),
            native_frames: AtomicU64::new(0),
            enabled: AtomicU8::new(0),
            held: AtomicU8::new(0),
            timeline_loaded: AtomicBool::new(false),
            timeline_playing: AtomicBool::new(false),
            timeline_position: AtomicU64::new(0),
            timeline_total: AtomicU64::new(0),
            timeline_generation: AtomicU64::new(0),
            timeline_play_id: AtomicU64::new(0),
            control: Mutex::new(Control::default()),
            retired: ArrayQueue::new(CAPACITY),
            retired_timelines: ArrayQueue::new(CAPACITY),
            highest_id: AtomicU64::new(0),
            cancel_before: AtomicU64::new(0),
            owner_cancel_before: std::array::from_fn(|_| AtomicU64::new(0)),
        }
    }
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineState {
    pub loaded: bool,
    pub playing: bool,
    pub position_frames: u64,
    pub total_frames: u64,
    pub ended: bool,
    pub generation: u64,
    pub play_command_id: u64,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChipStatus {
    pub clock: u32,
    pub registers: Vec<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ram: Option<Vec<u8>>,
    pub native_frames: u64,
    pub enabled: u8,
    pub held: Vec<u8>,
    pub frame: u64,
    pub revision: u64,
    pub timeline: TimelineState,
    pub pending_commands: Vec<u64>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub snes: ChipStatus,
    pub sample_rate: u32,
    pub frame: u64,
    pub revision: u64,
}
impl Shared {
    pub fn prepare_message(&self, message: &Message) -> Result<Arc<Prepared>, String> {
        message.prepare()
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
        while self.retired_timelines.pop().is_some() {}
        let Control {
            prepared,
            outcomes,
            cursor,
        } = c;
        prepared.retain(|v| {
            if v.retired.load(Ordering::Acquire) {
                if let Some(capture) = &v.capture {
                    if capture.created.elapsed() >= std::time::Duration::from_secs(30) {
                        capture.delivered.store(true, Ordering::Release);
                    }
                }
            }
            if !v.retired.load(Ordering::Acquire)
                || v.capture
                    .as_ref()
                    .is_some_and(|s| !s.delivered.load(Ordering::Acquire))
                || matches!(&v.edit,Edit::PrepareTimeline(t)if Arc::strong_count(t)>1)
            {
                return true;
            }
            if let Some(id) = v.id {
                outcomes[*cursor] = Completion {
                    id,
                    flags: v.result(),
                };
                *cursor = (*cursor + 1) % outcomes.len();
            }
            false
        });
        if prepared.capacity() > 64.max(prepared.len() * 2) {
            prepared.shrink_to_fit();
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
            .map_err(|_| "Native additional SNES control unavailable")?;
        self.collect(&mut c);
        if self.held.load(Ordering::Acquire) != 0 {
            for (v, _) in commands {
                match v.edit {
                    Edit::Load(..) | Edit::Uploads(..) => return Err(RAM_HELD.into()),
                    Edit::Reset
                    | Edit::ResetState(..)
                    | Edit::PrepareTimeline(..)
                    | Edit::PlayTimeline => return Err(STATE_HELD.into()),
                    _ => {}
                }
            }
        }
        if self.timeline_loaded.load(Ordering::Acquire)
            && !commands
                .iter()
                .any(|(v, _)| matches!(v.edit, Edit::Reset | Edit::ResetState(..)))
            && commands.iter().any(|(v, _)| {
                matches!(
                    v.edit,
                    Edit::Load(..) | Edit::Uploads(..) | Edit::Op(..) | Edit::Batch(..)
                )
            })
        {
            return Err(TIMELINE_EXCLUSIVE.into());
        }
        if c.prepared.len() + commands.len() > CAPACITY
            || c.prepared.iter().map(|v| v.retained_bytes()).sum::<usize>()
                + commands
                    .iter()
                    .map(|(v, _)| v.retained_bytes())
                    .sum::<usize>()
                > MAX_PENDING
        {
            return Err("Native additional SNES pending command limit reached (32 MiB)".into());
        }
        if c.prepared
            .iter()
            .chain(commands.iter().map(|(v, _)| v))
            .filter(|v| {
                v.capture
                    .as_ref()
                    .is_some_and(|s| !s.delivered.load(Ordering::Acquire))
            })
            .count()
            > 16
        {
            return Err("Native additional SNES pending RAM snapshot limit reached (16)".into());
        }
        let mut ids = std::collections::HashSet::new();
        for v in c.prepared.iter().chain(commands.iter().map(|(v, _)| v)) {
            if v.id.is_some_and(|id| !ids.insert(id)) {
                return Err("Duplicate pending Native additional SNES command ID".into());
            }
        }
        for (v, owner) in commands {
            if let (Some(capture), Some(owner)) = (&v.capture, owner) {
                capture
                    .owner_performance
                    .store(owner.performance, Ordering::Relaxed);
                capture.owner_id.store(owner.owner, Ordering::Release);
            }
            if let Some(id) = v.id {
                self.highest_id.fetch_max(id, Ordering::Release);
            }
            c.prepared.push(v.clone());
        }
        Ok(())
    }
    pub fn commit_cancel(&self, c: &Prepared) -> bool {
        let Edit::CancelMainOwner(target) = c.edit else {
            return false;
        };
        let control = self.control.lock().expect("SNES retain validated control");
        for v in &control.prepared {
            if v.group == Some(target) {
                if let Some(capture) = &v.capture {
                    v.cancelled.store(true, Ordering::Release);
                    capture.delivered.store(true, Ordering::Release);
                }
            }
            if v.group == Some(target) && !v.audition && !v.complete.load(Ordering::Acquire) {
                v.cancelled.store(true, Ordering::Release);
            }
        }
        true
    }
    pub fn expire_captures(&self, valid: impl Fn(Option<Owner>) -> bool) {
        if let Ok(mut c) = self.control.lock() {
            for v in &c.prepared {
                if let Some(capture) = &v.capture {
                    let id = capture.owner_id.load(Ordering::Acquire);
                    let owner = (id != 0).then(|| Owner {
                        performance: capture.owner_performance.load(Ordering::Relaxed),
                        owner: id,
                    });
                    if !valid(owner) {
                        v.cancelled.store(true, Ordering::Release);
                        capture.delivered.store(true, Ordering::Release);
                    }
                }
            }
            self.collect(&mut c);
        }
    }
    pub fn wait_applied(&self, id: u64) -> Result<(), String> {
        if !valid_id(Some(id)) || id > self.highest_id.load(Ordering::Acquire) {
            return Err("Unknown Native additional SNES command ID".into());
        }
        let command = {
            let c = self
                .control
                .lock()
                .map_err(|_| "Native additional SNES control unavailable")?;
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
                return Err(
                    "Native additional SNES command result expired; read current status".into(),
                );
            }
        };
        let start = std::time::Instant::now();
        while !command.complete.load(Ordering::Acquire) {
            if start.elapsed() >= std::time::Duration::from_secs(10) {
                return Err(
                    "Native additional SNES command was not applied within 10 seconds".into(),
                );
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        outcome(command.result())
    }
    pub fn rollback(&self, v: Arc<Prepared>) {
        self.retire(v)
    }
    pub fn retire(&self, v: Arc<Prepared>) {
        if !v.complete.load(Ordering::Acquire) {
            v.cancelled.store(true, Ordering::Release);
            if let Some(capture) = &v.capture {
                capture.delivered.store(true, Ordering::Release);
            }
        }
        v.complete.store(true, Ordering::Release);
        v.retired.store(true, Ordering::Release);
        self.retired
            .push(v)
            .unwrap_or_else(|_| unreachable!("Bounded SNES retirement"));
    }
    fn retire_timeline(&self, timeline: Arc<Timeline>) {
        self.retired_timelines
            .push(timeline)
            .unwrap_or_else(|_| unreachable!("Bounded SNES timeline retirement"));
    }
    fn publish(&self, r: &Renderer, command: Option<&Prepared>, _: bool) {
        let state = r.state();
        self.revision.fetch_add(1, Ordering::AcqRel);
        for (o, v) in self.registers.iter().zip(state.registers) {
            o.store(v, Ordering::Relaxed);
        }
        self.rate.store(r.rate, Ordering::Relaxed);
        self.native_frames
            .store(state.native_frames, Ordering::Relaxed);
        self.enabled.store(r.gates, Ordering::Relaxed);
        self.held
            .store(r.manual_mask(None, false), Ordering::Relaxed);
        self.timeline_loaded
            .store(state.timeline_loaded != 0, Ordering::Relaxed);
        self.timeline_playing
            .store(state.timeline_playing != 0, Ordering::Relaxed);
        self.timeline_position
            .store(state.timeline_position as u64, Ordering::Relaxed);
        self.timeline_total
            .store(state.timeline_total as u64, Ordering::Relaxed);
        self.timeline_generation
            .store(r.timeline_generation, Ordering::Relaxed);
        self.timeline_play_id
            .store(r.play_command_id, Ordering::Relaxed);
        if let Some(c) = command {
            if let Some(capture) = &c.capture {
                capture.record(r, self.revision.load(Ordering::Relaxed) / 2 + 1);
            }
            c.complete.store(true, Ordering::Release);
        }
        self.revision.fetch_add(1, Ordering::Release);
    }
    pub fn status(&self, frame: u64) -> Result<Status, String> {
        self.status_after(frame, None, false)
    }
    pub fn status_after(
        &self,
        frame: u64,
        after: Option<u64>,
        with_ram: bool,
    ) -> Result<Status, String> {
        let mut c = self
            .control
            .lock()
            .map_err(|_| "Native additional SNES control unavailable")?;
        if with_ram {
            let prepared = c
                .prepared
                .iter()
                .find(|v| v.id == after && v.capture.is_some())
                .ok_or("Native additional SNES RAM snapshot requires an applied capture barrier")?;
            if !prepared.complete.load(Ordering::Acquire) {
                return Err("Native additional SNES RAM snapshot has not applied".into());
            }
            outcome(prepared.result())?;
            let capture = prepared.capture.as_ref().unwrap();
            let snes = capture.status();
            let status = Status {
                frame: snes.frame,
                revision: snes.revision,
                sample_rate: self.rate.load(Ordering::Acquire),
                snes,
            };
            capture.delivered.store(true, Ordering::Release);
            return Ok(status);
        }
        self.collect(&mut c);
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() >= std::time::Duration::from_millis(100) {
                return Err("Native additional SNES state changed while reading; retry".into());
            }
            let before = self.revision.load(Ordering::Acquire);
            if before & 1 != 0 {
                std::thread::yield_now();
                continue;
            }
            let position = self.timeline_position.load(Ordering::Relaxed);
            let total = self.timeline_total.load(Ordering::Relaxed);
            let loaded = self.timeline_loaded.load(Ordering::Relaxed);
            let snes = ChipStatus {
                clock: CLOCK,
                registers: self
                    .registers
                    .iter()
                    .map(|v| v.load(Ordering::Relaxed))
                    .collect(),
                ram: None,
                native_frames: self.native_frames.load(Ordering::Relaxed),
                enabled: self.enabled.load(Ordering::Relaxed),
                held: (0..8)
                    .filter(|ch| self.held.load(Ordering::Relaxed) & (1 << ch) != 0)
                    .collect(),
                frame,
                revision: before / 2,
                timeline: TimelineState {
                    loaded,
                    playing: self.timeline_playing.load(Ordering::Relaxed),
                    position_frames: position,
                    total_frames: total,
                    ended: loaded && position >= total,
                    generation: self.timeline_generation.load(Ordering::Relaxed),
                    play_command_id: self.timeline_play_id.load(Ordering::Relaxed),
                },
                pending_commands: c
                    .prepared
                    .iter()
                    .filter(|v| !v.complete.load(Ordering::Relaxed))
                    .filter_map(|v| v.id)
                    .collect(),
            };
            std::sync::atomic::fence(Ordering::Acquire);
            if self.revision.load(Ordering::Relaxed) == before {
                return Ok(Status {
                    snes,
                    sample_rate: self.rate.load(Ordering::Acquire),
                    frame,
                    revision: before / 2,
                });
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
    ptr: *mut c_void,
    owners: [Option<Lease>; 8],
    gates: u8,
    gains: [f32; 2],
    targets: [f32; 2],
    ramp: u32,
    rate: u32,
    timeline: Option<Arc<Timeline>>,
    timeline_owner: Option<Owner>,
    timeline_group: Option<u64>,
    timeline_generation: u64,
    play_command_id: u64,
    current_group: Option<u64>,
    frame: u64,
}
unsafe impl Send for Renderer {}
impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe {
            native_snes_destroy(self.ptr);
        }
    }
}
impl Renderer {
    pub fn new(rate: u32) -> Result<Self, String> {
        let ptr = unsafe { native_snes_create(rate) };
        if ptr.is_null() {
            return Err("Could not allocate Native SNES S-DSP".into());
        }
        Ok(Self {
            ptr,
            owners: [None; 8],
            gates: 0,
            gains: [1.0; 2],
            targets: [1.0; 2],
            ramp: 0,
            rate,
            timeline: None,
            timeline_owner: None,
            timeline_group: None,
            timeline_generation: 0,
            play_command_id: 0,
            current_group: None,
            frame: 0,
        })
    }
    pub fn set_frame(&mut self, frame: u64) {
        self.frame = frame;
    }
    fn state(&self) -> CoreState {
        let mut state = CoreState::default();
        unsafe {
            native_snes_get_state(self.ptr, &mut state);
        }
        state
    }
    fn ram(&self) -> &[u8] {
        // SAFETY: new() owns a non-null Core with initialized, fixed 64 KiB RAM;
        // native_snes_ram_data returns that stable allocation. The slice cannot
        // outlive &self. Renderer is Send, not Sync, and every mutating core call
        // requires &mut self, so publication cannot overlap mutation/destruction.
        unsafe { std::slice::from_raw_parts(native_snes_ram_data(self.ptr), RAM) }
    }
    fn manual_mask(&self, voice: Option<u64>, audition: bool) -> u8 {
        self.owners
            .iter()
            .enumerate()
            .fold(0, |mask, (channel, lease)| {
                mask | if lease
                    .is_some_and(|lease| lease.audition && (!audition || lease.voice != voice))
                {
                    1 << channel
                } else {
                    0
                }
            })
    }
    fn foreign_mask(&self, owner: Option<Owner>, voice: Option<u64>) -> u8 {
        self.owners
            .iter()
            .enumerate()
            .fold(0, |mask, (channel, lease)| {
                mask | if lease.is_some_and(|lease| {
                    owner.is_some() && lease.owner != owner
                        || self.current_group.is_some() && lease.group != self.current_group
                        || voice.is_some() && lease.voice != voice
                }) {
                    1 << channel
                } else {
                    0
                }
            })
    }
    fn op(
        &mut self,
        op: Op,
        owner: Option<Owner>,
        voice: Option<u64>,
        audition: bool,
        preserve: bool,
        declared: Option<u8>,
    ) {
        let manual = self.manual_mask(voice, audition);
        let foreign = self.foreign_mask(owner, voice);
        match op {
            Op::KeyOn { channel } => {
                if manual & (1 << channel) != 0 {
                    return;
                }
                unsafe {
                    native_snes_key_on(self.ptr, channel);
                }
                self.gates |= 1 << channel;
                if !preserve {
                    self.owners[channel as usize] = Some(Lease {
                        group: self.current_group,
                        owner,
                        voice,
                        audition,
                    });
                }
            }
            Op::KeyOff { channel } => {
                if (manual | foreign) & (1 << channel) != 0 {
                    return;
                }
                unsafe {
                    native_snes_key_off(self.ptr, channel);
                }
                self.gates &= !(1 << channel);
                if !preserve {
                    self.owners[channel as usize] = None;
                }
            }
            Op::Write {
                register,
                mut value,
            } => {
                if local_channel(register).is_some_and(|ch| manual & (1 << ch) != 0) {
                    return;
                }
                if register == 0x4c {
                    let accepted = value & !manual & declared.map_or(255, |ch| 1 << ch);
                    for ch in 0..8 {
                        if accepted & (1 << ch) != 0 {
                            self.gates |= 1 << ch;
                            if !preserve {
                                self.owners[ch] = Some(Lease {
                                    group: self.current_group,
                                    owner,
                                    voice,
                                    audition,
                                });
                            }
                        }
                    }
                    value = accepted;
                }
                if register == 0x5c {
                    value &= !(manual | foreign);
                    if let Some(ch) = declared {
                        value &= 1 << ch;
                        let current = self.state().registers[0x5c];
                        value |= current & !(1 << ch);
                    }
                    for ch in 0..8 {
                        if value & (1 << ch) != 0 && (manual | foreign) & (1 << ch) == 0 {
                            self.gates &= !(1 << ch);
                            if !preserve {
                                self.owners[ch] = None;
                            }
                        }
                    }
                }
                if matches!(register, 0x2d | 0x3d | 0x4d) {
                    let protect = manual | declared.map_or(0, |ch| !(1 << ch));
                    let current = self.state().registers[register as usize];
                    value = (value & !protect) | (current & protect);
                }
                if manual != 0
                    && !matches!(register, 0x4c | 0x5c | 0x2d | 0x3d | 0x4d)
                    && local_channel(register).is_none()
                {
                    return;
                }
                unsafe {
                    native_snes_write_protected(
                        self.ptr,
                        register,
                        value,
                        if register == 0x5c {
                            manual | foreign
                        } else {
                            manual
                        },
                    );
                }
            }
        }
    }
    fn release(&mut self, channel: usize) {
        unsafe {
            native_snes_key_off(self.ptr, channel as u8);
        }
        self.gates &= !(1 << channel);
        self.owners[channel] = None;
    }
    fn clear_timeline(&mut self, shared: &Shared) {
        self.timeline_owner = None;
        self.timeline_group = None;
        self.play_command_id = 0;
        if let Some(old) = self.timeline.take() {
            shared.retire_timeline(old);
        }
    }
    pub fn apply(&mut self, command: &Prepared, owner: Option<Owner>, shared: &Shared) {
        self.current_group = command.group;
        let held = self.manual_mask(None, false);
        let rejection = if held != 0 && matches!(command.edit, Edit::Load(..) | Edit::Uploads(..)) {
            2
        } else if held != 0
            && matches!(
                command.edit,
                Edit::Reset | Edit::ResetState(..) | Edit::PrepareTimeline(..) | Edit::PlayTimeline
            )
        {
            4
        } else if self.timeline.is_some()
            && matches!(
                command.edit,
                Edit::Op(..) | Edit::Load(..) | Edit::Uploads(..) | Edit::Batch(..)
            )
        {
            8
        } else {
            0
        };
        if rejection != 0 {
            command.rejected.store(rejection, Ordering::Release);
            shared.publish(self, Some(command), false);
            self.current_group = None;
            return;
        }
        let mut ram_changed = false;
        match &command.edit {
            Edit::Op(op) => {
                if self.timeline.is_none() {
                    self.op(*op, owner, None, command.audition, false, None);
                }
            }
            Edit::Load(address, data) => {
                if self.timeline.is_none() {
                    unsafe {
                        native_snes_load(self.ptr, data.as_ptr(), *address, data.len() as u32);
                    }
                    ram_changed = true;
                }
            }
            Edit::Uploads(uploads) => {
                for u in uploads {
                    unsafe {
                        native_snes_load(self.ptr, u.data.as_ptr(), u.address, u.data.len() as u32);
                    }
                }
                ram_changed = true;
            }
            Edit::Reset | Edit::ResetState(..) => {
                unsafe {
                    native_snes_reset(self.ptr);
                }
                self.clear_timeline(shared);
                self.timeline_generation += 1;
                self.owners.fill(None);
                self.gates = 0;
                ram_changed = true;
                if let Edit::ResetState(dsp, uploads) = &command.edit {
                    for u in uploads {
                        unsafe {
                            native_snes_load(
                                self.ptr,
                                u.data.as_ptr(),
                                u.address,
                                u.data.len() as u32,
                            );
                        }
                    }
                    for (reg, value) in dsp.iter().enumerate() {
                        self.op(
                            Op::Write {
                                register: reg as u8,
                                value: *value,
                            },
                            owner,
                            None,
                            command.audition,
                            false,
                            None,
                        );
                    }
                }
            }
            Edit::Stop => {
                if owner.is_none() && command.group.is_none() && held == 0 {
                    unsafe {
                        if self.timeline.is_some() {
                            native_snes_stop_timeline(self.ptr);
                        } else {
                            native_snes_stop(self.ptr);
                        }
                    }
                    self.owners.fill(None);
                    self.gates = 0;
                    self.timeline_owner = None;
                    self.timeline_group = None;
                    self.play_command_id = 0;
                    self.timeline_generation += 1;
                } else {
                    if self.timeline.is_some()
                        && self.timeline_owner == owner
                        && (command.group.is_none() || self.timeline_group == command.group)
                    {
                        unsafe {
                            native_snes_stop_timeline(self.ptr);
                        }
                        self.timeline_owner = None;
                        self.timeline_group = None;
                        self.play_command_id = 0;
                        self.timeline_generation += 1;
                    }
                    for ch in 0..8 {
                        if self.owners[ch].is_some_and(|l| {
                            l.owner == owner
                                && (command.group.is_none() || l.group == command.group)
                                && (!l.audition || command.audition)
                        }) {
                            self.release(ch);
                        }
                    }
                }
            }
            Edit::CancelMainOwner(target) => {
                if self.timeline.is_some()
                    && self.timeline_owner.is_none()
                    && self.timeline_group == Some(*target)
                {
                    unsafe {
                        native_snes_stop_timeline(self.ptr);
                    }
                    self.timeline_group = None;
                    self.play_command_id = 0;
                    self.timeline_generation += 1;
                }
                for ch in 0..8 {
                    if self.owners[ch].is_some_and(|l| {
                        l.owner.is_none() && l.group == Some(*target) && !l.audition
                    }) {
                        self.release(ch);
                    }
                }
            }
            Edit::Barrier => {}
            Edit::Mix(volume, pan) => {
                self.targets = [
                    *volume * if *pan > 0.0 { 1.0 - *pan } else { 1.0 },
                    *volume * if *pan < 0.0 { 1.0 + *pan } else { 1.0 },
                ];
                self.ramp = (self.rate / 200).max(1);
            }
            Edit::Batch(ops, channel, voice, action) if self.timeline.is_none() => {
                let index = *channel as usize;
                let current = self.owners[index];
                let allowed = match action {
                    LeaseAction::On => command.audition || current.is_none_or(|v| !v.audition),
                    LeaseAction::Off => current.is_some_and(|v| {
                        v.owner == owner
                            && (command.group.is_none() || v.group == command.group)
                            && (voice.is_none() || v.voice == *voice)
                            && (!v.audition || command.audition)
                    }),
                    LeaseAction::Edit => {
                        current.is_none_or(|v| !v.audition || command.audition && v.voice == *voice)
                    }
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
                        self.op(
                            *op,
                            owner,
                            *voice,
                            command.audition,
                            *action == LeaseAction::Edit,
                            Some(*channel),
                        );
                    }
                    if *action == LeaseAction::Off {
                        self.owners[index] = None;
                        self.gates &= !(1 << channel);
                    }
                }
            }
            Edit::Batch(..) => {}
            Edit::PrepareTimeline(trace) if self.manual_mask(None, command.audition) == 0 => {
                unsafe {
                    native_snes_prepare_timeline(
                        self.ptr,
                        trace.ram.as_ptr(),
                        trace.dsp.as_ptr(),
                        trace.events.as_ptr(),
                        (trace.events.len() / 4) as u32,
                        trace.duration,
                    );
                }
                self.clear_timeline(shared);
                self.timeline = Some(trace.clone());
                self.timeline_owner = owner;
                self.timeline_group = command.group;
                self.timeline_generation += 1;
                self.owners.fill(None);
                self.gates = 0;
                ram_changed = true;
            }
            Edit::PrepareTimeline(..) => {}
            Edit::PlayTimeline => {
                if self.timeline.is_some() {
                    unsafe {
                        native_snes_play_timeline(self.ptr);
                    }
                    ram_changed = true;
                    self.timeline_owner = owner;
                    self.timeline_group = command.group;
                    self.timeline_generation += 1;
                    self.play_command_id = command.id.unwrap_or(0);
                }
            }
            Edit::StopTimeline => {
                if owner.is_none() && command.group.is_none()
                    || self.timeline_owner == owner
                        && (command.group.is_none() || self.timeline_group == command.group)
                {
                    unsafe {
                        native_snes_stop_timeline(self.ptr);
                    }
                    self.timeline_owner = None;
                    self.timeline_group = None;
                    self.play_command_id = 0;
                    self.timeline_generation += 1;
                }
            }
        }
        self.current_group = None;
        shared.publish(self, Some(command), ram_changed);
    }
    pub fn publish(&self, shared: &Shared) {
        shared.publish(self, None, false);
    }
    pub fn cancel_invalid(&mut self, valid: impl Fn(Option<Owner>) -> bool, shared: &Shared) {
        let mut changed = false;
        if self.timeline.is_some() && !valid(self.timeline_owner) {
            unsafe {
                native_snes_stop_timeline(self.ptr);
            }
            self.timeline_owner = None;
            self.timeline_group = None;
            self.play_command_id = 0;
            self.timeline_generation += 1;
            changed = true;
        }
        for channel in 0..8 {
            if self.owners[channel].is_some_and(|lease| !valid(lease.owner)) {
                self.release(channel);
                changed = true;
            }
        }
        if changed {
            shared.publish(self, None, false);
        }
    }
    pub fn mix_additional(&mut self, pcm: &mut [f32; 2], shared: &Shared) {
        self.mix(pcm);
        // Explicit commands publish immediately. Background voice/timeline
        // observation needs one DSP snapshot per render quantum, not per sample.
        if self.frame % 128 == 0 {
            self.publish(shared);
        }
    }
    pub fn mix(&mut self, pcm: &mut [f32; 2]) {
        self.frame += 1;
        let mut sample = [0.0; 2];
        unsafe {
            native_snes_sample(self.ptr, &mut sample[0], &mut sample[1]);
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
