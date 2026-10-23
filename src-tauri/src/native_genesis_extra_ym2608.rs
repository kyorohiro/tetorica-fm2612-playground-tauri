//! Callback-owned PC-98-26 YM2608: three FM and three SSG voices.
//! Allocation and reclamation stay on the control side of the event queue.
use super::{Owner, CAPACITY, MAX_SAFE_INTEGER, OWNERS};
use crossbeam_queue::ArrayQueue;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    ffi::c_void,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering},
        Arc, Mutex, OnceLock,
    },
};
pub const CLOCK: u32 = 8_000_000;
const RAM: usize = 2 * 1024 * 1024;
const ROM: usize = 8192;
pub const MAX_BATCH_UPLOAD_BYTES: usize = 4 * 1024 * 1024;
const VOICES: usize = 16;
const PENDING_BYTES: usize = 12 * 1024 * 1024;
#[repr(C)]
struct CoreState {
    registers: [u8; 512],
    clock: u32,
    source_mute_mask: u32,
    keys: [u8; 6],
    ssg_levels: [u8; 3],
    prescaler: u8,
    status: u8,
    status_hi: u8,
    irq: u8,
}
impl Default for CoreState {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}
extern "C" {
    fn native_ym2608_create_extra(rate: u32) -> *mut c_void;
    fn native_ym2608_destroy(p: *mut c_void);
    fn native_ym2608_reset(p: *mut c_void);
    fn native_ym2608_stop(p: *mut c_void);
    fn native_ym2608_write(p: *mut c_void, port: u8, register: u8, value: u8);
    fn native_ym2608_key_off(p: *mut c_void, voice: u8);
    fn native_ym2608_key_off_fm(p: *mut c_void, voice: u8);
    fn native_ym2608_cancel_csm(p: *mut c_void);
    fn native_ym2608_audition_fm_key_on(p: *mut c_void, channel: u8, mask: u8);
    fn native_ym2608_sample(p: *mut c_void, l: *mut f32, r: *mut f32);
    fn native_ym2608_set_clock(p: *mut c_void, hz: u32) -> i32;
    fn native_ym2608_set_source_mute_mask(p: *mut c_void, mask: u32);
    fn native_ym2608_set_retained_ssg_level(p: *mut c_void, ch: u8, level: u8);
    fn native_ym2608_load_rom(p: *mut c_void, data: *const u8, offset: u32, size: u32) -> i32;
    fn native_ym2608_load_memory(p: *mut c_void, data: *const u8, offset: u32, size: u32) -> i32;
    fn native_ym2608_get_extra_state(p: *mut c_void, s: *mut CoreState);
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
        port: u8,
        register: u8,
        value: u8,
    },
    KeyOff {
        channel: u8,
    },
    SetRetainedLevel {
        channel: u8,
        level: u8,
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
    CancelCsm,
    Rom {
        #[serde(default)]
        offset: u32,
        data: Vec<u8>,
    },
    Memory {
        offset: u32,
        data: Vec<u8>,
    },
    PrepareTimeline {
        events: Vec<(u64, u8, u32, u32)>,
        blocks: Vec<Vec<u8>>,
        #[serde(rename = "durationSamples")]
        duration_samples: u64,
    },
    PlayTimeline {
        token: u64,
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
    Write { port: u8, register: u8, value: u8 },
    KeyOff { channel: u8 },
    SetRetainedLevel { channel: u8, level: u8 },
    Clock { hz: u32 },
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
    audition: bool,
    group: Option<u64>,
    method: Method,
    complete: AtomicBool,
    clock_rejected: AtomicBool,
    outcome_observed: AtomicBool,
    schedule_finished: AtomicBool,
    cancel_target: OnceLock<Arc<Prepared>>,
    schedule_owner: OnceLock<Option<Owner>>,
    previous_schedule: OnceLock<(u64, u8)>,
    play_target: OnceLock<Arc<Prepared>>,
    invalidated: AtomicBool,
    applied: AtomicBool,
    cancel_prepare: OnceLock<Arc<Prepared>>,
    start_frame: AtomicU64,
}
fn valid_id(id: Option<u64>) -> bool {
    id.is_none_or(|v| v > 0 && v <= MAX_SAFE_INTEGER)
}
fn fm_key_channel(value: u8) -> Option<usize> {
    let low = value & 3;
    (low < 3).then_some(low as usize + if value & 4 != 0 { 3 } else { 0 })
}
fn declared_channel(op: &Op) -> Option<usize> {
    match op {
        Op::KeyOff { channel } => Some(*channel as usize),
        Op::SetRetainedLevel { channel, .. } => Some(6 + *channel as usize),
        Op::Write {
            port: 0,
            register: 0x28,
            value,
        } => fm_key_channel(*value),
        Op::Write {
            port: 0,
            register: 0..=5,
            ..
        } => match op {
            Op::Write { register, .. } => Some(6 + (*register / 2) as usize),
            _ => unreachable!(),
        },
        Op::Write {
            port: 0,
            register: 8..=10,
            ..
        } => match op {
            Op::Write { register, .. } => Some(6 + (*register - 8) as usize),
            _ => unreachable!(),
        },
        Op::Write {
            port: 0,
            register: 0x18..=0x1d,
            ..
        } => match op {
            Op::Write { register, .. } => Some(9 + (*register - 0x18) as usize),
            _ => unreachable!(),
        },
        Op::Write {
            port: 1,
            register: 0..=0x10,
            ..
        } => Some(15),
        Op::Write {
            port: 0,
            register: 0xa8..=0xae,
            ..
        } if register_low(op) < 3 => Some(2),
        Op::Write {
            port,
            register: 0x30..=0xb6,
            ..
        } if register_low(op) < 3 => Some((*port as usize) * 3 + register_low(op) as usize),
        _ => None,
    }
}
fn register_low(op: &Op) -> u8 {
    match op {
        Op::Write { register, .. } => register & 3,
        _ => 3,
    }
}
fn validate_op(op: &Op) -> Result<(), String> {
    match op {
        Op::Write { port, .. } if *port > 1 => Err("Native YM2608 port must be 0 or 1".into()),
        Op::KeyOff { channel } if *channel >= VOICES as u8 => {
            Err("Native YM2608 voice must be 0–15".into())
        }
        Op::SetRetainedLevel { channel, level } if *channel >= 3 || *level > 31 => {
            Err("Invalid Native YM2608 SSG retained level".into())
        }
        Op::Clock { hz } if !(100_000..=100_000_000).contains(hz) => {
            Err("Native YM2608 clock must be 100000–100000000 Hz".into())
        }
        _ => Ok(()),
    }
}
impl Message {
    pub fn prepared_bytes(&self) -> usize {
        std::mem::size_of::<Prepared>()
            + match &self.method {
                Method::Batch { commands, .. } => commands.len() * std::mem::size_of::<Op>(),
                Method::Rom { data, .. } | Method::Memory { data, .. } => data.len(),
                Method::PrepareTimeline { events, blocks, .. } => {
                    events.len() * std::mem::size_of::<(u64, u8, u32, u32)>()
                        + blocks.len() * std::mem::size_of::<Vec<u8>>()
                        + blocks.iter().map(Vec::len).sum::<usize>()
                }
                _ => 0,
            }
    }

    pub fn prepare(&self) -> Result<Arc<Prepared>, String> {
        if !valid_id(self.id) || !valid_id(self.group) {
            return Err("Invalid Native YM2608 command ID".into());
        }
        match &self.method {
            Method::Write {
                port,
                register,
                value,
            } => validate_op(&Op::Write {
                port: *port,
                register: *register,
                value: *value,
            })?,
            Method::KeyOff { channel } => validate_op(&Op::KeyOff { channel: *channel })?,
            Method::SetRetainedLevel { channel, level } => validate_op(&Op::SetRetainedLevel {
                channel: *channel,
                level: *level,
            })?,
            Method::Clock { hz } => validate_op(&Op::Clock { hz: *hz })?,
            Method::Mix { volume, pan }
                if !volume.is_finite()
                    || !(0.0..=4.).contains(volume)
                    || !pan.is_finite()
                    || !(-1.0..=1.).contains(pan) =>
            {
                return Err("Native YM2608 mixer volume must be 0–4, pan −1–1".into())
            }
            Method::SourceMute { mask } if *mask > 7 => {
                return Err("Native YM2608 source mute mask must be 0–7".into())
            }
            Method::Rom { offset, data } if *offset != 0 || data.len() != ROM => {
                return Err("Native additional YM2608 ROM must contain exactly 8192 bytes".into())
            }
            Method::Memory { offset, data }
                if data.is_empty()
                    || (*offset as usize)
                        .checked_add(data.len())
                        .is_none_or(|end| end > RAM) =>
            {
                return Err("Native additional YM2608 memory exceeds 2 MiB bounds".into())
            }
            Method::PrepareTimeline {
                events,
                blocks,
                duration_samples,
            } => {
                if *duration_samples == 0
                    || *duration_samples > MAX_SAFE_INTEGER
                    || events.len() > 131072
                {
                    return Err("Invalid Native YM2608 timeline or event limit (131072)".into());
                }
                let bytes = events
                    .len()
                    .checked_mul(32)
                    .and_then(|n| n.checked_add(blocks.len().checked_mul(24)?))
                    .and_then(|n| n.checked_add(256))
                    .and_then(|n| blocks.iter().try_fold(n, |sum, b| sum.checked_add(b.len())));
                if bytes.is_none_or(|n| n > 8 * 1024 * 1024) || blocks.len() > 131072 {
                    return Err("Native YM2608 timeline payload exceeds 8 MiB".into());
                }
                if blocks.iter().any(|b| b.len() > RAM) {
                    return Err("Native YM2608 timeline block exceeds 2 MiB".into());
                }
                let mut last = 0;
                for &(at, port, register, value) in events {
                    if at < last || at > *duration_samples || port > 2 {
                        return Err("Invalid Native YM2608 timeline event".into());
                    }
                    last = at;
                    if port == 2 {
                        let block = blocks
                            .get(register as usize)
                            .ok_or("Invalid Native YM2608 timeline block index")?;
                        if (value as usize)
                            .checked_add(block.len())
                            .is_none_or(|end| end > RAM)
                        {
                            return Err("Native YM2608 timeline copy exceeds 2 MiB bounds".into());
                        }
                    } else if register > 255 || value > 255 {
                        return Err("Invalid Native YM2608 timeline register write".into());
                    }
                }
            }
            Method::PlayTimeline { token } if !valid_id(Some(*token)) => {
                return Err("Invalid Native YM2608 timeline token".into())
            }
            Method::CancelMainOwner { target_group } if !valid_id(Some(*target_group)) => {
                return Err("Invalid Native YM2608 Main owner group".into())
            }
            Method::Batch {
                commands,
                channel,
                voice,
                action,
            } => {
                if *channel >= 16
                    || commands.is_empty()
                    || commands.len() > 512
                    || !valid_id(*voice)
                    || self.audition && *action != LeaseAction::Edit && voice.is_none()
                {
                    return Err("Invalid Native YM2608 batch or voice lease".into());
                }
                for op in commands {
                    validate_op(op)?;
                    if declared_channel(op).is_some_and(|ch| ch != *channel as usize) {
                        return Err(
                            "Native YM2608 batch must target its declared physical voice".into(),
                        );
                    }
                    if let Op::Write {
                        port: 0,
                        register: 0x10,
                        value,
                    } = op
                    {
                        if !(9..15).contains(channel) || value & 0x3f & !(1 << (*channel - 9)) != 0
                        {
                            return Err(
                                "Native YM2608 rhythm batch must target its declared voice".into(),
                            );
                        }
                    }
                    if matches!(
                        op,
                        Op::Write {
                            port: 0,
                            register: 7,
                            ..
                        }
                    ) && !(6..9).contains(channel)
                    {
                        return Err("Native YM2608 mixer batch must target an SSG voice".into());
                    }
                }
            }
            _ => {}
        }
        Ok(Arc::new(Prepared {
            id: self.id,
            audition: self.audition,
            group: self.group,
            method: self.method.clone(),
            complete: AtomicBool::new(false),
            clock_rejected: AtomicBool::new(false),
            outcome_observed: AtomicBool::new(false),
            schedule_finished: AtomicBool::new(false),
            cancel_target: OnceLock::new(),
            schedule_owner: OnceLock::new(),
            previous_schedule: OnceLock::new(),
            play_target: OnceLock::new(),
            invalidated: AtomicBool::new(false),
            applied: AtomicBool::new(false),
            cancel_prepare: OnceLock::new(),
            start_frame: AtomicU64::new(0),
        }))
    }
}
impl Prepared {
    pub fn set_start_frame(&self, frame: u64) {
        self.start_frame.store(frame, Ordering::Relaxed)
    }
    pub fn starts(&self) -> bool {
        match &self.method {
            Method::Write { .. } | Method::PlayTimeline { .. } => true,
            Method::Batch { action, .. } => *action != LeaseAction::Off,
            _ => false,
        }
    }
    pub fn stops(&self) -> bool {
        matches!(self.method, Method::StopVoices) && self.group.is_none()
    }
    pub fn deferred_retirement(&self) -> bool {
        matches!(self.method, Method::PlayTimeline { .. })
    }
    pub fn invalidates_schedule(&self) -> bool {
        matches!(
            self.method,
            Method::Reset | Method::StopVoices | Method::CancelMainOwner { .. }
        )
    }
    fn schedule_token(&self) -> Option<u64> {
        if let Method::PlayTimeline { token } = self.method {
            Some(token)
        } else {
            None
        }
    }
    pub fn upload_bytes(&self) -> usize {
        match &self.method {
            Method::Rom { data, .. } | Method::Memory { data, .. } => data.len(),
            _ => 0,
        }
    }
    pub fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + match &self.method {
                Method::Batch { commands, .. } => commands.capacity() * std::mem::size_of::<Op>(),
                Method::Rom { data, .. } | Method::Memory { data, .. } => data.capacity(),
                Method::PrepareTimeline { events, blocks, .. } => {
                    events.capacity() * std::mem::size_of::<(u64, u8, u32, u32)>()
                        + blocks.capacity() * std::mem::size_of::<Vec<u8>>()
                        + blocks.iter().map(Vec::capacity).sum::<usize>()
                }
                _ => 0,
            }
    }
    fn bytes(&self) -> usize {
        self.retained_bytes()
    }
}
#[derive(Default)]
struct Control {
    highest_schedule: u64,
    prepared: Vec<Arc<Prepared>>,
    rejected_clocks: HashSet<u64>,
    current_prepared: Option<Arc<Prepared>>,
    pending_previous: Option<Option<Arc<Prepared>>>,
}
pub struct Shared {
    additional: bool,
    schedule_token: AtomicU64,
    schedule_phase: AtomicU8,
    revision: AtomicU64,
    registers: [AtomicU8; 512],
    keys: [AtomicU8; 6],
    levels: [AtomicU8; 3],
    clock: AtomicU32,
    source_mute: AtomicU32,
    prescaler: AtomicU8,
    status: AtomicU8,
    status_hi: AtomicU8,
    memory_bytes: AtomicU32,
    rom_loaded: AtomicBool,
    irq: AtomicU8,
    held: AtomicU32,
    rate: AtomicU32,
    control: Mutex<Control>,
    retired: ArrayQueue<Arc<Prepared>>,
    highest_id: AtomicU64,
    cancel_before: AtomicU64,
    owner_cancel_before: [AtomicU64; OWNERS],
}
impl Default for Shared {
    fn default() -> Self {
        Self {
            additional: true,
            schedule_token: AtomicU64::new(0),
            schedule_phase: AtomicU8::new(0),
            revision: AtomicU64::new(0),
            registers: std::array::from_fn(|_| AtomicU8::new(0)),
            keys: std::array::from_fn(|_| AtomicU8::new(0)),
            levels: std::array::from_fn(|_| AtomicU8::new(0)),
            clock: AtomicU32::new(CLOCK),
            source_mute: AtomicU32::new(0),
            prescaler: AtomicU8::new(6),
            status: AtomicU8::new(0),
            status_hi: AtomicU8::new(0),
            memory_bytes: AtomicU32::new(0),
            rom_loaded: AtomicBool::new(false),
            irq: AtomicU8::new(0),
            held: AtomicU32::new(0),
            rate: AtomicU32::new(0),
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
pub struct Status {
    pub clock: u32,
    pub registers: Vec<Vec<u8>>,
    pub key_masks: Vec<u8>,
    pub ssg_levels: Vec<u8>,
    pub ssg_gates: Vec<bool>,
    pub source_mute_mask: u32,
    pub prescaler: u8,
    pub status: u8,
    pub status_hi: u8,
    pub rom_loaded: bool,
    pub adpcm_memory_bytes: u32,
    pub irq: bool,
    pub held: Vec<u8>,
    pub pending_commands: Vec<u64>,
    pub frame: u64,
    pub sample_rate: u32,
    pub revision: u64,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdditionalStatus {
    pub ym2608: AdditionalChipStatus,
    pub frame: u64,
    pub sample_rate: u32,
    pub revision: u64,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdditionalChipStatus {
    pub clock: u32,
    pub registers: Vec<Vec<u8>>,
    pub key_masks: Vec<u8>,
    pub ssg_levels: Vec<u8>,
    pub ssg_gates: Vec<bool>,
    pub source_mute_mask: u32,
    pub prescaler: u8,
    pub status: u8,
    pub status_hi: u8,
    pub rom_loaded: bool,
    pub adpcm_memory_bytes: u32,
    pub irq: bool,
    pub held: Vec<u8>,
    pub pending_commands: Vec<u64>,
    pub timeline: TimelineStatus,
}
#[derive(Debug, Serialize)]
pub struct TimelineStatus {
    pub token: u64,
    pub pending: bool,
    pub playing: bool,
    pub ended: bool,
    pub cancelled: bool,
    pub prepared: bool,
    #[serde(rename = "durationSamples")]
    pub duration_samples: u64,
}
impl Shared {
    fn clock_held_error(&self) -> &'static str {
        if self.additional {
            "Release the manually held additional YM2608 voices before changing the clock"
        } else {
            "Release the manually held YM2608 voices before changing the clock"
        }
    }
    pub fn additional_status(&self, frame: u64) -> Result<AdditionalStatus, String> {
        let value = self.status(frame)?;
        Ok(AdditionalStatus {
            frame: value.frame,
            sample_rate: value.sample_rate,
            revision: value.revision,
            ym2608: AdditionalChipStatus {
                clock: value.clock,
                registers: value.registers,
                key_masks: value.key_masks,
                ssg_levels: value.ssg_levels,
                ssg_gates: value.ssg_gates,
                source_mute_mask: value.source_mute_mask,
                prescaler: value.prescaler,
                status: value.status,
                status_hi: value.status_hi,
                rom_loaded: value.rom_loaded,
                adpcm_memory_bytes: value.adpcm_memory_bytes,
                irq: value.irq,
                held: value.held,
                pending_commands: value.pending_commands,
                timeline: self.schedule_status(),
            },
        })
    }
    fn schedule_status(&self) -> TimelineStatus {
        let phase = self.schedule_phase.load(Ordering::Acquire);
        TimelineStatus {
            token: self.schedule_token.load(Ordering::Acquire),
            pending: phase == 1,
            playing: phase == 2,
            ended: phase == 3,
            cancelled: phase == 4,
            prepared: self
                .control
                .lock()
                .ok()
                .is_some_and(|c| c.current_prepared.is_some()),
            duration_samples: self
                .control
                .lock()
                .ok()
                .and_then(|c| {
                    c.current_prepared.as_ref().and_then(|v| {
                        if let Method::PrepareTimeline {
                            duration_samples, ..
                        } = v.method
                        {
                            Some(duration_samples)
                        } else {
                            None
                        }
                    })
                })
                .unwrap_or(0),
        }
    }
    pub fn clock(&self) -> u32 {
        self.clock.load(Ordering::Acquire)
    }
    pub fn valid_event(&self, command: &Prepared, sequence: u64, owner: Option<Owner>) -> bool {
        !command.invalidated.load(Ordering::Acquire)
            && (!command.deferred_retirement()
                || !command.schedule_finished.load(Ordering::Acquire))
            && sequence >= self.cancel_before.load(Ordering::Acquire)
            && owner.is_none_or(|o| {
                sequence >= self.owner_cancel_before[(o.owner - 1) as usize].load(Ordering::Acquire)
            })
    }
    pub fn cancel_before(&self, _: &Prepared, sequence: u64, owner: Option<Owner>) {
        if let Some(o) = owner {
            self.owner_cancel_before[(o.owner - 1) as usize].fetch_max(sequence, Ordering::AcqRel);
        } else {
            self.cancel_before.fetch_max(sequence, Ordering::AcqRel);
        }
    }
    fn collect(&self, c: &mut Control) {
        if let Some(previous) = c.pending_previous.take() {
            match c.current_prepared.as_ref() {
                Some(cmd) if cmd.invalidated.load(Ordering::Acquire) => {
                    c.current_prepared = previous
                }
                Some(cmd) if !cmd.applied.load(Ordering::Acquire) => {
                    c.pending_previous = Some(previous)
                }
                _ => {}
            }
        }
        let mut retired = HashSet::new();
        while let Some(cmd) = self.retired.pop() {
            retired.insert(Arc::as_ptr(&cmd) as usize);
        }
        c.prepared.retain(|cmd| {
            let rejected = cmd.id.is_some() && cmd.clock_rejected.load(Ordering::Acquire);
            let observed = cmd.outcome_observed.load(Ordering::Acquire);
            rejected && !observed
                || !retired.contains(&(Arc::as_ptr(cmd) as usize)) && !(rejected && observed)
        });
        if c.prepared.capacity() > 64.max(c.prepared.len().saturating_mul(2)) {
            c.prepared.shrink_to_fit();
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
            .map_err(|_| "Native YM2608 control state unavailable")?;
        self.collect(&mut c);
        if self.held.load(Ordering::Acquire) != 0
            && commands
                .iter()
                .any(|(v, _)| !v.audition && matches!(v.method, Method::Clock { .. }))
        {
            return Err(self.clock_held_error().into());
        }
        if c.prepared.len() + c.rejected_clocks.len() + commands.len() > CAPACITY
            || c.prepared.iter().map(|v| v.bytes()).sum::<usize>()
                + commands.iter().map(|(v, _)| v.bytes()).sum::<usize>()
                > PENDING_BYTES
        {
            return Err("Native YM2608 pending command limit reached (12 MiB)".into());
        }
        let mut ids = HashSet::new();
        for id in c
            .prepared
            .iter()
            .filter_map(|v| v.id)
            .chain(c.rejected_clocks.iter().copied())
            .chain(commands.iter().filter_map(|(v, _)| v.id))
        {
            if !ids.insert(id) {
                return Err("Duplicate pending Native YM2608 command ID".into());
            }
        }
        let rate = self.rate.load(Ordering::Acquire);
        let to_frame = |sample: u64| (sample as u128 * rate as u128 + 44099) / 44100;
        let mut active = c
            .prepared
            .iter()
            .find(|v| v.deferred_retirement() && !v.schedule_finished.load(Ordering::Acquire))
            .cloned();
        let mut prepared = c.current_prepared.clone();
        let mut pending_prepare = c.pending_previous.is_some();
        let mut highest = c.highest_schedule;
        // Validate the complete wire-order transaction before publishing metadata.
        for (cmd, owner) in commands {
            if cmd.invalidates_schedule() {
                if active.as_ref().is_some_and(|target| {
                    let scope = match cmd.method {
                        Method::Reset => true,
                        Method::CancelMainOwner { target_group } => {
                            target.group == Some(target_group)
                        }
                        Method::StopVoices => cmd.group.is_none() || target.group == cmd.group,
                        _ => false,
                    };
                    scope
                        && (matches!(cmd.method, Method::Reset)
                            || owner.is_none()
                            || target.schedule_owner.get() == Some(owner))
                }) {
                    active = None;
                }
            }
            match &cmd.method {
                Method::PrepareTimeline {
                    events,
                    blocks,
                    duration_samples,
                } => {
                    if active.is_some() {
                        return Err("Native YM2608 timeline already playing or pending".into());
                    }
                    if pending_prepare {
                        return Err("Native YM2608 timeline preparation already pending".into());
                    }
                    if to_frame(*duration_samples) > MAX_SAFE_INTEGER as u128 {
                        return Err(
                            "Native YM2608 timeline duration exceeds safe frame range".into()
                        );
                    }
                    let mut first = 0usize;
                    let mut writes = 0usize;
                    let mut copies = 0usize;
                    for (index, &(at, port, register, _)) in events.iter().enumerate() {
                        if port == 2 {
                            copies += blocks[register as usize].len()
                        } else {
                            writes += 1
                        }
                        while first < index && to_frame(at) - to_frame(events[first].0) >= 128 {
                            let e = events[first];
                            if e.1 == 2 {
                                copies -= blocks[e.2 as usize].len()
                            } else {
                                writes -= 1
                            }
                            first += 1;
                        }
                        if writes > 256 {
                            return Err(
                                "Native YM2608 timeline exceeds 256 writes in 128 output frames"
                                    .into(),
                            );
                        }
                        if copies > 4 * 1024 * 1024 {
                            return Err(
                                "Native YM2608 timeline exceeds 4 MiB copies in 128 output frames"
                                    .into(),
                            );
                        }
                    }
                    prepared = Some(cmd.clone());
                    pending_prepare = true;
                }
                Method::PlayTimeline { token } => {
                    if active.is_some() {
                        return Err("Native YM2608 timeline already playing or pending".into());
                    }
                    if prepared.is_none() {
                        return Err("Native YM2608 timeline is not prepared".into());
                    }
                    if *token <= highest {
                        return Err(
                            "Native YM2608 timeline token must be unique and increasing".into()
                        );
                    }
                    if let Some(target) = &prepared {
                        if let Method::PrepareTimeline {
                            duration_samples, ..
                        } = target.method
                        {
                            if cmd.start_frame.load(Ordering::Relaxed) as u128
                                + to_frame(duration_samples)
                                > MAX_SAFE_INTEGER as u128
                            {
                                return Err(
                                    "Native YM2608 timeline end exceeds safe frame range".into()
                                );
                            }
                        }
                    }
                    highest = *token;
                    active = Some(cmd.clone());
                }
                _ => {}
            }
        }
        let mut active = c
            .prepared
            .iter()
            .find(|v| v.deferred_retirement() && !v.schedule_finished.load(Ordering::Acquire))
            .cloned();
        for (cmd, owner) in commands {
            if let Method::CancelMainOwner { target_group } = cmd.method {
                if let Some(target) = &c.current_prepared {
                    if target.group == Some(target_group) && !target.applied.load(Ordering::Acquire)
                    {
                        let _ = cmd.cancel_prepare.set(target.clone());
                    }
                }
            }
            if cmd.invalidates_schedule() {
                if let Some(target) = &active {
                    let matches = match cmd.method {
                        Method::Reset => true,
                        Method::CancelMainOwner { target_group } => {
                            target.group == Some(target_group)
                        }
                        Method::StopVoices => cmd.group.is_none() || target.group == cmd.group,
                        _ => false,
                    };
                    if matches {
                        let _ = cmd.cancel_target.set(target.clone());
                    }
                }
            }
            if matches!(cmd.method, Method::PrepareTimeline { .. }) {
                let previous = c.current_prepared.replace(cmd.clone());
                c.pending_previous = Some(previous);
            }
            if let Some(token) = cmd.schedule_token() {
                let _ = cmd.schedule_owner.set(*owner);
                let _ = cmd
                    .play_target
                    .set(c.current_prepared.as_ref().unwrap().clone());
                let _ = cmd.previous_schedule.set((
                    self.schedule_token.load(Ordering::Acquire),
                    self.schedule_phase.load(Ordering::Acquire),
                ));
                c.highest_schedule = token;
                active = Some(cmd.clone());
                self.schedule_token.store(token, Ordering::Relaxed);
                self.schedule_phase.store(1, Ordering::Release);
            }
            if let Some(id) = cmd.id {
                self.highest_id.fetch_max(id, Ordering::AcqRel);
            }
            c.prepared.push(cmd.clone());
        }
        Ok(())
    }
    pub fn wait_applied(&self, id: u64) -> Result<(), String> {
        if id == 0 || id > self.highest_id.load(Ordering::Acquire) {
            return Err("Unknown Native YM2608 command ID".into());
        }
        let cmd = {
            let mut control = self
                .control
                .lock()
                .map_err(|_| "Native YM2608 control state unavailable")?;
            if control.rejected_clocks.remove(&id) {
                return Err(self.clock_held_error().into());
            }
            control.prepared.iter().find(|v| v.id == Some(id)).cloned()
        };
        if let Some(cmd) = cmd {
            let start = std::time::Instant::now();
            while !cmd.complete.load(Ordering::Acquire) {
                // A scheduled Worker clock change awaits the same applied
                // command barrier as the authoring client, not an early ACK.
                let deadline = if matches!(
                    cmd.method,
                    Method::Clock { .. }
                        | Method::PlayTimeline { .. }
                        | Method::PrepareTimeline { .. }
                ) || self.additional
                    && matches!(cmd.method, Method::Reset | Method::CancelMainOwner { .. })
                {
                    5000
                } else {
                    100
                };
                if start.elapsed() > std::time::Duration::from_millis(deadline) {
                    return Err(format!(
                        "Native YM2608 command was not applied within {deadline} ms"
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            cmd.outcome_observed.store(true, Ordering::Release);
            if cmd.clock_rejected.load(Ordering::Acquire) {
                self.control
                    .lock()
                    .map_err(|_| "Native YM2608 control state unavailable")?
                    .rejected_clocks
                    .remove(&id);
                return Err(self.clock_held_error().into());
            }
        }
        Ok(())
    }
    pub fn rollback(&self, cmd: Arc<Prepared>) {
        let previous = cmd.previous_schedule.get().copied();
        let token = cmd.schedule_token();
        cmd.invalidated.store(true, Ordering::Release);
        self.retire(cmd);
        if token.is_some_and(|token| self.schedule_token.load(Ordering::Acquire) == token) {
            if let Some((token, phase)) = previous {
                self.schedule_token.store(token, Ordering::Relaxed);
                self.schedule_phase.store(phase, Ordering::Release);
            }
        }
    }
    pub fn retire(&self, cmd: Arc<Prepared>) {
        if !cmd.applied.load(Ordering::Acquire) {
            cmd.invalidated.store(true, Ordering::Release);
        }
        if let Some(token) = cmd.schedule_token() {
            if !cmd.schedule_finished.swap(true, Ordering::AcqRel)
                && self.schedule_token.load(Ordering::Acquire) == token
            {
                self.schedule_phase.store(4, Ordering::Release);
            }
        }
        if self.additional {
            // Only callback publishes the additional chip snapshot seqlock.
            cmd.complete.store(true, Ordering::Release);
        } else if !cmd.complete.load(Ordering::Relaxed) {
            self.revision.fetch_add(1, Ordering::AcqRel);
            cmd.complete.store(true, Ordering::Relaxed);
            self.revision.fetch_add(1, Ordering::Release);
        }
        self.retired
            .push(cmd)
            .unwrap_or_else(|_| unreachable!("Bounded YM2608 command retirement"));
    }

    fn publish(&self, r: &Renderer, command: Option<&Prepared>) {
        let state = r.state();
        self.revision.fetch_add(1, Ordering::AcqRel);
        for (out, value) in self.registers.iter().zip(state.registers) {
            out.store(value, Ordering::Relaxed)
        }
        for (out, value) in self.keys.iter().zip(state.keys) {
            out.store(value, Ordering::Relaxed)
        }
        for (out, value) in self.levels.iter().zip(state.ssg_levels) {
            out.store(value, Ordering::Relaxed)
        }
        self.clock.store(state.clock, Ordering::Relaxed);
        self.source_mute
            .store(state.source_mute_mask, Ordering::Relaxed);
        self.status_hi.store(state.status_hi, Ordering::Relaxed);
        self.memory_bytes.store(r.memory_bytes, Ordering::Relaxed);
        self.rom_loaded.store(r.rom_loaded, Ordering::Relaxed);
        self.prescaler.store(state.prescaler, Ordering::Relaxed);
        self.status.store(state.status, Ordering::Relaxed);
        self.status_hi.store(state.status_hi, Ordering::Relaxed);
        self.irq.store(state.irq, Ordering::Relaxed);
        self.held.store(
            r.owners
                .iter()
                .enumerate()
                .filter(|(_, v)| v.is_some_and(|l| l.audition))
                .fold(
                    if r.csm_owner.is_some_and(|l| l.audition) {
                        1 << 2
                    } else {
                        0
                    },
                    |bits, (ch, _)| bits | (1 << ch),
                ),
            Ordering::Relaxed,
        );
        self.rate.store(r.rate, Ordering::Relaxed);
        if let Some(c) = command {
            c.applied.store(true, Ordering::Release);
            c.complete.store(true, Ordering::Release)
        }
        self.revision.fetch_add(1, Ordering::Release);
    }
    fn refresh_running_status(&self, renderer: &Renderer) {
        let state = renderer.state();
        if self.status_hi.load(Ordering::Relaxed) == state.status_hi
            && self.status.load(Ordering::Relaxed) == state.status
            && self.irq.load(Ordering::Relaxed) == state.irq
            && self
                .keys
                .iter()
                .zip(state.keys)
                .all(|(out, value)| out.load(Ordering::Relaxed) == value)
        {
            return;
        }
        self.revision.fetch_add(1, Ordering::AcqRel);
        self.status.store(state.status, Ordering::Relaxed);
        self.status_hi.store(state.status_hi, Ordering::Relaxed);
        self.irq.store(state.irq, Ordering::Relaxed);
        for (out, value) in self.keys.iter().zip(state.keys) {
            out.store(value, Ordering::Relaxed);
        }
        self.revision.fetch_add(1, Ordering::Release);
    }
    pub fn status(&self, frame: u64) -> Result<Status, String> {
        let mut control = self
            .control
            .lock()
            .map_err(|_| "Native YM2608 control unavailable")?;
        self.collect(&mut control);
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() > std::time::Duration::from_millis(50) {
                return Err("Native YM2608 state changed while reading; retry".into());
            }
            let before = self.revision.load(Ordering::Acquire);
            if before & 1 != 0 {
                std::thread::yield_now();
                continue;
            }
            let held = self.held.load(Ordering::Relaxed);
            let out = Status {
                clock: self.clock(),
                registers: self
                    .registers
                    .chunks(256)
                    .map(|bank| bank.iter().map(|v| v.load(Ordering::Relaxed)).collect())
                    .collect(),
                key_masks: self
                    .keys
                    .iter()
                    .map(|v| v.load(Ordering::Relaxed))
                    .collect(),
                ssg_levels: self
                    .levels
                    .iter()
                    .map(|v| v.load(Ordering::Relaxed))
                    .collect(),
                ssg_gates: (8..11)
                    .map(|reg| self.registers[reg].load(Ordering::Relaxed) & 31 != 0)
                    .collect(),
                source_mute_mask: self.source_mute.load(Ordering::Relaxed),
                status_hi: self.status_hi.load(Ordering::Relaxed),
                rom_loaded: self.rom_loaded.load(Ordering::Relaxed),
                adpcm_memory_bytes: self.memory_bytes.load(Ordering::Relaxed),
                prescaler: self.prescaler.load(Ordering::Relaxed),
                status: self.status.load(Ordering::Relaxed),
                irq: self.irq.load(Ordering::Relaxed) != 0,
                held: (0..16).filter(|ch| held & (1 << ch) != 0).collect(),
                pending_commands: control
                    .prepared
                    .iter()
                    .filter(|v| !v.complete.load(Ordering::Relaxed))
                    .filter_map(|v| v.id)
                    .collect(),
                frame,
                sample_rate: self.rate.load(Ordering::Relaxed),
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
    group: Option<u64>,
    owner: Option<Owner>,
    voice: Option<u64>,
    audition: bool,
}
struct Schedule {
    command: Arc<Prepared>,
    owner: Option<Owner>,
    frame: u64,
    index: usize,
    prepared: Arc<Prepared>,
}
pub struct Renderer {
    additional: bool,
    current_group: Option<u64>,
    csm_owner: Option<Lease>,
    schedule: Option<Schedule>,
    prepared_timeline: Option<Arc<Prepared>>,
    rom_loaded: bool,
    memory_bytes: u32,
    ptr: *mut c_void,
    rate: u32,
    owners: [Option<Lease>; VOICES],
    gains: [f32; 2],
    targets: [f32; 2],
    ramp: u32,
}
unsafe impl Send for Renderer {}
impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe { native_ym2608_destroy(self.ptr) }
    }
}
impl Renderer {
    pub fn new_additional(rate: u32) -> Result<Self, String> {
        Self::new_flavor(rate, true)
    }
    fn new_flavor(rate: u32, additional: bool) -> Result<Self, String> {
        let ptr = unsafe { native_ym2608_create_extra(rate) };
        if ptr.is_null() {
            return Err("Could not allocate Native YM2608 core".into());
        }
        let rom = include_bytes!("../native-ym2608/snapshot/web/tetorica_ym2608_adpcm_rom.bin");
        unsafe {
            native_ym2608_load_rom(ptr, rom.as_ptr(), 0, ROM as u32);
        }
        Ok(Self {
            additional,
            current_group: None,
            csm_owner: None,
            schedule: None,
            ptr,
            prepared_timeline: None,
            rom_loaded: true,
            memory_bytes: 0,
            rate,
            owners: [None; VOICES],
            gains: [1.; 2],
            targets: [1.; 2],
            ramp: 0,
        })
    }
    fn state(&self) -> CoreState {
        let mut state = CoreState::default();
        unsafe { native_ym2608_get_extra_state(self.ptr, &mut state) }
        state
    }
    fn has_manual(&self) -> bool {
        self.owners.iter().flatten().any(|l| l.audition)
            || self.csm_owner.is_some_and(|l| l.audition)
    }
    fn protected(&self, ch: usize, voice: Option<u64>, audition: bool) -> bool {
        let protected = |l: Lease| l.audition && (!audition || voice.is_some() && l.voice != voice);
        self.owners[ch].is_some_and(protected) || ch == 2 && self.csm_owner.is_some_and(protected)
    }
    fn release(&mut self, ch: usize) {
        unsafe {
            if self.additional {
                native_ym2608_key_off_fm(self.ptr, ch as u8)
            } else {
                native_ym2608_key_off(self.ptr, ch as u8)
            }
        }
        self.owners[ch] = None;
    }
    fn lease(&self, owner: Option<Owner>, voice: Option<u64>, audition: bool) -> Lease {
        Lease {
            group: self.current_group,
            owner,
            voice,
            audition,
        }
    }
    fn foreign(&self, l: Lease, owner: Option<Owner>, voice: Option<u64>, audition: bool) -> bool {
        owner.is_some() && l.owner != owner
            || self.current_group.is_some() && l.group != self.current_group
            || voice.is_some() && l.voice != voice && (audition || l.audition)
            || l.audition && !audition
    }
    fn op(
        &mut self,
        op: &Op,
        owner: Option<Owner>,
        voice: Option<u64>,
        audition: bool,
        preserve: bool,
        declared: Option<usize>,
    ) {
        let state = self.state();
        let channel = declared_channel(op);
        if channel.is_some_and(|ch| self.protected(ch, voice, audition)) {
            return;
        }
        if !audition {
            let protected_group = match op {
                Op::Clock { .. } => Some(0..16),
                Op::Write {
                    port: 0,
                    register: 0x24..=0x27,
                    ..
                } => Some(0..3),
                Op::Write {
                    port: 0,
                    register: 0x2d..=0x2f,
                    ..
                } => Some(0..16),
                Op::Write {
                    port: 0,
                    register: 0x22,
                    ..
                } => Some(0..6),
                Op::Write {
                    port: 0,
                    register: 0x29,
                    ..
                } => Some(3..6),
                Op::Write {
                    port: 0,
                    register: 6 | 11..=13,
                    ..
                } => Some(6..9),
                Op::Write {
                    port: 0,
                    register: 0x11,
                    ..
                } => Some(9..15),
                _ => None,
            };
            if protected_group
                .is_some_and(|mut group| group.any(|ch| self.protected(ch, voice, false)))
            {
                return;
            }
        }
        if let Op::Write {
            port: 0,
            register: 0x27,
            value,
        } = *op
        {
            let on = value & 0x81 == 0x81;
            if !on
                && self
                    .csm_owner
                    .is_some_and(|l| self.foreign(l, owner, voice, audition))
            {
                return;
            }
            unsafe { native_ym2608_write(self.ptr, 0, 0x27, value) }
            if !on {
                self.csm_owner = None
            } else if !preserve {
                self.csm_owner = Some(self.lease(owner, voice, audition))
            }
            return;
        }
        let off = match *op {
            Op::KeyOff { .. } => true,
            Op::Write {
                port: 0,
                register: 0x28,
                value,
            } => value & 0xf0 == 0,
            Op::Write {
                port: 0,
                register: 8..=10,
                value,
            } => value & 31 == 0,
            Op::Write {
                port: 1,
                register: 0,
                value,
            } => value & 0x80 == 0 || value & 0x41 != 0,
            _ => false,
        };
        let on = match *op {
            Op::Write {
                port: 0,
                register: 0x28,
                value,
            } => value & 0xf0 != 0,
            Op::Write {
                port: 0,
                register: 8..=10,
                value,
            } => value & 31 != 0,
            Op::Write {
                port: 1,
                register: 0,
                value,
            } => value & 0x80 != 0 && value & 0x41 == 0,
            _ => false,
        };
        if off
            && channel.is_some_and(|ch| {
                self.owners[ch].is_some_and(|l| self.foreign(l, owner, voice, audition))
            })
        {
            return;
        }
        unsafe {
            match *op {
                Op::KeyOff { channel } => native_ym2608_key_off_fm(self.ptr, channel),
                Op::Clock { hz } => {
                    native_ym2608_set_clock(self.ptr, hz);
                }
                Op::SetRetainedLevel { channel, level } => {
                    native_ym2608_set_retained_ssg_level(self.ptr, channel, level)
                }
                Op::Write {
                    port: 0,
                    register: 7,
                    mut value,
                } => {
                    for ch in 6..9 {
                        let bits = (1 << (ch - 6)) | (1 << (ch - 3));
                        let old = state.registers[7] & bits;
                        let new = value & bits;
                        if self.protected(ch, voice, audition)
                            || declared.is_some_and(|v| v != ch)
                            || old != bits
                                && new == bits
                                && self.owners[ch]
                                    .is_some_and(|l| self.foreign(l, owner, voice, audition))
                        {
                            value = (value & !bits) | old;
                        } else if !preserve {
                            if old == bits && new != bits && state.registers[8 + ch - 6] & 31 != 0 {
                                self.owners[ch] = Some(self.lease(owner, voice, audition))
                            } else if old != bits && new == bits {
                                self.owners[ch] = None
                            }
                        }
                    }
                    native_ym2608_write(self.ptr, 0, 7, value)
                }
                Op::Write {
                    port: 0,
                    register: 0x10,
                    mut value,
                } => {
                    let stop = value & 0x80 != 0;
                    for ch in 9..15 {
                        if self.protected(ch, voice, audition)
                            || declared.is_some_and(|v| v != ch)
                            || stop
                                && self.owners[ch]
                                    .is_some_and(|l| self.foreign(l, owner, voice, audition))
                        {
                            value &= !(1 << (ch - 9));
                        }
                    }
                    native_ym2608_write(self.ptr, 0, 0x10, value);
                    if !preserve {
                        for ch in 9..15 {
                            if value & (1 << (ch - 9)) != 0 {
                                self.owners[ch] = if stop {
                                    None
                                } else {
                                    Some(self.lease(owner, voice, audition))
                                }
                            }
                        }
                    }
                }
                Op::Write {
                    port: 0,
                    register: 0x28,
                    value,
                } if audition && value & 0xf0 != 0 => {
                    if let Some(ch) = fm_key_channel(value) {
                        native_ym2608_audition_fm_key_on(self.ptr, ch as u8, value >> 4)
                    }
                }
                Op::Write {
                    port,
                    register,
                    value,
                } => native_ym2608_write(self.ptr, port, register, value),
            }
        }
        if !preserve {
            if let Some(ch) = channel {
                if off {
                    self.owners[ch] = None
                } else if on {
                    self.owners[ch] = Some(self.lease(owner, voice, audition))
                }
            }
        }
    }
    pub fn apply(&mut self, c: &Arc<Prepared>, owner: Option<Owner>, shared: &Shared) {
        if !c.audition
            && self.has_manual()
            && match &c.method {
                Method::Clock { .. } => true,
                Method::Batch { commands, .. } => {
                    commands.iter().any(|op| matches!(op, Op::Clock { .. }))
                }
                _ => false,
            }
        {
            c.clock_rejected.store(true, Ordering::Release);
            shared.publish(self, Some(c));
            return;
        }
        self.current_group = if self.additional { c.group } else { None };
        if let Some(target) = c.cancel_prepare.get() {
            if !target.applied.load(Ordering::Acquire) {
                target.invalidated.store(true, Ordering::Release);
            }
        }
        // Scheduling is transport state: reset cancels it even if a held
        // manual voice prevents the DSP reset itself.
        if self.additional && c.invalidates_schedule() {
            self.cancel_schedule(
                c.cancel_target.get(),
                if matches!(c.method, Method::Reset) {
                    None
                } else {
                    owner
                },
                shared,
            );
        }
        match &c.method {
            Method::Write {
                port,
                register,
                value,
            } => self.op(
                &Op::Write {
                    port: *port,
                    register: *register,
                    value: *value,
                },
                owner,
                None,
                c.audition,
                false,
                None,
            ),
            Method::KeyOff { channel } => self.op(
                &Op::KeyOff { channel: *channel },
                owner,
                None,
                c.audition,
                false,
                None,
            ),
            Method::SetRetainedLevel { channel, level } => self.op(
                &Op::SetRetainedLevel {
                    channel: *channel,
                    level: *level,
                },
                owner,
                None,
                c.audition,
                false,
                None,
            ),
            Method::Clock { hz } => {
                self.op(&Op::Clock { hz: *hz }, owner, None, c.audition, false, None)
            }
            Method::Mix { volume, pan } => {
                self.targets = [
                    *volume * if *pan > 0. { 1. - *pan } else { 1. },
                    *volume * if *pan < 0. { 1. + *pan } else { 1. },
                ];
                self.ramp = (self.rate / 200).max(1)
            }
            Method::SourceMute { mask } => unsafe {
                native_ym2608_set_source_mute_mask(self.ptr, *mask)
            },

            Method::Reset => {
                if c.audition || !self.has_manual() {
                    unsafe { native_ym2608_reset(self.ptr) }
                    self.owners.fill(None);
                    self.csm_owner = None;
                }
            }
            Method::CancelCsm => {
                let allowed = self.csm_owner.map_or_else(
                    || !self.protected(2, None, c.audition),
                    |lease| {
                        (owner.is_none() || lease.owner == owner)
                            && (c.group.is_none() || lease.group == c.group)
                            && (!lease.audition || c.audition)
                    },
                );
                if allowed {
                    unsafe {
                        native_ym2608_cancel_csm(self.ptr);
                    }
                    self.csm_owner = None;
                }
            }
            Method::CancelMainOwner { target_group } => {
                let matches = |lease: Lease| !lease.audition && lease.group == Some(*target_group);
                if self.csm_owner.is_some_and(matches) {
                    unsafe {
                        native_ym2608_cancel_csm(self.ptr);
                    }
                    self.csm_owner = None;
                }
                for channel in 0..16 {
                    if self.owners[channel].is_some_and(matches) {
                        self.release(channel);
                    }
                }
            }
            Method::StopVoices if self.additional => {
                if owner.is_none() && c.group.is_none() && (c.audition || !self.has_manual()) {
                    unsafe { native_ym2608_stop(self.ptr) }
                    self.owners.fill(None);
                    self.csm_owner = None;
                } else {
                    if self.csm_owner.is_some_and(|lease| {
                        (owner.is_none() || lease.owner == owner)
                            && (c.group.is_none() || lease.group == c.group)
                            && (!lease.audition || c.audition)
                    }) {
                        unsafe {
                            native_ym2608_cancel_csm(self.ptr);
                        }
                        self.csm_owner = None;
                    }
                    for ch in 0..16 {
                        if if owner.is_none() {
                            self.owners[ch].is_none_or(|l| {
                                (c.group.is_none() || l.group == c.group)
                                    && (!l.audition || c.audition)
                            })
                        } else {
                            self.owners[ch].is_some_and(|l| {
                                l.owner == owner
                                    && (c.group.is_none() || l.group == c.group)
                                    && (!l.audition || c.audition)
                            })
                        } {
                            self.release(ch)
                        }
                    }
                }
            }
            Method::StopVoices => unreachable!(),
            Method::Rom { offset, data } => unsafe {
                native_ym2608_load_rom(self.ptr, data.as_ptr(), *offset, data.len() as u32);
                self.rom_loaded = true;
            },
            Method::Memory { offset, data } => unsafe {
                native_ym2608_load_memory(self.ptr, data.as_ptr(), *offset, data.len() as u32);
                self.memory_bytes = self.memory_bytes.max(*offset + data.len() as u32);
            },
            Method::PrepareTimeline { .. } => {
                self.prepared_timeline = Some(c.clone());
            }
            Method::PlayTimeline { .. } => {
                if let Some(prepared) = c.play_target.get().filter(|p| {
                    p.applied.load(Ordering::Acquire) && !p.invalidated.load(Ordering::Acquire)
                }) {
                    if c.audition || !self.has_manual() {
                        unsafe { native_ym2608_reset(self.ptr) }
                        self.owners.fill(None);
                        self.csm_owner = None;
                    }
                    self.schedule = Some(Schedule {
                        command: c.clone(),
                        owner,
                        frame: 0,
                        index: 0,
                        prepared: prepared.clone(),
                    });
                    shared.schedule_phase.store(2, Ordering::Release);
                } else {
                    shared.retire(c.clone());
                }
            }
            Method::Batch {
                commands,
                channel,
                voice,
                action,
            } => {
                let ch = *channel as usize;
                let current = self.owners[ch];
                let allowed = match action {
                    LeaseAction::On => c.audition || current.is_none_or(|l| !l.audition),
                    LeaseAction::Off => current.is_some_and(|l| {
                        l.owner == owner
                            && (c.group.is_none() || l.group == c.group)
                            && (voice.is_none() || l.voice == *voice)
                            && (!l.audition || c.audition)
                    }),
                    LeaseAction::Edit => current.is_none_or(|l| {
                        !l.audition || c.audition && (voice.is_none() || l.voice == *voice)
                    }),
                };
                if allowed {
                    if *action == LeaseAction::On {
                        if ch == 2 && c.audition {
                            self.csm_owner = None;
                        }
                        self.owners[ch] = Some(Lease {
                            group: self.current_group,
                            owner,
                            voice: *voice,
                            audition: c.audition,
                        })
                    }
                    for op in commands {
                        self.op(
                            op,
                            owner,
                            *voice,
                            c.audition,
                            *action == LeaseAction::Edit,
                            Some(ch),
                        )
                    }
                    if *action == LeaseAction::On {
                        self.owners[ch] = Some(Lease {
                            group: self.current_group,
                            owner,
                            voice: *voice,
                            audition: c.audition,
                        })
                    } else if *action == LeaseAction::Off {
                        self.owners[ch] = None
                    }
                }
            }
        }
        shared.publish(self, Some(c));
        self.current_group = None;
    }
    fn release_scheduled_voices(&mut self, token: u64, owner: Option<Owner>) {
        let matches = |lease: Lease| {
            !lease.audition
                && lease.voice == Some(token)
                && owner.is_none_or(|owner| lease.owner == Some(owner))
        };
        if self.csm_owner.is_some_and(matches) {
            unsafe {
                native_ym2608_cancel_csm(self.ptr);
            }
            self.csm_owner = None;
        }
        for channel in 0..16 {
            if self.owners[channel].is_some_and(matches) {
                self.release(channel);
            }
        }
    }
    fn cancel_schedule(
        &mut self,
        target: Option<&Arc<Prepared>>,
        owner: Option<Owner>,
        shared: &Shared,
    ) {
        let Some(target) = target else {
            return;
        };
        if !owner.is_none_or(|owner| target.schedule_owner.get() == Some(&Some(owner))) {
            return;
        }
        if let Some(active) = &self.schedule {
            if Arc::ptr_eq(target, &active.command) {
                let active = self.schedule.take().unwrap();
                self.release_scheduled_voices(
                    active.command.schedule_token().unwrap(),
                    active.owner,
                );
                shared.retire(active.command);
                return;
            }
        }
        target.schedule_finished.store(true, Ordering::Release);
        if shared.schedule_token.load(Ordering::Acquire) == target.schedule_token().unwrap_or(0) {
            shared.schedule_phase.store(4, Ordering::Release);
        }
    }
    pub fn mix_additional(&mut self, pcm: &mut [f32; 2], shared: &Shared) {
        if let Some(mut active) = self.schedule.take() {
            let mut changed = false;
            if !active.command.schedule_finished.load(Ordering::Acquire) {
                if let Method::PrepareTimeline {
                    events,
                    blocks,
                    duration_samples,
                } = &active.prepared.method
                {
                    self.current_group = active.command.group;
                    let rate = self.rate;
                    let to_frame = |sample: u64| (sample as u128 * rate as u128 + 44099) / 44100;
                    while let Some(&(at, port, register, value)) = events.get(active.index) {
                        if to_frame(at) > active.frame as u128 {
                            break;
                        }
                        if port == 2 {
                            if !self.protected(15, None, false) {
                                let data = &blocks[register as usize];
                                unsafe {
                                    native_ym2608_load_memory(
                                        self.ptr,
                                        data.as_ptr(),
                                        value,
                                        data.len() as u32,
                                    );
                                }
                                self.memory_bytes =
                                    self.memory_bytes.max(value + data.len() as u32);
                            }
                        } else {
                            self.op(
                                &Op::Write {
                                    port,
                                    register: register as u8,
                                    value: value as u8,
                                },
                                active.owner,
                                active.command.schedule_token(),
                                false,
                                false,
                                None,
                            );
                        }
                        active.index += 1;
                        changed = true;
                    }
                    if active.frame as u128 >= to_frame(*duration_samples) {
                        shared.schedule_phase.store(3, Ordering::Release);
                        active
                            .command
                            .schedule_finished
                            .store(true, Ordering::Release);
                        shared.retire(active.command);
                    } else {
                        active.frame += 1;
                        self.schedule = Some(active);
                    }
                }
            } else {
                shared.retire(active.command);
            }
            if changed {
                shared.publish(self, None)
            }
            self.current_group = None;
        }
        self.mix(pcm);
    }
    pub fn publish(&self, s: &Shared) {
        s.publish(self, None)
    }
    pub fn cancel_invalid(&mut self, valid: impl Fn(Option<Owner>) -> bool, s: &Shared) {
        let mut changed = false;
        if self
            .schedule
            .as_ref()
            .is_some_and(|schedule| !valid(schedule.owner))
        {
            let active = self.schedule.take().unwrap();
            s.retire(active.command);
        }
        if self.additional && self.csm_owner.is_some_and(|lease| !valid(lease.owner)) {
            unsafe {
                native_ym2608_cancel_csm(self.ptr);
            }
            self.csm_owner = None;
            changed = true;
        }
        for ch in 0..16 {
            if self.owners[ch].is_some_and(|l| !valid(l.owner)) {
                self.release(ch);
                changed = true
            }
        }
        if changed {
            s.publish(self, None)
        } else if self.additional {
            s.refresh_running_status(self);
        }
    }
    pub fn mix(&mut self, pcm: &mut [f32; 2]) {
        let (mut l, mut r) = (0., 0.);
        unsafe { native_ym2608_sample(self.ptr, &mut l, &mut r) }
        if self.ramp > 0 {
            for ch in 0..2 {
                self.gains[ch] += (self.targets[ch] - self.gains[ch]) / self.ramp as f32
            }
            self.ramp -= 1
        }
        pcm[0] += l * self.gains[0];
        pcm[1] += r * self.gains[1];
    }
}
