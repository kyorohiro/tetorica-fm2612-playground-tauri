//! Optional Mega-CD PCM. Core, RAM and channel state belong to the device
//! callback. Memory uploads are retained and reclaimed on the control side.
use super::Owner;
use serde::Serialize;
use std::{
    ffi::c_void,
    sync::{
        atomic::{AtomicU16, AtomicU32, AtomicU64, AtomicU8, Ordering},
        Arc,
    },
};

pub const CLOCK: u32 = 12_500_000;
pub const RAM: usize = 65_536;
pub const PENDING_BYTES: usize = 8 * 1024 * 1024;
extern "C" {
    fn native_rf5c164_create(rate: u32) -> *mut c_void;
    fn native_rf5c164_destroy(chip: *mut c_void);
    fn native_rf5c164_reset(chip: *mut c_void);
    fn native_rf5c164_clear_memory(chip: *mut c_void);
    fn native_rf5c164_write(chip: *mut c_void, register: u8, value: u8);
    fn native_rf5c164_write_memory(chip: *mut c_void, offset: u16, value: u8);
    fn native_rf5c164_load(chip: *mut c_void, bytes: *const u8, offset: u32, size: u32) -> i32;
    fn native_rf5c164_sample(chip: *mut c_void, left: *mut f32, right: *mut f32);
    fn native_rf5c164_set_clock(chip: *mut c_void, clock: u32);
}

#[derive(Debug)]
pub struct Memory {
    pub bytes: Vec<u8>,
    pub offset: u32,
}
impl Memory {
    pub fn new(offset: u32, bytes: &[u8]) -> Result<Arc<Self>, String> {
        if offset as usize > RAM || bytes.len() > RAM - offset as usize {
            return Err("RF5C164 upload is outside 64 KiB RAM".into());
        }
        Ok(Arc::new(Self {
            bytes: bytes.to_vec(),
            offset,
        }))
    }
}

