//! Callback-owned PC Engine PSG using the exact Web WASM HuC6280 DSP.
//! Variable-size batches stay retained on the control thread until retirement.
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
const BATCH: usize = 512;
const PENDING_BYTES: usize = 8 * 1024 * 1024;
#[repr(C)]
struct CoreState {
    selected: u8,
    globals: [u8; 10],
    registers: [[u8; 8]; 8],
    waveforms: [[u8; 32]; 8],
    indices: [u8; 8],
    dda: [u8; 8],
    clock: u32,
}
impl Default for CoreState {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}
extern "C" {
    fn native_huc6280_create(rate: u32) -> *mut c_void;
    fn native_huc6280_set_mute_mask(ptr: *mut c_void, mask: u32);
    fn native_huc6280_destroy(ptr: *mut c_void);
    fn native_huc6280_reset(ptr: *mut c_void);
    fn native_huc6280_stop(ptr: *mut c_void);
    fn native_huc6280_write(ptr: *mut c_void, register: u32, value: u8, channel: i32);
    fn native_huc6280_waveform(ptr: *mut c_void, channel: u32, samples: *const u8);
    fn native_huc6280_key_off(ptr: *mut c_void, channel: u32);
    fn native_huc6280_set_clock(ptr: *mut c_void, hz: u32) -> i32;
    fn native_huc6280_sample(ptr: *mut c_void, left: *mut f32, right: *mut f32);
    fn native_huc6280_get_state(ptr: *mut c_void, out: *mut CoreState);
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
        channel: Option<u8>,
    },
    Waveform {
        channel: u8,
        samples: [u8; 32],
    },
    KeyOff {
        channel: u8,
    },
    Reset,
    StopVoices,
    AllNotesOff,
    SourceMute {
        mask: u8,
    },
    CancelMainOwner {
        #[serde(rename = "targetGroup")]
        target_group: u64,
    },
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
    },
}
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(tag = "method", rename_all = "kebab-case")]
pub enum Op {
    Write {
        register: u8,
        value: u8,
        channel: Option<u8>,
    },
    Waveform {
        channel: u8,
        samples: [u8; 32],
    },
    KeyOff {
        channel: u8,
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
enum Edit {
    Op(Op),
    Reset,
    Stop,
    Clock(u32),
    Mix(f32, f32),
    SourceMute(u8),
    CancelMainOwner(u64),
    Batch(Vec<Op>, u8, Option<u64>, LeaseAction),
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
}
fn valid_id(id: Option<u64>) -> bool {
    id.is_none_or(|id| id > 0 && id <= MAX_SAFE_INTEGER)
}
fn validate_op(op: &Op) -> Result<(), String> {
    match op {
        Op::Write {
            register, channel, ..
        } if *register > 9 || channel.is_some_and(|ch| ch > 7) => {
            Err("Native HuC6280 register must be 0–9 and raw channel 0–7".into())
        }
        Op::Waveform { channel, samples } if *channel > 5 || samples.iter().any(|v| *v > 31) => {
            Err("Native HuC6280 waveform needs 32 five-bit samples on channel 0–5".into())
        }
        Op::KeyOff { channel } if *channel > 5 => Err("Native HuC6280 channel must be 0–5".into()),
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
            return Err("Invalid Native HuC6280 command ID".into());
        }
        let edit = match &self.method {
            Method::Write {
                register,
                value,
                channel,
            } => Edit::Op(Op::Write {
                register: *register,
                value: *value,
                channel: *channel,
            }),
            Method::Waveform { channel, samples } => Edit::Op(Op::Waveform {
                channel: *channel,
                samples: *samples,
            }),
            Method::KeyOff { channel } => Edit::Op(Op::KeyOff { channel: *channel }),
            Method::Reset => Edit::Reset,
            Method::StopVoices | Method::AllNotesOff => Edit::Stop,
            Method::SourceMute { mask } => {
                if *mask > 63 {
                    return Err("Native HuC6280 source mute mask must be 0–63".into());
                }
                Edit::SourceMute(*mask)
            }
            Method::CancelMainOwner { target_group } => {
                if !valid_id(Some(*target_group)) {
                    return Err("Invalid Native HuC6280 Main owner group".into());
                }
                Edit::CancelMainOwner(*target_group)
            }
            Method::Clock { hz } => {
                if !(100_000..=100_000_000).contains(hz) {
                    return Err("Native HuC6280 clock must be 100000–100000000 Hz".into());
                }
                Edit::Clock(*hz)
            }
            Method::Mix { volume, pan } => {
                if !volume.is_finite()
                    || !(0.0..=4.0).contains(volume)
                    || !pan.is_finite()
                    || !(-1.0..=1.0).contains(pan)
                {
                    return Err("Native HuC6280 mixer volume must be 0–4, pan −1–1".into());
                }
                Edit::Mix(*volume, *pan)
            }
            Method::Batch {
                commands,
                channel,
                voice,
                action,
            } => {
                if *channel > 5
                    || commands.is_empty()
                    || commands.len() > BATCH
                    || !valid_id(*voice)
                    || self.audition && *action != LeaseAction::Edit && voice.is_none()
                {
                    return Err("Invalid Native HuC6280 batch or voice lease".into());
                }
                for op in commands {
                    validate_op(op)?;
                    let target = match op {
                        Op::Write {
                            register: 0, value, ..
                        } => Some(value & 7),
                        Op::Write {
                            register: 1 | 8 | 9,
                            ..
                        } => None,
                        Op::Write {
                            channel: target, ..
                        } => Some(target.unwrap_or(*channel)),
                        Op::Waveform { channel, .. } | Op::KeyOff { channel } => Some(*channel),
                    };
                    if target.is_some_and(|target| target != *channel) {
                        return Err("Native HuC6280 batch must target its declared channel".into());
                    }
                }
                // Supply the declared channel even when a compact batch omits
                // per-write metadata; no channel-select latch can interfere.
                let ops = commands
                    .iter()
                    .map(|op| match op {
                        Op::Write {
                            register,
                            value,
                            channel: None,
                        } if !matches!(register, 0 | 1 | 8 | 9) => Op::Write {
                            register: *register,
                            value: *value,
                            channel: Some(*channel),
                        },
                        op => *op,
                    })
                    .collect();
                Edit::Batch(ops, *channel, *voice, *action)
            }
        };
        if let Edit::Op(op) = &edit {
            validate_op(op)?;
        }
        Ok(Arc::new(Prepared {
            id: self.id,
            audition: self.audition,
            group: self.group,
            edit,
            complete: AtomicBool::new(false),
            retired: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
            clock_rejected: AtomicBool::new(false),
        }))
    }
}
const CLOCK_HELD: &str =
    "Release the manually held additional HuC6280 voices before changing the clock";
impl Prepared {
    fn outcome(&self) -> u8 {
        self.cancelled.load(Ordering::Acquire) as u8
            | ((self.clock_rejected.load(Ordering::Acquire) as u8) << 1)
    }
    pub fn is_owner_cancel(&self) -> bool {
        matches!(self.edit, Edit::CancelMainOwner(_))
    }
    pub fn starts(&self) -> bool {
        matches!(self.edit, Edit::Op(_))
            || matches!(self.edit,Edit::Batch(_,_,_,action)if action!=LeaseAction::Off)
    }
    pub fn stops(&self) -> bool {
        matches!(self.edit, Edit::Stop) && self.group.is_none()
    }
    pub fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + if let Edit::Batch(ops, ..) = &self.edit {
                ops.capacity() * std::mem::size_of::<Op>()
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
        Err("Native HuC6280 command was cancelled before it applied".into())
    } else if flags & 2 != 0 {
        Err(CLOCK_HELD.into())
    } else {
        Ok(())
    }
}
pub struct Shared {
    revision: AtomicU64,
    selected: AtomicU8,
    globals: [AtomicU8; 10],
    registers: [[AtomicU8; 8]; 8],
    waveforms: [[AtomicU8; 32]; 8],
    indices: [AtomicU8; 8],
    dda: [AtomicU8; 8],
    clock: AtomicU32,
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
            selected: AtomicU8::new(0),
            globals: std::array::from_fn(|_| AtomicU8::new(0)),
            registers: std::array::from_fn(|_| std::array::from_fn(|_| AtomicU8::new(0))),
            waveforms: std::array::from_fn(|_| std::array::from_fn(|_| AtomicU8::new(0))),
            indices: std::array::from_fn(|_| AtomicU8::new(0)),
            dda: std::array::from_fn(|_| AtomicU8::new(0)),
            clock: AtomicU32::new(CLOCK),
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
#[serde(rename_all = "camelCase")]
pub struct ChipStatus {
    pub clock: u32,
    pub held: Vec<u8>,
    pub source_mute_mask: u8,
    pub selected_channel: u8,
    pub globals: Vec<u8>,
    pub registers: Vec<Vec<u8>>,
    pub waveforms: Vec<Vec<u8>>,
    pub indices: Vec<u8>,
    pub pointer_known: Vec<bool>,
    pub waveform_known: Vec<bool>,
    pub known_samples: Vec<u32>,
    pub dda_values: Vec<u8>,
    pub initialized: bool,
    pub pending_commands: Vec<u64>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub huc6280: ChipStatus,
    pub frame: u64,
    pub sample_rate: u32,
    pub revision: u64,
}
impl Shared {
    pub fn prepare_message(&self, m: &Message) -> Result<Arc<Prepared>, String> {
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
            .map_err(|_| "Native HuC6280 control unavailable")?;
        self.collect(&mut c);
        if self.held.load(Ordering::Acquire) != 0
            && commands
                .iter()
                .any(|(v, _)| !v.audition && matches!(v.edit, Edit::Clock(_)))
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
            return Err("Native HuC6280 pending command limit reached (8 MiB)".into());
        }
        let mut ids = HashSet::new();
        for v in c.prepared.iter().chain(commands.iter().map(|(v, _)| v)) {
            if v.id.is_some_and(|id| !ids.insert(id)) {
                return Err("Duplicate pending Native HuC6280 command ID".into());
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
            .expect("HuC6280 control checked by transactional retain");
        for v in &control.prepared {
            if v.group == Some(target) && !v.audition && !v.complete.load(Ordering::Acquire) {
                v.cancelled.store(true, Ordering::Release);
            }
        }
        true
    }
    pub fn wait_applied(&self, id: u64) -> Result<(), String> {
        if id == 0 || id > self.highest_id.load(Ordering::Acquire) {
            return Err("Unknown Native HuC6280 command ID".into());
        }
        let cmd = {
            let c = self
                .control
                .lock()
                .map_err(|_| "Native HuC6280 control unavailable")?;
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
                return Err("Native HuC6280 command outcome has expired".into());
            }
        };
        let start = std::time::Instant::now();
        while !cmd.complete.load(Ordering::Acquire) {
            if start.elapsed() > std::time::Duration::from_millis(5000) {
                return Err("Native HuC6280 command was not applied within 5000 ms".into());
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
            .unwrap_or_else(|_| unreachable!("Bounded HuC6280 retirement"))
    }
    fn publish(&self, renderer: &Renderer, command: Option<&Prepared>) {
        let state = renderer.state();
        self.revision.fetch_add(1, Ordering::AcqRel);
        self.selected.store(state.selected, Ordering::Relaxed);
        for (out, value) in self.globals.iter().zip(state.globals) {
            out.store(value, Ordering::Relaxed);
        }
        for ch in 0..8 {
            for (out, value) in self.registers[ch].iter().zip(state.registers[ch]) {
                out.store(value, Ordering::Relaxed);
            }
            for (out, value) in self.waveforms[ch].iter().zip(state.waveforms[ch]) {
                out.store(value, Ordering::Relaxed);
            }
            self.indices[ch].store(state.indices[ch], Ordering::Relaxed);
            self.dda[ch].store(state.dda[ch], Ordering::Relaxed);
        }
        self.clock.store(state.clock, Ordering::Relaxed);
        self.rate.store(renderer.rate, Ordering::Relaxed);
        self.mute.store(renderer.mute, Ordering::Relaxed);
        self.held
            .store(renderer.manual_mask(None, false), Ordering::Relaxed);
        if let Some(command) = command {
            command.complete.store(true, Ordering::Release);
        }
        self.revision.fetch_add(1, Ordering::Release);
    }
    pub fn status(&self, frame: u64) -> Result<Status, String> {
        let mut registry = self
            .control
            .lock()
            .map_err(|_| "Native HuC6280 control state unavailable")?;
        self.collect(&mut registry);
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() >= std::time::Duration::from_millis(50) {
                return Err("Native HuC6280 state changed while reading; retry".into());
            }
            let before = self.revision.load(Ordering::Acquire);
            if before & 1 != 0 {
                std::thread::yield_now();
                continue;
            }
            let status = Status {
                frame,
                sample_rate: self.rate.load(Ordering::Relaxed),
                revision: before / 2,
                huc6280: ChipStatus {
                    held: (0..6)
                        .filter(|ch| self.held.load(Ordering::Relaxed) & (1 << ch) != 0)
                        .collect(),
                    source_mute_mask: self.mute.load(Ordering::Relaxed),
                    clock: self.clock(),
                    selected_channel: self.selected.load(Ordering::Relaxed),
                    globals: self
                        .globals
                        .iter()
                        .map(|v| v.load(Ordering::Relaxed))
                        .collect(),
                    registers: self
                        .registers
                        .iter()
                        .map(|r| r.iter().map(|v| v.load(Ordering::Relaxed)).collect())
                        .collect(),
                    waveforms: self
                        .waveforms
                        .iter()
                        .map(|r| r.iter().map(|v| v.load(Ordering::Relaxed)).collect())
                        .collect(),
                    indices: self
                        .indices
                        .iter()
                        .map(|v| v.load(Ordering::Relaxed))
                        .collect(),
                    dda_values: self.dda.iter().map(|v| v.load(Ordering::Relaxed)).collect(),
                    pointer_known: vec![true; 8],
                    waveform_known: vec![true; 8],
                    known_samples: vec![u32::MAX; 8],
                    initialized: true,
                    pending_commands: registry
                        .prepared
                        .iter()
                        .filter(|v| !v.complete.load(Ordering::Relaxed))
                        .filter_map(|v| v.id)
                        .collect(),
                },
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
    group: Option<u64>,
    owner: Option<Owner>,
    voice: Option<u64>,
    audition: bool,
}
pub struct Renderer {
    ptr: *mut c_void,
    current_group: Option<u64>,
    owners: [Option<Lease>; 8],
    mute: u8,
    gains: [f32; 2],
    targets: [f32; 2],
    ramp: u32,
    rate: u32,
}
unsafe impl Send for Renderer {}
impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe { native_huc6280_destroy(self.ptr) }
    }
}
impl Renderer {
    pub fn new(rate: u32) -> Result<Self, String> {
        let ptr = unsafe { native_huc6280_create(rate) };
        if ptr.is_null() {
            return Err("Could not allocate Native HuC6280 core".into());
        }
        Ok(Self {
            ptr,
            current_group: None,
            owners: [None; 8],
            mute: 0,
            gains: [1.0; 2],
            targets: [1.0; 2],
            ramp: 0,
            rate,
        })
    }
    fn state(&self) -> CoreState {
        let mut state = CoreState::default();
        unsafe { native_huc6280_get_state(self.ptr, &mut state) }
        state
    }
    fn manual_mask(&self, voice: Option<u64>, audition: bool) -> u8 {
        self.owners.iter().enumerate().fold(0, |mask, (ch, lease)| {
            if lease.is_some_and(|l| l.audition && (!audition || l.voice != voice)) {
                mask | (1 << ch)
            } else {
                mask
            }
        })
    }
    fn foreign(&self, ch: usize, owner: Option<Owner>, voice: Option<u64>) -> bool {
        self.owners[ch].is_some_and(|l| {
            owner.is_some() && l.owner != owner
                || self.current_group.is_some() && l.group != self.current_group
                || voice.is_some() && l.voice != voice
        })
    }
    fn channel(&self, op: &Op) -> Option<usize> {
        match op {
            Op::Write {
                register: 0 | 1 | 8 | 9,
                ..
            } => None,
            Op::Write { channel, .. } => {
                Some(channel.unwrap_or_else(|| self.state().selected) as usize)
            }
            Op::Waveform { channel, .. } | Op::KeyOff { channel } => Some(*channel as usize),
        }
    }
    fn release(&mut self, ch: usize) {
        unsafe { native_huc6280_key_off(self.ptr, ch as u32) }
        self.owners[ch] = None;
    }
    fn write(
        &mut self,
        op: &Op,
        owner: Option<Owner>,
        voice: Option<u64>,
        audition: bool,
        preserve: bool,
    ) {
        let target = self.channel(op);
        let manual = self.manual_mask(voice, audition);
        if target.is_some_and(|ch| manual & (1 << ch) != 0) {
            return;
        }
        let state = self.state();
        let off = matches!(op, Op::KeyOff { .. })
            || matches!(op,Op::Write{register:4,value,..} if value&128==0);
        if off && target.is_some_and(|ch| self.foreign(ch, owner, voice)) {
            return;
        }
        // The master balance and LFO registers are shared. A command belonging
        // to one performance must not retune or attenuate another held voice.
        let protected = |ch: usize| {
            manual & (1 << ch) != 0
                || state.registers[ch][4] & 128 != 0 && self.foreign(ch, owner, voice)
        };
        let manual_edit = audition
            && preserve
            && voice.is_some_and(|v| {
                self.owners
                    .iter()
                    .flatten()
                    .any(|l| l.audition && l.voice == Some(v))
            });
        if !manual_edit && matches!(op, Op::Write { register: 1, .. }) && (0..6).any(protected) {
            return;
        }
        if !manual_edit
            && matches!(
                op,
                Op::Write {
                    register: 8 | 9,
                    ..
                }
            )
            && (0..2).any(protected)
        {
            return;
        }
        // CH2's waveform, frequency and DDA are also CH1's modulator while LFO
        // is enabled. Guard that shared resource even if CH2 itself is silent.
        // CH2's balance/volume do not scale the LFO source. A control write is
        // safe if it neither changes DDA mode nor retriggers the source tick.
        let lfo_safe_local = matches!(op, Op::KeyOff { .. } | Op::Write { register: 5, .. })
            || matches!(op, Op::Write { register:4,value,.. } if (value ^ state.registers[1][4]) & 64 == 0 && (value & 128 == 0 || state.registers[1][4] & 128 != 0));
        if !lfo_safe_local && target == Some(1) && state.globals[9] & 3 != 0 && protected(0) {
            return;
        }
        unsafe {
            match op {
                Op::Write {
                    register,
                    value,
                    channel,
                } => native_huc6280_write(
                    self.ptr,
                    *register as u32,
                    *value,
                    channel.map_or(-1, i32::from),
                ),
                Op::Waveform { channel, samples } => {
                    native_huc6280_waveform(self.ptr, *channel as u32, samples.as_ptr())
                }
                Op::KeyOff { channel } => native_huc6280_key_off(self.ptr, *channel as u32),
            }
        }
        if !preserve {
            if let Some(ch) = target {
                if matches!(op, Op::Write { register: 4, .. } | Op::KeyOff { .. }) {
                    self.owners[ch] = if off {
                        None
                    } else {
                        Some(Lease {
                            group: self.current_group,
                            owner,
                            voice,
                            audition,
                        })
                    };
                }
            }
        }
    }
    pub fn apply(&mut self, command: &Prepared, owner: Option<Owner>, shared: &Shared) {
        if !command.audition
            && matches!(command.edit, Edit::Clock(_))
            && self.manual_mask(None, false) != 0
        {
            command.clock_rejected.store(true, Ordering::Release);
            shared.publish(self, Some(command));
            return;
        }
        self.current_group = command.group;
        match &command.edit {
            Edit::Op(op) => self.write(op, owner, None, command.audition, false),
            Edit::Reset if self.manual_mask(None, command.audition) == 0 => {
                unsafe { native_huc6280_reset(self.ptr) }
                self.owners.fill(None);
            }
            Edit::Reset => {}
            Edit::SourceMute(mask) => {
                unsafe { native_huc6280_set_mute_mask(self.ptr, *mask as u32) }
                self.mute = *mask;
            }
            Edit::CancelMainOwner(target) => {
                for ch in 0..8 {
                    if self.owners[ch].is_some_and(|l| !l.audition && l.group == Some(*target)) {
                        self.release(ch)
                    }
                }
            }
            Edit::Stop => {
                if owner.is_none()
                    && command.group.is_none()
                    && (command.audition || self.manual_mask(None, false) == 0)
                {
                    unsafe { native_huc6280_stop(self.ptr) }
                    self.owners.fill(None);
                } else {
                    for ch in 0..8 {
                        if self.owners[ch].is_some_and(|l| {
                            (!l.audition || command.audition)
                                && (owner.is_none() || l.owner == owner)
                                && (command.group.is_none() || l.group == command.group)
                        }) {
                            self.release(ch)
                        }
                    }
                }
            }
            Edit::Clock(hz) => unsafe {
                native_huc6280_set_clock(self.ptr, *hz);
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
                    LeaseAction::On => command.audition || current.is_none_or(|l| !l.audition),
                    LeaseAction::Off => current.is_some_and(|l| {
                        l.owner == owner
                            && (command.group.is_none() || l.group == command.group)
                            && (voice.is_none() || l.voice == *voice)
                            && (!l.audition || command.audition)
                    }),
                    LeaseAction::Edit => {
                        current.is_none_or(|l| !l.audition || command.audition && l.voice == *voice)
                    }
                };
                if allowed {
                    if *action == LeaseAction::On {
                        self.owners[ch] = Some(Lease {
                            group: self.current_group,
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
                        );
                    }
                    if *action == LeaseAction::On && self.state().registers[ch][4] & 128 != 0 {
                        self.owners[ch] = Some(Lease {
                            group: self.current_group,
                            owner,
                            voice: *voice,
                            audition: command.audition,
                        });
                    }
                    if *action == LeaseAction::Off || self.state().registers[ch][4] & 128 == 0 {
                        self.owners[ch] = None;
                    }
                }
            }
        }
        shared.publish(self, Some(command));
        self.current_group = None;
    }
    pub fn publish(&self, shared: &Shared) {
        shared.publish(self, None);
    }
    pub fn cancel_invalid(&mut self, valid: impl Fn(Option<Owner>) -> bool, shared: &Shared) {
        let mut changed = false;
        for ch in 0..8 {
            if self.owners[ch].is_some_and(|l| !valid(l.owner)) {
                self.release(ch);
                changed = true;
            }
        }
        if changed {
            shared.publish(self, None);
        }
    }
    pub fn mix(&mut self, pcm: &mut [f32; 2]) {
        let mut sample = [0.0; 2];
        unsafe {
            native_huc6280_sample(self.ptr, &mut sample[0], &mut sample[1]);
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
