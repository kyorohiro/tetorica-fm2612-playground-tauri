//! Independently allocated DMG core. Control retains all variable-sized commands;
//! the render callback accesses its own core and atomic status only.
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

pub const CLOCK: u32 = 4_194_304;
const BATCH: usize = 512;
const PENDING_BYTES: usize = 8 * 1024 * 1024;
#[repr(C)]
struct CoreState {
    registers: [u8; 48],
    active_mask: u32,
    clock: u32,
    source_mute_mask: u32,
}
impl Default for CoreState {
    fn default() -> Self {
        Self {
            registers: [0; 48],
            active_mask: 0,
            clock: CLOCK,
            source_mute_mask: 0,
        }
    }
}
extern "C" {
    fn native_gameboy_create(rate: u32) -> *mut c_void;
    fn native_gameboy_destroy(ptr: *mut c_void);
    fn native_gameboy_reset(ptr: *mut c_void);
    fn native_gameboy_stop(ptr: *mut c_void);
    fn native_gameboy_stop_channel(ptr: *mut c_void, channel: u8);
    fn native_gameboy_write(ptr: *mut c_void, address: u8, value: u8);
    fn native_gameboy_set_clock(ptr: *mut c_void, hz: u32) -> i32;
    fn native_gameboy_set_mute_mask(ptr: *mut c_void, mask: u32);
    fn native_gameboy_sample(ptr: *mut c_void, left: *mut f32, right: *mut f32);
    fn native_gameboy_get_state(ptr: *mut c_void, out: *mut CoreState);
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
        address: u8,
        value: u8,
    },
    Reset,
    StopVoices,
    AllNotesOff,
    Clock {
        hz: u32,
    },
    Mix {
        volume: f32,
        pan: f32,
    },
    SourceMute {
        mask: u8,
    },
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
    Write { address: u8, value: u8 },
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
const CLOCK_HELD: &str =
    "Release the manually held additional Game Boy voices before changing the clock";