// A seqlock snapshot of applied register state. There is one writer (callback),
// readers copy atomics without locking or allocating on the audio thread.
pub struct Shared {
    revision: AtomicU64,
    registers: [[AtomicU8; 7]; 8],
    configured_steps: [AtomicU16; 8],
    control: AtomicU8,
    selected: AtomicU8,
    mask: AtomicU8,
    clock: AtomicU32,
}
impl Default for Shared {
    fn default() -> Self {
        Self {
            revision: AtomicU64::new(0),
            registers: std::array::from_fn(|_| std::array::from_fn(|_| AtomicU8::new(0))),
            configured_steps: std::array::from_fn(|_| AtomicU16::new(0)),
            control: AtomicU8::new(0),
            selected: AtomicU8::new(0),
            mask: AtomicU8::new(255),
            clock: AtomicU32::new(CLOCK),
        }
    }
}
#[derive(Debug, Serialize)]
pub struct Pan {
    left: u8,
    right: u8,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Channel {
    channel: u8,
    start: u32,
    loop_start: u16,
    step: u16,
    configured_step: u16,
    volume: u8,
    pan: Pan,
    active: bool,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    clock: u32,
    sample_rate: u32,
    frame: u64,
    enabled: bool,
    selected: u8,
    mask: u8,
    channels: Vec<Channel>,
}
impl Shared {
    pub fn clock(&self) -> u32 {
        self.clock.load(Ordering::Acquire)
    }
    fn publish(&self, renderer: &Renderer) {
        self.revision.fetch_add(1, Ordering::AcqRel);
        for (out, values) in self.registers.iter().zip(renderer.registers) {
            for (out, value) in out.iter().zip(values) {
                out.store(value, Ordering::Relaxed);
            }
        }
        for (out, value) in self.configured_steps.iter().zip(renderer.configured_steps) {
            out.store(value, Ordering::Relaxed);
        }
        self.control.store(renderer.control, Ordering::Relaxed);
        self.selected.store(renderer.selected, Ordering::Relaxed);
        self.mask.store(renderer.mask, Ordering::Relaxed);
        self.clock.store(renderer.clock, Ordering::Relaxed);
        self.revision.fetch_add(1, Ordering::Release);
    }
    pub fn status(&self, rate: u32, frame: u64) -> Result<Status, String> {
        for _ in 0..64 {
            let before = self.revision.load(Ordering::Acquire);
            if before & 1 != 0 {
                std::hint::spin_loop();
                continue;
            }
            let registers: [[u8; 7]; 8] = std::array::from_fn(|c| {
                std::array::from_fn(|r| self.registers[c][r].load(Ordering::Relaxed))
            });
            let configured_steps: [u16; 8] = std::array::from_fn(|channel| {
                self.configured_steps[channel].load(Ordering::Relaxed)
            });
            let control = self.control.load(Ordering::Relaxed);
            let selected = self.selected.load(Ordering::Relaxed);
            let mask = self.mask.load(Ordering::Relaxed);
            let clock = self.clock.load(Ordering::Relaxed);
            // Prevent data loads being reordered after the closing revision.
            std::sync::atomic::fence(Ordering::Acquire);
            if self.revision.load(Ordering::Relaxed) != before {
                continue;
            }
            return Ok(Status {
                clock,
                sample_rate: rate,
                frame,
                enabled: control & 128 != 0,
                selected,
                mask,
                channels: registers
                    .iter()
                    .enumerate()
                    .map(|(channel, r)| Channel {
                        channel: channel as u8,
                        start: (r[6] as u32) << 8,
                        loop_start: u16::from_le_bytes([r[4], r[5]]),
                        step: u16::from_le_bytes([r[2], r[3]]),
                        configured_step: configured_steps[channel],
                        volume: r[0],
                        pan: Pan {
                            left: r[1] & 15,
                            right: r[1] >> 4,
                        },
                        active: control & 128 != 0 && mask & (1 << channel) == 0,
                    })
                    .collect(),
            });
        }
        Err("Native RF5C164 state is changing; retry the snapshot".into())
    }
}

pub struct Renderer {
    core: *mut c_void,
    registers: [[u8; 7]; 8],
    configured_steps: [u16; 8],
    control: u8,
    selected: u8,
    mask: u8,
    clock: u32,
    owners: [Option<Owner>; 8],
    voices: [Option<u64>; 8],
    gains: [f32; 2],
    targets: [f32; 2],
    ramp: u32,
    rate: u32,
}
// Unique ownership moves into one device callback; no other thread accesses C++.
unsafe impl Send for Renderer {}
impl Renderer {
    pub fn new(rate: u32) -> Result<Self, String> {
        let core = unsafe { native_rf5c164_create(rate) };
        if core.is_null() {
            return Err("Could not allocate native RF5C164 core".into());
        }
        Ok(Self {
            core,
            registers: [[0; 7]; 8],
            configured_steps: [0; 8],
            control: 0,
            selected: 0,
            mask: 255,
            clock: CLOCK,
            owners: [None; 8],
            voices: [None; 8],
            gains: [1.0; 2],
            targets: [1.0; 2],
            ramp: 0,
            rate,
        })
    }
    pub fn write(
        &mut self,
        register: u8,
        value: u8,
        owner: Option<Owner>,
        audition: bool,
        shared: &Shared,
    ) {
        unsafe {
            native_rf5c164_write(self.core, register, value);
        }
        match register {
            0..=6 => {
                self.registers[self.selected as usize][register as usize] = value;
                if !audition && (register == 2 || register == 3) {
                    let step = &mut self.configured_steps[self.selected as usize];
                    *step = if register == 2 {
                        (*step & 0xff00) | value as u16
                    } else {
                        (*step & 0xff) | (value as u16) << 8
                    };
                }
            }
            7 => {
                self.control = value;
                if value & 64 != 0 {
                    self.selected = value & 7;
                }
            }
            8 => {
                for channel in 0..8 {
                    let bit = 1 << channel;
                    if value & bit != 0 {
                        self.owners[channel] = None;
                        self.voices[channel] = None;
                    } else if self.mask & bit != 0 {
                        self.owners[channel] = owner;
                        self.voices[channel] = None;
                    }
                }
                self.mask = value;
            }
            _ => {}
        }
        shared.publish(self);
    }
    pub fn key(
        &mut self,
        channel: u8,
        on: bool,
        owner: Option<Owner>,
        voice: Option<u64>,
        shared: &Shared,
    ) {
        if !on
            && (owner.is_some() && self.owners[channel as usize] != owner
                || voice.is_some() && self.voices[channel as usize] != voice)
        {
            return;
        }
        if on {
            self.write(8, self.mask | (1 << channel), None, false, shared);
        }
        let mask = if on {
            self.mask & !(1 << channel)
        } else {
            self.mask | (1 << channel)
        };
        self.write(8, mask, owner, false, shared);
        if on {
            self.owners[channel as usize] = owner;
            self.voices[channel as usize] = voice;
        }
    }
    pub fn reset(&mut self, shared: &Shared) {
        unsafe {
            native_rf5c164_reset(self.core);
        }
        self.registers = [[0; 7]; 8];
        self.configured_steps = [0; 8];
        self.control = 0;
        self.selected = 0;
        self.mask = 255;
        self.owners = [None; 8];
        self.voices = [None; 8];
        shared.publish(self);
    }
    pub fn stop(&mut self, shared: &Shared) {
        self.write(8, 255, None, false, shared);
    }
    pub fn cancel_invalid(&mut self, valid: impl Fn(Option<Owner>) -> bool, shared: &Shared) {
        let mut mask = self.mask;
        for channel in 0..8 {
            if self.owners[channel].is_some() && !valid(self.owners[channel]) {
                mask |= 1 << channel;
            }
        }
        if mask != self.mask {
            self.write(8, mask, None, false, shared);
        }
    }
    pub fn load(&mut self, memory: &Memory) {
        unsafe {
            native_rf5c164_load(
                self.core,
                memory.bytes.as_ptr(),
                memory.offset,
                memory.bytes.len() as u32,
            );
        }
    }
    pub fn write_memory(&mut self, offset: u16, value: u8) {
        unsafe {
            native_rf5c164_write_memory(self.core, offset, value);
        }
    }
    pub fn clear_memory(&mut self) {
        unsafe {
            native_rf5c164_clear_memory(self.core);
        }
    }
    pub fn set_clock(&mut self, clock: u32, shared: &Shared) {
        unsafe {
            native_rf5c164_set_clock(self.core, clock);
        }
        self.clock = clock;
        shared.publish(self);
    }
    pub fn set_mix(&mut self, gains: [f32; 2]) {
        self.targets = gains;
        self.ramp = (self.rate / 200).max(1);
    }
    pub fn mix(&mut self, pcm: &mut [f32; 2]) {
        if self.ramp != 0 {
            for channel in 0..2 {
                self.gains[channel] +=
                    (self.targets[channel] - self.gains[channel]) / self.ramp as f32;
            }
            self.ramp -= 1;
        }
        let mut left = 0.0;
        let mut right = 0.0;
        unsafe {
            native_rf5c164_sample(self.core, &mut left, &mut right);
        }
        pcm[0] += left * self.gains[0];
        pcm[1] += right * self.gains[1];
    }
}
impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe {
            native_rf5c164_destroy(self.core);
        }
    }
}
