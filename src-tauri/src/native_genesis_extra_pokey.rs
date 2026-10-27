//! Callback-owned independent POKEY using the fixed SDK raw core.
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

pub const CLOCK: u32 = 1_789_790;
const BATCH: usize = 512;
const PENDING_BYTES: usize = 8 * 1024 * 1024;
#[repr(C)]
#[derive(Default)]
struct CoreState {
    registers: [u8; 16],
    reads: [u8; 16],
    levels: [u8; 4],
    clock: u32,
}
extern "C" {
    fn native_pokey_create(rate: u32, clock: u32) -> *mut c_void;
    fn native_pokey_destroy(ptr: *mut c_void);
    fn native_pokey_reset(ptr: *mut c_void);
    fn native_pokey_stop(ptr: *mut c_void);
    fn native_pokey_write(ptr: *mut c_void, register: u32, value: u8);
    fn native_pokey_key_off(ptr: *mut c_void, channel: u32);
    fn native_pokey_set_retained_level(ptr: *mut c_void, ch: u32, level: u8);
    fn native_pokey_set_mute_mask(ptr: *mut c_void, mask: u32);
    fn native_pokey_sample(ptr: *mut c_void, left: *mut f32, right: *mut f32);
    fn native_pokey_get_state(ptr: *mut c_void, out: *mut CoreState);
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
    KeyOff {
        channel: u8,
    },
    Reset,
    Barrier,
    SetRetainedLevel {
        channel: u8,
        level: u8,
    },
    ChannelMute {
        channel: u8,
        muted: bool,
    },
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
    Barrier,
    SetRetainedLevel(u8, u8),
    ChannelMute(u8, bool),
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
    fixed_clock_rejected: AtomicBool,
}
fn valid_id(id: Option<u64>) -> bool {
    id.is_none_or(|id| id > 0 && id <= MAX_SAFE_INTEGER)
}
fn op_channel(op: &Op) -> Option<u8> {
    match op {
        Op::Write { register, .. } if *register < 8 => Some(*register / 2),
        Op::KeyOff { channel } => Some(*channel),
        _ => None,
    }
}
fn validate_op(op: &Op) -> Result<(), String> {
    match op {
        Op::Write {
            register, channel, ..
        } if *register > 15
            || channel.is_some_and(|ch| ch > 3)
            || channel.is_some_and(|ch| *register < 8 && ch != *register / 2) =>
        {
            Err("Native POKEY register must be 0–15 and channel metadata must match 0–3".into())
        }
        Op::KeyOff { channel } if *channel > 3 => Err("Native POKEY channel must be 0–3".into()),
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
            return Err("Invalid Native POKEY command ID".into());
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
            Method::KeyOff { channel } => Edit::Op(Op::KeyOff { channel: *channel }),
            Method::Reset => Edit::Reset,
            Method::Barrier => Edit::Barrier,
            Method::SetRetainedLevel { channel, level } => {
                if *channel > 3 || *level > 15 {
                    return Err("Invalid POKEY retained level".into());
                }
                Edit::SetRetainedLevel(*channel, *level)
            }
            Method::ChannelMute { channel, muted } => {
                if *channel > 3 {
                    return Err("Invalid POKEY mute channel".into());
                }
                Edit::ChannelMute(*channel, *muted)
            }
            Method::StopVoices | Method::AllNotesOff => Edit::Stop,
            Method::SourceMute { mask } => {
                if *mask > 15 {
                    return Err("Native POKEY source mute mask must be 0–15".into());
                }
                Edit::SourceMute(*mask)
            }
            Method::CancelMainOwner { target_group } => {
                if !valid_id(Some(*target_group)) {
                    return Err("Invalid Native POKEY Main owner group".into());
                }
                Edit::CancelMainOwner(*target_group)
            }
            Method::Clock { hz } => {
                if !(100_000..=100_000_000).contains(hz) {
                    return Err("Native POKEY clock must be 100000–100000000 Hz".into());
                }
                Edit::Clock(*hz)
            }
            Method::Mix { volume, pan } => {
                if !volume.is_finite()
                    || !(0.0..=4.0).contains(volume)
                    || !pan.is_finite()
                    || !(-1.0..=1.0).contains(pan)
                {
                    return Err("Native POKEY mixer volume must be 0–4, pan −1–1".into());
                }
                Edit::Mix(*volume, *pan)
            }
            Method::Batch {
                commands,
                channel,
                voice,
                action,
            } => {
                if *channel > 3
                    || commands.is_empty()
                    || commands.len() > BATCH
                    || !valid_id(*voice)
                    || self.audition && *action != LeaseAction::Edit && voice.is_none()
                {
                    return Err("Invalid Native POKEY batch or voice lease".into());
                }
                for op in commands {
                    validate_op(op)?;
                    if op_channel(op).is_some_and(|target| {
                        target != *channel
                            && !(*channel & 1 != 0
                                && target + 1 == *channel
                                && matches!(op,Op::Write{register,..}if *register==target*2))
                    }) {
                        return Err("Native POKEY batch must target its declared channel".into());
                    }
                }
                let ops = commands.clone();
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
            fixed_clock_rejected: AtomicBool::new(false),
        }))
    }
}
const CLOCK_HELD: &str =
    "Release the manually held additional POKEY voices before changing the clock";
