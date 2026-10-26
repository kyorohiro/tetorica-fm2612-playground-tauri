//! Callback-owned WonderSwan using the fixed SDK raw core and Browser speaker boundary.
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

pub const CLOCK: u32 = 3_072_000;
const BATCH: usize = 512;
const PENDING_BYTES: usize = 8 * 1024 * 1024;
extern "C" {
    fn native_wonderswan_core_size() -> usize;
    fn native_wonderswan_create(rate: u32, clock: u32) -> *mut c_void;
    fn native_wonderswan_set_mute_mask(ptr: *mut c_void, mask: u32);
    fn native_wonderswan_destroy(ptr: *mut c_void);
    fn native_wonderswan_reset(ptr: *mut c_void);
    fn native_wonderswan_stop(ptr: *mut c_void);
    fn native_wonderswan_write(ptr: *mut c_void, address: u32, value: u8, control_mask: u8);
    fn native_wonderswan_write_memory(ptr: *mut c_void, address: u32, value: u8);
    fn native_wonderswan_read_register(ptr: *mut c_void, address: u32) -> u8;
    fn native_wonderswan_read_memory(ptr: *mut c_void, address: u32) -> u8;
    fn native_wonderswan_key_off(ptr: *mut c_void, channel: u32);
    fn native_wonderswan_set_clock(ptr: *mut c_void, hz: u32) -> i32;
    fn native_wonderswan_sample(ptr: *mut c_void, left: *mut f32, right: *mut f32);
}
pub fn core_bytes() -> usize {
    unsafe { native_wonderswan_core_size() }
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
        address: u32,
        value: u8,
        channel: Option<u8>,
        #[serde(rename = "controlMask")]
        control_mask: Option<u8>,
    },
    Memory {
        address: u32,
        value: u8,
        channel: Option<u8>,
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
        address: u32,
        value: u8,
        channel: Option<u8>,
        #[serde(rename = "controlMask")]
        control_mask: Option<u8>,
    },
    Memory {
        address: u32,
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
    audition_rejected: AtomicBool,
    clock_reserved: AtomicBool,
}
fn valid_id(id: Option<u64>) -> bool {
    id.is_none_or(|id| id > 0 && id <= MAX_SAFE_INTEGER)
}
fn register_channel(address: u32) -> Option<u8> {
    match address {
        0x80..=0x87 => Some(((address - 0x80) / 2) as u8),
        0x88..=0x8b => Some((address - 0x88) as u8),
        0x8c | 0x8d => Some(2),
        0x8e | 0x92 | 0x93 => Some(3),
        0x94 => Some(1),
        _ => None,
    }
}
fn validate_op(op: &Op) -> Result<(), String> {
    match op {
        Op::Write {
            address,
            channel,
            control_mask,
            ..
        } => {
            if !(0x80..=0x9f).contains(address)
                || channel.is_some_and(|ch| ch > 3)
                || control_mask.is_some() && *address != 0x90
            {
                return Err("Native WonderSwan register must be 0x80–0x9f; channel 0–3; controlMask only applies to 0x90".into());
            }
            if let (Some(physical), Some(declared)) = (register_channel(*address), channel) {
                if physical != *declared {
                    return Err(
                        "Native WonderSwan register channel does not match its address".into(),
                    );
                }
            }
        }
        Op::Memory {
            address, channel, ..
        } if *address >= 16384 || channel.is_some_and(|ch| ch > 3) => {
            return Err("Native WonderSwan memory must be 0–16383 and channel 0–3".into())
        }
        Op::KeyOff { channel } if *channel > 3 => {
            return Err("Native WonderSwan channel must be 0–3".into())
        }
        _ => {}
    }
    Ok(())
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
            return Err("Invalid Native WonderSwan command ID".into());
        }
        let edit = match &self.method {
            Method::Write {
                address,
                value,
                channel,
                control_mask,
            } => Edit::Op(Op::Write {
                address: *address,
                value: *value,
                channel: *channel,
                control_mask: *control_mask,
            }),
            Method::Memory {
                address,
                value,
                channel,
            } => Edit::Op(Op::Memory {
                address: *address,
                value: *value,
                channel: *channel,
            }),
            Method::KeyOff { channel } => Edit::Op(Op::KeyOff { channel: *channel }),
            Method::Reset => Edit::Reset,
            Method::StopVoices | Method::AllNotesOff => Edit::Stop,
            Method::SourceMute { mask } => {
                if *mask > 15 {
                    return Err("Native WonderSwan source mute mask must be 0–15".into());
                }
                Edit::SourceMute(*mask)
            }
            Method::CancelMainOwner { target_group } => {
                if !valid_id(Some(*target_group)) {
                    return Err("Invalid Native WonderSwan Main owner group".into());
                }
                Edit::CancelMainOwner(*target_group)
            }
            Method::Clock { hz } => {
                if !(100_000..=100_000_000).contains(hz) {
                    return Err("Native WonderSwan clock must be 100000–100000000 Hz".into());
                }
                Edit::Clock(*hz)
            }
            Method::Mix { volume, pan } => {
                if !volume.is_finite()
                    || !(0.0..=4.0).contains(volume)
                    || !pan.is_finite()
                    || !(-1.0..=1.0).contains(pan)
                {
                    return Err("Native WonderSwan mixer volume must be 0–4, pan −1–1".into());
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
                    return Err("Invalid Native WonderSwan batch or voice lease".into());
                }
                for op in commands {
                    validate_op(op)?;
                    let target = match op {
                        Op::Write {
                            address,
                            channel: target,
                            ..
                        } => register_channel(*address).or(*target),
                        Op::Memory {
                            channel: target, ..
                        } => *target,
                        Op::KeyOff { channel } => Some(*channel),
                    };
                    if target.is_some_and(|target| target != *channel) {
                        return Err(
                            "Native WonderSwan batch must target its declared channel".into()
                        );
                    }
                    if let Op::Write {
                        address: 0x90,
                        control_mask: Some(mask),
                        ..
                    } = op
                    {
                        let allowed = (1u8 << *channel)
                            | match channel {
                                1 => 0x20,
                                2 => 0x40,
                                3 => 0x80,
                                _ => 0,
                            };
                        if mask & !allowed != 0 {
                            return Err("Native WonderSwan batch control mask must target its declared channel".into());
                        }
                    }
                }
                let ops = commands
                    .iter()
                    .map(|op| match op {
                        Op::Write {
                            address,
                            value,
                            channel: None,
                            control_mask,
                        } => Op::Write {
                            address: *address,
                            value: *value,
                            channel: Some(*channel),
                            control_mask: *control_mask,
                        },
                        Op::Memory {
                            address,
                            value,
                            channel: None,
                        } => Op::Memory {
                            address: *address,
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
            audition_rejected: AtomicBool::new(false),
            clock_reserved: AtomicBool::new(false),
        }))
    }
}
const CLOCK_HELD: &str = "Release the manually held WonderSwan voices before changing the clock";
impl Prepared {
    fn outcome(&self) -> u8 {
        self.cancelled.load(Ordering::Acquire) as u8
            | ((self.clock_rejected.load(Ordering::Acquire) as u8) << 1)
            | ((self.audition_rejected.load(Ordering::Acquire) as u8) << 2)
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
        Err("Native WonderSwan command was cancelled before it applied".into())
    } else if flags & 2 != 0 {
        Err(CLOCK_HELD.into())
    } else if flags & 4 != 0 {
        Err("WonderSwan clock change is scheduled; audition after it is applied".into())
    } else {
        Ok(())
    }
}
pub struct Shared {
    revision: AtomicU64,
    registers: [AtomicU8; 32],
    memory: [AtomicU8; 16384],
    authored_gates: AtomicU8,
    pending_clocks: AtomicU32,
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
            memory: std::array::from_fn(|_| AtomicU8::new(0)),
            authored_gates: AtomicU8::new(0),
            pending_clocks: AtomicU32::new(0),
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
    pub pending_clocks: u32,
    pub registers: Vec<u8>,
    pub authored_registers: Vec<u8>,
    pub memory: Vec<u8>,
    pub initialized: bool,
    pub pending_commands: Vec<u64>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub wonderswan: ChipStatus,
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
            .map_err(|_| "Native WonderSwan control unavailable")?;
        self.collect(&mut c);
        if self.held.load(Ordering::Acquire) != 0
            && commands
                .iter()
                .any(|(v, _)| matches!(v.edit, Edit::Clock(_)))
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
            return Err("Native WonderSwan pending command limit reached (8 MiB)".into());
        }
        let mut ids = HashSet::new();
        for v in c.prepared.iter().chain(commands.iter().map(|(v, _)| v)) {
            if v.id.is_some_and(|id| !ids.insert(id)) {
                return Err("Duplicate pending Native WonderSwan command ID".into());
            }
        }
        for (v, _) in commands {
            if let Some(id) = v.id {
                self.highest_id.fetch_max(id, Ordering::AcqRel);
            }
            if matches!(v.edit, Edit::Clock(_)) && !v.clock_reserved.swap(true, Ordering::AcqRel) {
                self.pending_clocks.fetch_add(1, Ordering::AcqRel);
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
            .expect("WonderSwan control checked by transactional retain");
        for v in &control.prepared {
            if v.group == Some(target) && !v.audition && !v.complete.load(Ordering::Acquire) {
                v.cancelled.store(true, Ordering::Release);
            }
        }
        true
    }
    pub fn wait_applied(&self, id: u64) -> Result<(), String> {
        if id == 0 || id > self.highest_id.load(Ordering::Acquire) {
            return Err("Unknown Native WonderSwan command ID".into());
        }
        let cmd = {
            let c = self
                .control
                .lock()
                .map_err(|_| "Native WonderSwan control unavailable")?;
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
                return Err("Native WonderSwan command outcome has expired".into());
            }
        };
        let start = std::time::Instant::now();
        while !cmd.complete.load(Ordering::Acquire) {
            if start.elapsed() > std::time::Duration::from_millis(5000) {
                return Err("Native WonderSwan command was not applied within 5000 ms".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        outcome(cmd.outcome())
    }
    pub fn rollback(&self, v: Arc<Prepared>) {
        self.retire(v)
    }
    fn finish_clock(&self, v: &Prepared) {
        if v.clock_reserved.swap(false, Ordering::AcqRel) {
            self.pending_clocks.fetch_sub(1, Ordering::AcqRel);
        }
    }
    pub fn retire(&self, v: Arc<Prepared>) {
        self.finish_clock(&v);
        if !v.complete.load(Ordering::Acquire) {
            v.cancelled.store(true, Ordering::Release)
        }
        v.complete.store(true, Ordering::Release);
        v.retired.store(true, Ordering::Release);
        self.retired
            .push(v)
            .unwrap_or_else(|_| unreachable!("Bounded WonderSwan retirement"))
    }
    fn publish(&self, renderer: &Renderer, command: Option<&Prepared>) {
        self.revision.fetch_add(1, Ordering::AcqRel);
        for (index, out) in self.registers.iter().enumerate() {
            out.store(renderer.register(0x80 + index as u32), Ordering::Relaxed);
        }
        let publish_byte = |address: u32| {
            self.memory[address as usize].store(
                unsafe { native_wonderswan_read_memory(renderer.ptr, address) },
                Ordering::Relaxed,
            );
        };
        // RAM changes are known from the bounded applied payload. Ordinary
        // register edits never copy the entire 16 KiB bank on the callback.
        match command.map(|c| &c.edit) {
            Some(Edit::Op(Op::Memory { address, .. })) => publish_byte(*address),
            Some(Edit::Batch(ops, ..)) => {
                for op in ops {
                    if let Op::Memory { address, .. } = op {
                        publish_byte(*address)
                    }
                }
            }
            Some(Edit::Reset) => {
                for address in 0..16384 {
                    publish_byte(address)
                }
            }
            // Fresh cores and Shared both start with zeroed RAM. Periodic
            // status publication and voice cancellation cannot change it.
            _ => {}
        }
        self.authored_gates
            .store(renderer.authored_gates, Ordering::Relaxed);
        self.clock.store(renderer.clock, Ordering::Relaxed);
        self.rate.store(renderer.rate, Ordering::Relaxed);
        self.mute.store(renderer.mute, Ordering::Relaxed);
        self.held
            .store(renderer.manual_mask(None, false), Ordering::Relaxed);
        if let Some(command) = command {
            self.finish_clock(command);
            command.complete.store(true, Ordering::Release);
        }
        self.revision.fetch_add(1, Ordering::Release);
    }
    pub fn status(&self, frame: u64) -> Result<Status, String> {
        let mut registry = self
            .control
            .lock()
            .map_err(|_| "Native WonderSwan control state unavailable")?;
        self.collect(&mut registry);
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() >= std::time::Duration::from_millis(50) {
                return Err("Native WonderSwan state changed while reading; retry".into());
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
                wonderswan: ChipStatus {
                    held: (0..4)
                        .filter(|ch| self.held.load(Ordering::Relaxed) & (1 << ch) != 0)
                        .collect(),
                    source_mute_mask: self.mute.load(Ordering::Relaxed),
                    pending_clocks: self.pending_clocks.load(Ordering::Relaxed),
                    clock: self.clock(),
                    registers: self
                        .registers
                        .iter()
                        .map(|v| v.load(Ordering::Relaxed))
                        .collect(),
                    authored_registers: self
                        .registers
                        .iter()
                        .enumerate()
                        .map(|(index, v)| {
                            let byte = v.load(Ordering::Relaxed);
                            if index == 16 {
                                (byte & 0xf0) | self.authored_gates.load(Ordering::Relaxed)
                            } else {
                                byte
                            }
                        })
                        .collect(),
                    memory: self
                        .memory
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
    authored_gates: u8,
    owners: [Option<Lease>; 4],
    mute: u8,
    gains: [f32; 2],
    targets: [f32; 2],
    ramp: u32,
    rate: u32,
    clock: u32,
}
unsafe impl Send for Renderer {}
impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe { native_wonderswan_destroy(self.ptr) }
    }
}
impl Renderer {
    pub fn new(rate: u32) -> Result<Self, String> {
        Self::new_with_clock(rate, CLOCK)
    }
    pub fn new_with_clock(rate: u32, clock: u32) -> Result<Self, String> {
        if !(100_000..=100_000_000).contains(&clock) {
            return Err("Native WonderSwan clock must be 100000–100000000 Hz".into());
        }
        let ptr = unsafe { native_wonderswan_create(rate, clock) };
        if ptr.is_null() {
            return Err("Could not allocate Native WonderSwan core".into());
        }
        Ok(Self {
            ptr,
            current_group: None,
            authored_gates: 0,
            owners: [None; 4],
            mute: 0,
            gains: [1.0; 2],
            targets: [1.0; 2],
            ramp: 0,
            rate,
            clock,
        })
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
    fn register(&self, address: u32) -> u8 {
        unsafe { native_wonderswan_read_register(self.ptr, address) }
    }
    fn channel(&self, op: &Op) -> Option<usize> {
        match op {
            Op::Write {
                address, channel, ..
            } => register_channel(*address).or(*channel).map(usize::from),
            Op::Memory { channel, .. } => channel.map(usize::from),
            Op::KeyOff { channel } => Some(*channel as usize),
        }
    }
    fn release(&mut self, ch: usize) {
        unsafe { native_wonderswan_key_off(self.ptr, ch as u32) };
        if self.owners[ch].is_none_or(|l| !l.audition) {
            self.authored_gates &= !(1 << ch);
        }
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
        let control = self.register(0x90);
        let protected = |ch: usize| {
            manual & (1 << ch) != 0 || control & (1 << ch) != 0 && self.foreign(ch, owner, voice)
        };
        if !matches!(op, Op::Write { address: 0x90, .. })
            && target.is_some_and(|ch| manual & (1 << ch) != 0)
        {
            return;
        }
        match op {
            Op::Write {
                address,
                value,
                control_mask,
                ..
            } if *address == 0x90 => {
                let mut mask = control_mask.unwrap_or(255);
                // A channel-scoped batch may carry an older shared-byte mirror.
                // Adopt only its declared gate/mode fields, never sibling gates.
                if let Some(ch) = target {
                    mask &= (1 << ch)
                        | match ch {
                            1 => 0x20,
                            2 => 0x40,
                            3 => 0x80,
                            _ => 0,
                        };
                }
                for ch in 0..4 {
                    let bits = (1u8 << ch)
                        | match ch {
                            1 => 0x20,
                            2 => 0x40,
                            3 => 0x80,
                            _ => 0,
                        };
                    if manual & (1 << ch) != 0 {
                        mask &= !bits;
                        continue;
                    }
                    if protected(ch) {
                        // Taking over a nonmanual voice explicitly is allowed;
                        // another channel's shared snapshot cannot change it.
                        if target != Some(ch) {
                            mask &= !bits;
                        } else if value & (1 << ch) == 0 {
                            mask &= !(1 << ch);
                        }
                    }
                }
                unsafe { native_wonderswan_write(self.ptr, *address, *value, mask) };
                let applied = self.register(0x90);
                for ch in 0..4 {
                    if mask & (1 << ch) != 0 {
                        if !audition {
                            self.authored_gates =
                                (self.authored_gates & !(1 << ch)) | (applied & (1 << ch));
                        }
                        if !preserve {
                            self.owners[ch] = if applied & (1 << ch) != 0 {
                                Some(Lease {
                                    group: self.current_group,
                                    owner,
                                    voice,
                                    audition,
                                })
                            } else {
                                None
                            };
                        }
                    }
                }
            }
            Op::Write { address, value, .. } => {
                // Moving the shared wave bank/output path would affect held
                // siblings even when this operation supplies channel metadata.
                if matches!(*address, 0x8f | 0x91 | 0x9e) && (0..4).any(protected) {
                    return;
                }
                unsafe { native_wonderswan_write(self.ptr, *address, *value, 255) };
            }
            Op::Memory { address, value, .. } => {
                let base = (self.register(0x8f) as u32) << 6;
                for ch in 0..4 {
                    let start = (base + ch as u32 * 16) & 0x3fff;
                    if (address.wrapping_sub(start) & 0x3fff) < 16 && protected(ch) {
                        return;
                    }
                }
                unsafe { native_wonderswan_write_memory(self.ptr, *address, *value) };
            }
            Op::KeyOff { channel } => {
                let ch = *channel as usize;
                if manual & (1 << ch) != 0 || self.foreign(ch, owner, voice) {
                    return;
                }
                self.release(ch);
            }
        }
    }
    pub fn apply(&mut self, command: &Prepared, owner: Option<Owner>, shared: &Shared) {
        if matches!(command.edit, Edit::Clock(_)) && self.manual_mask(None, false) != 0 {
            command.clock_rejected.store(true, Ordering::Release);
            shared.publish(self, Some(command));
            return;
        }
        if command.audition
            && matches!(command.edit, Edit::Batch(_, _, _, LeaseAction::On))
            && shared.pending_clocks.load(Ordering::Acquire) != 0
        {
            command.audition_rejected.store(true, Ordering::Release);
            shared.publish(self, Some(command));
            return;
        }
        self.current_group = command.group;
        match &command.edit {
            Edit::Op(op) => self.write(op, owner, None, command.audition, false),
            Edit::Reset
                if self.manual_mask(None, command.audition) == 0
                    && !(0..4).any(|ch| {
                        self.register(0x90) & (1 << ch) != 0 && self.foreign(ch, owner, None)
                    }) =>
            {
                unsafe { native_wonderswan_reset(self.ptr) }
                self.owners.fill(None);
                self.authored_gates = 0;
            }
            Edit::Reset => {}
            Edit::SourceMute(mask) => {
                unsafe { native_wonderswan_set_mute_mask(self.ptr, *mask as u32) }
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
                    unsafe { native_wonderswan_stop(self.ptr) }
                    self.owners.fill(None);
                    self.authored_gates = 0;
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
            Edit::Clock(hz) => unsafe {
                if native_wonderswan_set_clock(self.ptr, *hz) != 0 {
                    self.clock = *hz;
                }
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
                    if *action == LeaseAction::On && self.register(0x90) & (1 << ch) != 0 {
                        self.owners[ch] = Some(Lease {
                            group: self.current_group,
                            owner,
                            voice: *voice,
                            audition: command.audition,
                        });
                    }
                    if *action == LeaseAction::Off || self.register(0x90) & (1 << ch) == 0 {
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
    pub fn stop(&mut self, shared: &Shared) {
        unsafe { native_wonderswan_stop(self.ptr) };
        self.owners.fill(None);
        self.authored_gates = 0;
        shared.publish(self, None);
    }
    pub fn mix(&mut self, pcm: &mut [f32; 2]) {
        let mut sample = [0.0; 2];
        unsafe {
            native_wonderswan_sample(self.ptr, &mut sample[0], &mut sample[1]);
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
