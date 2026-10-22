//! Callback-owned X68000 OPM + ADPCM. Sample stores are preallocated by C++;
//! command vectors and sample metadata are allocated/reclaimed on control only.
use super::{Owner, CAPACITY, MAX_SAFE_INTEGER, OWNERS};
use crossbeam_queue::ArrayQueue;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    ffi::c_void,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering},
        Arc, Mutex,
    },
};

pub const FM_CLOCK: u32 = 4_000_000;
pub const OKI_CLOCK: u32 = 8_000_000;
pub const MAX_SAMPLE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_BATCH_UPLOAD_BYTES: usize = MAX_SAMPLE_BYTES;
const CHUNK: usize = 65536;
const PENDING_BYTES: usize = 40 * 1024 * 1024;
const VOICES: usize = 9;
#[repr(C)]
struct CoreState {
    registers: [u8; 256],
    fm_clock: u32,
    oki_clock: u32,
    sample_bytes: u32,
    position: u32,
    fed_bytes: u32,
    divider: u32,
    fm_mute_mask: u32,
    status: u8,
    irq: u8,
    pan: u8,
    playing: u8,
    streaming: u8,
    repeat: u8,
    amd: u8,
    pmd: u8,
}
impl Default for CoreState {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}
extern "C" {
    fn native_x68000_create(rate: u32) -> *mut c_void;
    fn native_x68000_destroy(ptr: *mut c_void);
    fn native_x68000_reset(ptr: *mut c_void, chip: u8);
    fn native_x68000_stop(ptr: *mut c_void, chip: u8);
    fn native_x68000_key_off(ptr: *mut c_void, voice: u8);
    fn native_x68000_audition_key_on(ptr: *mut c_void, channel: u8, mask: u8);
    fn native_x68000_write(ptr: *mut c_void, chip: u8, register: u8, value: u8);
    fn native_x68000_sample(
        ptr: *mut c_void,
        fm_left: *mut f32,
        fm_right: *mut f32,
        oki_left: *mut f32,
        oki_right: *mut f32,
    );
    fn native_x68000_set_clock(ptr: *mut c_void, chip: u8, hz: u32) -> i32;
    fn native_x68000_set_fm_mute_mask(ptr: *mut c_void, mask: u32);
    fn native_x68000_sample_begin(ptr: *mut c_void, size: u32) -> i32;
    fn native_x68000_sample_chunk(
        ptr: *mut c_void,
        bytes: *const u8,
        offset: u32,
        size: u32,
    ) -> i32;
    fn native_x68000_sample_commit(ptr: *mut c_void) -> i32;
    fn native_x68000_sample_abort(ptr: *mut c_void);
    fn native_x68000_oki_play(ptr: *mut c_void, repeat: u8) -> i32;
    fn native_x68000_oki_set_repeat(ptr: *mut c_void, repeat: u8);
    fn native_x68000_get_state(ptr: *mut c_void, out: *mut CoreState);
}
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Chip {
    Ym2151,
    Okim6258,
}
impl Chip {
    fn code(self) -> u8 {
        if self == Self::Ym2151 {
            0
        } else {
            1
        }
    }
    fn voice(self, ch: u8) -> usize {
        if self == Self::Ym2151 {
            ch as usize
        } else {
            8
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    #[serde(default)]
    pub name: String,
    pub frames: u32,
    pub sample_rate: f64,
    #[serde(default, deserialize_with = "nullable_waveform")]
    pub waveform: Vec<f32>,
    pub source: String,
}
fn nullable_waveform<'de, D: serde::Deserializer<'de>>(de: D) -> Result<Vec<f32>, D::Error> {
    Ok(Option::<Vec<f32>>::deserialize(de)?.unwrap_or_default())
}
#[derive(Clone, Debug, Deserialize)]
pub struct Message {
    pub chip: Chip,
    #[serde(default)]
    pub id: Option<u64>,
    #[serde(default)]
    pub audition: bool,
    #[serde(flatten)]
    pub method: Method,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "method", rename_all = "kebab-case")]
pub enum Method {
    Write {
        register: u8,
        value: u8,
    },
    KeyOff {
        channel: u8,
    },
    Clock {
        hz: u32,
    },
    Mix {
        volume: f32,
        pan: f32,
    },
    SourceMute {
        mask: u32,
    },
    Reset,
    StopVoices,
    MemoryBegin {
        #[serde(rename = "sampleId")]
        sample_id: u64,
        #[serde(rename = "byteLength")]
        byte_length: u32,
        metadata: Metadata,
    },
    MemoryChunk {
        #[serde(rename = "sampleId")]
        sample_id: u64,
        offset: u32,
        data: Vec<u8>,
    },
    MemoryCommit {
        #[serde(rename = "sampleId")]
        sample_id: u64,
    },
    Play {
        #[serde(default)]
        repeat: bool,
    },
    Stop,
    SetRepeat {
        repeat: bool,
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
    },
    KeyOff {
        channel: u8,
    },
    Clock {
        hz: u32,
    },
    Play {
        #[serde(default)]
        repeat: bool,
    },
    Stop,
    SetRepeat {
        repeat: bool,
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
pub struct Prepared {
    pub id: Option<u64>,
    chip: Chip,
    audition: bool,
    method: Method,
    complete: AtomicBool,
}
fn valid_id(id: Option<u64>) -> bool {
    id.is_none_or(|v| v > 0 && v <= MAX_SAFE_INTEGER)
}
fn channel(op: &Op, chip: Chip) -> Option<usize> {
    if chip == Chip::Okim6258 {
        return Some(8);
    }
    match *op {
        Op::KeyOff { channel } => Some(channel as usize),
        Op::Write { register: 8, value } => Some((value & 7) as usize),
        Op::Write { register: 15, .. } => Some(7),
        Op::Write {
            register: 0x20..=0xff,
            ..
        } => {
            if let Op::Write { register, .. } = op {
                Some((register & 7) as usize)
            } else {
                None
            }
        }
        _ => None,
    }
}
fn validate_op(op: &Op, chip: Chip) -> Result<(), String> {
    match *op {
        Op::KeyOff { channel } if channel >= if chip == Chip::Ym2151 { 8 } else { 1 } => {
            Err("Invalid Native X68000 channel".into())
        }
        Op::Clock { hz } if !(100_000..=100_000_000).contains(&hz) => {
            Err("Native X68000 clock must be 100000–100000000 Hz".into())
        }
        Op::Write { register, .. }
            if chip == Chip::Okim6258 && !matches!(register,0..=2|8..=12) =>
        {
            Err("Invalid Native OKIM6258 register".into())
        }
        Op::Play { .. } | Op::Stop | Op::SetRepeat { .. } if chip == Chip::Ym2151 => {
            Err("Sample playback requires OKIM6258".into())
        }
        _ => Ok(()),
    }
}
impl Message {
    pub fn prepare(&self) -> Result<Arc<Prepared>, String> {
        if !valid_id(self.id) {
            return Err("Invalid Native X68000 command ID".into());
        }
        match &self.method {
            Method::Write { register, value } => validate_op(
                &Op::Write {
                    register: *register,
                    value: *value,
                },
                self.chip,
            )?,
            Method::KeyOff { channel } => {
                validate_op(&Op::KeyOff { channel: *channel }, self.chip)?
            }
            Method::Clock { hz } => validate_op(&Op::Clock { hz: *hz }, self.chip)?,
            Method::Mix { volume, pan }
                if !volume.is_finite()
                    || !(0.0..=4.0).contains(volume)
                    || !pan.is_finite()
                    || !(-1.0..=1.0).contains(pan) =>
            {
                return Err("Native X68000 mixer volume must be 0–4, pan −1–1".into())
            }
            Method::SourceMute { mask } if self.chip != Chip::Ym2151 || *mask > 255 => {
                return Err("Native YM2151 source mask must be 0–255".into())
            }
            Method::MemoryBegin {
                sample_id,
                byte_length,
                metadata,
            } => {
                if self.chip != Chip::Okim6258
                    || !valid_id(Some(*sample_id))
                    || *byte_length == 0
                    || *byte_length as usize > MAX_SAMPLE_BYTES
                    || metadata.name.len() > 1024
                    || metadata.name.chars().count() > 256
                    || metadata.frames == 0
                    || metadata.frames > *byte_length * 2
                    || metadata.frames + 1 < *byte_length * 2
                    || !metadata.sample_rate.is_finite()
                    || !(1.0..=1_000_000.0).contains(&metadata.sample_rate)
                    || metadata.waveform.len() > 512
                    || metadata
                        .waveform
                        .iter()
                        .any(|v| !v.is_finite() || !(-1.0..=1.0).contains(v))
                    || !matches!(metadata.source.as_str(), "raw" | "sample")
                {
                    return Err(
                        "Invalid Native OKIM6258 sample metadata or 16 MiB upload size".into(),
                    );
                }
            }
            Method::MemoryChunk {
                sample_id,
                offset,
                data,
            } => {
                if self.chip != Chip::Okim6258
                    || !valid_id(Some(*sample_id))
                    || data.is_empty()
                    || data.len() > CHUNK
                    || (*offset as usize)
                        .checked_add(data.len())
                        .is_none_or(|end| end > MAX_SAMPLE_BYTES)
                {
                    return Err(
                        "Native OKIM6258 chunks must fit 16 MiB (64 KiB per command)".into(),
                    );
                }
            }
            Method::MemoryCommit { sample_id } => {
                if self.chip != Chip::Okim6258 || !valid_id(Some(*sample_id)) {
                    return Err("Invalid Native OKIM6258 sample ID".into());
                }
            }
            Method::Play { .. } | Method::Stop | Method::SetRepeat { .. }
                if self.chip != Chip::Okim6258 =>
            {
                return Err("Sample playback requires OKIM6258".into())
            }
            Method::Batch {
                commands,
                channel: declared,
                voice,
                action,
            } => {
                if *declared >= if self.chip == Chip::Ym2151 { 8 } else { 1 }
                    || commands.is_empty()
                    || commands.len() > 512
                    || !valid_id(*voice)
                    || self.audition && *action != LeaseAction::Edit && voice.is_none()
                {
                    return Err("Invalid Native X68000 batch or voice lease".into());
                }
                for op in commands {
                    validate_op(op, self.chip)?;
                    if channel(op, self.chip).is_some_and(|ch| ch != self.chip.voice(*declared)) {
                        return Err("Native X68000 batch must target its declared voice".into());
                    }
                }
            }
            _ => {}
        }
        Ok(Arc::new(Prepared {
            id: self.id,
            chip: self.chip,
            audition: self.audition,
            method: self.method.clone(),
            complete: AtomicBool::new(false),
        }))
    }
}
impl Prepared {
    pub fn starts(&self) -> bool {
        fn starts(op: &Op, chip: Chip) -> bool {
            match op {
                Op::Play { .. } => true,
                Op::Write { register: 8, value } if chip == Chip::Ym2151 => value & 0x78 != 0,
                Op::Write { register: 0, value } if chip == Chip::Okim6258 => {
                    value & 2 != 0 && value & 1 == 0
                }
                _ => false,
            }
        }
        match &self.method {
            Method::Write { register, value } => starts(
                &Op::Write {
                    register: *register,
                    value: *value,
                },
                self.chip,
            ),
            Method::Play { .. } => true,
            Method::Batch {
                commands, action, ..
            } => *action == LeaseAction::On || commands.iter().any(|op| starts(op, self.chip)),
            _ => false,
        }
    }
    pub fn stops(&self) -> bool {
        matches!(self.method, Method::StopVoices)
    }
    fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + match &self.method {
                Method::MemoryChunk { data, .. } => data.len(),
                Method::MemoryBegin { metadata, .. } => {
                    metadata.name.len() + metadata.waveform.len() * 4
                }
                Method::Batch { commands, .. } => commands.len() * std::mem::size_of::<Op>(),
                _ => 0,
            }
    }
}
#[derive(Clone, Copy)]
struct UploadPlan {
    id: u64,
    size: u32,
    received: u32,
    owner: Option<Owner>,
}
#[derive(Default)]
struct Control {
    prepared: Vec<Arc<Prepared>>,
    metadata: HashMap<u64, (u32, Metadata)>,
    upload: Option<UploadPlan>,
}
pub struct Shared {
    revision: AtomicU64,
    registers: [AtomicU8; 256],
    key_masks: [AtomicU8; 8],
    clock: [AtomicU32; 2],
    mute: AtomicU32,
    status: AtomicU8,
    irq: AtomicU8,
    amd: AtomicU8,
    pmd: AtomicU8,
    held: AtomicU32,
    rate: AtomicU32,
    sample_id: AtomicU64,
    staging_id: AtomicU64,
    memory_revision: AtomicU64,
    position: AtomicU32,
    fed_bytes: AtomicU32,
    divider: AtomicU32,
    pan: AtomicU8,
    playing: AtomicBool,
    streaming: AtomicBool,
    repeat: AtomicBool,
    control: Mutex<Control>,
    retired: ArrayQueue<Arc<Prepared>>,
    highest_id: AtomicU64,
    cancel_before: [AtomicU64; 2],
    owner_cancel_before: [[AtomicU64; OWNERS]; 2],
}
impl Default for Shared {
    fn default() -> Self {
        Self {
            revision: AtomicU64::new(0),
            registers: std::array::from_fn(|_| AtomicU8::new(0)),
            key_masks: std::array::from_fn(|_| AtomicU8::new(0)),
            clock: [AtomicU32::new(FM_CLOCK), AtomicU32::new(OKI_CLOCK)],
            mute: AtomicU32::new(0),
            status: AtomicU8::new(0),
            irq: AtomicU8::new(0),
            amd: AtomicU8::new(0),
            pmd: AtomicU8::new(0),
            held: AtomicU32::new(0),
            rate: AtomicU32::new(0),
            sample_id: AtomicU64::new(0),
            staging_id: AtomicU64::new(0),
            memory_revision: AtomicU64::new(0),
            position: AtomicU32::new(0),
            fed_bytes: AtomicU32::new(0),
            divider: AtomicU32::new(512),
            pan: AtomicU8::new(0),
            playing: AtomicBool::new(false),
            streaming: AtomicBool::new(false),
            repeat: AtomicBool::new(false),
            control: Mutex::new(Control::default()),
            retired: ArrayQueue::new(CAPACITY),
            highest_id: AtomicU64::new(0),
            cancel_before: std::array::from_fn(|_| AtomicU64::new(0)),
            owner_cancel_before: std::array::from_fn(|_| {
                std::array::from_fn(|_| AtomicU64::new(0))
            }),
        }
    }
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FmStatus {
    pub clock: u32,
    pub registers: Vec<u8>,
    pub key_masks: Vec<u8>,
    pub am_depth: u8,
    pub pm_depth: u8,
    pub status: u8,
    pub irq: bool,
    pub source_mute_mask: u32,
    pub held: Vec<u8>,
    pub pending_commands: Vec<u64>,
    pub sample_rate: u32,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sample {
    pub name: String,
    pub bytes: u32,
    pub frames: u32,
    pub sample_rate: f64,
    pub duration_seconds: f64,
    pub waveform: Vec<f32>,
    pub source: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OkiStatus {
    pub clock: u32,
    pub divider: u32,
    pub pan: u8,
    pub playing: bool,
    pub streaming: bool,
    pub repeat: bool,
    pub position_frames: u32,
    pub fed_bytes: u32,
    pub memory_revision: u64,
    pub sample_id: u64,
    pub sample: Option<Sample>,
    pub held: Vec<u8>,
    pub pending_commands: Vec<u64>,
    pub sample_rate: u32,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub ym2151: FmStatus,
    pub okim6258: OkiStatus,
    pub sample_rate: u32,
    pub frame: u64,
    pub revision: u64,
}
impl Shared {
    pub fn valid_event(&self, command: &Prepared, sequence: u64, owner: Option<Owner>) -> bool {
        let chip = command.chip.code() as usize;
        sequence >= self.cancel_before[chip].load(Ordering::Acquire)
            && owner.is_none_or(|owner| {
                sequence
                    >= self.owner_cancel_before[chip][(owner.owner - 1) as usize]
                        .load(Ordering::Acquire)
            })
    }
    pub fn cancel_before(&self, command: &Prepared, sequence: u64, owner: Option<Owner>) {
        let chip = command.chip.code() as usize;
        if let Some(owner) = owner {
            self.owner_cancel_before[chip][(owner.owner - 1) as usize]
                .fetch_max(sequence, Ordering::AcqRel);
        } else {
            self.cancel_before[chip].fetch_max(sequence, Ordering::AcqRel);
        }
    }
    pub fn clock(&self) -> u32 {
        self.clock[0].load(Ordering::Acquire)
    }
    pub fn oki_clock(&self) -> u32 {
        self.clock[1].load(Ordering::Acquire)
    }
    fn collect(&self, c: &mut Control) {
        let mut retired = HashSet::new();
        while let Some(cmd) = self.retired.pop() {
            retired.insert(Arc::as_ptr(&cmd) as usize);
        }
        c.prepared
            .retain(|cmd| !retired.contains(&(Arc::as_ptr(cmd) as usize)));
        let mut live = HashSet::from([
            self.sample_id.load(Ordering::Acquire),
            self.staging_id.load(Ordering::Acquire),
        ]);
        for cmd in &c.prepared {
            if let Method::MemoryBegin { sample_id, .. } = cmd.method {
                live.insert(sample_id);
            }
        }
        c.metadata.retain(|id, _| live.contains(id));
    }
    pub fn retain(&self, commands: &[(Arc<Prepared>, Option<Owner>)]) -> Result<(), String> {
        let mut c = self
            .control
            .lock()
            .map_err(|_| "Native X68000 control state unavailable")?;
        self.collect(&mut c);
        if c.prepared.len() + commands.len() > CAPACITY
            || c.prepared.iter().map(|v| v.bytes()).sum::<usize>()
                + commands.iter().map(|(v, _)| v.bytes()).sum::<usize>()
                > PENDING_BYTES
        {
            return Err("Native X68000 pending command limit reached (40 MiB)".into());
        }
        let mut ids = HashSet::new();
        for id in c
            .prepared
            .iter()
            .filter_map(|v| v.id)
            .chain(commands.iter().filter_map(|(v, _)| v.id))
        {
            if !ids.insert(id) {
                return Err("Duplicate pending Native X68000 command ID".into());
            }
        }
        let mut plan = c.upload;
        let mut sample_ids: HashSet<u64> = c.metadata.keys().copied().collect();
        for (cmd, owner) in commands {
            match &cmd.method {
                Method::MemoryBegin {
                    sample_id,
                    byte_length,
                    ..
                } => {
                    if self.held.load(Ordering::Acquire) & 256 != 0 && !cmd.audition {
                        return Err("Native OKIM6258 sample is held by manual audition".into());
                    }
                    if !sample_ids.insert(*sample_id) {
                        return Err("Native OKIM6258 sample ID is already in use".into());
                    }
                    plan = Some(UploadPlan {
                        id: *sample_id,
                        size: *byte_length,
                        received: 0,
                        owner: *owner,
                    });
                }
                Method::MemoryChunk {
                    sample_id,
                    offset,
                    data,
                } => {
                    let p = plan
                        .as_mut()
                        .ok_or("Native OKIM6258 upload has not begun")?;
                    if p.id != *sample_id
                        || p.owner != *owner
                        || p.received != *offset
                        || p.received + data.len() as u32 > p.size
                    {
                        return Err("Native OKIM6258 upload chunks must be contiguous and retain their owner".into());
                    }
                    p.received += data.len() as u32;
                }
                Method::MemoryCommit { sample_id } => {
                    let p = plan.ok_or("Native OKIM6258 upload has not begun")?;
                    if p.id != *sample_id || p.owner != *owner || p.received != p.size {
                        return Err(
                            "Native OKIM6258 upload must be complete and retain its owner".into(),
                        );
                    }
                    plan = None;
                }
                Method::StopVoices | Method::Reset if cmd.chip == Chip::Okim6258 => {
                    if owner.is_none() || plan.is_some_and(|p| p.owner == *owner) {
                        plan = None;
                    }
                }
                _ => {}
            }
        }
        c.upload = plan;
        for (cmd, _) in commands {
            if let Method::MemoryBegin {
                sample_id,
                byte_length,
                metadata,
            } = &cmd.method
            {
                c.metadata
                    .insert(*sample_id, (*byte_length, metadata.clone()));
            }
            c.prepared.push(cmd.clone());
            if let Some(id) = cmd.id {
                self.highest_id.fetch_max(id, Ordering::Release);
            }
        }
        Ok(())
    }
    pub fn invalidate_upload(&self) {
        if let Ok(mut c) = self.control.lock() {
            c.upload = None;
        }
    }
    pub fn wait_applied(&self, id: u64) -> Result<(), String> {
        if !valid_id(Some(id)) || id > self.highest_id.load(Ordering::Acquire) {
            return Err("Unknown Native X68000 command ID".into());
        }
        let cmd = self
            .control
            .lock()
            .map_err(|_| "Native X68000 control state unavailable")?
            .prepared
            .iter()
            .find(|v| v.id == Some(id))
            .cloned();
        if let Some(cmd) = cmd {
            let start = std::time::Instant::now();
            while !cmd.complete.load(Ordering::Acquire) {
                if start.elapsed() >= std::time::Duration::from_millis(100) {
                    return Err("Native X68000 command was not applied within 100 ms".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        Ok(())
    }
    pub fn retire(&self, cmd: Arc<Prepared>) {
        if !cmd.complete.load(Ordering::Relaxed) {
            self.revision.fetch_add(1, Ordering::AcqRel);
            cmd.complete.store(true, Ordering::Relaxed);
            self.revision.fetch_add(1, Ordering::Release);
        }
        self.retired
            .push(cmd)
            .unwrap_or_else(|_| unreachable!("Bounded X68000 command retirement"));
    }
    fn publish(&self, r: &Renderer, command: Option<&Prepared>) {
        let s = r.state();
        self.revision.fetch_add(1, Ordering::AcqRel);
        for (out, value) in self.registers.iter().zip(s.registers) {
            out.store(value, Ordering::Relaxed)
        }
        for (out, value) in self.key_masks.iter().zip(r.key_masks) {
            out.store(value, Ordering::Relaxed)
        }
        self.clock[0].store(s.fm_clock, Ordering::Relaxed);
        self.clock[1].store(s.oki_clock, Ordering::Relaxed);
        self.mute.store(s.fm_mute_mask, Ordering::Relaxed);
        self.status.store(s.status, Ordering::Relaxed);
        self.irq.store(s.irq, Ordering::Relaxed);
        self.amd.store(s.amd, Ordering::Relaxed);
        self.pmd.store(s.pmd, Ordering::Relaxed);
        self.held.store(
            r.owners.iter().enumerate().fold(0, |bits, (ch, lease)| {
                bits | if lease.is_some_and(|l| l.audition) {
                    1 << ch
                } else {
                    0
                }
            }),
            Ordering::Relaxed,
        );
        self.rate.store(r.rate, Ordering::Relaxed);
        self.sample_id.store(r.sample_id, Ordering::Relaxed);
        self.staging_id
            .store(r.upload.map_or(0, |v| v.id), Ordering::Relaxed);
        self.memory_revision
            .store(r.memory_revision, Ordering::Relaxed);
        self.position.store(s.position, Ordering::Relaxed);
        self.fed_bytes.store(s.fed_bytes, Ordering::Relaxed);
        self.divider.store(s.divider, Ordering::Relaxed);
        self.pan.store(s.pan, Ordering::Relaxed);
        self.playing.store(s.playing != 0, Ordering::Relaxed);
        self.streaming.store(s.streaming != 0, Ordering::Relaxed);
        self.repeat.store(s.repeat != 0, Ordering::Relaxed);
        if let Some(cmd) = command {
            cmd.complete.store(true, Ordering::Relaxed)
        }
        self.revision.fetch_add(1, Ordering::Release);
    }
    pub fn status(&self, frame: u64) -> Result<Status, String> {
        let mut c = self
            .control
            .lock()
            .map_err(|_| "Native X68000 control state unavailable")?;
        self.collect(&mut c);
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() >= std::time::Duration::from_millis(50) {
                return Err("Native X68000 state changed while reading; retry".into());
            }
            let before = self.revision.load(Ordering::Acquire);
            if before & 1 != 0 {
                std::thread::yield_now();
                continue;
            }
            let held = self.held.load(Ordering::Relaxed);
            let rate = self.rate.load(Ordering::Relaxed);
            let pending = |chip| {
                c.prepared
                    .iter()
                    .filter(|v| v.chip == chip && !v.complete.load(Ordering::Relaxed))
                    .filter_map(|v| v.id)
                    .collect()
            };
            let sample =
                c.metadata
                    .get(&self.sample_id.load(Ordering::Relaxed))
                    .map(|(bytes, m)| Sample {
                        name: m.name.clone(),
                        bytes: *bytes,
                        frames: m.frames,
                        sample_rate: m.sample_rate,
                        duration_seconds: m.frames as f64 / m.sample_rate,
                        waveform: m.waveform.clone(),
                        source: m.source.clone(),
                    });
            let out = Status {
                ym2151: FmStatus {
                    clock: self.clock[0].load(Ordering::Relaxed),
                    registers: self
                        .registers
                        .iter()
                        .map(|v| v.load(Ordering::Relaxed))
                        .collect(),
                    key_masks: self
                        .key_masks
                        .iter()
                        .map(|v| v.load(Ordering::Relaxed))
                        .collect(),
                    am_depth: self.amd.load(Ordering::Relaxed),
                    pm_depth: self.pmd.load(Ordering::Relaxed),
                    status: self.status.load(Ordering::Relaxed),
                    irq: self.irq.load(Ordering::Relaxed) != 0,
                    source_mute_mask: self.mute.load(Ordering::Relaxed),
                    held: (0..8).filter(|ch| held & (1 << ch) != 0).collect(),
                    pending_commands: pending(Chip::Ym2151),
                    sample_rate: rate,
                },
                okim6258: OkiStatus {
                    clock: self.clock[1].load(Ordering::Relaxed),
                    divider: self.divider.load(Ordering::Relaxed),
                    pan: self.pan.load(Ordering::Relaxed),
                    playing: self.playing.load(Ordering::Relaxed),
                    streaming: self.streaming.load(Ordering::Relaxed),
                    repeat: self.repeat.load(Ordering::Relaxed),
                    position_frames: self.position.load(Ordering::Relaxed),
                    fed_bytes: self.fed_bytes.load(Ordering::Relaxed),
                    memory_revision: self.memory_revision.load(Ordering::Relaxed),
                    sample_id: self.sample_id.load(Ordering::Relaxed),
                    sample,
                    held: if held & 256 != 0 { vec![0] } else { vec![] },
                    pending_commands: pending(Chip::Okim6258),
                    sample_rate: rate,
                },
                sample_rate: rate,
                frame,
                revision: before / 2,
            };
            std::sync::atomic::fence(Ordering::Acquire);
            if self.revision.load(Ordering::Relaxed) == before {
                return Ok(out);
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
    rate: u32,
    owners: [Option<Lease>; VOICES],
    key_masks: [u8; 8],
    gains: [[f32; 2]; 2],
    targets: [[f32; 2]; 2],
    ramps: [u32; 2],
    upload: Option<UploadPlan>,
    sample_id: u64,
    memory_revision: u64,
}
unsafe impl Send for Renderer {}
impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe { native_x68000_destroy(self.ptr) }
    }
}
impl Renderer {
    pub fn new(rate: u32) -> Result<Self, String> {
        let ptr = unsafe { native_x68000_create(rate) };
        if ptr.is_null() {
            return Err("Could not allocate Native X68000 core".into());
        }
        Ok(Self {
            ptr,
            rate,
            owners: [None; VOICES],
            key_masks: [0; 8],
            gains: [[1.0; 2]; 2],
            targets: [[1.0; 2]; 2],
            ramps: [0; 2],
            upload: None,
            sample_id: 0,
            memory_revision: 0,
        })
    }
    fn state(&self) -> CoreState {
        let mut state = CoreState::default();
        unsafe { native_x68000_get_state(self.ptr, &mut state) }
        state
    }
    fn protected(&self, ch: usize, voice: Option<u64>, audition: bool) -> bool {
        self.owners[ch]
            .is_some_and(|l| l.audition && (!audition || voice.is_some() && l.voice != voice))
    }
    fn release(&mut self, ch: usize) {
        unsafe { native_x68000_key_off(self.ptr, ch as u8) }
        self.owners[ch] = None;
        if ch < 8 {
            self.key_masks[ch] = 0
        }
    }
    fn abort_upload(&mut self) {
        unsafe { native_x68000_sample_abort(self.ptr) }
        self.upload = None;
    }
    fn op(
        &mut self,
        chip: Chip,
        op: &Op,
        owner: Option<Owner>,
        voice: Option<u64>,
        audition: bool,
        preserve: bool,
    ) {
        let target = channel(op, chip);
        if target.is_some_and(|ch| self.protected(ch, voice, audition)) {
            return;
        }
        if chip == Chip::Ym2151
            && !audition
            && (matches!(op, Op::Write {register, ..} if *register < 0x20 && !matches!(*register,8|15))
                || matches!(op, Op::Clock { .. }))
            && (0..8).any(|ch| self.protected(ch, voice, false))
        {
            return;
        }
        let off = matches!(op, Op::KeyOff { .. } | Op::Stop)
            || matches!(op,Op::Write{register:8,value}if chip==Chip::Ym2151&&value&0x78==0)
            || matches!(op,Op::Write{register:0,value}if chip==Chip::Okim6258&&value&1!=0);
        if off
            && target.is_some_and(|ch| {
                self.owners[ch].is_some_and(|l| {
                    owner.is_some() && l.owner != owner || voice.is_some() && l.voice != voice
                })
            })
        {
            return;
        }
        unsafe {
            match *op {
                Op::Write { register, value } => {
                    if chip == Chip::Ym2151 && audition && register == 8 && value & 0x78 != 0 {
                        native_x68000_audition_key_on(self.ptr, value & 7, (value >> 3) & 15);
                    } else {
                        native_x68000_write(self.ptr, chip.code(), register, value);
                    }
                    if chip == Chip::Ym2151 && register == 8 {
                        self.key_masks[(value & 7) as usize] = (value >> 3) & 15;
                    }
                }
                Op::KeyOff { channel } => {
                    native_x68000_key_off(self.ptr, chip.voice(channel) as u8);
                    if chip == Chip::Ym2151 {
                        self.key_masks[channel as usize] = 0
                    }
                }
                Op::Clock { hz } => {
                    native_x68000_set_clock(self.ptr, chip.code(), hz);
                }
                Op::Play { repeat } => {
                    native_x68000_oki_play(self.ptr, repeat as u8);
                }
                Op::Stop => native_x68000_stop(self.ptr, 1),
                Op::SetRepeat { repeat } => native_x68000_oki_set_repeat(self.ptr, repeat as u8),
            }
        }
        if !preserve {
            if let Some(ch) = target {
                if off {
                    self.owners[ch] = None
                } else if matches!(op, Op::Play { .. })
                    || matches!(op,Op::Write{register:8,..}if chip==Chip::Ym2151)
                    || matches!(op,Op::Write{register:0,value}if chip==Chip::Okim6258&&value&2!=0)
                {
                    self.owners[ch] = Some(Lease {
                        owner,
                        voice,
                        audition,
                    });
                }
            }
        }
    }
    pub fn apply(&mut self, cmd: &Prepared, owner: Option<Owner>, shared: &Shared) {
        let chip = cmd.chip;
        match &cmd.method {
            Method::Write { register, value } => self.op(
                chip,
                &Op::Write {
                    register: *register,
                    value: *value,
                },
                owner,
                None,
                cmd.audition,
                false,
            ),
            Method::KeyOff { channel } => self.op(
                chip,
                &Op::KeyOff { channel: *channel },
                owner,
                None,
                cmd.audition,
                false,
            ),
            Method::Clock { hz } => self.op(
                chip,
                &Op::Clock { hz: *hz },
                owner,
                None,
                cmd.audition,
                false,
            ),
            Method::Play { repeat } => self.op(
                chip,
                &Op::Play { repeat: *repeat },
                owner,
                None,
                cmd.audition,
                false,
            ),
            Method::Stop => self.op(chip, &Op::Stop, owner, None, cmd.audition, false),
            Method::SetRepeat { repeat } => self.op(
                chip,
                &Op::SetRepeat { repeat: *repeat },
                owner,
                None,
                cmd.audition,
                false,
            ),
            Method::Mix { volume, pan } => {
                let i = chip.code() as usize;
                self.targets[i] = [
                    *volume * if *pan > 0.0 { 1.0 - *pan } else { 1.0 },
                    *volume * if *pan < 0.0 { 1.0 + *pan } else { 1.0 },
                ];
                self.ramps[i] = (self.rate / 200).max(1);
            }
            Method::SourceMute { mask } => unsafe {
                native_x68000_set_fm_mute_mask(self.ptr, *mask)
            },
            Method::Reset => {
                let range = if chip == Chip::Ym2151 { 0..8 } else { 8..9 };
                if cmd.audition || !range.clone().any(|ch| self.protected(ch, None, false)) {
                    unsafe { native_x68000_reset(self.ptr, chip.code()) }
                    for ch in range {
                        self.owners[ch] = None;
                    }
                    if chip == Chip::Ym2151 {
                        self.key_masks.fill(0)
                    } else {
                        self.abort_upload()
                    }
                }
            }
            Method::StopVoices => {
                let range = if chip == Chip::Ym2151 { 0..8 } else { 8..9 };
                if owner.is_none() {
                    unsafe { native_x68000_stop(self.ptr, chip.code()) }
                    for ch in range {
                        self.owners[ch] = None;
                        if ch < 8 {
                            self.key_masks[ch] = 0
                        }
                    }
                    if chip == Chip::Okim6258 {
                        self.abort_upload()
                    }
                } else {
                    for ch in range {
                        if self.owners[ch]
                            .is_some_and(|l| l.owner == owner && (!l.audition || cmd.audition))
                        {
                            self.release(ch)
                        }
                    }
                    if chip == Chip::Okim6258 && self.upload.is_some_and(|u| u.owner == owner) {
                        self.abort_upload()
                    }
                }
            }
            Method::MemoryBegin {
                sample_id,
                byte_length,
                ..
            } => {
                if !self.protected(8, None, cmd.audition) {
                    self.abort_upload();
                    if unsafe { native_x68000_sample_begin(self.ptr, *byte_length) } != 0 {
                        self.upload = Some(UploadPlan {
                            id: *sample_id,
                            size: *byte_length,
                            received: 0,
                            owner,
                        });
                    }
                }
            }
            Method::MemoryChunk {
                sample_id,
                offset,
                data,
            } => {
                if let Some(p) = &mut self.upload {
                    if p.id == *sample_id
                        && p.owner == owner
                        && p.received == *offset
                        && p.received + data.len() as u32 <= p.size
                    {
                        if unsafe {
                            native_x68000_sample_chunk(
                                self.ptr,
                                data.as_ptr(),
                                *offset,
                                data.len() as u32,
                            )
                        } != 0
                        {
                            p.received += data.len() as u32;
                        }
                    }
                }
            }
            Method::MemoryCommit { sample_id } => {
                if self
                    .upload
                    .is_some_and(|p| p.id == *sample_id && p.owner == owner && p.received == p.size)
                    && !self.protected(8, None, cmd.audition)
                {
                    if unsafe { native_x68000_sample_commit(self.ptr) } != 0 {
                        self.sample_id = *sample_id;
                        self.memory_revision += 1;
                        self.upload = None;
                        self.owners[8] = None;
                    }
                } else if self
                    .upload
                    .is_some_and(|p| p.id == *sample_id && p.owner == owner)
                {
                    // A preview may have taken the voice while this script was
                    // uploading. Keep its committed sample and discard staging.
                    self.abort_upload();
                }
            }
            Method::Batch {
                commands,
                channel,
                voice,
                action,
            } => {
                let ch = chip.voice(*channel);
                let current = self.owners[ch];
                let allowed = match action {
                    LeaseAction::On => cmd.audition || current.is_none_or(|l| !l.audition),
                    LeaseAction::Off => current.is_some_and(|l| {
                        l.owner == owner
                            && (voice.is_none() || l.voice == *voice)
                            && (!l.audition || cmd.audition)
                    }),
                    LeaseAction::Edit => current.is_none_or(|l| {
                        !l.audition || cmd.audition && (voice.is_none() || l.voice == *voice)
                    }),
                };
                if allowed {
                    if *action == LeaseAction::On {
                        self.owners[ch] = Some(Lease {
                            owner,
                            voice: *voice,
                            audition: cmd.audition,
                        });
                    }
                    for op in commands {
                        self.op(
                            chip,
                            op,
                            owner,
                            *voice,
                            cmd.audition,
                            *action == LeaseAction::Edit,
                        );
                    }
                    if *action == LeaseAction::On {
                        self.owners[ch] = Some(Lease {
                            owner,
                            voice: *voice,
                            audition: cmd.audition,
                        });
                    }
                    if *action == LeaseAction::Off {
                        self.owners[ch] = None;
                    }
                }
            }
        }
        shared.publish(self, Some(cmd));
    }
    pub fn publish(&self, shared: &Shared) {
        shared.publish(self, None)
    }
    pub fn cancel_invalid(&mut self, valid: impl Fn(Option<Owner>) -> bool, shared: &Shared) {
        let mut changed = false;
        for ch in 0..VOICES {
            if self.owners[ch].is_some_and(|l| !valid(l.owner)) {
                self.release(ch);
                changed = true
            }
        }
        if self.upload.is_some_and(|u| !valid(u.owner)) {
            self.abort_upload();
            changed = true
        }
        if changed {
            shared.publish(self, None)
        }
    }
    pub fn stop(&mut self, shared: &Shared) {
        unsafe { native_x68000_stop(self.ptr, 2) }
        self.abort_upload();
        self.owners.fill(None);
        self.key_masks.fill(0);
        shared.publish(self, None)
    }
    pub fn mix(&mut self, pcm: &mut [f32; 2]) {
        let (mut fl, mut fr, mut ol, mut or) = (0.0, 0.0, 0.0, 0.0);
        unsafe { native_x68000_sample(self.ptr, &mut fl, &mut fr, &mut ol, &mut or) }
        for i in 0..2 {
            if self.ramps[i] > 0 {
                for ch in 0..2 {
                    self.gains[i][ch] +=
                        (self.targets[i][ch] - self.gains[i][ch]) / self.ramps[i] as f32;
                }
                self.ramps[i] -= 1;
            }
        }
        pcm[0] += fl * self.gains[0][0] + ol * self.gains[1][0];
        pcm[1] += fr * self.gains[0][1] + or * self.gains[1][1];
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
    fn apply(r: &mut Renderer, s: &Shared, value: Value, owner: Option<Owner>) {
        let c = prepare(value);
        s.retain(&[(c.clone(), owner)]).unwrap();
        r.apply(&c, owner, s);
        s.retire(c);
    }
    fn write(chip: &str, register: u8, value: u8) -> Value {
        json!({"chip":chip,"method":"write","register":register,"value":value})
    }
    fn begin(id: u64, len: u32, name: &str) -> Value {
        json!({"chip":"okim6258","method":"memory-begin","sampleId":id,"byteLength":len,"metadata":{"name":name,"frames":len*2,"sampleRate":15625,"waveform":[0.0,0.5,-0.5],"source":"raw"}})
    }
    fn chunk(id: u64, offset: u32, data: Vec<u8>) -> Value {
        json!({"chip":"okim6258","method":"memory-chunk","sampleId":id,"offset":offset,"data":data})
    }
    fn commit(id: u64) -> Value {
        json!({"chip":"okim6258","method":"memory-commit","sampleId":id})
    }
    fn upload(r: &mut Renderer, s: &Shared, id: u64, bytes: Vec<u8>) {
        apply(r, s, begin(id, bytes.len() as u32, "original"), None);
        apply(r, s, chunk(id, 0, bytes), None);
        apply(r, s, commit(id), None)
    }
    fn state(s: &Shared) -> Status {
        s.status(0).unwrap()
    }
    fn owner(n: u32) -> Option<Owner> {
        Some(Owner {
            performance: 1,
            owner: n,
        })
    }
    fn lease(chip: &str, ch: u8, voice: u64, action: &str, commands: Vec<Value>) -> Value {
        json!({"chip":chip,"method":"batch","channel":ch,"voice":voice,"action":action,"audition":true,"commands":commands})
    }
    #[test]
    fn x68000_native_validates_upload_metadata_commands_and_voice_ranges_before_mutation() {
        for v in [
            write("okim6258", 7, 0),
            json!({"chip":"ym2151","method":"play"}),
            json!({"chip":"ym2151","method":"clock","hz":99999}),
            json!({"chip":"ym2151","method":"source-mute","mask":256}),
            json!({"chip":"ym2151","method":"mix","volume":5,"pan":0}),
            begin(1, MAX_SAMPLE_BYTES as u32 + 1, "huge"),
            chunk(1, 0, vec![1; CHUNK + 1]),
            lease("ym2151", 0, 1, "on", vec![write("ym2151", 0x29, 74)]),
            lease("okim6258", 1, 1, "on", vec![json!({"method":"play"})]),
        ] {
            assert!(serde_json::from_value::<Message>(v)
                .unwrap()
                .prepare()
                .is_err());
        }
        let mut v = begin(1, 2, "bad");
        v["metadata"]["waveform"] = json!([2]);
        assert!(serde_json::from_value::<Message>(v)
            .unwrap()
            .prepare()
            .is_err());
        let mut v = begin(1, 2, "bad");
        v["metadata"]["frames"] = json!(2);
        assert!(serde_json::from_value::<Message>(v)
            .unwrap()
            .prepare()
            .is_err());
        let mut v = begin(1, 2, "raw");
        v["metadata"]["waveform"] = Value::Null;
        assert!(serde_json::from_value::<Message>(v)
            .unwrap()
            .prepare()
            .is_ok());
    }
    #[test]
    fn x68000_native_initial_snapshot_clocks_registers_and_independent_am_pm_depths() {
        let s = Shared::default();
        let mut r = Renderer::new(48000).unwrap();
        r.publish(&s);
        let initial = state(&s);
        assert_eq!(initial.ym2151.clock, FM_CLOCK);
        assert_eq!(initial.okim6258.clock, OKI_CLOCK);
        assert_eq!(initial.sample_rate, 48000);
        assert_eq!(initial.ym2151.registers.len(), 256);
        assert_eq!(initial.okim6258.divider, 512);
        assert!(initial.okim6258.sample.is_none());
        apply(&mut r, &s, write("ym2151", 0x19, 23), None);
        apply(&mut r, &s, write("ym2151", 0x19, 0x80 | 47), None);
        let out = state(&s);
        assert_eq!(out.ym2151.am_depth, 23);
        assert_eq!(out.ym2151.pm_depth, 47);
        assert_eq!(out.ym2151.registers[0x19], 0x80 | 47);
    }
    #[test]
    fn x68000_native_atomic_sample_commit_preserves_playing_data_until_complete() {
        let s = Shared::default();
        let mut r = Renderer::new(48000).unwrap();
        upload(&mut r, &s, 1, vec![0x17; 128]);
        apply(
            &mut r,
            &s,
            json!({"chip":"okim6258","method":"play","repeat":true}),
            None,
        );
        for _ in 0..50 {
            r.mix(&mut [0.0; 2])
        }
        r.publish(&s);
        assert!(state(&s).okim6258.playing);
        apply(&mut r, &s, begin(2, 4, "replacement"), None);
        apply(&mut r, &s, chunk(2, 0, vec![0x77, 0x77]), None);
        let before = state(&s);
        assert_eq!(before.okim6258.sample.unwrap().bytes, 128);
        assert!(before.okim6258.playing);
        assert_eq!(before.okim6258.memory_revision, 1);
        let invalid = prepare(commit(2));
        assert!(s.retain(&[(invalid, None)]).is_err());
        assert_eq!(state(&s).okim6258.memory_revision, 1);
        apply(&mut r, &s, chunk(2, 2, vec![0x11, 0x11]), None);
        apply(&mut r, &s, commit(2), None);
        let after = state(&s);
        assert!(!after.okim6258.playing);
        assert_eq!(after.okim6258.memory_revision, 2);
        let sample = after.okim6258.sample.unwrap();
        assert_eq!(sample.name, "replacement");
        assert_eq!(sample.bytes, 4);
        assert_eq!(sample.frames, 8);
        assert_eq!(sample.duration_seconds, 8.0 / 15625.0);
    }
    #[test]
    fn x68000_native_upload_owner_order_bounds_and_control_reclamation() {
        let s = Shared::default();
        let mut r = Renderer::new(48000).unwrap();
        upload(&mut r, &s, 1, vec![0x71; 32]);
        apply(&mut r, &s, begin(2, 4, "owner"), owner(1));
        for (v, o) in [
            (chunk(2, 2, vec![1, 2]), owner(1)),
            (chunk(2, 0, vec![1, 2]), owner(2)),
            (commit(2), owner(1)),
        ] {
            assert!(s.retain(&[(prepare(v), o)]).is_err());
        }
        apply(&mut r, &s, chunk(2, 0, vec![1, 2]), owner(1));
        r.cancel_invalid(|o| o != owner(1), &s);
        assert!(r.upload.is_none());
        assert_eq!(state(&s).okim6258.memory_revision, 1);
        assert_eq!(state(&s).okim6258.sample.unwrap().bytes, 32);
        // A new owner replaces an aborted upload without freeing the active sample.
        apply(&mut r, &s, begin(3, 2, "new owner"), owner(2));
        apply(&mut r, &s, chunk(3, 0, vec![0x17; 2]), owner(2));
        apply(&mut r, &s, commit(3), owner(2));
        let out = state(&s);
        assert_eq!(out.okim6258.sample.unwrap().name, "new owner");
        let c = s.control.lock().unwrap();
        assert!(c.prepared.is_empty());
        assert_eq!(c.metadata.len(), 1);
    }
    #[test]
    fn x68000_native_all_eight_fm_leases_protect_parameters_lfo_noise_and_ignore_stale_off() {
        let s = Shared::default();
        let mut r = Renderer::new(48000).unwrap();
        for ch in 0..8 {
            apply(
                &mut r,
                &s,
                lease(
                    "ym2151",
                    ch,
                    ch as u64 + 1,
                    "on",
                    vec![
                        json!({"method":"write","register":0x28+ch,"value":74}),
                        json!({"method":"write","register":8,"value":0x78|ch}),
                    ],
                ),
                None,
            )
        }
        assert_eq!(state(&s).ym2151.held, (0..8).collect::<Vec<_>>());
        for ch in 0..8 {
            apply(&mut r, &s, write("ym2151", 0x28 + ch, 10), owner(1));
            apply(&mut r, &s, write("ym2151", 8, ch), owner(1));
        }
        for register in [1, 15, 0x10, 0x11, 0x12, 0x14, 0x18, 0x19, 0x1a, 0x1b] {
            apply(&mut r, &s, write("ym2151", register, 31), owner(1));
        }
        let out = state(&s);
        assert_eq!(out.ym2151.key_masks, vec![15; 8]);
        for ch in 0..8 {
            assert_eq!(out.ym2151.registers[0x28 + ch], 74)
        }
        for register in [1, 15, 0x10, 0x11, 0x12, 0x14, 0x18, 0x19, 0x1a, 0x1b] {
            assert_eq!(out.ym2151.registers[register], 0)
        }
        apply(
            &mut r,
            &s,
            lease(
                "ym2151",
                0,
                99,
                "on",
                vec![json!({"method":"write","register":8,"value":0x78})],
            ),
            None,
        );
        apply(
            &mut r,
            &s,
            lease(
                "ym2151",
                0,
                1,
                "off",
                vec![json!({"method":"write","register":8,"value":0})],
            ),
            None,
        );
        assert_eq!(state(&s).ym2151.key_masks[0], 15);
        apply(
            &mut r,
            &s,
            json!({"chip":"ym2151","method":"batch","channel":0,"action":"edit","audition":true,"commands":[{"method":"write","register":0x18,"value":88}]}),
            None,
        );
        assert_eq!(state(&s).ym2151.registers[0x18], 88);
        apply(
            &mut r,
            &s,
            lease(
                "ym2151",
                0,
                99,
                "off",
                vec![json!({"method":"write","register":8,"value":0})],
            ),
            None,
        );
        assert_eq!(state(&s).ym2151.key_masks[0], 0);
    }
    #[test]
    fn x68000_native_oki_manual_lease_protects_raw_playback_upload_and_stale_note_off() {
        let s = Shared::default();
        let mut r = Renderer::new(48000).unwrap();
        upload(&mut r, &s, 1, vec![0x17; 256]);
        apply(
            &mut r,
            &s,
            lease(
                "okim6258",
                0,
                1,
                "on",
                vec![json!({"method":"play","repeat":true})],
            ),
            None,
        );
        for v in [
            json!({"chip":"okim6258","method":"stop"}),
            write("okim6258", 2, 3),
            json!({"chip":"okim6258","method":"clock","hz":4_000_000}),
            json!({"chip":"okim6258","method":"reset"}),
        ] {
            apply(&mut r, &s, v, owner(1))
        }
        assert!(s
            .retain(&[(prepare(begin(2, 2, "blocked")), owner(1))])
            .unwrap_err()
            .contains("held by manual audition"));
        assert_eq!(state(&s).okim6258.held, vec![0]);
        assert!(state(&s).okim6258.playing);
        assert_eq!(state(&s).okim6258.pan, 0);
        assert_eq!(state(&s).okim6258.clock, OKI_CLOCK);
        assert!(r.upload.is_none());
        apply(
            &mut r,
            &s,
            lease(
                "okim6258",
                0,
                2,
                "on",
                vec![json!({"method":"play","repeat":true})],
            ),
            None,
        );
        apply(
            &mut r,
            &s,
            lease("okim6258", 0, 1, "off", vec![json!({"method":"stop"})]),
            None,
        );
        assert!(state(&s).okim6258.playing);
        apply(
            &mut r,
            &s,
            lease("okim6258", 0, 2, "off", vec![json!({"method":"stop"})]),
            None,
        );
        assert!(!state(&s).okim6258.playing);
        assert!(state(&s).okim6258.held.is_empty());
    }
    #[test]
    fn x68000_native_script_commit_cannot_replace_a_new_manual_preview_but_ui_upload_can() {
        let s = Shared::default();
        let mut r = Renderer::new(48000).unwrap();
        upload(&mut r, &s, 1, vec![0x17; 256]);
        apply(&mut r, &s, begin(2, 2, "script"), owner(1));
        apply(&mut r, &s, chunk(2, 0, vec![0x10; 2]), owner(1));
        apply(
            &mut r,
            &s,
            lease(
                "okim6258",
                0,
                1,
                "on",
                vec![json!({"method":"play","repeat":true})],
            ),
            None,
        );
        apply(&mut r, &s, commit(2), owner(1));
        let unchanged = state(&s);
        assert_eq!(unchanged.okim6258.sample_id, 1);
        assert!(unchanged.okim6258.playing);
        assert!(r.upload.is_none());
        for mut v in [
            begin(3, 2, "UI replacement"),
            chunk(3, 0, vec![0x17; 2]),
            commit(3),
        ] {
            v["audition"] = json!(true);
            apply(&mut r, &s, v, None)
        }
        let replaced = state(&s);
        assert_eq!(replaced.okim6258.sample_id, 3);
        assert_eq!(replaced.okim6258.sample.unwrap().name, "UI replacement");
        assert!(replaced.okim6258.held.is_empty());
        assert!(!replaced.okim6258.playing);
        assert_eq!(replaced.okim6258.memory_revision, 2);
    }
    #[test]
    fn x68000_native_stop_and_reset_preserve_committed_sample_patch_and_clocks() {
        let s = Shared::default();
        let mut r = Renderer::new(48000).unwrap();
        upload(&mut r, &s, 1, vec![0x77; 128]);
        for v in [
            write("ym2151", 0x60, 42),
            json!({"chip":"ym2151","method":"clock","hz":3_579_545}),
            json!({"chip":"okim6258","method":"clock","hz":4_000_000}),
            write("okim6258", 12, 1),
            write("okim6258", 2, 1),
            json!({"chip":"okim6258","method":"play","repeat":true}),
        ] {
            apply(&mut r, &s, v, None)
        }
        r.stop(&s);
        let stopped = state(&s);
        assert_eq!(stopped.ym2151.registers[0x60], 42);
        assert_eq!(stopped.ym2151.clock, 3_579_545);
        assert_eq!(stopped.okim6258.clock, 4_000_000);
        assert_eq!(stopped.okim6258.divider, 768);
        assert_eq!(stopped.okim6258.pan, 1);
        assert!(stopped.okim6258.sample.is_some());
        assert!(!stopped.okim6258.playing);
        apply(
            &mut r,
            &s,
            json!({"chip":"okim6258","method":"reset"}),
            None,
        );
        let reset = state(&s);
        assert_eq!(reset.okim6258.divider, 512);
        assert_eq!(reset.okim6258.pan, 0);
        assert_eq!(reset.okim6258.sample.unwrap().bytes, 128);
        assert_eq!(reset.okim6258.clock, 4_000_000);
    }
    #[test]
    fn x68000_native_retained_admission_is_bounded_deduplicated_and_atomic() {
        let s = Shared::default();
        let c =
            prepare(json!({"chip":"ym2151","id":1,"method":"write","register":0x60,"value":20}));
        assert!(s.retain(&[(c.clone(), None), (c, None)]).is_err());
        assert!(state(&s).ym2151.pending_commands.is_empty());
        let c = prepare(chunk(1, 0, vec![0; CHUNK]));
        assert!(s
            .retain(&vec![(c, None); PENDING_BYTES / CHUNK + 1])
            .is_err());
        assert!(state(&s).okim6258.pending_commands.is_empty());
        let a = prepare(begin(1, 4, "atomic"));
        let b = prepare(chunk(1, 2, vec![1, 2]));
        assert!(s.retain(&[(a, None), (b, None)]).is_err());
        let c = s.control.lock().unwrap();
        assert!(c.prepared.is_empty());
        assert!(c.metadata.is_empty());
        assert!(c.upload.is_none());
    }
}

#[cfg(test)]
mod pipeline_tests {
    use super::super::*;
    use serde_json::{json, Value};
    fn command(frame: u64, message: Value) -> Command {
        serde_json::from_value(json!({"frame":frame,"type":"x68000","message":message})).unwrap()
    }
    fn tagged(frame: u64, message: Value, owner: u32) -> Command {
        let mut c = command(frame, message);
        c.performance = Some(1);
        c.owner = Some(owner);
        c
    }
    fn setup() -> (Shared, Renderer) {
        let s = Shared::new_modes(48000, 1, false, false, false, Some("x68000"));
        let r = Renderer::new_modes(48000, false, false, false, Some("x68000")).unwrap();
        r.publish_primary(&s);
        (s, r)
    }
    fn render(s: &Shared, r: &mut Renderer, frames: usize) -> Vec<[f32; 2]> {
        r.prepare(s);
        let out = (0..frames).map(|_| r.sample(s)).collect();
        s.frame.store(r.frame, Ordering::Release);
        r.publish_primary(s);
        out
    }
    fn snapshot(s: &Shared) -> x68000::Status {
        s.x68000
            .as_ref()
            .unwrap()
            .status(s.frame.load(Ordering::Acquire))
            .unwrap()
    }
    fn note(ch: u8, voice: u64, audition: bool) -> Value {
        let mut ops = vec![
            json!({"method":"write","register":0x20+ch,"value":0xc7}),
            json!({"method":"write","register":0x28+ch,"value":0x4a}),
            json!({"method":"write","register":0x30+ch,"value":0}),
        ];
        for slot in [0, 8, 16, 24] {
            for (base, value) in [
                (0x40, 1),
                (0x60, 0x20),
                (0x80, 31),
                (0xa0, 0),
                (0xc0, 0),
                (0xe0, 15),
            ] {
                ops.push(json!({"method":"write","register":base+slot+ch,"value":value}));
            }
        }
        ops.push(json!({"method":"write","register":8,"value":0x78|ch}));
        json!({"chip":"ym2151","method":"batch","channel":ch,"voice":voice,"action":"on","audition":audition,"commands":ops})
    }
    fn off(ch: u8, voice: u64) -> Value {
        json!({"chip":"ym2151","method":"batch","channel":ch,"voice":voice,"action":"off","commands":[{"method":"write","register":8,"value":ch}]})
    }
    #[test]
    fn x68000_native_primary_has_no_hidden_genesis_and_rejects_foreign_commands_atomically() {
        let (s, mut r) = setup();
        assert!(r.core.0.is_null());
        assert!(r.x68000.is_some());
        assert!(
            r.ym2608.is_none()
                && r.ym2610b.is_none()
                && r.dx7.is_none()
                && r.nes.is_none()
                && r.rf5c164.is_none()
                && r.pwm.is_none()
        );
        assert_eq!(s.status().mode, "x68000");
        assert_eq!(s.status().ym2151_clock, Some(4_000_000));
        assert_eq!(s.status().okim6258_clock, Some(8_000_000));
        assert_eq!(s.status().ym_clock, 4_000_000);
        assert_eq!(s.status().psg_clock, 0);
        for value in [
            json!({"frame":0,"type":"write","register":40,"value":0}),
            json!({"frame":0,"type":"psg-write","value":128}),
            json!({"frame":0,"type":"rf5c164-reset"}),
        ] {
            let invalid: Command = serde_json::from_value(value).unwrap();
            assert!(s
                .enqueue(1, &[command(0, note(0, 1, false)), invalid])
                .is_err());
            assert_eq!(s.pending.load(Ordering::Acquire), 0)
        }
        assert!(s.enqueue(2, &[command(0, note(0, 1, false))]).is_err());
        assert!(render(&s, &mut r, 128).iter().all(|v| *v == [0.0; 2]));
        let genesis = Shared::new(48000, 2);
        assert!(genesis
            .enqueue(2, &[command(0, note(0, 1, false))])
            .is_err());
    }
    #[test]
    fn x68000_native_due_frames_and_callback_segmentation_preserve_real_fm_pcm() {
        let (a, mut left) = setup();
        let (b, mut right) = setup();
        for s in [&a, &b] {
            s.enqueue(
                1,
                &[command(200, note(7, 1, false)), command(6000, off(7, 1))],
            )
            .unwrap();
        }
        let expected = render(&a, &mut left, 8192);
        let mut actual = vec![];
        for count in [1, 127, 53, 512, 31, 1024, 4096, 2348] {
            actual.extend(render(&b, &mut right, count));
        }
        assert_eq!(expected, actual);
        assert!(expected[..199].iter().all(|v| *v == [0.0; 2]));
        assert!(expected[512..4000].iter().any(|v| v[0].abs() > 0.001));
        assert_eq!(snapshot(&a).ym2151.key_masks[7], 0);
    }
    #[test]
    fn x68000_native_cancelled_owner_releases_only_its_fm_and_discards_partial_sample() {
        let (s, mut r) = setup();
        s.performance("start", 1, None).unwrap();
        let begin = json!({"chip":"okim6258","method":"memory-begin","sampleId":1,"byteLength":4,"metadata":{"name":"pending","frames":8,"sampleRate":15625,"waveform":[],"source":"raw"}});
        s.enqueue(1,&[tagged(0,note(0,1,false),1),tagged(0,note(1,2,false),2),tagged(0,begin,1),tagged(0,json!({"chip":"okim6258","method":"memory-chunk","sampleId":1,"offset":0,"data":[1,2]}),1),tagged(1000,note(0,3,false),1)]).unwrap();
        render(&s, &mut r, 256);
        assert_eq!(&snapshot(&s).ym2151.key_masks[..2], &[15, 15]);
        s.performance("cancel-owner", 1, Some(1)).unwrap();
        render(&s, &mut r, 1500);
        assert_eq!(&snapshot(&s).ym2151.key_masks[..2], &[0, 15]);
        assert!(r.x68000.as_ref().unwrap().upload.is_none());
        assert!(snapshot(&s).okim6258.sample.is_none());
        assert!(snapshot(&s).ym2151.pending_commands.is_empty());
    }
    #[test]
    fn x68000_native_epoch_stop_cancels_future_fm_and_uploads_preserves_sample_and_resumes() {
        let (s, mut r) = setup();
        let upload = [
            json!({"chip":"okim6258","method":"memory-begin","sampleId":1,"byteLength":4,"metadata":{"name":"kept","frames":8,"sampleRate":15625,"waveform":[],"source":"raw"}}),
            json!({"chip":"okim6258","method":"memory-chunk","sampleId":1,"offset":0,"data":[0x17,0x17,0x17,0x17]}),
            json!({"chip":"okim6258","method":"memory-commit","sampleId":1}),
        ];
        s.enqueue(
            1,
            &upload
                .into_iter()
                .map(|v| command(0, v))
                .collect::<Vec<_>>(),
        )
        .unwrap();
        render(&s, &mut r, 1);
        s.enqueue(
            1,
            &[
                command(0, note(0, 1, true)),
                command(5000, note(1, 2, false)),
            ],
        )
        .unwrap();
        render(&s, &mut r, 512);
        s.epoch.fetch_add(1, Ordering::AcqRel);
        render(&s, &mut r, 1024);
        let out = snapshot(&s);
        assert!(out.ym2151.held.is_empty());
        assert_eq!(out.ym2151.key_masks, vec![0; 8]);
        assert_eq!(out.okim6258.sample.unwrap().name, "kept");
        assert_eq!(s.pending.load(Ordering::Acquire), 0);
        assert!(render(&s, &mut r, 5000).iter().all(|v| *v == [0.0; 2]));
        s.enqueue(1, &[command(0, note(0, 3, false))]).unwrap();
        assert!(render(&s, &mut r, 2048).iter().any(|v| v[0].abs() > 0.001));
    }
    #[test]
    fn x68000_native_source_mixer_fx_monitor_and_recording_share_final_pcm() {
        let (dry, mut reference) = setup();
        let (wet, mut r) = setup();
        let fx:FxMessage=serde_json::from_value(json!({"op":"chain","children":[{"type":"gain","slot":0}],"units":[{"type":"gain","slot":0,"values":{"gain":0.5}}],"beatSeconds":0.5})).unwrap();
        let graph = wet.fx.lock().unwrap().command(fx, 48000).unwrap().unwrap();
        wet.enqueue_special(Action::Fx(graph)).unwrap();
        wet.enqueue(
            1,
            &[
                command(
                    0,
                    json!({"chip":"ym2151","method":"mix","volume":0.5,"pan":0}),
                ),
                command(0, note(0, 1, false)),
            ],
        )
        .unwrap();
        dry.enqueue(1, &[command(0, note(0, 1, false))]).unwrap();
        wet.record_start(0.02).unwrap();
        let expected = render(&dry, &mut reference, 2048);
        let actual = render(&wet, &mut r, 2048);
        assert!(expected[512..].iter().any(|v| v[0].abs() > 0.001));
        for (a, b) in expected[512..].iter().zip(&actual[512..]) {
            for ch in 0..2 {
                assert!((a[ch] * 0.25 - b[ch]).abs() < 1e-7);
            }
        }
        let capture = wet.record_stop().unwrap();
        assert_eq!(capture.frames, 960);
        for ch in 0..2 {
            assert_eq!(
                capture.channels[ch],
                actual[..960].iter().map(|v| v[ch]).collect::<Vec<_>>()
            );
        }
    }
    fn sample_commands(id: u64) -> Vec<Command> {
        vec![
            command(
                0,
                json!({"chip":"okim6258","method":"memory-begin","sampleId":id,"byteLength":8192,"metadata":{"name":"bus","frames":16384,"sampleRate":15625,"waveform":[],"source":"raw"}}),
            ),
            command(
                0,
                json!({"chip":"okim6258","method":"memory-chunk","sampleId":id,"offset":0,"data":vec![0x10;8192]}),
            ),
            command(
                0,
                json!({"chip":"okim6258","method":"memory-commit","sampleId":id}),
            ),
        ]
    }
    #[test]
    fn x68000_native_chip_scoped_stop_cancels_only_matching_chip_and_owner_future() {
        let (s, mut r) = setup();
        s.enqueue(1, &sample_commands(1)).unwrap();
        render(&s, &mut r, 1);
        s.performance("start", 1, None).unwrap();
        s.enqueue(
            1,
            &[
                tagged(0, note(0, 1, false), 1),
                tagged(0, note(1, 2, false), 2),
                tagged(1000, note(0, 3, false), 1),
                tagged(1200, note(1, 4, false), 2),
                tagged(
                    1100,
                    json!({"chip":"okim6258","method":"play","repeat":true}),
                    1,
                ),
            ],
        )
        .unwrap();
        render(&s, &mut r, 256);
        s.enqueue(
            1,
            &[tagged(
                0,
                json!({"chip":"ym2151","method":"stop-voices"}),
                1,
            )],
        )
        .unwrap();
        render(&s, &mut r, 1500);
        let out = snapshot(&s);
        assert_eq!(&out.ym2151.key_masks[..2], &[0, 15]);
        assert!(out.okim6258.playing);
        assert!(out.ym2151.pending_commands.is_empty());
        assert!(out.okim6258.pending_commands.is_empty());
        s.enqueue(
            1,
            &[
                tagged(3000, note(2, 5, false), 2),
                tagged(
                    3000,
                    json!({"chip":"okim6258","method":"play","repeat":true}),
                    2,
                ),
            ],
        )
        .unwrap();
        s.enqueue(
            1,
            &[tagged(
                0,
                json!({"chip":"okim6258","method":"stop-voices"}),
                2,
            )],
        )
        .unwrap();
        render(&s, &mut r, 2000);
        assert_eq!(snapshot(&s).ym2151.key_masks[2], 15);
        assert!(snapshot(&s).okim6258.playing); // Owner 1's existing sample survives owner 2's stop.
    }
    #[test]
    fn x68000_native_fm_and_oki_mixer_pan_and_gain_are_independent_real_pcm_buses() {
        let (fm, mut fr) = setup();
        let (oki, mut or) = setup();
        let (both, mut br) = setup();
        for s in [&oki, &both] {
            s.enqueue(1, &sample_commands(1)).unwrap();
        }
        for s in [&fm, &both] {
            s.enqueue(1, &[command(0, note(0, 1, false))]).unwrap();
        }
        for s in [&oki, &both] {
            s.enqueue(
                1,
                &[command(
                    0,
                    json!({"chip":"okim6258","method":"play","repeat":true}),
                )],
            )
            .unwrap();
        }
        both.enqueue(
            1,
            &[
                command(
                    0,
                    json!({"chip":"ym2151","method":"mix","volume":0.5,"pan":1}),
                ),
                command(
                    0,
                    json!({"chip":"okim6258","method":"mix","volume":0.25,"pan":-1}),
                ),
            ],
        )
        .unwrap();
        let fp = render(&fm, &mut fr, 2048);
        let op = render(&oki, &mut or, 2048);
        let bp = render(&both, &mut br, 2048);
        assert!(fp[512..].iter().any(|v| v[1].abs() > 0.001));
        assert!(op[512..].iter().any(|v| v[0].abs() > 0.001));
        for i in 512..2048 {
            assert!((bp[i][0] - op[i][0] * 0.25).abs() < 1e-7);
            assert!((bp[i][1] - fp[i][1] * 0.5).abs() < 1e-7);
        }
    }
}
