//! Callback-owned PS1 SPU, shared by the primary device and additional instances.
//! Raw registers/RAM are prepared on the control thread; the output callback
//! receives no JS or WASM and does not acquire the control registry lock.
use super::{Owner, CAPACITY, MAX_SAFE_INTEGER, OWNERS};
use crossbeam_queue::ArrayQueue;
use serde::{Deserialize, Serialize};
use std::{
    cell::UnsafeCell,
    ffi::c_void,
    sync::{
        atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicU64, Ordering},
        Arc, Mutex,
    },
};
pub const CLOCK: u32 = 33_868_800;
pub const RAM: usize = 512 * 1024;
pub const INSTANCE_BYTES: usize = super::extra_budget::INSTANCE_BYTES + 12 * 1024 * 1024;
const PENDING_BYTES: usize = 8 * 1024 * 1024;
const REGISTERS: usize = 512;
extern "C" {
    fn ps1_spu_create() -> *mut c_void;
    fn ps1_spu_destroy(ptr: *mut c_void);
    fn ps1_spu_reset(ptr: *mut c_void, clear_ram: i32) -> i32;
    fn ps1_spu_write(ptr: *mut c_void, register: u32, value: u16) -> i32;
    fn ps1_spu_read(ptr: *mut c_void, register: u32) -> u32;
    fn ps1_spu_load_ram(ptr: *mut c_void, address: u32, data: *const u8, length: u32) -> i32;
    fn ps1_spu_read_ram(ptr: *mut c_void, address: u32, data: *mut u8, length: u32) -> i32;
    fn ps1_spu_render(ptr: *mut c_void, left: *mut f32, right: *mut f32, frames: u32) -> i32;
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
        register: u16,
        value: u16,
    },
    LoadMemory {
        address: u32,
        data: Vec<u8>,
        channel: Option<u8>,
        voice: Option<u64>,
    },
    Reset {
        #[serde(default, rename = "clearRam")]
        clear_ram: bool,
    },
    StopVoices,
    AllNotesOff,
    Barrier {
        #[serde(default)]
        ram: bool,
    },
    CancelMainOwner {
        #[serde(rename = "targetGroup")]
        target_group: u64,
    },
    Mix {
        volume: f32,
        pan: f32,
    },
    Batch {
        commands: Vec<Op>,
        channel: Option<u8>,
        voice: Option<u64>,
        action: LeaseAction,
    },
}
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(tag = "method", rename_all = "kebab-case")]
pub enum Op {
    Write { register: u16, value: u16 },
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
    Write(Op),
    Load(u32, Vec<u8>, Option<u8>, Option<u64>),
    Reset(bool),
    Stop,
    Barrier,
    CancelMainOwner(u64),
    Mix(f32, f32),
    Batch(Vec<Op>, Option<u8>, Option<u64>, LeaseAction),
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
    rejected: AtomicBool,
    capture: Option<Arc<Capture>>,
}
fn valid_id(id: Option<u64>) -> bool {
    id.is_none_or(|id| id > 0 && id <= MAX_SAFE_INTEGER)
}
fn local(register: u16) -> Option<usize> {
    (register < 0x180).then_some(register as usize / 16)
}
fn validate(op: &Op) -> Result<(), String> {
    let Op::Write { register, value } = *op;
    if register > 0x3fe || register & 1 != 0 {
        return Err("Native PS1 registers must be even byte offsets 0–1022".into());
    }
    if ((register < 0x180 && register & 15 < 4) || matches!(register, 0x180 | 0x182))
        && value & 0x8000 != 0
    {
        return Err("PS1 volume sweeps are unavailable".into());
    }
    if register == 0x1ac && value & 14 != 4 {
        return Err("Unsupported PS1 RAM transfer control".into());
    }
    if register < 0x180 && matches!(register & 15, 6 | 14) && value & 1 != 0 {
        return Err("PS1 sample addresses must be packet-aligned".into());
    }
    Ok(())
}
impl Message {
    pub fn prepared_bytes(&self) -> usize {
        std::mem::size_of::<Prepared>()
            + match &self.method {
                Method::LoadMemory { data, .. } => data.len(),
                Method::Batch { commands, .. } => commands.len() * std::mem::size_of::<Op>(),
                Method::Barrier { ram: true } => {
                    RAM + REGISTERS * 2 + std::mem::size_of::<Capture>()
                }
                _ => 0,
            }
    }
    pub fn prepare(&self) -> Result<Arc<Prepared>, String> {
        if !valid_id(self.id) || !valid_id(self.group) {
            return Err("Invalid Native PS1 command ID".into());
        }
        let edit = match &self.method {
            Method::Write { register, value } => {
                let op = Op::Write {
                    register: *register,
                    value: *value,
                };
                validate(&op)?;
                Edit::Write(op)
            }
            Method::LoadMemory {
                address,
                data,
                channel,
                voice,
            } => {
                if *address as usize > RAM
                    || data.len() > RAM - *address as usize
                    || channel.is_some_and(|ch| ch > 23)
                    || !valid_id(*voice)
                {
                    return Err("Invalid Native PS1 RAM upload or voice lease".into());
                }
                Edit::Load(*address, data.clone(), *channel, *voice)
            }
            Method::Reset { clear_ram } => Edit::Reset(*clear_ram),
            Method::StopVoices | Method::AllNotesOff => Edit::Stop,
            Method::Barrier { ram } => {
                if *ram && self.id.is_none() {
                    return Err("PS1 RAM capture requires a command ID".into());
                }
                Edit::Barrier
            }
            Method::CancelMainOwner { target_group } => {
                if !valid_id(Some(*target_group)) {
                    return Err("Invalid Native PS1 owner group".into());
                }
                Edit::CancelMainOwner(*target_group)
            }
            Method::Mix { volume, pan } => {
                if !volume.is_finite()
                    || !(0.0..=4.0).contains(volume)
                    || !pan.is_finite()
                    || !(-1.0..=1.0).contains(pan)
                {
                    return Err("Invalid Native PS1 mixer gain/pan".into());
                }
                Edit::Mix(*volume, *pan)
            }
            Method::Batch {
                commands,
                channel,
                voice,
                action,
            } => {
                if commands.is_empty()
                    || commands.len() > 512
                    || channel.is_some_and(|ch| ch > 23)
                    || !valid_id(*voice)
                    || self.audition && *action != LeaseAction::Edit && voice.is_none()
                {
                    return Err("Invalid Native PS1 batch or voice lease".into());
                }
                if *action != LeaseAction::Edit && channel.is_none() {
                    return Err("PS1 note batches require a channel".into());
                }
                for op in commands {
                    validate(op)?;
                    let Op::Write { register, .. } = *op;
                    if channel.is_some_and(|ch| {
                        local(register).is_some_and(|target| target != ch as usize)
                    }) {
                        return Err("PS1 batch must target its declared channel".into());
                    }
                }
                Edit::Batch(commands.clone(), *channel, *voice, *action)
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
            rejected: AtomicBool::new(false),
            capture: matches!(self.method, Method::Barrier { ram: true })
                .then(|| Arc::new(Capture::new())),
        }))
    }
}
impl Prepared {
    pub fn starts(&self) -> bool {
        match &self.edit {
            Edit::Write(Op::Write { register, value }) => {
                matches!(*register, 0x188 | 0x18a) && *value != 0
            }
            Edit::Batch(ops, _, _, action) => {
                *action == LeaseAction::On
                    || ops
                        .iter()
                        .any(|op| matches!(op,Op::Write{register:0x188|0x18a,value}if *value!=0))
            }
            _ => false,
        }
    }
    pub fn stops(&self) -> bool {
        matches!(self.edit, Edit::Stop) && self.group.is_none()
    }
    pub fn is_owner_cancel(&self) -> bool {
        matches!(self.edit, Edit::CancelMainOwner(_))
    }
    pub fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + match &self.edit {
                Edit::Load(_, v, ..) => v.capacity(),
                Edit::Batch(v, ..) => v.capacity() * std::mem::size_of::<Op>(),
                _ => 0,
            }
            + self
                .capture
                .as_ref()
                .map_or(0, |_| RAM + REGISTERS * 2 + std::mem::size_of::<Capture>())
    }
    pub fn payload_unused(&self) -> bool {
        self.capture
            .as_ref()
            .is_none_or(|c| c.delivered.load(Ordering::Acquire))
    }
    fn flags(&self) -> u8 {
        self.cancelled.load(Ordering::Acquire) as u8
            | ((self.rejected.load(Ordering::Acquire) as u8) << 1)
    }
}
#[derive(Debug)]
struct Capture {
    ram: UnsafeCell<Box<[u8]>>,
    registers: UnsafeCell<Box<[u16]>>,
    native_frames: AtomicU64,
    enabled: AtomicU32,
    held: AtomicU32,
    frame: AtomicU64,
    rate: AtomicU32,
    revision: AtomicU64,
    delivered: AtomicBool,
    created: std::time::Instant,
}
unsafe impl Sync for Capture {}
impl Capture {
    fn new() -> Self {
        Self {
            ram: UnsafeCell::new(vec![0; RAM].into_boxed_slice()),
            registers: UnsafeCell::new(vec![0; REGISTERS].into_boxed_slice()),
            native_frames: AtomicU64::new(0),
            enabled: AtomicU32::new(0),
            held: AtomicU32::new(0),
            frame: AtomicU64::new(0),
            rate: AtomicU32::new(0),
            revision: AtomicU64::new(0),
            delivered: AtomicBool::new(false),
            created: std::time::Instant::now(),
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
        Err("Native PS1 command was cancelled before it applied".into())
    } else if flags & 2 != 0 {
        Err("Release the manually held PS1 voice before replacing its sample RAM or state".into())
    } else {
        Ok(())
    }
}
pub struct Shared {
    revision: AtomicU64,
    registers: [AtomicU16; REGISTERS],
    rate: AtomicU32,
    native_frames: AtomicU64,
    enabled: AtomicU32,
    held: AtomicU32,
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
            registers: std::array::from_fn(|_| AtomicU16::new(0)),
            rate: AtomicU32::new(0),
            native_frames: AtomicU64::new(0),
            enabled: AtomicU32::new(0),
            held: AtomicU32::new(0),
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
    pub registers: Vec<u16>,
    pub native_frames: u64,
    pub enabled: u32,
    pub held: Vec<u8>,
    pub initialized: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ram: Option<Vec<u8>>,
    pub pending_commands: Vec<u64>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub ps1: ChipStatus,
    pub frame: u64,
    pub sample_rate: u32,
    pub revision: u64,
}
impl Shared {
    pub fn prepare_message(&self, m: &Message) -> Result<Arc<Prepared>, String> {
        m.prepare()
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
            if let Some(cap) = &v.capture {
                if cap.created.elapsed() >= std::time::Duration::from_secs(30) {
                    cap.delivered.store(true, Ordering::Release);
                }
                if !cap.delivered.load(Ordering::Acquire) {
                    return true;
                }
            }
            if let Some(id) = v.id {
                outcomes[*cursor] = Completion {
                    id,
                    flags: v.flags(),
                };
                *cursor = (*cursor + 1) % outcomes.len();
            }
            false
        });
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
            .map_err(|_| "Native PS1 control unavailable")?;
        self.collect(&mut c);
        if c.prepared.len() + commands.len() > CAPACITY
            || c.prepared.iter().map(|v| v.retained_bytes()).sum::<usize>()
                + commands
                    .iter()
                    .map(|(v, _)| v.retained_bytes())
                    .sum::<usize>()
                > PENDING_BYTES
        {
            return Err("Native PS1 pending command limit reached (8 MiB)".into());
        }
        let mut ids = std::collections::HashSet::new();
        for v in c.prepared.iter().chain(commands.iter().map(|(v, _)| v)) {
            if v.id.is_some_and(|id| !ids.insert(id)) {
                return Err("Duplicate pending Native PS1 command ID".into());
            }
        }
        for (v, _) in commands {
            if let Some(id) = v.id {
                self.highest_id.fetch_max(id, Ordering::Release);
            }
            c.prepared.push(v.clone());
        }
        Ok(())
    }
    pub fn commit_cancel(&self, c: &Prepared) -> bool {
        if let Edit::CancelMainOwner(target) = c.edit {
            let mut changed = false;
            if let Ok(control) = self.control.lock() {
                for other in &control.prepared {
                    if other.group == Some(target)
                        && !other.audition
                        && !other.complete.load(Ordering::Acquire)
                    {
                        other.cancelled.store(true, Ordering::Release);
                        changed = true;
                    }
                }
            }
            changed
        } else {
            false
        }
    }
    pub fn wait_applied(&self, id: u64) -> Result<(), String> {
        if !valid_id(Some(id)) || id > self.highest_id.load(Ordering::Acquire) {
            return Err("Unknown Native PS1 command ID".into());
        }
        let found = {
            let c = self
                .control
                .lock()
                .map_err(|_| "Native PS1 control unavailable")?;
            if let Some(v) = c.prepared.iter().find(|v| v.id == Some(id)) {
                Some(v.clone())
            } else if let Some(v) = c.outcomes.iter().find(|v| v.id == id) {
                return outcome(v.flags);
            } else {
                return Err("Expired Native PS1 command ID".into());
            }
        };
        if let Some(v) = found {
            let start = std::time::Instant::now();
            while !v.complete.load(Ordering::Acquire) {
                if start.elapsed() >= std::time::Duration::from_millis(100) {
                    return Err("Native PS1 command was not applied within 100 ms".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            outcome(v.flags())?;
        }
        Ok(())
    }
    pub fn rollback(&self, c: Arc<Prepared>) {
        c.cancelled.store(true, Ordering::Release);
        c.complete.store(true, Ordering::Release);
        c.retired.store(true, Ordering::Release);
        if let Some(cap) = &c.capture {
            cap.delivered.store(true, Ordering::Release);
        }
    }
    pub fn retire(&self, c: Arc<Prepared>) {
        if !c.complete.load(Ordering::Acquire) {
            c.cancelled.store(true, Ordering::Release);
            if let Some(cap) = &c.capture {
                cap.delivered.store(true, Ordering::Release);
            }
        }
        c.complete.store(true, Ordering::Release);
        c.retired.store(true, Ordering::Release);
        self.retired
            .push(c)
            .unwrap_or_else(|_| unreachable!("Bounded PS1 command retirement"));
    }
    pub fn status(&self, frame: u64) -> Result<Status, String> {
        self.status_after(frame, None, false)
    }
    pub fn status_after(
        &self,
        frame: u64,
        after: Option<u64>,
        ram: bool,
    ) -> Result<Status, String> {
        if let Some(id) = after {
            self.wait_applied(id)?;
        }
        let mut c = self
            .control
            .lock()
            .map_err(|_| "Native PS1 control unavailable")?;
        self.collect(&mut c);
        if ram {
            let id = after.ok_or("PS1 RAM snapshots require an applied capture barrier ID")?;
            let v = c
                .prepared
                .iter()
                .find(|v| v.id == Some(id))
                .ok_or("Expired Native PS1 RAM capture")?;
            let cap = v
                .capture
                .as_ref()
                .ok_or("PS1 RAM snapshots require a RAM capture barrier")?;
            if !v.complete.load(Ordering::Acquire) {
                return Err("PS1 RAM capture is not complete".into());
            }
            let data = unsafe { (&*cap.ram.get()).to_vec() };
            let registers = unsafe { (&*cap.registers.get()).to_vec() };
            let held = cap.held.load(Ordering::Relaxed);
            let captured = Status {
                frame: cap.frame.load(Ordering::Relaxed),
                sample_rate: cap.rate.load(Ordering::Relaxed),
                revision: cap.revision.load(Ordering::Relaxed),
                ps1: ChipStatus {
                    clock: CLOCK,
                    registers,
                    native_frames: cap.native_frames.load(Ordering::Relaxed),
                    enabled: cap.enabled.load(Ordering::Relaxed),
                    held: (0..24).filter(|ch| held & (1 << ch) != 0).collect(),
                    initialized: true,
                    ram: Some(data),
                    pending_commands: c
                        .prepared
                        .iter()
                        .filter(|v| !v.complete.load(Ordering::Relaxed))
                        .filter_map(|v| v.id)
                        .collect(),
                },
            };
            cap.delivered.store(true, Ordering::Release);
            return Ok(captured);
        }
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() >= std::time::Duration::from_millis(100) {
                return Err("Native PS1 state changed while reading; retry".into());
            }
            let before = self.revision.load(Ordering::Acquire);
            if before & 1 != 0 {
                std::thread::yield_now();
                continue;
            }
            let mask = self.held.load(Ordering::Relaxed);
            let state = Status {
                frame,
                sample_rate: self.rate.load(Ordering::Relaxed),
                revision: before / 2,
                ps1: ChipStatus {
                    clock: CLOCK,
                    registers: self
                        .registers
                        .iter()
                        .map(|v| v.load(Ordering::Relaxed))
                        .collect(),
                    native_frames: self.native_frames.load(Ordering::Relaxed),
                    enabled: self.enabled.load(Ordering::Relaxed),
                    held: (0..24).filter(|ch| mask & (1 << ch) != 0).collect(),
                    initialized: true,
                    ram: None,
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
                return Ok(state);
            }
        }
    }
    fn publish(&self, r: &Renderer, c: Option<&Prepared>) {
        self.revision.fetch_add(1, Ordering::AcqRel);
        for (index, out) in self.registers.iter().enumerate() {
            out.store(
                unsafe { ps1_spu_read(r.ptr, index as u32 * 2) } as u16,
                Ordering::Relaxed,
            );
        }
        self.rate.store(r.rate, Ordering::Relaxed);
        self.native_frames.store(r.native_frames, Ordering::Relaxed);
        self.enabled.store(r.gates, Ordering::Relaxed);
        self.held
            .store(r.manual_mask(None, false), Ordering::Relaxed);
        if let Some(c) = c {
            if let Some(cap) = &c.capture {
                unsafe {
                    let buffer = &mut *cap.ram.get();
                    ps1_spu_read_ram(r.ptr, 0, buffer.as_mut_ptr(), RAM as u32);
                    let registers = &mut *cap.registers.get();
                    for (index, out) in registers.iter_mut().enumerate() {
                        *out = ps1_spu_read(r.ptr, index as u32 * 2) as u16;
                    }
                }
                cap.native_frames.store(r.native_frames, Ordering::Relaxed);
                cap.enabled.store(r.gates, Ordering::Relaxed);
                cap.held
                    .store(r.manual_mask(None, false), Ordering::Relaxed);
                cap.frame.store(r.frame, Ordering::Relaxed);
                cap.rate.store(r.rate, Ordering::Relaxed);
                cap.revision.store(
                    (self.revision.load(Ordering::Relaxed) + 1) / 2,
                    Ordering::Relaxed,
                );
            }
            c.complete.store(true, Ordering::Release);
        }
        self.revision.fetch_add(1, Ordering::Release);
    }
}
#[derive(Clone, Copy)]
struct Lease {
    owner: Option<Owner>,
    group: Option<u64>,
    voice: Option<u64>,
    audition: bool,
}
pub struct Renderer {
    ptr: *mut c_void,
    owners: [Option<Lease>; 24],
    gates: u32,
    current_group: Option<u64>,
    gains: [f32; 2],
    targets: [f32; 2],
    ramp: u32,
    rate: u32,
    phase: u64,
    previous: [f32; 2],
    next: [f32; 2],
    primed: bool,
    native_frames: u64,
    upload_high_water: usize,
    frame: u64,
}
unsafe impl Send for Renderer {}
impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe { ps1_spu_destroy(self.ptr) }
    }
}
impl Renderer {
    pub fn new(rate: u32) -> Result<Self, String> {
        Self::new_with_clock(rate, CLOCK)
    }
    pub fn new_with_clock(rate: u32, clock: u32) -> Result<Self, String> {
        if !(8000..=384000).contains(&rate) || clock != CLOCK {
            return Err("PS1 SPU uses a fixed 33.8688 MHz clock / 44100 Hz core".into());
        }
        let ptr = unsafe { ps1_spu_create() };
        if ptr.is_null() {
            return Err("Could not allocate Native PS1 SPU".into());
        }
        Ok(Self {
            ptr,
            owners: [None; 24],
            gates: 0,
            current_group: None,
            gains: [1.0; 2],
            targets: [1.0; 2],
            ramp: 0,
            rate,
            phase: 0,
            previous: [0.0; 2],
            next: [0.0; 2],
            primed: false,
            native_frames: 0,
            upload_high_water: 0x1000,
            frame: 0,
        })
    }
    pub fn set_frame(&mut self, frame: u64) {
        self.frame = frame;
    }
    fn manual_mask(&self, voice: Option<u64>, audition: bool) -> u32 {
        self.owners.iter().enumerate().fold(0, |mask, (ch, l)| {
            mask | if l.is_some_and(|l| l.audition && (!audition || l.voice != voice)) {
                1 << ch
            } else {
                0
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
    fn write(
        &mut self,
        op: &Op,
        owner: Option<Owner>,
        voice: Option<u64>,
        audition: bool,
        preserve: bool,
        declared: Option<u8>,
    ) {
        let Op::Write {
            register,
            mut value,
        } = *op;
        let manual = self.manual_mask(voice, audition);
        if let Some(ch) = local(register) {
            if manual & (1 << ch) != 0 {
                return;
            }
        } else if matches!(register, 0x188 | 0x18a | 0x18c | 0x18e) {
            let off = register >= 0x18c;
            let base = if register & 2 != 0 { 16 } else { 0 };
            for bit in 0..16 {
                let ch = base + bit;
                if ch >= 24
                    || manual & (1 << ch) != 0
                    || declared.is_some_and(|target| target as usize != ch)
                    || off && self.foreign(ch, owner, voice)
                {
                    value &= !(1 << bit);
                    continue;
                }
                if value & (1 << bit) != 0 {
                    if off {
                        self.gates &= !(1 << ch);
                        if !preserve {
                            self.owners[ch] = None;
                        }
                    } else {
                        self.gates |= 1 << ch;
                        if !preserve {
                            self.owners[ch] = Some(Lease {
                                owner,
                                group: self.current_group,
                                voice,
                                audition,
                            });
                        }
                    }
                }
            }
        } else if matches!(register, 0x190 | 0x192 | 0x194 | 0x196 | 0x198 | 0x19a) {
            let base = if register & 2 != 0 { 16 } else { 0 };
            let mut protect = 0u16;
            for bit in 0..16 {
                let ch = base + bit;
                if ch >= 24
                    || manual & (1 << ch) != 0
                    || declared.is_some_and(|target| target as usize != ch)
                {
                    protect |= 1 << bit;
                }
            }
            let current = unsafe { ps1_spu_read(self.ptr, register as u32) } as u16;
            value = (value & !protect) | (current & protect);
        } else if manual != 0 {
            return;
        }
        unsafe {
            ps1_spu_write(self.ptr, register as u32, value);
        }
    }
    fn release(&mut self, ch: usize) {
        unsafe {
            ps1_spu_write(
                self.ptr,
                0x18c + if ch >= 16 { 2 } else { 0 },
                1 << (ch % 16),
            );
        }
        self.owners[ch] = None;
        self.gates &= !(1 << ch);
    }
    fn reset_resampler(&mut self) {
        self.phase = 0;
        self.previous = [0.0; 2];
        self.next = [0.0; 2];
        self.primed = false;
    }
    fn silence(&mut self) {
        // Reset decoder/envelopes/private reverb tails while preserving RAM and
        // requested register settings. Never replay KON/KOFF side effects.
        let mut registers = [0u16; 512];
        for (i, value) in registers.iter_mut().enumerate() {
            *value = unsafe { ps1_spu_read(self.ptr, i as u32 * 2) } as u16;
        }
        unsafe {
            ps1_spu_reset(self.ptr, 0);
            for (i, value) in registers.iter().enumerate() {
                let register = i as u32 * 2;
                let authored = register < 0x180 && register % 16 != 12
                    || matches!(
                        register,
                        0x180
                            | 0x182
                            | 0x184
                            | 0x186
                            | 0x190
                            | 0x192
                            | 0x194
                            | 0x196
                            | 0x198
                            | 0x19a
                            | 0x1a2
                            | 0x1aa
                            | 0x1ac
                    )
                    || (0x1c0..=0x1fe).contains(&register);
                if authored {
                    ps1_spu_write(self.ptr, register, *value);
                }
            }
        }
        self.owners.fill(None);
        self.gates = 0;
        self.reset_resampler();
    }
    pub fn apply(&mut self, c: &Prepared, owner: Option<Owner>, shared: &Shared) {
        self.current_group = c.group;
        let held = self.manual_mask(None, false);
        match &c.edit {
            Edit::Write(op) => self.write(op, owner, None, c.audition, false, None),
            Edit::Load(address, data, channel, voice) => {
                let protected = self.manual_mask(*voice, c.audition);
                let appended = *address as usize >= self.upload_high_water;
                if !appended
                    && (protected != 0
                        || held != 0
                            && (!c.audition
                                || channel.is_none()
                                || channel.is_some_and(|ch| held & !(1 << ch) != 0)))
                {
                    c.rejected.store(true, Ordering::Release);
                } else {
                    unsafe {
                        ps1_spu_load_ram(self.ptr, *address, data.as_ptr(), data.len() as u32);
                    }
                    self.upload_high_water =
                        self.upload_high_water.max(*address as usize + data.len());
                }
            }
            Edit::Reset(clear) => {
                if held != 0 {
                    c.rejected.store(true, Ordering::Release);
                } else {
                    unsafe {
                        ps1_spu_reset(self.ptr, *clear as i32);
                    }
                    self.owners.fill(None);
                    self.gates = 0;
                    if *clear {
                        self.upload_high_water = 0x1000;
                    }
                    self.reset_resampler();
                }
            }
            Edit::Stop => {
                if owner.is_none() && c.group.is_none() && (c.audition || held == 0) {
                    self.silence();
                } else {
                    for ch in 0..24 {
                        if self.owners[ch].is_some_and(|l| {
                            (!l.audition || c.audition)
                                && (owner.is_none() || l.owner == owner)
                                && (c.group.is_none() || l.group == c.group)
                        }) {
                            self.release(ch);
                        }
                    }
                }
            }
            Edit::Barrier => {}
            Edit::CancelMainOwner(group) => {
                for ch in 0..24 {
                    if self.owners[ch].is_some_and(|l| {
                        l.owner.is_none() && !l.audition && l.group == Some(*group)
                    }) {
                        self.release(ch);
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
            Edit::Batch(ops, channel, voice, action) => {
                let current = channel.and_then(|ch| self.owners[ch as usize]);
                let allowed = match action {
                    LeaseAction::On => c.audition || current.is_none_or(|l| !l.audition),
                    LeaseAction::Off => current.is_some_and(|l| {
                        l.owner == owner
                            && (c.group.is_none() || l.group == c.group)
                            && (voice.is_none() || l.voice == *voice)
                            && (!l.audition || c.audition)
                    }),
                    LeaseAction::Edit => {
                        current.is_none_or(|l| !l.audition || c.audition && l.voice == *voice)
                    }
                };
                if allowed {
                    if *action == LeaseAction::On {
                        self.owners[channel.unwrap() as usize] = Some(Lease {
                            owner,
                            group: c.group,
                            voice: *voice,
                            audition: c.audition,
                        });
                    }
                    for op in ops {
                        self.write(
                            op,
                            owner,
                            *voice,
                            c.audition,
                            *action == LeaseAction::Edit,
                            *channel,
                        );
                    }
                    if *action == LeaseAction::Off {
                        let ch = channel.unwrap() as usize;
                        self.owners[ch] = None;
                        self.gates &= !(1 << ch);
                    }
                }
            }
        }
        self.current_group = None;
        shared.publish(self, Some(c));
    }
    pub fn publish(&self, shared: &Shared) {
        shared.publish(self, None)
    }
    pub fn cancel_invalid(&mut self, valid: impl Fn(Option<Owner>) -> bool, shared: &Shared) {
        let mut changed = false;
        for ch in 0..24 {
            if self.owners[ch].is_some_and(|l| !valid(l.owner)) {
                self.release(ch);
                changed = true;
            }
        }
        if changed {
            self.publish(shared);
        }
    }
    pub fn stop(&mut self, shared: &Shared) {
        self.silence();
        self.publish(shared);
    }
    fn native_sample(&mut self) -> [f32; 2] {
        let mut out = [0.0; 2];
        unsafe {
            ps1_spu_render(self.ptr, &mut out[0], &mut out[1], 1);
        }
        self.native_frames += 1;
        out
    }
    pub fn mix(&mut self, pcm: &mut [f32; 2]) {
        if !self.primed {
            self.next = self.native_sample();
            self.previous = self.next;
            self.primed = true;
        }
        let fraction = self.phase as f32 / self.rate as f32;
        let sample = [
            self.previous[0] + (self.next[0] - self.previous[0]) * fraction,
            self.previous[1] + (self.next[1] - self.previous[1]) * fraction,
        ];
        self.phase += 44100;
        while self.phase >= self.rate as u64 {
            self.phase -= self.rate as u64;
            self.previous = self.next;
            self.next = self.native_sample();
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
        self.frame += 1;
    }
    pub fn mix_additional(&mut self, pcm: &mut [f32; 2], shared: &Shared) {
        self.mix(pcm);
        if self.frame % 128 == 0 {
            self.publish(shared);
        }
    }
}
