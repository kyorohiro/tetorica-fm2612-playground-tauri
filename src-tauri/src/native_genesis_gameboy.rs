//! Existing browser DMG core, owned by the Native render callback. Variable-size
//! command data is retained and reclaimed on the control side, never in mix().
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
    fn native_gameboy_sample(ptr: *mut c_void, left: *mut f32, right: *mut f32);
    fn native_gameboy_get_state(ptr: *mut c_void, out: *mut CoreState);
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
        address: u8,
        value: u8,
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
    Batch(Vec<Op>, u8, Option<u64>, LeaseAction),
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
        Err("Native Game Boy address must be 0–47".into())
    } else {
        Ok(())
    }
}
impl Message {
    pub fn prepare(&self) -> Result<Arc<Prepared>, String> {
        if !valid_id(self.id) {
            return Err("Invalid Native Game Boy command ID".into());
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
            Method::StopVoices => Edit::Stop,
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
            Method::Batch {
                commands,
                channel: target,
                voice,
                action,
            } => {
                if *target > 3
                    || commands.len() > BATCH
                    || commands.is_empty()
                    || !valid_id(*voice)
                    || (self.audition && *action != LeaseAction::Edit && voice.is_none())
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
                Edit::Batch(commands.to_vec(), *target, *voice, *action)
            }
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
        let starts = |op: &Op| {
            let Op::Write { address, value } = op;
            trigger(*address, *value)
        };
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
    registers: [AtomicU8; 48],
    voice: [AtomicU8; 48],
    active: AtomicU8,
    clock: AtomicU64,
    prepared: Mutex<Vec<Arc<Prepared>>>,
    retired: ArrayQueue<Arc<Prepared>>,
    highest_id: AtomicU64,
}
impl Default for Shared {
    fn default() -> Self {
        Self {
            revision: AtomicU64::new(0),
            registers: std::array::from_fn(|_| AtomicU8::new(0)),
            voice: std::array::from_fn(|_| AtomicU8::new(0)),
            active: AtomicU8::new(0),
            clock: AtomicU64::new(CLOCK as u64),
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
    pub registers: Vec<u8>,
    pub voice: Vec<u8>,
    pub gates: [bool; 4],
    pub frame: u64,
    pub revision: u64,
    pub pending_commands: Vec<u64>,
}
impl Shared {
    pub fn clock(&self) -> u32 {
        self.clock.load(Ordering::Acquire) as u32
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
            .map_err(|_| "Native Game Boy control state is unavailable")?;
        self.collect(&mut registry);
        if commands.len() + registry.len() > CAPACITY
            || registry
                .iter()
                .chain(commands)
                .map(|c| c.bytes())
                .sum::<usize>()
                > PENDING_BYTES
        {
            return Err("Native Game Boy pending command limit reached (8 MiB pending)".into());
        }
        let mut ids = std::collections::HashSet::new();
        for command in registry.iter().chain(commands) {
            if command.id.is_some_and(|id| !ids.insert(id)) {
                return Err("Duplicate pending Native Game Boy command ID".into());
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
            return Err("Unknown Native Game Boy applied command ID".into());
        }
        let command = {
            let registry = self
                .prepared
                .lock()
                .map_err(|_| "Native Game Boy control state is unavailable")?;
            registry.iter().find(|c| c.id == Some(id)).cloned()
        };
        if let Some(command) = command {
            let start = std::time::Instant::now();
            while !command.complete.load(Ordering::Acquire) {
                if start.elapsed() >= std::time::Duration::from_millis(100) {
                    return Err("Native Game Boy command was not applied within 100 ms".into());
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
            .unwrap_or_else(|_| unreachable!("Bounded Game Boy command retirement"));
    }
    fn publish(&self, renderer: &Renderer, command: Option<&Prepared>) {
        let state = renderer.state();
        self.revision.fetch_add(1, Ordering::AcqRel);
        for ((out, voice), (value, configured)) in self
            .registers
            .iter()
            .zip(&self.voice)
            .zip(state.registers.into_iter().zip(renderer.voice))
        {
            out.store(value, Ordering::Relaxed);
            voice.store(configured, Ordering::Relaxed);
        }
        self.active
            .store(state.active_mask as u8, Ordering::Relaxed);
        self.clock.store(state.clock as u64, Ordering::Relaxed);
        if let Some(command) = command {
            command.complete.store(true, Ordering::Relaxed);
        }
        self.revision.fetch_add(1, Ordering::Release);
    }
    pub fn status(&self, frame: u64) -> Result<Status, String> {
        let mut registry = self
            .prepared
            .lock()
            .map_err(|_| "Native Game Boy control state is unavailable")?;
        self.collect(&mut registry);
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() >= std::time::Duration::from_millis(50) {
                return Err("Native Game Boy state changed while reading; retry".into());
            }
            let before = self.revision.load(Ordering::Acquire);
            if before & 1 != 0 {
                std::thread::yield_now();
                continue;
            }
            let active = self.active.load(Ordering::Relaxed);
            let status = Status {
                clock: self.clock.load(Ordering::Relaxed) as u32,
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
                gates: std::array::from_fn(|c| active & (1 << c) != 0),
                frame,
                revision: before / 2,
                pending_commands: registry
                    .iter()
                    .filter(|c| !c.complete.load(Ordering::Relaxed))
                    .filter_map(|c| c.id)
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
            owner.is_some() && lease.owner != owner || voice.is_some() && lease.voice != voice
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
        if address == 20 && manual != 0 {
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
            Edit::Stop => {
                if owner.is_none() {
                    unsafe {
                        native_gameboy_stop(self.ptr);
                    }
                    self.owners.fill(None);
                } else {
                    for index in 0..4 {
                        if self.owners[index].is_some_and(|lease| {
                            (!lease.audition || command.audition) && lease.owner == owner
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
                    if *action == LeaseAction::On {
                        self.owners[index] = Some(Lease {
                            owner,
                            voice: *voice,
                            audition: command.audition,
                        });
                    }
                    if *action == LeaseAction::Off {
                        self.owners[index] = None;
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
        for index in 0..4 {
            if self.owners[index].is_some_and(|lease| !valid(lease.owner)) {
                self.release(index);
                changed = true;
            }
        }
        if changed {
            shared.publish(self, None);
        }
    }
    pub fn stop(&mut self, shared: &Shared) {
        unsafe {
            native_gameboy_stop(self.ptr);
        }
        self.owners.fill(None);
        shared.publish(self, None);
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    fn apply(shared: &Shared, renderer: &mut Renderer, value: Value, owner: Option<Owner>) {
        let message: Message = serde_json::from_value(value).unwrap();
        let prepared = message.prepare().unwrap();
        shared.retain(&[prepared.clone()]).unwrap();
        renderer.apply(&prepared, owner, shared);
        shared.retire(prepared);
    }
    fn setup() -> (Shared, Renderer) {
        let shared = Shared::default();
        let mut renderer = Renderer::new(48000).unwrap();
        for (address, value) in [(22, 128), (20, 0x77), (21, 0xff)] {
            apply(
                &shared,
                &mut renderer,
                json!({"method":"write","address":address,"value":value}),
                None,
            );
        }
        renderer.publish(&shared);
        (shared, renderer)
    }
    fn on(channel: u8, voice: u64, audition: bool) -> Value {
        let base = channel * 5;
        json!({"method":"batch","channel":channel,"voice":voice,"audition":audition,"action":"on","commands":[{"method":"write","address":base+1,"value":128},{"method":"write","address":base+2,"value":160},{"method":"write","address":base+3,"value":34},{"method":"write","address":base+4,"value":135}]})
    }
    fn off(channel: u8, voice: u64, audition: bool) -> Value {
        json!({"method":"batch","channel":channel,"voice":voice,"audition":audition,"action":"off","commands":[{"method":"write","address":channel*5+2,"value":0}]})
    }
    fn owner(n: u32) -> Option<Owner> {
        Some(Owner {
            performance: 1,
            owner: n,
        })
    }
    #[test]
    fn gameboy_native_applied_status_voice_ram_stop_and_reset() {
        let (shared, mut renderer) = setup();
        apply(&shared, &mut renderer, on(0, 1, false), owner(1));
        apply(
            &shared,
            &mut renderer,
            json!({"method":"write","address":32,"value":18}),
            None,
        );
        apply(
            &shared,
            &mut renderer,
            json!({"method":"clock","hz":2097152}),
            None,
        );
        let mut peak = 0.0_f32;
        for _ in 0..1000 {
            let mut pcm = [0.0; 2];
            renderer.mix(&mut pcm);
            peak = peak.max(pcm[0].abs());
        }
        assert!(peak > 0.1);
        renderer.stop(&shared);
        let status = shared.status(1000).unwrap();
        assert_eq!(status.gates, [false; 4]);
        assert_eq!(status.voice[2], 160);
        assert_eq!(status.registers[2], 160);
        assert_eq!(status.registers[32], 18);
        assert_eq!(status.clock, 2097152);
        assert_eq!(status.registers[22] & 128, 128);
        for _ in 0..128 {
            let mut pcm = [0.0; 2];
            renderer.mix(&mut pcm);
            assert_eq!(pcm, [0.0; 2]);
        }
        apply(&shared, &mut renderer, json!({"method":"reset"}), None);
        let status = shared.status(0).unwrap();
        assert_eq!(status.registers[32], 0xac);
        assert_eq!(status.voice[2], 0);
        assert_eq!(status.clock, 2097152);
    }
    #[test]
    fn gameboy_native_manual_gates_and_global_controls_are_protected() {
        let (shared, mut renderer) = setup();
        apply(&shared, &mut renderer, on(0, 10, true), None);
        let initial = shared.status(0).unwrap();
        for value in [
            json!({"method":"write","address":2,"value":0}),
            json!({"method":"write","address":21,"value":0}),
            json!({"method":"write","address":20,"value":0}),
            json!({"method":"write","address":22,"value":0}),
            json!({"method":"reset"}),
        ] {
            apply(&shared, &mut renderer, value, owner(1));
        }
        let status = shared.status(0).unwrap();
        assert!(status.gates[0]);
        assert_eq!(status.voice[2], initial.voice[2]);
        assert_eq!(status.registers[21] & 0x11, 0x11);
        assert_eq!(status.registers[20], 0x77);
        assert_eq!(status.registers[22] & 128, 128);
        apply(&shared, &mut renderer, off(0, 9, true), None);
        assert!(shared.status(0).unwrap().gates[0]);
        apply(&shared, &mut renderer, off(0, 10, true), None);
        let status = shared.status(0).unwrap();
        assert!(!status.gates[0]);
        assert_eq!(status.registers[2], 0);
        assert_eq!(status.voice[2], 160);
    }
    #[test]
    fn gameboy_native_old_owner_off_and_poweroff_do_not_kill_new_owner() {
        let (shared, mut renderer) = setup();
        apply(&shared, &mut renderer, on(0, 1, false), owner(1));
        apply(&shared, &mut renderer, on(0, 2, false), owner(2));
        apply(&shared, &mut renderer, off(0, 1, false), owner(1));
        apply(
            &shared,
            &mut renderer,
            json!({"method":"write","address":2,"value":0}),
            owner(1),
        );
        apply(
            &shared,
            &mut renderer,
            json!({"method":"write","address":22,"value":0}),
            owner(1),
        );
        assert!(shared.status(0).unwrap().gates[0]);
        renderer.cancel_invalid(|current| current != owner(1), &shared);
        assert!(shared.status(0).unwrap().gates[0]);
        renderer.cancel_invalid(|current| current != owner(2), &shared);
        assert!(!shared.status(0).unwrap().gates[0]);
    }
    #[test]
    fn gameboy_native_wave_ram_respects_core_access_and_manual_lease() {
        let (shared, mut renderer) = setup();
        for (address, value) in [(10, 0), (32, 18), (12, 32), (10, 128), (13, 34), (14, 135)] {
            apply(
                &shared,
                &mut renderer,
                json!({"method":"write","address":address,"value":value}),
                None,
            );
        }
        apply(
            &shared,
            &mut renderer,
            json!({"method":"write","address":32,"value":255}),
            None,
        );
        assert_eq!(shared.status(0).unwrap().registers[32], 18);
        apply(
            &shared,
            &mut renderer,
            json!({"method":"batch","channel":2,"voice":5,"audition":true,"action":"on","commands":[{"method":"write","address":10,"value":128},{"method":"write","address":14,"value":135}]}),
            None,
        );
        apply(
            &shared,
            &mut renderer,
            json!({"method":"write","address":10,"value":0}),
            owner(1),
        );
        apply(
            &shared,
            &mut renderer,
            json!({"method":"write","address":32,"value":255}),
            owner(1),
        );
        assert_eq!(shared.status(0).unwrap().registers[32], 18);
        assert!(shared.status(0).unwrap().gates[2]);
    }
    #[test]
    fn gameboy_native_batch_bounds_duplicate_ids_and_retirement() {
        let (shared, mut renderer) = setup();
        for value in [
            json!({"method":"write","address":48,"value":0}),
            json!({"method":"clock","hz":99999}),
            json!({"method":"batch","channel":0,"action":"on","commands":[{"method":"write","address":7,"value":160}]}),
            json!({"method":"batch","channel":0,"action":"on","commands":[{"method":"write","address":22,"value":128}]}),
        ] {
            assert!(serde_json::from_value::<Message>(value)
                .unwrap()
                .prepare()
                .is_err());
        }
        let mut value = on(0, 4, false);
        value["id"] = json!(41);
        let prepared = serde_json::from_value::<Message>(value)
            .unwrap()
            .prepare()
            .unwrap();
        assert!(prepared.starts());
        shared.retain(&[prepared.clone()]).unwrap();
        assert!(shared.retain(&[prepared.clone()]).is_err());
        assert_eq!(shared.status(0).unwrap().pending_commands, vec![41]);
        renderer.apply(&prepared, owner(1), &shared);
        shared.wait_applied(41).unwrap();
        shared.retire(prepared.clone());
        assert!(shared.status(0).unwrap().pending_commands.is_empty());
        assert_eq!(Arc::strong_count(&prepared), 1);
    }
}