fn valid_id(v: Option<u64>) -> bool {
    v.is_none_or(|v| v > 0 && v <= MAX_SAFE_INTEGER)
}
fn channel(address: u8) -> Option<u8> {
    match address {
        0..=4 => Some(0),
        5..=9 => Some(1),
        10..=14 | 32..=47 => Some(2),
        15..=19 => Some(3),
        _ => None,
    }
}
fn trigger(address: u8, value: u8) -> bool {
    matches!(address, 4 | 9 | 14 | 19) && value & 128 != 0
}
fn off(address: u8, value: u8) -> bool {
    matches!(address, 2 | 7 | 17) && value & 248 == 0 || address == 10 && value & 128 == 0
}
fn validate(op: Op) -> Result<(), String> {
    let Op::Write { address, .. } = op;
    if address >= 48 {
        Err("Native additional Game Boy address must be 0–47".into())
    } else {
        Ok(())
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
            return Err("Invalid Native additional Game Boy command ID".into());
        }
        let edit = match &self.method {
            Method::Write { address, value } => {
                let op = Op::Write {
                    address: *address,
                    value: *value,
                };
                validate(op)?;
                Edit::Op(op)
            }
            Method::Reset => Edit::Reset,
            Method::StopVoices | Method::AllNotesOff => Edit::Stop,
            Method::Clock { hz } => {
                if !(100_000..=100_000_000).contains(hz) {
                    return Err("Native Game Boy clock must be 100000–100000000 Hz".into());
                }
                Edit::Clock(*hz)
            }
            Method::Mix { volume, pan } => {
                if !volume.is_finite()
                    || !(0.0..=4.0).contains(volume)
                    || !pan.is_finite()
                    || !(-1.0..=1.0).contains(pan)
                {
                    return Err("Native Game Boy mixer volume must be 0–4, pan −1–1".into());
                }
                Edit::Mix(*volume, *pan)
            }
            Method::SourceMute { mask } => {
                if *mask > 15 {
                    return Err("Native Game Boy source mute mask must be 0–15".into());
                }
                Edit::SourceMute(*mask)
            }
            Method::CancelMainOwner { target_group } => {
                if !valid_id(Some(*target_group)) {
                    return Err("Invalid Native Game Boy Main owner group".into());
                }
                Edit::CancelMainOwner(*target_group)
            }
            Method::Batch {
                commands,
                channel: target,
                voice,
                action,
            } => {
                if *target > 3
                    || commands.is_empty()
                    || commands.len() > BATCH
                    || !valid_id(*voice)
                    || self.audition && *action != LeaseAction::Edit && voice.is_none()
                {
                    return Err("Invalid Native Game Boy channel batch or voice lease".into());
                }
                for op in commands {
                    validate(*op)?;
                    let Op::Write { address, .. } = op;
                    if channel(*address).is_some_and(|c| c != *target) || *address == 22 {
                        return Err("Native Game Boy batch must target its declared channel; NR52 is global".into());
                    }
                }
                Edit::Batch(commands.clone(), *target, *voice, *action)
            }
        };
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
        Err("Native Game Boy command was cancelled before it applied".into())
    } else if flags & 2 != 0 {
        Err(CLOCK_HELD.into())
    } else {
        Ok(())
    }
}
pub struct Shared {
    revision: AtomicU64,
    registers: [AtomicU8; 48],
    voice: [AtomicU8; 48],
    active: AtomicU8,
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
            voice: std::array::from_fn(|_| AtomicU8::new(0)),
            active: AtomicU8::new(0),
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
    pub registers: Vec<u8>,
    pub voice: Vec<u8>,
    pub gates: [bool; 4],
    pub held: Vec<u8>,
    pub source_mute_mask: u8,
    pub pending_commands: Vec<u64>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub gameboy: ChipStatus,
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
            .map_err(|_| "Native Game Boy control unavailable")?;
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
            return Err("Native Game Boy pending command limit reached (8 MiB)".into());
        }
        let mut ids = HashSet::new();
        for v in c.prepared.iter().chain(commands.iter().map(|(v, _)| v)) {
            if v.id.is_some_and(|id| !ids.insert(id)) {
                return Err("Duplicate pending Native Game Boy command ID".into());
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
            .expect("Game Boy control checked by transactional retain");
        for v in &control.prepared {
            if v.group == Some(target) && !v.audition && !v.complete.load(Ordering::Acquire) {
                v.cancelled.store(true, Ordering::Release);
            }
        }
        true
    }
    pub fn wait_applied(&self, id: u64) -> Result<(), String> {
        if id == 0 || id > self.highest_id.load(Ordering::Acquire) {
            return Err("Unknown Native Game Boy command ID".into());
        }
        let cmd = {
            let c = self
                .control
                .lock()
                .map_err(|_| "Native Game Boy control unavailable")?;
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
                return Err("Native Game Boy command outcome has expired".into());
            }
        };
        let start = std::time::Instant::now();
        while !cmd.complete.load(Ordering::Acquire) {
            if start.elapsed() > std::time::Duration::from_millis(5000) {
                return Err("Native Game Boy command was not applied within 5000 ms".into());
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
            .unwrap_or_else(|_| unreachable!("Bounded Game Boy retirement"))
    }
    fn publish(&self, r: &Renderer, c: Option<&Prepared>) {
        let s = r.state();
        self.revision.fetch_add(1, Ordering::AcqRel);
        for ((out, voice), (value, configured)) in self
            .registers
            .iter()
            .zip(&self.voice)
            .zip(s.registers.into_iter().zip(r.voice))
        {
            out.store(value, Ordering::Relaxed);
            voice.store(configured, Ordering::Relaxed)
        }
        self.active.store(s.active_mask as u8, Ordering::Relaxed);
        self.clock.store(s.clock, Ordering::Relaxed);
        self.mute.store(s.source_mute_mask as u8, Ordering::Relaxed);
        self.rate.store(r.rate, Ordering::Relaxed);
        self.held.store(
            r.owners
                .iter()
                .enumerate()
                .filter(|(_, v)| v.is_some_and(|l| l.audition))
                .fold(0, |bits, (ch, _)| bits | (1 << ch)),
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
            .map_err(|_| "Native Game Boy control unavailable")?;
        self.collect(&mut c);
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() > std::time::Duration::from_millis(50) {
                return Err("Native Game Boy snapshot changing; retry".into());
            }
            let before = self.revision.load(Ordering::Acquire);
            if before & 1 != 0 {
                std::thread::yield_now();
                continue;
            }
            let active = self.active.load(Ordering::Relaxed);
            let held = self.held.load(Ordering::Relaxed);
            let out = Status {
                frame,
                sample_rate: self.rate.load(Ordering::Relaxed),
                revision: before / 2,
                gameboy: ChipStatus {
                    clock: self.clock(),
                    registers: self
                        .registers
                        .iter()
                        .map(|v| v.load(Ordering::Relaxed))
                        .collect(),
                    voice: self
                        .voice
                        .iter()
                        .map(|v| v.load(Ordering::Relaxed))
                        .collect(),
                    gates: std::array::from_fn(|ch| active & (1 << ch) != 0),
                    held: (0..4).filter(|ch| held & (1 << ch) != 0).collect(),
                    source_mute_mask: self.mute.load(Ordering::Relaxed),
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
    group: Option<u64>,
    owner: Option<Owner>,
    voice: Option<u64>,
    audition: bool,
}
pub struct Renderer {
    current_group: Option<u64>,
    ptr: *mut c_void,
    owners: [Option<Lease>; 4],
    voice: [u8; 48],
    gains: [f32; 2],
    targets: [f32; 2],
    ramp: u32,
    rate: u32,
}
unsafe impl Send for Renderer {}
impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe {
            native_gameboy_destroy(self.ptr);
        }
    }
}
impl Renderer {
    pub fn new(rate: u32) -> Result<Self, String> {
        let ptr = unsafe { native_gameboy_create(rate) };
        if ptr.is_null() {
            return Err("Could not allocate Native Game Boy core".into());
        }
        let mut renderer = Self {
            ptr,
            current_group: None,
            owners: [None; 4],
            voice: [0; 48],
            gains: [1.0; 2],
            targets: [1.0; 2],
            ramp: 0,
            rate,
        };
        renderer.voice = renderer.state().registers;
        Ok(renderer)
    }
    fn state(&self) -> CoreState {
        let mut state = CoreState::default();
        unsafe {
            native_gameboy_get_state(self.ptr, &mut state);
        }
        state
    }
    fn manual_mask(&self, voice: Option<u64>, audition: bool) -> u8 {
        let mut mask = 0;
        for (channel, lease) in self.owners.iter().enumerate() {
            if lease.is_some_and(|lease| lease.audition && (!audition || lease.voice != voice)) {
                mask |= 1 << channel;
            }
        }
        mask
    }
    fn foreign(&self, index: usize, owner: Option<Owner>, voice: Option<u64>) -> bool {
        self.owners[index].is_some_and(|lease| {
            owner.is_some() && lease.owner != owner
                || self.current_group.is_some() && lease.group != self.current_group
                || voice.is_some() && lease.voice != voice
        })
    }
    fn release(&mut self, index: usize) {
        unsafe {
            native_gameboy_stop_channel(self.ptr, index as u8);
        }
        self.owners[index] = None;
    }
    fn write(
        &mut self,
        op: Op,
        owner: Option<Owner>,
        voice: Option<u64>,
        audition: bool,
        preserve: bool,
    ) {
        let Op::Write { address, mut value } = op;
        let manual = self.manual_mask(voice, audition);
        let target = channel(address);
        if target.is_some_and(|channel| manual & (1 << channel) != 0) {
            return;
        }
        if target.is_some_and(|channel| {
            off(address, value) && self.foreign(channel as usize, owner, voice)
        }) {
            return;
        }
        let state = self.state();
        if address == 20
            && (manual != 0
                || (0..4).any(|index| {
                    state.active_mask & (1 << index) != 0 && self.foreign(index, owner, voice)
                }))
        {
            return;
        }
        if address == 21 {
            let mut protected = manual;
            for index in 0..4 {
                if self.foreign(index, owner, voice) {
                    protected |= 1 << index;
                }
            }
            let mask = protected | (protected << 4);
            value = (value & !mask) | (state.registers[21] & mask);
        }
        if address == 22 && value & 128 == 0 {
            let mut protected = manual;
            for index in 0..4 {
                if state.active_mask & (1 << index) != 0 && self.foreign(index, owner, voice) {
                    protected |= 1 << index;
                }
            }
            if protected != 0 {
                for index in 0..4 {
                    if protected & (1 << index) == 0 {
                        self.release(index);
                    }
                }
                return;
            }
        }
        unsafe {
            native_gameboy_write(self.ptr, address, value);
        }
        let after = self.state();
        if address == 22 && value & 128 == 0 {
            self.owners.fill(None);
            self.voice[..23].fill(0);
        } else {
            // Retain last configured envelope/DAC values across keyOff, matching
            // GameboyVoiceInspector. Hardware-rejected writes stay unapplied.
            let allowed =
                state.registers[22] & 128 != 0 || matches!(address, 1 | 6 | 11 | 16 | 22 | 32..=47);
            if allowed && !off(address, value) && address < 32 {
                self.voice[address as usize] = if trigger(address, value) {
                    value & 127
                } else {
                    value
                };
            }
            if !preserve {
                if let Some(index) = target {
                    let index = index as usize;
                    if trigger(address, value) && after.active_mask & (1 << index) != 0 {
                        self.owners[index] = Some(Lease {
                            group: self.current_group,
                            owner,
                            voice,
                            audition,
                        });
                    } else if after.active_mask & (1 << index) == 0 && off(address, value) {
                        self.owners[index] = None;
                    }
                }
            }
        }
        self.voice[32..48].copy_from_slice(&after.registers[32..48]);
        self.voice[22] = after.registers[22] & 128;
    }
    pub fn apply(&mut self, command: &Prepared, owner: Option<Owner>, shared: &Shared) {
        self.reconcile();
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
            Edit::Op(op) => self.write(*op, owner, None, command.audition, false),
            Edit::Reset if self.manual_mask(None, command.audition) == 0 => {
                unsafe {
                    native_gameboy_reset(self.ptr);
                }
                self.owners.fill(None);
                self.voice = self.state().registers;
            }
            Edit::Reset => {}
            Edit::SourceMute(mask) => unsafe {
                native_gameboy_set_mute_mask(self.ptr, *mask as u32)
            },
            Edit::CancelMainOwner(target) => {
                for index in 0..4 {
                    if self.owners[index].is_some_and(|l| !l.audition && l.group == Some(*target)) {
                        self.release(index)
                    }
                }
            }
            Edit::Stop => {
                if owner.is_none()
                    && command.group.is_none()
                    && (command.audition || self.manual_mask(None, false) == 0)
                {
                    unsafe {
                        native_gameboy_stop(self.ptr);
                    }
                    self.owners.fill(None);
                } else {
                    for index in 0..4 {
                        if self.owners[index].is_some_and(|lease| {
                            (!lease.audition || command.audition)
                                && (owner.is_none() || lease.owner == owner)
                                && (command.group.is_none() || lease.group == command.group)
                        }) {
                            self.release(index);
                        }
                    }
                }
            }
            Edit::Clock(hz) => unsafe {
                native_gameboy_set_clock(self.ptr, *hz);
            },
            Edit::Mix(volume, pan) => {
                self.targets = [
                    *volume * if *pan > 0.0 { 1.0 - *pan } else { 1.0 },
                    *volume * if *pan < 0.0 { 1.0 + *pan } else { 1.0 },
                ];
                self.ramp = (self.rate / 200).max(1);
            }
            Edit::Batch(ops, target, voice, action) => {
                let index = *target as usize;
                let current = self.owners[index];
                let allowed = match action {
                    LeaseAction::On => {
                        command.audition || current.is_none_or(|lease| !lease.audition)
                    }
                    LeaseAction::Off => current.is_some_and(|lease| {
                        lease.owner == owner
                            && (command.group.is_none() || lease.group == command.group)
                            && (voice.is_none() || lease.voice == *voice)
                            && (!lease.audition || command.audition)
                    }),
                    LeaseAction::Edit => current.is_none_or(|lease| {
                        !lease.audition || command.audition && lease.voice == *voice
                    }),
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
                        self.write(
                            *op,
                            owner,
                            *voice,
                            command.audition,
                            *action == LeaseAction::Edit,
                        );
                    }
                    if *action == LeaseAction::On && self.state().active_mask & (1 << index) != 0 {
                        self.owners[index] = Some(Lease {
                            group: self.current_group,
                            owner,
                            voice: *voice,
                            audition: command.audition,
                        });
                    }
                    if *action == LeaseAction::Off || self.state().active_mask & (1 << index) == 0 {
                        self.owners[index] = None;
                    }
                }
            }
        }
        self.reconcile();
        shared.publish(self, Some(command));
        self.current_group = None;
    }
    fn reconcile(&mut self) -> bool {
        let active = self.state().active_mask;
        let mut changed = false;
        for (channel, lease) in self.owners.iter_mut().enumerate() {
            if active & (1 << channel) == 0 && lease.is_some() {
                *lease = None;
                changed = true;
            }
        }
        changed
    }
    pub fn publish(&self, shared: &Shared) {
        shared.publish(self, None);
    }
    pub fn cancel_invalid(&mut self, valid: impl Fn(Option<Owner>) -> bool, shared: &Shared) {
        let mut changed = self.reconcile();
        for index in 0..4 {
            if self.owners[index].is_some_and(|lease| !valid(lease.owner)) {
                self.release(index);
                changed = true;
            }
        }
        if changed || shared.active.load(Ordering::Relaxed) != self.state().active_mask as u8 {
            shared.publish(self, None);
        }
    }
    pub fn mix(&mut self, pcm: &mut [f32; 2]) {
        let mut sample = [0.0; 2];
        unsafe {
            native_gameboy_sample(self.ptr, &mut sample[0], &mut sample[1]);
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
