//! Accurate callback-owned CPU-free S-DSP. Control-side registries retain all
//! variable data until callback retirement; rendering never allocates or locks.
use super::{Owner, CAPACITY, MAX_SAFE_INTEGER};
use crossbeam_queue::ArrayQueue;
use serde::{Deserialize, Serialize};
use std::{
    ffi::c_void,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering},
        Arc, Mutex,
    },
};
pub const CLOCK: u32 = 1_024_000;
const RAM: usize = 65_536;
const MAX_EVENTS: usize = 1_000_000;
const MAX_PENDING: usize = 32 * 1024 * 1024;
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
    #[cfg(test)]
    fn native_snes_read_memory(ptr: *mut c_void, address: u16) -> u8;
}
#[derive(Debug, Deserialize)]
pub struct Message {
    #[serde(default)]
    pub id: Option<u64>,
    #[serde(default)]
    pub audition: bool,
    #[serde(flatten)]
    pub method: Method,
}
#[derive(Debug, Deserialize)]
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
    StopVoices,
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
impl Message {
    pub fn prepare(&self) -> Result<Arc<Prepared>, String> {
        if !valid_id(self.id) {
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
            Method::StopVoices => Edit::Stop,
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
        }))
    }
}
impl Prepared {
    pub fn starts(&self) -> bool {
        fn starts(op: &Op) -> bool {
            matches!(op, Op::KeyOn { .. })
                || matches!(op,Op::Write{register:0x4c,value} if *value!=0)
        }
        match &self.edit {
            Edit::Op(op) => starts(op),
            Edit::Batch(ops, _, _, action) => *action == LeaseAction::On || ops.iter().any(starts),
            Edit::PlayTimeline => true,
            _ => false,
        }
    }
    pub fn stops(&self) -> bool {
        matches!(self.edit, Edit::Stop | Edit::StopTimeline)
    }
    fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + match &self.edit {
                Edit::Load(_, data) => data.len(),
                Edit::Batch(ops, ..) => ops.len() * std::mem::size_of::<Op>(),
                Edit::PrepareTimeline(trace) => RAM + 128 + trace.events.len() * 4,
                _ => 0,
            }
    }
}
pub struct Shared {
    revision: AtomicU64,
    registers: [AtomicU8; 128],
    ram: [AtomicU8; RAM],
    native_frames: AtomicU64,
    enabled: AtomicU8,
    timeline_loaded: AtomicBool,
    timeline_playing: AtomicBool,
    timeline_position: AtomicU64,
    timeline_total: AtomicU64,
    prepared: Mutex<Vec<Arc<Prepared>>>,
    retired: ArrayQueue<Arc<Prepared>>,
    retired_timelines: ArrayQueue<Arc<Timeline>>,
    highest_id: AtomicU64,
}
impl Default for Shared {
    fn default() -> Self {
        Self {
            revision: AtomicU64::new(0),
            registers: std::array::from_fn(|_| AtomicU8::new(0)),
            ram: std::array::from_fn(|_| AtomicU8::new(0)),
            native_frames: AtomicU64::new(0),
            enabled: AtomicU8::new(0),
            timeline_loaded: AtomicBool::new(false),
            timeline_playing: AtomicBool::new(false),
            timeline_position: AtomicU64::new(0),
            timeline_total: AtomicU64::new(0),
            prepared: Mutex::new(Vec::new()),
            retired: ArrayQueue::new(CAPACITY),
            retired_timelines: ArrayQueue::new(CAPACITY),
            highest_id: AtomicU64::new(0),
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
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub clock: u32,
    pub registers: Vec<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ram: Option<Vec<u8>>,
    pub native_frames: u64,
    pub enabled: u8,
    pub frame: u64,
    pub revision: u64,
    pub timeline: TimelineState,
    pub pending_commands: Vec<u64>,
}
impl Shared {
    pub fn clock(&self) -> u32 {
        CLOCK
    }
    fn collect(&self, registry: &mut Vec<Arc<Prepared>>) {
        let mut retired = std::collections::HashSet::new();
        while let Some(command) = self.retired.pop() {
            retired.insert(Arc::as_ptr(&command) as usize);
        }
        registry.retain(|command| !retired.contains(&(Arc::as_ptr(command) as usize)));
        while self.retired_timelines.pop().is_some() {}
    }
    pub fn retain(&self, commands: &[Arc<Prepared>]) -> Result<(), String> {
        let mut registry = self
            .prepared
            .lock()
            .map_err(|_| "Native SNES control registry is unavailable")?;
        self.collect(&mut registry);
        if registry.len() + commands.len() > CAPACITY
            || registry
                .iter()
                .chain(commands)
                .map(|v| v.bytes())
                .sum::<usize>()
                > MAX_PENDING
        {
            return Err("Native SNES pending command limit reached".into());
        }
        let mut ids = std::collections::HashSet::new();
        for command in registry.iter().chain(commands) {
            if command.id.is_some_and(|id| !ids.insert(id)) {
                return Err("Duplicate pending Native SNES command ID".into());
            }
        }
        registry.extend(commands.iter().cloned());
        for command in commands {
            if let Some(id) = command.id {
                self.highest_id.fetch_max(id, Ordering::Release);
            }
        }
        Ok(())
    }
    pub fn wait_applied(&self, id: u64) -> Result<(), String> {
        if !valid_id(Some(id)) || id > self.highest_id.load(Ordering::Acquire) {
            return Err("Unknown Native SNES command ID".into());
        }
        let command = self
            .prepared
            .lock()
            .map_err(|_| "Native SNES control registry is unavailable")?
            .iter()
            .find(|v| v.id == Some(id))
            .cloned();
        if let Some(command) = command {
            let start = std::time::Instant::now();
            while !command.complete.load(Ordering::Acquire) {
                if start.elapsed() >= std::time::Duration::from_millis(100) {
                    return Err("Native SNES command was not applied within 100 ms".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        Ok(())
    }
    pub fn retire(&self, command: Arc<Prepared>) {
        command.complete.store(true, Ordering::Release);
        self.retired
            .push(command)
            .unwrap_or_else(|_| unreachable!("Bounded SNES command retirement"));
    }
    fn retire_timeline(&self, timeline: Arc<Timeline>) {
        self.retired_timelines
            .push(timeline)
            .unwrap_or_else(|_| unreachable!("Bounded SNES timeline retirement"));
    }
    fn publish(&self, renderer: &Renderer, command: Option<&Prepared>, ram_changed: bool) {
        let state = renderer.state();
        self.revision.fetch_add(1, Ordering::AcqRel);
        for (out, &value) in self.registers.iter().zip(&state.registers) {
            out.store(value, Ordering::Relaxed);
        }
        // Instruments disable echo RAM writes. Raw echo/timeline playback needs
        // current physical RAM too; copy only then or after a RAM/reset edit.
        if ram_changed || state.registers[0x6c] & 0x20 == 0 || state.timeline_loaded != 0 {
            // Borrow the callback-owned, fixed C++ allocation. Moving a 64 KiB
            // array into zip/IntoIter inflated debug callback stack frames and
            // could exhaust a small audio-thread stack. No such copy is needed.
            for (out, &value) in self.ram.iter().zip(renderer.ram()) {
                out.store(value, Ordering::Relaxed);
            }
        }
        self.native_frames
            .store(state.native_frames, Ordering::Relaxed);
        self.enabled.store(renderer.gates, Ordering::Relaxed);
        self.timeline_loaded
            .store(state.timeline_loaded != 0, Ordering::Relaxed);
        self.timeline_playing
            .store(state.timeline_playing != 0, Ordering::Relaxed);
        self.timeline_position
            .store(state.timeline_position as u64, Ordering::Relaxed);
        self.timeline_total
            .store(state.timeline_total as u64, Ordering::Relaxed);
        if let Some(command) = command {
            command.complete.store(true, Ordering::Relaxed);
        }
        self.revision.fetch_add(1, Ordering::Release);
    }
    pub fn status(&self, frame: u64) -> Result<Status, String> {
        self.read_status(frame, false)
    }
    pub fn status_with_ram(&self, frame: u64) -> Result<Status, String> {
        self.read_status(frame, true)
    }
    fn read_status(&self, frame: u64, with_ram: bool) -> Result<Status, String> {
        let mut registry = self
            .prepared
            .lock()
            .map_err(|_| "Native SNES control registry is unavailable")?;
        self.collect(&mut registry);
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() >= std::time::Duration::from_millis(100) {
                return Err("Native SNES state changed while reading; retry".into());
            }
            let before = self.revision.load(Ordering::Acquire);
            if before & 1 != 0 {
                std::thread::yield_now();
                continue;
            }
            let position = self.timeline_position.load(Ordering::Relaxed);
            let total = self.timeline_total.load(Ordering::Relaxed);
            let loaded = self.timeline_loaded.load(Ordering::Relaxed);
            let state = Status {
                clock: CLOCK,
                registers: self
                    .registers
                    .iter()
                    .map(|v| v.load(Ordering::Relaxed))
                    .collect(),
                ram: with_ram.then(|| self.ram.iter().map(|v| v.load(Ordering::Relaxed)).collect()),
                native_frames: self.native_frames.load(Ordering::Relaxed),
                enabled: self.enabled.load(Ordering::Relaxed),
                frame,
                revision: before / 2,
                timeline: TimelineState {
                    loaded,
                    playing: self.timeline_playing.load(Ordering::Relaxed),
                    position_frames: position,
                    total_frames: total,
                    ended: loaded && position >= total,
                },
                pending_commands: registry
                    .iter()
                    .filter(|v| !v.complete.load(Ordering::Relaxed))
                    .filter_map(|v| v.id)
                    .collect(),
            };
            std::sync::atomic::fence(Ordering::Acquire);
            if self.revision.load(Ordering::Relaxed) == before {
                return Ok(state);
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
    owners: [Option<Lease>; 8],
    gates: u8,
    gains: [f32; 2],
    targets: [f32; 2],
    ramp: u32,
    rate: u32,
    timeline: Option<Arc<Timeline>>,
    timeline_owner: Option<Owner>,
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
        })
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
        if let Some(old) = self.timeline.take() {
            shared.retire_timeline(old);
        }
    }
    pub fn apply(&mut self, command: &Prepared, owner: Option<Owner>, shared: &Shared) {
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
            Edit::Reset if self.manual_mask(None, command.audition) == 0 => {
                unsafe {
                    native_snes_reset(self.ptr);
                }
                self.clear_timeline(shared);
                self.owners.fill(None);
                self.gates = 0;
                ram_changed = true;
            }
            Edit::Reset => {}
            Edit::Stop => {
                if owner.is_none() {
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
                    ram_changed = true;
                } else {
                    if self.timeline.is_some() && self.timeline_owner == owner {
                        unsafe {
                            native_snes_stop_timeline(self.ptr);
                        }
                        self.timeline_owner = None;
                    }
                    for channel in 0..8 {
                        if self.owners[channel].is_some_and(|lease| {
                            lease.owner == owner && (!lease.audition || command.audition)
                        }) {
                            self.release(channel);
                        }
                    }
                }
            }
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
                }
            }
            Edit::StopTimeline => {
                if owner.is_none() || self.timeline_owner == owner {
                    unsafe {
                        native_snes_stop_timeline(self.ptr);
                    }
                    self.timeline_owner = None;
                }
            }
        }
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
    pub fn stop(&mut self, shared: &Shared) {
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
        shared.publish(self, None, true);
    }
    pub fn mix(&mut self, pcm: &mut [f32; 2]) {
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
    #[cfg(test)]
    pub fn memory_byte(&self, address: u16) -> u8 {
        unsafe { native_snes_read_memory(self.ptr, address) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    fn apply(shared: &Shared, renderer: &mut Renderer, value: Value, owner: Option<Owner>) {
        let message: Message = serde_json::from_value(value).unwrap();
        let command = message.prepare().unwrap();
        shared.retain(std::slice::from_ref(&command)).unwrap();
        renderer.apply(&command, owner, shared);
        shared.retire(command);
    }
    fn setup() -> (Shared, Renderer) {
        let shared = Shared::default();
        let mut renderer = Renderer::new(48000).unwrap();
        apply(
            &shared,
            &mut renderer,
            json!({"method":"load-memory","address":0x400,"data":[0xb3,0x12,0x34,0x56,0x78,0x9a,0xbc,0xde,0xf0]}),
            None,
        );
        apply(
            &shared,
            &mut renderer,
            json!({"method":"load-memory","address":0,"data":[0,4,0,4]}),
            None,
        );
        for (register, value) in [
            (0x6c, 0x20),
            (0x0c, 127),
            (0x1c, 127),
            (0x5d, 0),
            (0x5c, 255),
            (0x2c, 0),
            (0x3c, 0),
            (0x2d, 0),
            (0x3d, 0),
            (0x4d, 0),
            (0, 100),
            (1, 100),
            (2, 0),
            (3, 16),
            (4, 0),
            (5, 0),
            (6, 0),
            (7, 127),
        ] {
            apply(
                &shared,
                &mut renderer,
                json!({"method":"write","register":register,"value":value}),
                None,
            );
        }
        (shared, renderer)
    }
    fn sample(renderer: &mut Renderer, frames: usize) -> f32 {
        let mut peak = 0.0f32;
        for _ in 0..frames {
            let mut pcm = [0.0; 2];
            renderer.mix(&mut pcm);
            assert!(pcm.iter().all(|v| v.is_finite()));
            peak = peak.max(pcm[0].abs());
        }
        peak
    }

    // Keep the potentially aborting stack-overflow check in a child process.
    // Shared's large fixed atomic mirror and all command payloads are prepared
    // on the control/test thread, then only small handles enter the audio-sized
    // thread. These are the real debug-profile callback paths, not a copy of the
    // publication implementation or a test that increases its stack allowance.
    fn run_small_stack_callback_paths(stack_bytes: usize) {
        let shared = Arc::new(Shared::default());
        let mut renderer = Renderer::new(48000).unwrap();
        let ram: Vec<u8> = (0..RAM).map(|address| (address * 37 + 11) as u8).collect();
        let mut timeline_dsp = vec![0; 128];
        timeline_dsp[0x6c] = 0x20;
        let commands: Vec<_> = [
            Method::Reset,
            Method::LoadMemory {
                address: 0,
                data: ram,
            },
            Method::Write {
                register: 0x6c,
                value: 0,
            },
            Method::PrepareTimeline {
                ram: vec![0xa5; RAM],
                dsp: timeline_dsp,
                events: vec![0, 1, 0x1234, 0x6e, 32, 1, 0xffff, 0x91],
                duration_clocks: 64000,
            },
            Method::PlayTimeline,
            Method::StopTimeline,
        ]
        .into_iter()
        .map(|method| {
            Message {
                id: None,
                audition: false,
                method,
            }
            .prepare()
            .unwrap()
        })
        .collect();
        shared.retain(&commands).unwrap();
        for phase in 0..9 {
            let callback_shared = Arc::clone(&shared);
            let callback_commands = commands.clone();
            let (next_renderer, completed_commands) = std::thread::Builder::new()
                .name(format!("snes-callback-stack-{stack_bytes}"))
                .stack_size(stack_bytes)
                .spawn(move || {
                    match phase {
                        0 => renderer.publish(&callback_shared),
                        1 | 7 => renderer.apply(&callback_commands[0], None, &callback_shared),
                        2 => renderer.apply(&callback_commands[1], None, &callback_shared),
                        3 => {
                            renderer.apply(&callback_commands[2], None, &callback_shared);
                            for _ in 0..256 {
                                renderer.mix(&mut [0.0; 2]);
                            }
                        }
                        4 => {
                            renderer.apply(&callback_commands[3], None, &callback_shared);
                            renderer.apply(&callback_commands[4], None, &callback_shared);
                            for _ in 0..256 {
                                renderer.mix(&mut [0.0; 2]);
                            }
                        }
                        5 => renderer.apply(&callback_commands[5], None, &callback_shared),
                        6 => renderer.stop(&callback_shared),
                        8 => {
                            renderer.apply(&callback_commands[1], None, &callback_shared);
                            renderer.stop(&callback_shared);
                        }
                        _ => unreachable!(),
                    }
                    renderer.publish(&callback_shared);
                    // Return variable-size storage to the control thread for
                    // reclamation, just as the live renderer's registry does.
                    (renderer, callback_commands)
                })
                .unwrap()
                .join()
                .unwrap();
            renderer = next_renderer;
            drop(completed_commands);
            let snapshot = shared.status_with_ram(phase).unwrap();
            let applied_ram = snapshot.ram.unwrap();
            for (address, byte) in applied_ram.iter().enumerate() {
                assert_eq!(
                    *byte,
                    renderer.memory_byte(address as u16),
                    "incoherent RAM in phase {phase} at {address}"
                );
            }
            match phase {
                1 | 7 => assert!(applied_ram.iter().all(|byte| *byte == 0)),
                2 | 8 => assert_eq!(applied_ram[0xffff], (0xffffusize * 37 + 11) as u8),
                4..=6 => {
                    assert!(snapshot.timeline.loaded);
                    assert_eq!(applied_ram[0x1234], 0x6e);
                    assert_eq!(applied_ram[0xffff], 0x91);
                    if phase >= 5 {
                        assert!(!snapshot.timeline.playing);
                    }
                }
                _ => {}
            }
        }
        eprintln!("SNES callback snapshot paths passed with {stack_bytes} stack bytes");
    }

    #[test]
    fn callback_publication_fits_audio_thread_stack() {
        const STACK_ENV: &str = "TETORICA_SNES_CALLBACK_STACK_BYTES";
        if let Some(value) = std::env::var_os(STACK_ENV) {
            let bytes: usize = value.to_str().unwrap().parse().unwrap();
            assert!([64 * 1024, 128 * 1024].contains(&bytes));
            run_small_stack_callback_paths(bytes);
            return;
        }
        for bytes in [64 * 1024, 128 * 1024] {
            let result = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "native_genesis::snes::tests::callback_publication_fits_audio_thread_stack",
                    "--nocapture",
                ])
                .env(STACK_ENV, bytes.to_string())
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&result.stdout);
            let stderr = String::from_utf8_lossy(&result.stderr);
            assert!(
                result.status.success(),
                "SNES callback failed at {bytes} stack bytes: {}\n{stdout}\n{stderr}",
                result.status
            );
            assert!(
                stderr.contains(&format!(
                    "SNES callback snapshot paths passed with {bytes} stack bytes"
                )),
                "The child did not execute the stack regression: {stdout}\n{stderr}"
            );
        }
    }
    #[test]
    fn validation_is_bounded_before_callback() {
        for value in [
            json!({"method":"write","register":128,"value":0}),
            json!({"method":"key-on","channel":8}),
            json!({"method":"load-memory","address":65536,"data":[1]}),
            json!({"method":"batch","channel":0,"action":"edit","commands":[{"method":"write","register":0x10,"value":1}]}),
            json!({"method":"batch","channel":0,"action":"edit","commands":[{"method":"write","register":0x6c,"value":0}]}),
        ] {
            let message: Message = serde_json::from_value(value).unwrap();
            assert!(message.prepare().is_err());
        }
        let message:Message=serde_json::from_value(json!({"method":"prepare-timeline","ram":vec![0;RAM],"dsp":vec![0;128],"events":[32,0,0,0,31,0,0,0],"durationClocks":64})).unwrap();
        assert!(message.prepare().is_err());
    }
    #[test]
    fn applied_ram_registers_and_id_are_published_without_default_ram_transfer() {
        let (shared, mut renderer) = setup();
        apply(
            &shared,
            &mut renderer,
            json!({"id":7,"method":"write","register":7,"value":99}),
            None,
        );
        shared.wait_applied(7).unwrap();
        let state = shared.status(123).unwrap();
        assert_eq!(state.clock, CLOCK);
        assert_eq!(state.frame, 123);
        assert_eq!(state.registers[7], 99);
        assert!(state.ram.is_none());
        assert_eq!(
            shared.status_with_ram(123).unwrap().ram.unwrap()[0x400],
            0xb3
        );
    }
    #[test]
    fn manual_lease_survives_foreign_key_masks_and_stale_release() {
        let (shared, mut renderer) = setup();
        let owner = Some(Owner {
            performance: 1,
            owner: 1,
        });
        apply(
            &shared,
            &mut renderer,
            json!({"method":"batch","channel":0,"voice":21,"audition":true,"action":"on","commands":[{"method":"key-on","channel":0}]}),
            None,
        );
        assert!(sample(&mut renderer, 1000) > 0.001);
        for value in [
            json!({"method":"write","register":7,"value":1}),
            json!({"method":"write","register":0x5c,"value":255}),
            json!({"method":"key-off","channel":0}),
        ] {
            apply(&shared, &mut renderer, value, owner);
        }
        assert_eq!(shared.status(0).unwrap().registers[7], 127);
        assert_eq!(shared.status(0).unwrap().enabled & 1, 1);
        apply(
            &shared,
            &mut renderer,
            json!({"method":"batch","channel":0,"voice":22,"audition":true,"action":"on","commands":[{"method":"key-on","channel":0}]}),
            None,
        );
        apply(
            &shared,
            &mut renderer,
            json!({"method":"batch","channel":0,"voice":21,"audition":true,"action":"off","commands":[{"method":"key-off","channel":0}]}),
            None,
        );
        assert_eq!(shared.status(0).unwrap().enabled & 1, 1);
        apply(
            &shared,
            &mut renderer,
            json!({"method":"batch","channel":0,"voice":22,"audition":true,"action":"off","commands":[{"method":"key-off","channel":0}]}),
            None,
        );
        assert_eq!(shared.status(0).unwrap().enabled & 1, 0);
    }
    #[test]
    fn stop_keeps_applied_brr_and_registers_reset_clears_ram() {
        let (shared, mut renderer) = setup();
        apply(
            &shared,
            &mut renderer,
            json!({"method":"key-on","channel":0}),
            None,
        );
        assert!(sample(&mut renderer, 2000) > 0.001);
        renderer.stop(&shared);
        let state = shared.status(0).unwrap();
        assert_eq!(state.enabled, 0);
        assert_eq!(state.registers[0x5c], 255);
        assert_eq!(state.registers[7], 127);
        assert_eq!(renderer.memory_byte(0x400), 0xb3);
        assert_eq!(sample(&mut renderer, 512), 0.0);
        apply(&shared, &mut renderer, json!({"method":"reset"}), None);
        assert_eq!(renderer.memory_byte(0x400), 0);
        assert_eq!(shared.status_with_ram(0).unwrap().ram.unwrap()[0x400], 0);
    }
    #[test]
    fn owner_cancel_releases_only_owned_voice() {
        let (shared, mut renderer) = setup();
        let owner = Some(Owner {
            performance: 3,
            owner: 1,
        });
        apply(
            &shared,
            &mut renderer,
            json!({"method":"key-on","channel":0}),
            owner,
        );
        apply(
            &shared,
            &mut renderer,
            json!({"method":"batch","channel":7,"voice":22,"audition":true,"action":"on","commands":[{"method":"key-on","channel":7}]}),
            None,
        );
        renderer.cancel_invalid(|value| value != owner, &shared);
        assert_eq!(shared.status(0).unwrap().enabled, 128);
    }
    #[test]
    fn captured_timeline_storage_survives_control_collection_pause_restart_and_completion() {
        let (shared, mut renderer) = setup();
        let state = shared.status_with_ram(0).unwrap();
        let mut registers = state.registers;
        registers[0x5c] = 0;
        apply(
            &shared,
            &mut renderer,
            json!({"method":"prepare-timeline","ram":state.ram.unwrap(),"dsp":registers,"events":[0,0,0x4c,1,333,1,0x402,255],"durationClocks":64000}),
            None,
        );
        assert!(shared.status(0).unwrap().timeline.loaded);
        assert!(!shared.status(0).unwrap().timeline.playing);
        assert_eq!(sample(&mut renderer, 100), 0.0);
        // status() has reclaimed the Prepared, but Renderer retains its borrowed
        // immutable timeline storage until it is replaced on the control queue.
        apply(
            &shared,
            &mut renderer,
            json!({"method":"play-timeline"}),
            None,
        );
        assert!(sample(&mut renderer, 1000) > 0.001);
        renderer.stop(&shared);
        let paused = shared.status(0).unwrap().timeline;
        assert!(paused.loaded && !paused.playing);
        assert_eq!(sample(&mut renderer, 100), 0.0);
        apply(
            &shared,
            &mut renderer,
            json!({"method":"play-timeline"}),
            None,
        );
        sample(&mut renderer, 4000);
        renderer.publish(&shared);
        let ended = shared.status(0).unwrap().timeline;
        assert!(ended.ended && !ended.playing);
        apply(&shared, &mut renderer, json!({"method":"reset"}), None);
        assert!(!shared.status(0).unwrap().timeline.loaded);
        assert!(renderer.timeline.is_none());
    }

    #[test]
    fn timeline_owner_cancellation_and_stale_stop_preserve_newer_playback() {
        let (shared, mut renderer) = setup();
        let state = shared.status_with_ram(0).unwrap();
        apply(
            &shared,
            &mut renderer,
            json!({"method":"prepare-timeline","ram":state.ram.unwrap(),"dsp":state.registers,"events":[],"durationClocks":64000}),
            None,
        );
        let old = Some(Owner {
            performance: 1,
            owner: 1,
        });
        let new = Some(Owner {
            performance: 1,
            owner: 2,
        });
        apply(
            &shared,
            &mut renderer,
            json!({"method":"play-timeline"}),
            old,
        );
        renderer.cancel_invalid(|owner| owner != old, &shared);
        assert!(!shared.status(0).unwrap().timeline.playing);
        apply(
            &shared,
            &mut renderer,
            json!({"method":"play-timeline"}),
            new,
        );
        for method in ["stop-timeline", "stop-voices"] {
            apply(&shared, &mut renderer, json!({"method":method}), old);
            assert!(
                shared.status(0).unwrap().timeline.playing,
                "stale scoped stop muted a newer timeline"
            );
        }
        apply(
            &shared,
            &mut renderer,
            json!({"method":"stop-timeline"}),
            new,
        );
        assert!(!shared.status(0).unwrap().timeline.playing);
        let message: Message = serde_json::from_value(json!({"method":"stop-timeline"})).unwrap();
        assert!(message.prepare().unwrap().stops());
    }
}
