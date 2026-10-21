//! Shared-C continuous noise, with fixed native resource identities. Logical
//! controller slots are namespaced; only explicit disposal or global Stop
//! releases a lease. Release/cancel preserves RNG and parameters for restart.
use serde::Deserialize;
use std::{
    ffi::c_void,
    sync::atomic::{AtomicU64, Ordering},
};
pub const VOICES: usize = 32;
const MAX_SAFE: u64 = 9_007_199_254_740_991;
#[derive(Debug, Deserialize)]
pub struct Message {
    op: String,
    #[serde(default)]
    namespace: u64,
    slot: u8,
    token: u64,
    #[serde(flatten)]
    action: MessageAction,
}
#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "kebab-case")]
enum MessageAction {
    Create {
        #[serde(rename = "type")]
        kind: u8,
        seed: u32,
    },
    Parameter {
        param: u8,
        value: f64,
        #[serde(default)]
        seconds: f64,
    },
    Filter {
        mode: u8,
    },
    Start,
    Stop,
    Dispose,
    Retire,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Key {
    namespace: u64,
    slot: u8,
    token: u64,
}
#[derive(Clone, Copy, Debug)]
struct Creation {
    kind: u8,
    seed: u32,
    parameters: [f64; 6],
    mode: u8,
}
#[derive(Clone, Copy)]
struct Lease {
    key: Key,
    generation: u64,
    creation: Creation,
    started: bool,
}
#[derive(Clone)]
pub struct Control {
    leases: [Option<Lease>; VOICES],
    sequence: u64,
}
impl Default for Control {
    fn default() -> Self {
        Self {
            leases: [None; VOICES],
            sequence: 0,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub enum Operation {
    Create { kind: u8, seed: u32 },
    Parameter { param: u8, value: f64, frames: i32 },
    Filter(u8),
    Start,
    Stop,
    Dispose,
    Retire,
}
#[derive(Clone, Copy, Debug)]
pub struct Prepared {
    slot: usize,
    generation: u64,
    operation: Operation,
    creation: Creation,
}
impl Prepared {
    pub fn starts(&self) -> bool {
        matches!(self.operation, Operation::Start)
    }
}
impl Control {
    #[cfg(test)]
    pub fn reserved(&self) -> usize {
        self.leases.iter().flatten().count()
    }

    pub fn collect(&mut self, completed: &[AtomicU64; VOICES]) {
        for (slot, lease) in self.leases.iter_mut().enumerate() {
            if lease
                .is_some_and(|lease| completed[slot].load(Ordering::Acquire) >= lease.generation)
            {
                *lease = None;
            }
        }
    }
    pub fn clear(&mut self) {
        self.leases.fill(None);
    }
    pub fn prepare(&mut self, message: &Message, rate: u32) -> Result<Option<Prepared>, String> {
        if message.op != "noise"
            || message.namespace > MAX_SAFE
            || message.slot >= VOICES as u8
            || message.token == 0
            || message.token > MAX_SAFE
        {
            return Err("Invalid native noise controller identity".into());
        }
        let operation = match message.action {
            MessageAction::Create { kind, seed } => {
                if kind > 4 {
                    return Err("Invalid native noise type".into());
                }
                Operation::Create {
                    kind,
                    seed: if seed == 0 {
                        message.slot as u32 + 1
                    } else {
                        seed
                    },
                }
            }
            MessageAction::Parameter {
                param,
                value,
                seconds,
            } => {
                let ranges = [
                    (0.0, 8.0),
                    (-1.0, 1.0),
                    (10.0, 20000.0),
                    (0.0001, 1000.0),
                    (0.0, 60.0),
                    (0.0, 60.0),
                ];
                let Some(&(low, high)) = ranges.get(param as usize) else {
                    return Err("Invalid native noise parameter".into());
                };
                if !value.is_finite()
                    || value < low
                    || value > high
                    || !seconds.is_finite()
                    || seconds < 0.0
                    || seconds * rate as f64 > i32::MAX as f64
                {
                    return Err("Invalid native noise value or ramp".into());
                }
                Operation::Parameter {
                    param,
                    value,
                    frames: (seconds * rate as f64).round() as i32,
                }
            }
            MessageAction::Filter { mode } => {
                if mode > 7 {
                    return Err("Invalid native noise filter mode".into());
                }
                Operation::Filter(mode)
            }
            MessageAction::Start => Operation::Start,
            MessageAction::Stop => Operation::Stop,
            MessageAction::Dispose => Operation::Dispose,
            MessageAction::Retire => Operation::Retire,
        };
        let key = Key {
            namespace: message.namespace,
            slot: message.slot,
            token: message.token,
        };
        let current = self
            .leases
            .iter()
            .position(|lease| lease.is_some_and(|lease| lease.key == key));
        let slot = if let Some(slot) = current {
            slot
        } else if matches!(operation, Operation::Create { .. }) {
            let slot = self
                .leases
                .iter()
                .position(Option::is_none)
                .ok_or("Native noise supports at most 32 undisposed or future voices")?;
            self.sequence = self
                .sequence
                .checked_add(1)
                .ok_or("Native noise generation limit reached")?;
            let Operation::Create { kind, seed } = operation else {
                unreachable!()
            };
            self.leases[slot] = Some(Lease {
                key,
                generation: self.sequence,
                creation: Creation {
                    kind,
                    seed,
                    parameters: [0.3, 0.0, 1600.0, 0.2, 0.0, 0.0],
                    mode: 0,
                },
                started: false,
            });
            slot
        } else {
            return Ok(None);
        };
        let lease = self.leases[slot].as_mut().unwrap();
        // The first accepted Start closes the preparation phase. Capture all
        // factory/pre-start settings even when IPC timestamps span frames;
        // subsequent automation retains its scheduled audio time.
        if !lease.started {
            match operation {
                Operation::Parameter { param, value, .. } => {
                    lease.creation.parameters[param as usize] = value
                }
                Operation::Filter(mode) => lease.creation.mode = mode,
                _ => {}
            }
        }
        if matches!(operation, Operation::Start) {
            lease.started = true;
        }
        Ok(Some(Prepared {
            slot,
            generation: self.leases[slot].unwrap().generation,
            operation,
            creation: self.leases[slot].unwrap().creation,
        }))
    }
}
extern "C" {
    fn native_genesis_noise_create(rate: u32) -> *mut c_void;
    fn native_genesis_noise_destroy(ptr: *mut c_void);
    fn native_genesis_noise_reset(ptr: *mut c_void);
    fn native_genesis_noise_voice_create(ptr: *mut c_void, slot: i32, kind: i32, seed: u32) -> i32;
    fn native_genesis_noise_set(
        ptr: *mut c_void,
        slot: i32,
        param: i32,
        value: f64,
        frames: i32,
    ) -> i32;
    fn native_genesis_noise_filter(ptr: *mut c_void, slot: i32, mode: i32);
    fn native_genesis_noise_action(ptr: *mut c_void, slot: i32, action: i32);
    fn native_genesis_noise_mix(ptr: *mut c_void, stereo: *mut f32);
    fn native_genesis_noise_active(ptr: *mut c_void, slot: i32) -> i32;
}
#[derive(Clone, Copy, Default)]
struct Slot {
    generation: u64,
    used: bool,
    terminal: bool,
    retiring: bool,
    owner: Option<super::Owner>,
}
pub struct Renderer {
    pointer: *mut c_void,
    slots: [Slot; VOICES],
    used: usize,
}
// Unique instance ownership moves to one audio callback; C has no threads.
unsafe impl Send for Renderer {}
impl Renderer {
    pub fn new(rate: u32) -> Result<Self, String> {
        let pointer = unsafe { native_genesis_noise_create(rate) };
        if pointer.is_null() {
            return Err("Could not allocate native noise voices".into());
        }
        Ok(Self {
            pointer,
            slots: [Slot::default(); VOICES],
            used: 0,
        })
    }
    pub fn apply(
        &mut self,
        command: Prepared,
        owner: Option<super::Owner>,
        completed: &[AtomicU64; VOICES],
    ) {
        let slot = &mut self.slots[command.slot];
        if let Operation::Create { kind, seed } = command.operation {
            if command.generation < slot.generation
                || (command.generation == slot.generation
                    && (slot.terminal || slot.retiring || slot.used))
            {
                return;
            }
            if !slot.used {
                self.used += 1;
            }
            *slot = Slot {
                generation: command.generation,
                used: true,
                terminal: false,
                retiring: false,
                owner,
            };
            unsafe {
                native_genesis_noise_voice_create(
                    self.pointer,
                    command.slot as i32,
                    kind as i32,
                    seed,
                );
            };
            return;
        }
        if command.generation > slot.generation
            && matches!(command.operation, Operation::Dispose | Operation::Retire)
        {
            // Disposal can arrive before a future Create that was cancelled
            // with its loop. Terminalize that reservation without starting it.
            *slot = Slot {
                generation: command.generation,
                terminal: true,
                ..Slot::default()
            };
            completed[command.slot].fetch_max(command.generation, Ordering::Release);
            return;
        }
        if command.generation > slot.generation
            && matches!(
                command.operation,
                Operation::Start | Operation::Parameter { .. } | Operation::Filter(_)
            )
        {
            // A reservation may outlive the loop that scheduled its Create.
            // A valid new owner can initialize that prepared resource without
            // admitting any cancelled owner's original audio commands.
            unsafe {
                native_genesis_noise_voice_create(
                    self.pointer,
                    command.slot as i32,
                    command.creation.kind as i32,
                    command.creation.seed,
                );
                for (param, value) in command.creation.parameters.iter().enumerate() {
                    native_genesis_noise_set(
                        self.pointer,
                        command.slot as i32,
                        param as i32,
                        *value,
                        0,
                    );
                }
                native_genesis_noise_filter(
                    self.pointer,
                    command.slot as i32,
                    command.creation.mode as i32,
                );
            }
            *slot = Slot {
                generation: command.generation,
                used: true,
                ..Slot::default()
            };
            self.used += 1;
        }
        if !slot.used || slot.generation != command.generation {
            return;
        }
        if matches!(
            command.operation,
            Operation::Stop | Operation::Dispose | Operation::Retire
        ) && owner.is_some()
            && slot.owner != owner
        {
            return;
        }
        if slot.retiring && !matches!(command.operation, Operation::Dispose) {
            return;
        }
        unsafe {
            match command.operation {
                Operation::Parameter {
                    param,
                    value,
                    frames,
                } => {
                    native_genesis_noise_set(
                        self.pointer,
                        command.slot as i32,
                        param as i32,
                        value,
                        frames,
                    );
                }
                Operation::Filter(mode) => {
                    native_genesis_noise_filter(self.pointer, command.slot as i32, mode as i32)
                }
                Operation::Start => {
                    native_genesis_noise_action(self.pointer, command.slot as i32, 0);
                    slot.owner = owner;
                }
                Operation::Stop => {
                    native_genesis_noise_action(self.pointer, command.slot as i32, 1)
                }
                Operation::Retire => {
                    native_genesis_noise_action(self.pointer, command.slot as i32, 1);
                    slot.retiring = true;
                }
                Operation::Dispose => {
                    native_genesis_noise_action(self.pointer, command.slot as i32, 2);
                    slot.used = false;
                    slot.terminal = true;
                    slot.owner = None;
                    self.used -= 1;
                    completed[command.slot].fetch_max(command.generation, Ordering::Release);
                }
                Operation::Create { .. } => unreachable!(),
            }
        }
    }
    pub fn cancel_invalid(&mut self, valid: impl Fn(Option<super::Owner>) -> bool) {
        for (index, slot) in self.slots.iter_mut().enumerate() {
            if slot.used && slot.owner.is_some() && !valid(slot.owner) {
                unsafe { native_genesis_noise_action(self.pointer, index as i32, 1) };
                slot.owner = None;
            }
        }
    }
    pub fn reset(&mut self, completed: &[AtomicU64; VOICES]) {
        unsafe { native_genesis_noise_reset(self.pointer) };
        self.used = 0;
        for (index, slot) in self.slots.iter_mut().enumerate() {
            if slot.generation != 0 {
                completed[index].fetch_max(slot.generation, Ordering::Release);
            }
            slot.used = false;
            slot.terminal = true;
            slot.retiring = false;
            slot.owner = None;
        }
    }
    pub fn mix(&mut self, pcm: &mut [f32; 2], completed: &[AtomicU64; VOICES]) {
        if self.used > 0 {
            unsafe { native_genesis_noise_mix(self.pointer, pcm.as_mut_ptr()) }
            for (index, slot) in self.slots.iter_mut().enumerate() {
                if slot.used
                    && slot.retiring
                    && unsafe { native_genesis_noise_active(self.pointer, index as i32) } == 0
                {
                    unsafe { native_genesis_noise_action(self.pointer, index as i32, 2) };
                    slot.used = false;
                    slot.terminal = true;
                    slot.retiring = false;
                    slot.owner = None;
                    self.used -= 1;
                    completed[index].fetch_max(slot.generation, Ordering::Release);
                }
            }
        }
    }
}
impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe { native_genesis_noise_destroy(self.pointer) }
    }
}
