//! Callback-owned SCSP, shared by the primary device and additional instances.
//! Raw registers/RAM are prepared on the control thread; the output callback
//! receives no JS or WASM and does not acquire the control registry lock.
use super::{Owner, CAPACITY, MAX_SAFE_INTEGER, OWNERS};
use crossbeam_queue::ArrayQueue;
use serde::{Deserialize, Serialize};
use std::{
    cell::UnsafeCell,
    sync::{
        atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicU64, Ordering},
        Arc, Mutex,
    },
};
pub const CLOCK: u32 = 22_579_200;
pub const RAM: usize = 512 * 1024;
pub const INSTANCE_BYTES: usize = super::extra_budget::INSTANCE_BYTES + 2 * 1024 * 1024;
const PENDING_BYTES: usize = 8 * 1024 * 1024;
const REGISTERS: usize = 513;
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
#[serde(tag = "method", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Op {
    Write {
        register: u16,
        value: u16,
        #[serde(default, deserialize_with = "deserialize_mask")]
        mask: Option<u16>,
    },
}
fn deserialize_mask<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<u16>, D::Error> {
    u16::deserialize(deserializer).map(Some)
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
    Load(u32, Vec<u8>),
    Reset(bool),
    Stop,
    AllOff,
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
    invalid: AtomicBool,
    capture: Option<Arc<Capture>>,
}
fn valid_id(id: Option<u64>) -> bool {
    id.is_none_or(|id| id > 0 && id <= MAX_SAFE_INTEGER)
}
fn local(register: u16) -> Option<usize> {
    (register < 0x400).then_some(register as usize / 32)
}
fn validate(op: &Op) -> Result<(), String> {
    let Op::Write {
        register,
        value,
        mask,
    } = *op;
    if register & 1 != 0 || register > 0x400 || mask == Some(0) {
        return Err("Native SCSP supports slot registers and master only".into());
    }
    if register == 0x400
        && (mask.is_some_and(|m| m != 65535) || value & !0x030f != 0 || value & 0x0200 == 0)
    {
        return Err("Unsupported Native SCSP master configuration".into());
    }
    if register < 0x400 && register % 32 == 0 && mask.is_some_and(|m| m != 65535 && m != 0x1800) {
        return Err("Unsupported Native SCSP control mask".into());
    }
    if register < 0x400 && register % 32 == 0 && (value >> 7) & 3 == 3 {
        return Err("Unsupported Native SCSP sample source".into());
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
            return Err("Invalid Native SCSP command ID".into());
        }
        let edit = match &self.method {
            Method::Write { register, value } => {
                let op = Op::Write {
                    register: *register,
                    value: *value,
                    mask: None,
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
                    || channel.is_some_and(|ch| ch > 31)
                    || !valid_id(*voice)
                {
                    return Err("Invalid Native SCSP RAM upload or voice lease".into());
                }
                Edit::Load(*address, data.clone())
            }
            Method::Reset { clear_ram } => Edit::Reset(*clear_ram),
            Method::StopVoices => Edit::Stop,
            Method::AllNotesOff => Edit::AllOff,
            Method::Barrier { ram } => {
                if *ram && self.id.is_none() {
                    return Err("SCSP RAM capture requires a command ID".into());
                }
                Edit::Barrier
            }
            Method::CancelMainOwner { target_group } => {
                if !valid_id(Some(*target_group)) {
                    return Err("Invalid Native SCSP owner group".into());
                }
                Edit::CancelMainOwner(*target_group)
            }
            Method::Mix { volume, pan } => {
                if !volume.is_finite()
                    || !(0.0..=4.0).contains(volume)
                    || !pan.is_finite()
                    || !(-1.0..=1.0).contains(pan)
                {
                    return Err("Invalid Native SCSP mixer gain/pan".into());
                }
                Edit::Mix(*volume, *pan)
            }
            Method::Batch {
                commands,
                channel,
                voice,
                action,
            } => {
                if commands.len() > 512
                    || channel.is_some_and(|ch| ch > 31)
                    || !valid_id(*voice)
                    || self.audition && *action != LeaseAction::Edit && voice.is_none()
                {
                    return Err("Invalid Native SCSP batch or voice lease".into());
                }
                if *action != LeaseAction::Edit && channel.is_none() {
                    return Err("SCSP note batches require a channel".into());
                }
                for op in commands {
                    validate(op)?;
                    let Op::Write {
                        register,
                        value,
                        mask,
                    } = *op;
                    if *action == LeaseAction::Edit
                        && register < 0x400
                        && register % 32 == 0
                        && value & mask.unwrap_or(65535) & 0x1800 != 0
                    {
                        return Err("SCSP key writes require a note action".into());
                    }
                    if *action == LeaseAction::Off
                        && register < 0x400
                        && register % 32 == 0
                        && value & mask.unwrap_or(65535) & 0x0800 != 0
                    {
                        return Err("SCSP note-off cannot start a slot".into());
                    }
                    let Op::Write { register, .. } = *op;
                    if channel.is_some_and(|ch| local(register) != Some(ch as usize)) {
                        return Err("SCSP batch must target its declared slot".into());
                    }
                    if channel.is_none() && register < 0x400 {
                        return Err("SCSP slot writes require a declared slot".into());
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
            invalid: AtomicBool::new(false),
            capture: matches!(self.method, Method::Barrier { ram: true })
                .then(|| Arc::new(Capture::new())),
        }))
    }
}
impl Prepared {
    pub fn starts(&self) -> bool {
        matches!(&self.edit, Edit::Batch(_, _, _, LeaseAction::On))
    }
    pub fn stops(&self) -> bool {
        matches!(self.edit, Edit::Stop | Edit::Reset(_)) && self.group.is_none()
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
            | ((self.invalid.load(Ordering::Acquire) as u8) << 2)
    }
}
#[derive(Debug)]
struct Capture {
    ram: UnsafeCell<Box<[u8]>>,
    registers: UnsafeCell<Box<[u16]>>,
    native_frames: AtomicU64,
    playing_addresses: [AtomicU32; 32],
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
            playing_addresses: std::array::from_fn(|_| AtomicU32::new(u32::MAX)),
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
        Err("Native SCSP command was cancelled before it applied".into())
    } else if flags & 4 != 0 {
        Err("Native SCSP note exceeds supported PCM RAM or loop bounds".into())
    } else if flags & 2 != 0 {
        Err("Stop SCSP voices before replacing protected sample RAM or state".into())
    } else {
        Ok(())
    }
}
pub struct Shared {
    revision: AtomicU64,
    registers: [AtomicU16; REGISTERS],
    rate: AtomicU32,
    native_frames: AtomicU64,
    playing_addresses: [AtomicU32; 32],
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
            playing_addresses: std::array::from_fn(|_| AtomicU32::new(u32::MAX)),
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
    pub playing_addresses: Vec<Option<u32>>,
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
    pub scsp: ChipStatus,
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
            .map_err(|_| "Native SCSP control unavailable")?;
        self.collect(&mut c);
        if c.prepared.len() + commands.len() > CAPACITY
            || c.prepared.iter().map(|v| v.retained_bytes()).sum::<usize>()
                + commands
                    .iter()
                    .map(|(v, _)| v.retained_bytes())
                    .sum::<usize>()
                > PENDING_BYTES
        {
            return Err("Native SCSP pending command limit reached (8 MiB)".into());
        }
        let mut ids = std::collections::HashSet::new();
        for v in c.prepared.iter().chain(commands.iter().map(|(v, _)| v)) {
            if v.id.is_some_and(|id| !ids.insert(id)) {
                return Err("Duplicate pending Native SCSP command ID".into());
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
            return Err("Unknown Native SCSP command ID".into());
        }
        let found = {
            let c = self
                .control
                .lock()
                .map_err(|_| "Native SCSP control unavailable")?;
            if let Some(v) = c.prepared.iter().find(|v| v.id == Some(id)) {
                Some(v.clone())
            } else if let Some(v) = c.outcomes.iter().find(|v| v.id == id) {
                return outcome(v.flags);
            } else {
                return Err("Expired Native SCSP command ID".into());
            }
        };
        if let Some(v) = found {
            let start = std::time::Instant::now();
            while !v.complete.load(Ordering::Acquire) {
                if start.elapsed() >= std::time::Duration::from_millis(100) {
                    return Err("Native SCSP command was not applied within 100 ms".into());
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
            .unwrap_or_else(|_| unreachable!("Bounded SCSP command retirement"));
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
            .map_err(|_| "Native SCSP control unavailable")?;
        self.collect(&mut c);
        if ram {
            let id = after.ok_or("SCSP RAM snapshots require an applied capture barrier ID")?;
            let v = c
                .prepared
                .iter()
                .find(|v| v.id == Some(id))
                .ok_or("Expired Native SCSP RAM capture")?;
            let cap = v
                .capture
                .as_ref()
                .ok_or("SCSP RAM snapshots require a RAM capture barrier")?;
            if !v.complete.load(Ordering::Acquire) {
                return Err("SCSP RAM capture is not complete".into());
            }
            let data = unsafe { (&*cap.ram.get()).to_vec() };
            let registers = unsafe { (&*cap.registers.get()).to_vec() };
            let held = cap.held.load(Ordering::Relaxed);
            let captured = Status {
                frame: cap.frame.load(Ordering::Relaxed),
                sample_rate: cap.rate.load(Ordering::Relaxed),
                revision: cap.revision.load(Ordering::Relaxed),
                scsp: ChipStatus {
                    clock: CLOCK,
                    registers,
                    native_frames: cap.native_frames.load(Ordering::Relaxed),
                    playing_addresses: cap
                        .playing_addresses
                        .iter()
                        .map(|v| {
                            let a = v.load(Ordering::Relaxed);
                            (a != u32::MAX).then_some(a)
                        })
                        .collect(),
                    enabled: cap.enabled.load(Ordering::Relaxed),
                    held: (0..32).filter(|ch| held & (1 << ch) != 0).collect(),
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
                return Err("Native SCSP state changed while reading; retry".into());
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
                scsp: ChipStatus {
                    clock: CLOCK,
                    registers: self
                        .registers
                        .iter()
                        .map(|v| v.load(Ordering::Relaxed))
                        .collect(),
                    native_frames: self.native_frames.load(Ordering::Relaxed),
                    playing_addresses: self
                        .playing_addresses
                        .iter()
                        .map(|v| {
                            let a = v.load(Ordering::Relaxed);
                            (a != u32::MAX).then_some(a)
                        })
                        .collect(),
                    enabled: self.enabled.load(Ordering::Relaxed),
                    held: (0..32).filter(|ch| mask & (1 << ch) != 0).collect(),
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
            out.store(r.core.read(index as u32 * 2).unwrap(), Ordering::Relaxed);
        }
        for (ch, v) in self.playing_addresses.iter().enumerate() {
            v.store(
                r.playing[ch].map_or(u32::MAX, |(a, _)| a),
                Ordering::Relaxed,
            );
        }
        self.rate.store(r.rate, Ordering::Relaxed);
        self.native_frames
            .store(r.core.native_frames(), Ordering::Relaxed);
        self.enabled.store(r.gates, Ordering::Relaxed);
        self.held
            .store(r.manual_mask(None, false), Ordering::Relaxed);
        if let Some(c) = c {
            if let Some(cap) = &c.capture {
                unsafe {
                    r.core.read_ram(0, &mut *cap.ram.get()).unwrap();
                    for (index, out) in (&mut *cap.registers.get()).iter_mut().enumerate() {
                        *out = r.core.read(index as u32 * 2).unwrap();
                    }
                }
                for (ch, v) in cap.playing_addresses.iter().enumerate() {
                    v.store(
                        r.playing[ch].map_or(u32::MAX, |(a, _)| a),
                        Ordering::Relaxed,
                    );
                }
                cap.native_frames
                    .store(r.core.native_frames(), Ordering::Relaxed);
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
    core: super::scsp_core::Core,
    owners: [Option<Lease>; 32],
    playing: [Option<(u32, u32)>; 32],
    gates: u32,
    gains: [f32; 2],
    targets: [f32; 2],
    ramp: u32,
    rate: u32,
    frame: u64,
}
impl Renderer {
    pub fn new(rate: u32) -> Result<Self, String> {
        Self::new_with_clock(rate, CLOCK)
    }
    pub fn new_with_clock(rate: u32, clock: u32) -> Result<Self, String> {
        if clock != CLOCK {
            return Err("SCSP uses a fixed 44100 Hz core".into());
        }
        Ok(Self {
            core: super::scsp_core::Core::new(rate).map_err(str::to_owned)?,
            owners: [None; 32],
            playing: [None; 32],
            gates: 0,
            gains: [1.; 2],
            targets: [1.; 2],
            ramp: 0,
            rate,
            frame: 0,
        })
    }
    pub fn set_frame(&mut self, frame: u64) {
        self.frame = frame;
    }
    fn manual_mask(&self, voice: Option<u64>, audition: bool) -> u32 {
        self.owners.iter().enumerate().fold(0, |m, (ch, l)| {
            m | if l.is_some_and(|l| l.audition && (!audition || l.voice != voice)) {
                1u32 << ch
            } else {
                0
            }
        })
    }
    fn write(&mut self, op: &Op) {
        let Op::Write {
            register,
            value,
            mask,
        } = *op;
        self.core
            .write(register as u32, value, mask.unwrap_or(65535))
            .unwrap();
        if local(register).is_some() && register % 32 == 0 {
            let ch = register as usize / 32;
            if mask.unwrap_or(65535) & 0x800 != 0 {
                if value & 0x800 != 0 {
                    self.gates |= 1u32 << ch;
                } else {
                    self.gates &= !(1u32 << ch);
                }
            }
        }
    }
    fn pin(&mut self, ch: usize) {
        let b = ch as u32 * 32;
        let control = self.core.read(b).unwrap();
        let address = ((control as u32 & 15) << 16) | self.core.read(b + 2).unwrap() as u32;
        let len = self.core.read(b + 6).unwrap() as u32;
        let bytes = (len + 1) * if control & 16 != 0 { 1 } else { 2 };
        self.playing[ch] = Some((address, (address + bytes).min(RAM as u32)));
    }
    fn valid_note(&self, ch: usize, ops: &[Op]) -> bool {
        let mut words = std::array::from_fn::<_, 16, _>(|i| {
            self.core.read(ch as u32 * 32 + i as u32 * 2).unwrap()
        });
        for op in ops {
            let Op::Write {
                register,
                value,
                mask,
            } = *op;
            let i = (register as usize % 32) / 2;
            let mask = mask.unwrap_or(65535);
            words[i] = (words[i] & !mask) | (value & mask);
        }
        let c = words[0];
        let at = ((c as u32 & 15) << 16) | words[1] as u32;
        let end = words[3] as u32;
        let width = if c & 16 != 0 { 1 } else { 2 };
        let looping = c & 0x60;
        if c & 0x180 != 0
            || !matches!(looping, 0 | 0x20)
            || end < 2
            || at % width != 0
            || at + (end + 1) * width > RAM as u32
        {
            return false;
        }
        if looping == 0x20 {
            let start = words[2] as u32;
            if start >= end {
                return false;
            }
            let octave = ((words[8] >> 11) & 15) as i32;
            let octave = if octave >= 8 { octave - 16 } else { octave };
            let step = (1. + (words[8] & 1023) as f64 / 1024.) * 2f64.powi(octave);
            if step > (end - start) as f64 {
                return false;
            }
        }
        true
    }
    fn release(&mut self, ch: usize) {
        self.core.write(ch as u32 * 32, 0x1000, 0x1800).unwrap();
        self.gates &= !(1u32 << ch);
        self.owners[ch] = None;
        // Release may be infinite. Only retrigger/Stop/reset retires its RAM ref.
    }
    fn silence(&mut self) {
        self.core.stop_voices();
        self.gates = 0;
        self.owners.fill(None);
        self.playing.fill(None);
    }
    pub fn apply(&mut self, c: &Prepared, owner: Option<Owner>, shared: &Shared) {
        let held = self.manual_mask(None, false);
        match &c.edit {
            Edit::Write(op) => {
                let Op::Write { register, .. } = op;
                if held == 0 && local(*register).is_none() {
                    self.write(op);
                } else {
                    c.rejected.store(true, Ordering::Release);
                }
            }
            Edit::Load(address, bytes) => {
                let end = *address + bytes.len() as u32;
                if self
                    .playing
                    .iter()
                    .flatten()
                    .any(|&(a, b)| *address < b && a < end)
                {
                    c.rejected.store(true, Ordering::Release);
                } else {
                    self.core.load_ram(*address, bytes).unwrap();
                }
            }
            Edit::Reset(clear) => {
                if held != 0 {
                    c.rejected.store(true, Ordering::Release);
                } else {
                    self.core.reset(*clear);
                    self.core.write(0x400, 0x020f, 65535).unwrap();
                    self.gates = 0;
                    self.owners.fill(None);
                    self.playing.fill(None);
                }
            }
            Edit::AllOff => {
                for ch in 0..32 {
                    if self.owners[ch].is_some_and(|l| {
                        !l.audition
                            && (owner.is_none() || l.owner == owner)
                            && (c.group.is_none() || l.group == c.group)
                    }) {
                        self.release(ch);
                    }
                }
            }
            Edit::Stop => {
                if owner.is_none() && c.group.is_none() && (c.audition || held == 0) {
                    self.silence();
                } else {
                    for ch in 0..32 {
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
                for ch in 0..32 {
                    if self.owners[ch].is_some_and(|l| {
                        l.owner.is_none() && !l.audition && l.group == Some(*group)
                    }) {
                        self.release(ch);
                    }
                }
            }
            Edit::Mix(volume, pan) => {
                self.targets = [
                    *volume * if *pan > 0. { 1. - *pan } else { 1. },
                    *volume * if *pan < 0. { 1. + *pan } else { 1. },
                ];
                self.ramp = (self.rate / 200).max(1);
            }
            Edit::Batch(ops, channel, voice, action) => {
                let lease = channel.and_then(|ch| self.owners[ch as usize]);
                let allowed = match action {
                    LeaseAction::On => c.audition || lease.is_none_or(|l| !l.audition),
                    LeaseAction::Off => lease.is_some_and(|l| {
                        l.owner == owner
                            && (c.group.is_none() || l.group == c.group)
                            && (voice.is_none() || l.voice == *voice)
                            && (!l.audition || c.audition)
                    }),
                    LeaseAction::Edit => {
                        lease.is_none_or(|l| !l.audition || c.audition && l.voice == *voice)
                    }
                };
                let allowed = allowed && (channel.is_some() || held == 0);
                if *action == LeaseAction::On && !self.valid_note(channel.unwrap() as usize, ops) {
                    c.invalid.store(true, Ordering::Release);
                } else if !allowed {
                    c.rejected.store(true, Ordering::Release);
                } else {
                    for op in ops {
                        self.write(op);
                    }
                    if let Some(ch) = channel {
                        let ch = *ch as usize;
                        if *action == LeaseAction::On {
                            self.pin(ch);
                            self.owners[ch] = Some(Lease {
                                owner,
                                group: c.group,
                                voice: *voice,
                                audition: c.audition,
                            });
                        } else if *action == LeaseAction::Off {
                            self.owners[ch] = None;
                            self.gates &= !(1u32 << ch);
                        }
                    }
                }
            }
        }
        shared.publish(self, Some(c));
    }
    pub fn publish(&self, shared: &Shared) {
        shared.publish(self, None);
    }
    pub fn cancel_invalid(&mut self, valid: impl Fn(Option<Owner>) -> bool, shared: &Shared) {
        let mut changed = false;
        for ch in 0..32 {
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
    pub fn mix(&mut self, pcm: &mut [f32; 2]) {
        let sample = self.core.sample();
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