impl Prepared {
    fn outcome(&self) -> u8 {
        self.cancelled.load(Ordering::Acquire) as u8
            | ((self.clock_rejected.load(Ordering::Acquire) as u8) << 1)
            | ((self.fixed_clock_rejected.load(Ordering::Acquire) as u8) << 2)
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
        Err("Native POKEY command was cancelled before it applied".into())
    } else if flags & 4 != 0 {
        Err("Create a new POKEY instance to change its clock".into())
    } else if flags & 2 != 0 {
        Err(CLOCK_HELD.into())
    } else {
        Ok(())
    }
}
pub struct Shared {
    revision: AtomicU64,
    registers: [AtomicU8; 16],
    reads: [AtomicU8; 16],
    levels: [AtomicU8; 4],
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
            registers: std::array::from_fn(|_| AtomicU8::new(0)),
            reads: std::array::from_fn(|_| AtomicU8::new(0)),
            levels: std::array::from_fn(|_| AtomicU8::new(8)),
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
    pub registers: Vec<u8>,
    pub read_registers: Vec<u8>,
    pub levels: Vec<u8>,
    pub initialized: bool,
    pub pending_commands: Vec<u64>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub pokey: ChipStatus,
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
            .map_err(|_| "Native POKEY control unavailable")?;
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
            return Err("Native POKEY pending command limit reached (8 MiB)".into());
        }
        let mut ids = HashSet::new();
        for v in c.prepared.iter().chain(commands.iter().map(|(v, _)| v)) {
            if v.id.is_some_and(|id| !ids.insert(id)) {
                return Err("Duplicate pending Native POKEY command ID".into());
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
            .expect("POKEY control checked by transactional retain");
        for v in &control.prepared {
            if v.group == Some(target) && !v.audition && !v.complete.load(Ordering::Acquire) {
                v.cancelled.store(true, Ordering::Release);
            }
        }
        true
    }
    pub fn wait_applied(&self, id: u64) -> Result<(), String> {
        if id == 0 || id > self.highest_id.load(Ordering::Acquire) {
            return Err("Unknown Native POKEY command ID".into());
        }
        let cmd = {
            let c = self
                .control
                .lock()
                .map_err(|_| "Native POKEY control unavailable")?;
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
                return Err("Native POKEY command outcome has expired".into());
            }
        };
        let start = std::time::Instant::now();
        while !cmd.complete.load(Ordering::Acquire) {
            if start.elapsed() > std::time::Duration::from_millis(5000) {
                return Err("Native POKEY command was not applied within 5000 ms".into());
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
            .unwrap_or_else(|_| unreachable!("Bounded POKEY retirement"))
    }
    fn publish(&self, renderer: &Renderer, command: Option<&Prepared>) {
        let state = renderer.state();
        self.revision.fetch_add(1, Ordering::AcqRel);
        for (out, value) in self.registers.iter().zip(state.registers) {
            out.store(value, Ordering::Relaxed);
        }
        for (out, value) in self.reads.iter().zip(state.reads) {
            out.store(value, Ordering::Relaxed);
        }
        for (out, value) in self.levels.iter().zip(state.levels) {
            out.store(value, Ordering::Relaxed);
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
            .map_err(|_| "Native POKEY control state unavailable")?;
        self.collect(&mut registry);
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() >= std::time::Duration::from_millis(50) {
                return Err("Native POKEY state changed while reading; retry".into());
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
                pokey: ChipStatus {
                    held: (0..4)
                        .filter(|ch| self.held.load(Ordering::Relaxed) & (1 << ch) != 0)
                        .collect(),
                    source_mute_mask: self.mute.load(Ordering::Relaxed),
                    clock: self.clock(),
                    registers: self
                        .registers
                        .iter()
                        .map(|v| v.load(Ordering::Relaxed))
                        .collect(),
                    read_registers: self
                        .reads
                        .iter()
                        .map(|v| v.load(Ordering::Relaxed))
                        .collect(),
                    levels: self
                        .levels
                        .iter()
                        .map(|v| v.load(Ordering::Relaxed))
                        .collect(),
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
    owners: [Option<Lease>; 4],
    mute: u8,
    gains: [f32; 2],
    targets: [f32; 2],
    ramp: u32,
    rate: u32,
}
unsafe impl Send for Renderer {}
impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe { native_pokey_destroy(self.ptr) }
    }
}
impl Renderer {
    pub fn new_with_clock(rate: u32, clock: u32) -> Result<Self, String> {
        let ptr = unsafe { native_pokey_create(rate, clock) };
        if ptr.is_null() {
            return Err("Could not allocate Native POKEY core".into());
        }
        Ok(Self {
            ptr,
            current_group: None,
            owners: [None; 4],
            mute: 0,
            gains: [1.0; 2],
            targets: [1.0; 2],
            ramp: 0,
            rate,
        })
    }
    fn state(&self) -> CoreState {
        let mut state = CoreState::default();
        unsafe { native_pokey_get_state(self.ptr, &mut state) }
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
    fn release(&mut self, ch: usize) {
        unsafe { native_pokey_key_off(self.ptr, ch as u32) }
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
        let target = op_channel(op).map(usize::from);
        let manual = self.manual_mask(voice, audition);
        if target.is_some_and(|ch| manual & (1 << ch) != 0) {
            return;
        }
        // A joined lower timer and high-pass modulator can affect an audible
        // peer. Protect that shared physical resource across owner boundaries.
        let registers = self.state().registers;
        if let Some(ch) = target {
            let joined = ch & 1 == 0 && registers[8] & (if ch == 0 { 16 } else { 8 }) != 0;
            let peer = if joined {
                Some(ch + 1)
            } else if ch == 2 && registers[8] & 4 != 0 {
                Some(0)
            } else if ch == 3 && registers[8] & 2 != 0 {
                Some(1)
            } else {
                None
            };
            if peer.is_some_and(|peer| {
                manual & (1 << peer) != 0
                    || registers[peer * 2 + 1] & 15 != 0 && self.foreign(peer, owner, voice)
            }) {
                return;
            }
        }

        let off = matches!(op, Op::KeyOff { .. })
            || matches!(op,Op::Write{register,value,..}if *register<8&&*register&1!=0&&*value&15==0);
        if off && target.is_some_and(|ch| self.foreign(ch, owner, voice)) {
            return;
        }
        // Global clock, join, timer-reset and polynomial writes affect all four
        // voices. A foreign owner must not disturb another live or manual note.
        if target.is_none()
            && (0..4).any(|ch| {
                manual & (1 << ch) != 0
                    || self.state().registers[ch * 2 + 1] & 15 != 0
                        && self.foreign(ch, owner, voice)
            })
        {
            return;
        }
        unsafe {
            match op {
                Op::Write {
                    register, value, ..
                } => native_pokey_write(self.ptr, *register as u32, *value),
                Op::KeyOff { channel } => native_pokey_key_off(self.ptr, *channel as u32),
            }
        }
        if !preserve {
            if let Some(ch) = target {
                if matches!(op, Op::KeyOff { .. })
                    || matches!(op,Op::Write{register,..}if *register&1!=0)
                {
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
                unsafe { native_pokey_reset(self.ptr) }
                self.owners.fill(None);
                self.mute = 0;
            }
            Edit::Reset => {}
            Edit::Barrier => {}
            Edit::SetRetainedLevel(ch, level) => unsafe {
                native_pokey_set_retained_level(self.ptr, *ch as u32, *level)
            },
            Edit::ChannelMute(ch, muted) => {
                self.mute = if *muted {
                    self.mute | (1 << ch)
                } else {
                    self.mute & !(1 << ch)
                };
                unsafe { native_pokey_set_mute_mask(self.ptr, self.mute as u32) }
            }
            Edit::SourceMute(mask) => {
                unsafe { native_pokey_set_mute_mask(self.ptr, *mask as u32) }
                self.mute = *mask;
            }
            Edit::CancelMainOwner(target) => {
                for ch in 0..4 {
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
                    unsafe { native_pokey_stop(self.ptr) }
                    self.owners.fill(None);
                } else {
                    for ch in 0..4 {
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
            Edit::Clock(hz) => {
                if self.state().clock != *hz {
                    command.fixed_clock_rejected.store(true, Ordering::Release);
                }
            }
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
                    if *action == LeaseAction::On && self.state().registers[ch * 2 + 1] & 15 != 0 {
                        self.owners[ch] = Some(Lease {
                            group: self.current_group,
                            owner,
                            voice: *voice,
                            audition: command.audition,
                        });
                    }
                    if *action == LeaseAction::Off || self.state().registers[ch * 2 + 1] & 15 == 0 {
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
        for ch in 0..4 {
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
            native_pokey_sample(self.ptr, &mut sample[0], &mut sample[1]);
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
