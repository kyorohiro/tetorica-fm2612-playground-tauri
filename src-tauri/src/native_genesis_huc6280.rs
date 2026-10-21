//! Callback-owned PC Engine PSG using the exact Web WASM HuC6280 DSP.
//! Variable-size batches stay retained on the control thread until retirement.
use super::{Owner, CAPACITY, MAX_SAFE_INTEGER};
use crossbeam_queue::ArrayQueue;
use serde::{Deserialize, Serialize};
use std::{
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
    Batch(Vec<Op>, u8, Option<u64>, LeaseAction),
}
#[derive(Debug)]
pub struct Prepared {
    pub id: Option<u64>,
    audition: bool,
    edit: Edit,
    complete: AtomicBool,
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
    pub fn prepare(&self) -> Result<Arc<Prepared>, String> {
        if !valid_id(self.id) {
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
            Method::StopVoices => Edit::Stop,
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
            edit,
            complete: AtomicBool::new(false),
        }))
    }
}
impl Prepared {
    pub fn starts(&self) -> bool {
        fn starts(op: &Op) -> bool {
            matches!(op, Op::Write { register: 4, value, .. } if value & 128 != 0)
        }
        match &self.edit {
            Edit::Op(op) => starts(op),
            Edit::Batch(ops, _, _, action) => *action == LeaseAction::On || ops.iter().any(starts),
            _ => false,
        }
    }
    pub fn stops(&self) -> bool {
        matches!(self.edit, Edit::Stop)
    }
    fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + match &self.edit {
                Edit::Batch(ops, ..) => ops.len() * std::mem::size_of::<Op>(),
                _ => 0,
            }
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
    prepared: Mutex<Vec<Arc<Prepared>>>,
    retired: ArrayQueue<Arc<Prepared>>,
    highest_id: AtomicU64,
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
            prepared: Mutex::new(Vec::new()),
            retired: ArrayQueue::new(CAPACITY),
            highest_id: AtomicU64::new(0),
        }
    }
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub clock: u32,
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
    pub frame: u64,
    pub revision: u64,
    pub pending_commands: Vec<u64>,
}
impl Shared {
    pub fn clock(&self) -> u32 {
        self.clock.load(Ordering::Acquire)
    }
    fn collect(&self, registry: &mut Vec<Arc<Prepared>>) {
        let mut retired = std::collections::HashSet::new();
        while let Some(command) = self.retired.pop() {
            retired.insert(Arc::as_ptr(&command) as usize);
        }
        registry.retain(|command| !retired.contains(&(Arc::as_ptr(command) as usize)));
    }
    pub fn retain(&self, commands: &[Arc<Prepared>]) -> Result<(), String> {
        let mut registry = self
            .prepared
            .lock()
            .map_err(|_| "Native HuC6280 control state unavailable")?;
        self.collect(&mut registry);
        if commands.len() + registry.len() > CAPACITY
            || registry
                .iter()
                .chain(commands)
                .map(|v| v.bytes())
                .sum::<usize>()
                > PENDING_BYTES
        {
            return Err("Native HuC6280 pending command limit reached (8 MiB)".into());
        }
        let mut ids = std::collections::HashSet::new();
        for command in registry.iter().chain(commands) {
            if command.id.is_some_and(|id| !ids.insert(id)) {
                return Err("Duplicate pending Native HuC6280 command ID".into());
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
            return Err("Unknown Native HuC6280 command ID".into());
        }
        let command = self
            .prepared
            .lock()
            .map_err(|_| "Native HuC6280 control state unavailable")?
            .iter()
            .find(|v| v.id == Some(id))
            .cloned();
        if let Some(command) = command {
            let start = std::time::Instant::now();
            while !command.complete.load(Ordering::Acquire) {
                if start.elapsed() >= std::time::Duration::from_millis(100) {
                    return Err("Native HuC6280 command was not applied within 100 ms".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        Ok(())
    }
    pub fn retire(&self, command: Arc<Prepared>) {
        if !command.complete.load(Ordering::Relaxed) {
            self.revision.fetch_add(1, Ordering::AcqRel);
            command.complete.store(true, Ordering::Relaxed);
            self.revision.fetch_add(1, Ordering::Release);
        }
        self.retired
            .push(command)
            .unwrap_or_else(|_| unreachable!("Bounded HuC6280 command retirement"));
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
        if let Some(command) = command {
            command.complete.store(true, Ordering::Relaxed);
        }
        self.revision.fetch_add(1, Ordering::Release);
    }
    pub fn status(&self, frame: u64) -> Result<Status, String> {
        let mut registry = self
            .prepared
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
                clock: self.clock.load(Ordering::Relaxed),
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
                frame,
                revision: before / 2,
                pending_commands: registry
                    .iter()
                    .filter(|v| !v.complete.load(Ordering::Relaxed))
                    .filter_map(|v| v.id)
                    .collect(),
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
    owner: Option<Owner>,
    voice: Option<u64>,
    audition: bool,
}
pub struct Renderer {
    ptr: *mut c_void,
    owners: [Option<Lease>; 8],
    gains: [f32; 2],
    targets: [f32; 2],
    ramp: u32,
    rate: u32,
}
unsafe impl Send for Renderer {}
impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe {
            native_huc6280_destroy(self.ptr);
        }
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
            owners: [None; 8],
            gains: [1.0; 2],
            targets: [1.0; 2],
            ramp: 0,
            rate,
        })
    }
    fn state(&self) -> CoreState {
        let mut state = CoreState::default();
        unsafe {
            native_huc6280_get_state(self.ptr, &mut state);
        }
        state
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
    fn write(
        &mut self,
        op: &Op,
        owner: Option<Owner>,
        voice: Option<u64>,
        audition: bool,
        preserve: bool,
    ) {
        let channel = self.channel(op);
        if channel.is_some_and(|ch| {
            self.owners[ch]
                .is_some_and(|lease| lease.audition && (!audition || lease.voice != voice))
        }) {
            return;
        }
        let off = matches!(op, Op::KeyOff { .. })
            || matches!(op, Op::Write { register: 4, value, .. } if value & 128 == 0);
        if off
            && channel.is_some_and(|ch| {
                self.owners[ch].is_some_and(|lease| {
                    owner.is_some() && lease.owner != owner
                        || voice.is_some() && lease.voice != voice
                })
            })
        {
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
            if let Some(ch) = channel {
                if matches!(op, Op::Write { register: 4, .. } | Op::KeyOff { .. }) {
                    self.owners[ch] = if off {
                        None
                    } else {
                        Some(Lease {
                            owner,
                            voice,
                            audition,
                        })
                    };
                }
            }
        }
    }
    fn release(&mut self, channel: usize) {
        unsafe {
            native_huc6280_key_off(self.ptr, channel as u32);
        }
        self.owners[channel] = None;
    }
    pub fn apply(&mut self, command: &Prepared, owner: Option<Owner>, shared: &Shared) {
        match &command.edit {
            Edit::Op(op) => self.write(op, owner, None, command.audition, false),
            Edit::Reset => {
                if command.audition || !self.owners.iter().flatten().any(|lease| lease.audition) {
                    unsafe {
                        native_huc6280_reset(self.ptr);
                    }
                    self.owners.fill(None);
                }
            }
            Edit::Stop => {
                if owner.is_none() {
                    unsafe {
                        native_huc6280_stop(self.ptr);
                    }
                    self.owners.fill(None);
                } else {
                    for ch in 0..8 {
                        if self.owners[ch].is_some_and(|lease| {
                            lease.owner == owner && (!lease.audition || command.audition)
                        }) {
                            self.release(ch);
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
                    LeaseAction::On => {
                        command.audition || current.is_none_or(|lease| !lease.audition)
                    }
                    LeaseAction::Off => current.is_some_and(|lease| {
                        lease.owner == owner
                            && (voice.is_none() || lease.voice == *voice)
                            && (!lease.audition || command.audition)
                    }),
                    LeaseAction::Edit => current.is_none_or(|lease| {
                        !lease.audition || command.audition && lease.voice == *voice
                    }),
                };
                if allowed {
                    if *action == LeaseAction::On {
                        self.owners[ch] = Some(Lease {
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
                    if *action == LeaseAction::On {
                        self.owners[ch] = Some(Lease {
                            owner,
                            voice: *voice,
                            audition: command.audition,
                        });
                    }
                    if *action == LeaseAction::Off {
                        self.owners[ch] = None;
                    }
                }
            }
        }
        shared.publish(self, Some(command));
    }
    pub fn publish(&self, shared: &Shared) {
        shared.publish(self, None);
    }
    pub fn cancel_invalid(&mut self, valid: impl Fn(Option<Owner>) -> bool, shared: &Shared) {
        let mut changed = false;
        for ch in 0..8 {
            if self.owners[ch].is_some_and(|lease| !valid(lease.owner)) {
                self.release(ch);
                changed = true;
            }
        }
        if changed {
            shared.publish(self, None);
        }
    }
    pub fn stop(&mut self, shared: &Shared) {
        unsafe {
            native_huc6280_stop(self.ptr);
        }
        self.owners.fill(None);
        shared.publish(self, None);
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    fn prepare(value: Value) -> Arc<Prepared> {
        serde_json::from_value::<Message>(value)
            .unwrap()
            .prepare()
            .unwrap()
    }
    fn apply(renderer: &mut Renderer, shared: &Shared, value: Value, owner: Option<Owner>) {
        let command = prepare(value);
        shared.retain(&[command.clone()]).unwrap();
        renderer.apply(&command, owner, shared);
        shared.retire(command);
    }
    fn state(shared: &Shared) -> Status {
        shared.status(0).unwrap()
    }
    fn note(voice: u64, audition: bool) -> Value {
        json!({"method":"batch","channel":0,"voice":voice,"audition":audition,"action":"on","commands":[
            {"method":"write","register":2,"value":253},
            {"method":"write","register":3,"value":0},
            {"method":"write","register":5,"value":255},
            {"method":"write","register":4,"value":159}]})
    }
    #[test]
    fn huc6280_native_preparation_rejects_invalid_controls_without_partial_batches() {
        for value in [
            json!({"method":"write","register":10,"value":0}),
            json!({"method":"write","register":4,"value":0,"channel":8}),
            json!({"method":"clock","hz":99999}),
            json!({"method":"mix","volume":5,"pan":0}),
            json!({"method":"waveform","channel":0,"samples":vec![32;32]}),
            json!({"method":"batch","channel":0,"action":"edit","commands":[{"method":"write","register":2,"value":1,"channel":1}]}),
            json!({"method":"batch","channel":0,"action":"on","audition":true,"commands":[{"method":"write","register":4,"value":159}]}),
        ] {
            let command: Message = serde_json::from_value(value).unwrap();
            assert!(command.prepare().is_err());
        }
        assert!(prepare(json!({"method":"write","register":4,"value":159,"channel":7})).starts());
    }
    #[test]
    fn huc6280_native_applied_state_targets_channels_and_preserves_raw_unused_latch() {
        let shared = Shared::default();
        let mut renderer = Renderer::new(48000).unwrap();
        renderer.publish(&shared);
        apply(
            &mut renderer,
            &shared,
            json!({"method":"write","register":0,"value":7}),
            None,
        );
        apply(
            &mut renderer,
            &shared,
            json!({"method":"write","register":4,"value":95,"channel":7}),
            None,
        );
        apply(
            &mut renderer,
            &shared,
            json!({"method":"write","register":6,"value":29,"channel":7}),
            None,
        );
        apply(
            &mut renderer,
            &shared,
            json!({"method":"write","register":2,"value":177,"channel":2}),
            None,
        );
        let s = state(&shared);
        assert_eq!(s.selected_channel, 7);
        assert_eq!(s.globals[0], 7);
        assert_eq!(s.registers[2][2], 177);
        assert_eq!(s.dda_values[7], 29);
        apply(
            &mut renderer,
            &shared,
            json!({"method":"waveform","channel":2,"samples":(0..32).collect::<Vec<_>>()}),
            None,
        );
        assert_eq!(state(&shared).waveforms[2], (0..32).collect::<Vec<u8>>());
        assert_eq!(state(&shared).selected_channel, 7);
    }
    #[test]
    fn huc6280_native_manual_lease_blocks_stale_writes_and_wrong_voice_off() {
        let shared = Shared::default();
        let mut renderer = Renderer::new(48000).unwrap();
        let owner = Some(Owner {
            performance: 1,
            owner: 1,
        });
        apply(&mut renderer, &shared, note(11, true), None);
        apply(
            &mut renderer,
            &shared,
            json!({"method":"write","register":2,"value":9,"channel":0}),
            owner,
        );
        apply(
            &mut renderer,
            &shared,
            json!({"method":"key-off","channel":0}),
            owner,
        );
        apply(
            &mut renderer,
            &shared,
            json!({"method":"batch","channel":0,"voice":10,"audition":true,"action":"off","commands":[{"method":"key-off","channel":0}]}),
            None,
        );
        assert_eq!(state(&shared).registers[0][2], 253);
        assert_eq!(state(&shared).registers[0][4], 159);
        apply(
            &mut renderer,
            &shared,
            json!({"method":"batch","channel":0,"voice":11,"audition":true,"action":"edit","commands":[{"method":"write","register":2,"value":171}]}),
            None,
        );
        assert_eq!(state(&shared).registers[0][2], 171);
        apply(
            &mut renderer,
            &shared,
            json!({"method":"batch","channel":0,"voice":11,"audition":true,"action":"off","commands":[{"method":"key-off","channel":0}]}),
            None,
        );
        assert_eq!(state(&shared).registers[0][4], 31);
    }
    #[test]
    fn huc6280_native_stop_retains_ram_settings_clock_and_reset_clears_ram() {
        let shared = Shared::default();
        let mut renderer = Renderer::new(44100).unwrap();
        apply(
            &mut renderer,
            &shared,
            json!({"method":"waveform","channel":0,"samples":(0..32).collect::<Vec<_>>()}),
            None,
        );
        apply(
            &mut renderer,
            &shared,
            json!({"method":"write","register":1,"value":250}),
            None,
        );
        apply(
            &mut renderer,
            &shared,
            json!({"method":"clock","hz":7159090}),
            None,
        );
        apply(&mut renderer, &shared, note(1, false), None);
        let mut pcm = [0.0; 2];
        let mut peak = 0.0f32;
        for _ in 0..1024 {
            pcm.fill(0.0);
            renderer.mix(&mut pcm);
            peak = peak.max(pcm[0].abs());
        }
        assert!(peak > 0.01);
        renderer.stop(&shared);
        let s = state(&shared);
        assert_eq!(s.registers[0][4], 31);
        assert_eq!(s.waveforms[0], (0..32).collect::<Vec<u8>>());
        assert_eq!(s.clock, 7159090);
        assert_eq!(s.globals[1], 250);
        apply(&mut renderer, &shared, json!({"method":"reset"}), None);
        let s = state(&shared);
        assert_eq!(s.waveforms[0], vec![0; 32]);
        assert_eq!(s.globals[1], 0);
        assert_eq!(s.clock, 7159090);
    }
    #[test]
    fn huc6280_native_cancel_is_scoped_and_status_ids_retire_on_control_thread() {
        let shared = Shared::default();
        let mut renderer = Renderer::new(48000).unwrap();
        let owner = Some(Owner {
            performance: 3,
            owner: 4,
        });
        let command = prepare(json!({"method":"write","register":1,"value":255,"id":1}));
        shared.retain(&[command.clone()]).unwrap();
        assert_eq!(state(&shared).pending_commands, vec![1]);
        renderer.apply(&command, None, &shared);
        shared.wait_applied(1).unwrap();
        shared.retire(command);
        assert!(state(&shared).pending_commands.is_empty());
        apply(&mut renderer, &shared, note(1, false), owner);
        renderer.cancel_invalid(|lease| lease != owner, &shared);
        assert_eq!(state(&shared).registers[0][4], 31);
        apply(&mut renderer, &shared, note(2, true), None);
        apply(
            &mut renderer,
            &shared,
            json!({"method":"stop-voices"}),
            owner,
        );
        assert_eq!(state(&shared).registers[0][4], 159);
        renderer.stop(&shared);
        assert_eq!(state(&shared).registers[0][4], 31);
        assert!(shared.wait_applied(2).is_err());
    }
}
