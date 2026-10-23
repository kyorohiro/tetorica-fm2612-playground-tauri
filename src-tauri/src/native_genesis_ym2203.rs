//! Callback-owned PC-98-26 YM2203: three FM and three SSG voices.
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
pub const CLOCK: u32 = 4_000_000;
const VOICES: usize = 6;
const PENDING_BYTES: usize = 4 * 1024 * 1024;
#[repr(C)]
struct CoreState {
    registers: [u8; 256],
    clock: u32,
    source_mute_mask: u32,
    channel_mute_mask: u32,
    keys: [u8; 3],
    ssg_levels: [u8; 3],
    prescaler: u8,
    status: u8,
    irq: u8,
}
impl Default for CoreState {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}
extern "C" {
    fn native_ym2203_create(rate: u32) -> *mut c_void;
    fn native_ym2203_create_extra(rate: u32) -> *mut c_void;
    fn native_ym2203_destroy(p: *mut c_void);
    fn native_ym2203_reset(p: *mut c_void);
    fn native_ym2203_stop(p: *mut c_void);
    fn native_ym2203_write(p: *mut c_void, port: u8, register: u8, value: u8);
    fn native_ym2203_key_off(p: *mut c_void, voice: u8);
    fn native_ym2203_key_off_fm(p: *mut c_void, voice: u8);
    fn native_ym2203_cancel_csm(p: *mut c_void);
    fn native_ym2203_audition_key_on(p: *mut c_void, voice: u8);
    fn native_ym2203_audition_fm_key_on(p: *mut c_void, channel: u8, mask: u8);
    fn native_ym2203_sample(p: *mut c_void, l: *mut f32, r: *mut f32);
    fn native_ym2203_set_clock(p: *mut c_void, hz: u32) -> i32;
    fn native_ym2203_set_source_mute_mask(p: *mut c_void, mask: u32);
    fn native_ym2203_set_mute_mask(p: *mut c_void, mask: u32);
    fn native_ym2203_set_retained_ssg_level(p: *mut c_void, ch: u8, level: u8);
    fn native_ym2203_get_state(p: *mut c_void, s: *mut CoreState);
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
    AuditionKeyOn {
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
    ChannelMute {
        mask: u32,
    },
    Reset,
    StopVoices,
    CancelCsm,
    Schedule {
        entries: Vec<(u64, u8, u8)>,
        #[serde(rename = "durationSamples")]
        duration_samples: u64,
        token: u64,
    },
    CancelSchedule {
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
    AuditionKeyOn { channel: u8 },
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
}
fn valid_id(id: Option<u64>) -> bool {
    id.is_none_or(|v| v > 0 && v <= MAX_SAFE_INTEGER)
}
fn declared_channel(op: &Op) -> Option<usize> {
    match *op {
        Op::KeyOff { channel } | Op::AuditionKeyOn { channel } => Some(channel as usize),
        Op::SetRetainedLevel { channel, .. } => Some(3 + channel as usize),
        Op::Write {
            register: 0x28,
            value,
            ..
        } => {
            let ch = value & 7;
            (ch < 3).then_some(ch as usize)
        }
        Op::Write {
            register: 0..=5, ..
        } => {
            if let Op::Write { register, .. } = *op {
                Some(3 + (register / 2) as usize)
            } else {
                None
            }
        }
        Op::Write {
            register: 8..=10, ..
        } => {
            if let Op::Write { register, .. } = *op {
                Some(3 + (register - 8) as usize)
            } else {
                None
            }
        }
        Op::Write {
            register: 0xa8..=0xae,
            ..
        } => {
            if let Op::Write { register, .. } = *op {
                (register & 3 < 3).then_some(2)
            } else {
                None
            }
        }
        Op::Write {
            register: 0x30..=0xb6,
            ..
        } => {
            if let Op::Write { register, .. } = *op {
                (register & 3 < 3).then_some((register & 3) as usize)
            } else {
                None
            }
        }
        _ => None,
    }
}
fn validate_op(op: &Op) -> Result<(), String> {
    match *op {
        Op::Write { port, .. } if port != 0 => Err("Native YM2203 port must be 0".into()),
        Op::KeyOff { channel } | Op::AuditionKeyOn { channel } if channel >= 6 => {
            Err("Native YM2203 voice must be 0–5".into())
        }
        Op::SetRetainedLevel { channel, level } if channel >= 3 || level > 31 => {
            Err("Invalid Native YM2203 SSG retained level".into())
        }
        Op::Clock { hz } if !(100_000..=100_000_000).contains(&hz) => {
            Err("Native YM2203 clock must be 100000–100000000 Hz".into())
        }
        _ => Ok(()),
    }
}
impl Message {
    pub fn prepared_bytes(&self) -> usize {
        std::mem::size_of::<Prepared>()
            + match &self.method {
                Method::Batch { commands, .. } => commands.len() * std::mem::size_of::<Op>(),
                Method::Schedule { entries, .. } => {
                    entries.len() * std::mem::size_of::<(u64, u8, u8)>()
                }
                _ => 0,
            }
    }

    pub fn prepare(&self) -> Result<Arc<Prepared>, String> {
        if !valid_id(self.id) || !valid_id(self.group) {
            return Err("Invalid Native YM2203 command ID".into());
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
            Method::KeyOff { channel } | Method::AuditionKeyOn { channel } => {
                validate_op(&Op::KeyOff { channel: *channel })?
            }
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
                return Err("Native YM2203 mixer volume must be 0–4, pan −1–1".into())
            }
            Method::SourceMute { mask } if *mask > 3 => {
                return Err("Native YM2203 source mute mask must be 0–3".into())
            }
            Method::ChannelMute { mask } if *mask > 7 => {
                return Err("Native YM2203 FM mute mask must be 0–7".into())
            }
            Method::Schedule {
                entries,
                duration_samples,
                token,
            } => {
                if *duration_samples == 0
                    || *duration_samples > MAX_SAFE_INTEGER
                    || !valid_id(Some(*token))
                    || entries.len() > 131072
                {
                    return Err("Invalid Native YM2203 schedule or entry limit (131072)".into());
                }
                let mut previous = 0;
                for &(sample, _, _) in entries {
                    if sample < previous || sample > *duration_samples {
                        return Err("Invalid Native YM2203 scheduled sample".into());
                    }
                    previous = sample;
                }
            }
            Method::CancelMainOwner { target_group } if !valid_id(Some(*target_group)) => {
                return Err("Invalid Native YM2203 Main owner group".into())
            }
            Method::CancelSchedule { token } if !valid_id(Some(*token)) => {
                return Err("Invalid Native YM2203 schedule token".into())
            }
            Method::Batch {
                commands,
                channel,
                voice,
                action,
            } => {
                if *channel >= 6
                    || commands.is_empty()
                    || commands.len() > 512
                    || !valid_id(*voice)
                    || self.audition && *action != LeaseAction::Edit && voice.is_none()
                {
                    return Err("Invalid Native YM2203 batch or voice lease".into());
                }
                for op in commands {
                    validate_op(op)?;
                    if declared_channel(op).is_some_and(|target| target != *channel as usize) {
                        return Err(
                            "Native YM2203 batch must target its declared physical voice".into(),
                        );
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
        }))
    }
}
impl Prepared {
    pub fn starts(&self) -> bool {
        match &self.method {
            Method::KeyOff { .. }
            | Method::SetRetainedLevel { .. }
            | Method::Clock { .. }
            | Method::Mix { .. }
            | Method::SourceMute { .. }
            | Method::ChannelMute { .. }
            | Method::Reset
            | Method::StopVoices
            | Method::CancelCsm
            | Method::CancelSchedule { .. }
            | Method::CancelMainOwner { .. } => false,
            Method::Batch { action, .. } => *action != LeaseAction::Off,
            _ => true,
        }
    }
    pub fn stops(&self) -> bool {
        matches!(self.method, Method::StopVoices) && self.group.is_none()
    }
    pub fn deferred_retirement(&self) -> bool {
        matches!(self.method, Method::Schedule { .. })
    }
    pub fn invalidates_schedule(&self) -> bool {
        matches!(
            self.method,
            Method::Reset
                | Method::CancelSchedule { .. }
                | Method::CancelMainOwner { .. }
                | Method::StopVoices
        )
    }
    fn schedule_token(&self) -> Option<u64> {
        if let Method::Schedule { token, .. } = self.method {
            Some(token)
        } else {
            None
        }
    }
    fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + if let Method::Batch { commands, .. } = &self.method {
                commands.len() * std::mem::size_of::<Op>()
            } else if let Method::Schedule { entries, .. } = &self.method {
                entries.len() * std::mem::size_of::<(u64, u8, u8)>()
            } else {
                0
            }
    }
    pub fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + if let Method::Batch { commands, .. } = &self.method {
                commands.capacity() * std::mem::size_of::<Op>()
            } else if let Method::Schedule { entries, .. } = &self.method {
                entries.capacity() * std::mem::size_of::<(u64, u8, u8)>()
            } else {
                0
            }
    }
}
#[derive(Default)]
struct Control {
    highest_schedule: u64,
    prepared: Vec<Arc<Prepared>>,
    rejected_clocks: HashSet<u64>,
}
pub struct Shared {
    additional: bool,
    schedule_token: AtomicU64,
    schedule_phase: AtomicU8,
    revision: AtomicU64,
    registers: [AtomicU8; 256],
    keys: [AtomicU8; 3],
    levels: [AtomicU8; 3],
    clock: AtomicU32,
    source_mute: AtomicU32,
    channel_mute: AtomicU32,
    prescaler: AtomicU8,
    status: AtomicU8,
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
            additional: false,
            schedule_token: AtomicU64::new(0),
            schedule_phase: AtomicU8::new(0),
            revision: AtomicU64::new(0),
            registers: std::array::from_fn(|_| AtomicU8::new(0)),
            keys: std::array::from_fn(|_| AtomicU8::new(0)),
            levels: std::array::from_fn(|_| AtomicU8::new(0)),
            clock: AtomicU32::new(CLOCK),
            source_mute: AtomicU32::new(0),
            channel_mute: AtomicU32::new(0),
            prescaler: AtomicU8::new(6),
            status: AtomicU8::new(0),
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
    pub channel_mute_mask: u32,
    pub prescaler: u8,
    pub status: u8,
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
    pub ym2203: AdditionalChipStatus,
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
    pub channel_mute_mask: u32,
    pub prescaler: u8,
    pub status: u8,
    pub irq: bool,
    pub held: Vec<u8>,
    pub pending_commands: Vec<u64>,
    pub schedule: ScheduleStatus,
}
#[derive(Debug, Serialize)]
pub struct ScheduleStatus {
    pub token: u64,
    pub pending: bool,
    pub playing: bool,
    pub ended: bool,
    pub cancelled: bool,
}
impl Shared {
    pub fn additional() -> Self {
        Self {
            additional: true,
            ..Self::default()
        }
    }
    fn clock_held_error(&self) -> &'static str {
        if self.additional {
            "Release the manually held additional YM2203 voices before changing the clock"
        } else {
            "Release the manually held YM2203 voices before changing the clock"
        }
    }
    pub fn additional_status(&self, frame: u64) -> Result<AdditionalStatus, String> {
        let value = self.status(frame)?;
        Ok(AdditionalStatus {
            frame: value.frame,
            sample_rate: value.sample_rate,
            revision: value.revision,
            ym2203: AdditionalChipStatus {
                clock: value.clock,
                registers: value.registers,
                key_masks: value.key_masks,
                ssg_levels: value.ssg_levels,
                ssg_gates: value.ssg_gates,
                source_mute_mask: value.source_mute_mask,
                channel_mute_mask: value.channel_mute_mask,
                prescaler: value.prescaler,
                status: value.status,
                irq: value.irq,
                held: value.held,
                pending_commands: value.pending_commands,
                schedule: self.schedule_status(),
            },
        })
    }
    fn schedule_status(&self) -> ScheduleStatus {
        let phase = self.schedule_phase.load(Ordering::Acquire);
        ScheduleStatus {
            token: self.schedule_token.load(Ordering::Acquire),
            pending: phase == 1,
            playing: phase == 2,
            ended: phase == 3,
            cancelled: phase == 4,
        }
    }
    pub fn clock(&self) -> u32 {
        self.clock.load(Ordering::Acquire)
    }
    pub fn valid_event(&self, command: &Prepared, sequence: u64, owner: Option<Owner>) -> bool {
        (!command.deferred_retirement() || !command.schedule_finished.load(Ordering::Acquire))
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
    pub fn collect_retired(&self) {
        if let Ok(mut c) = self.control.lock() {
            self.collect(&mut c)
        }
    }
    fn collect(&self, c: &mut Control) {
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
    pub fn retain(&self, commands: &[(Arc<Prepared>, Option<Owner>)]) -> Result<(), String> {
        let mut c = self
            .control
            .lock()
            .map_err(|_| "Native YM2203 control state unavailable")?;
        self.collect(&mut c);
        if self.held.load(Ordering::Acquire) != 0
            && commands.iter().any(|(v, _)| {
                !v.audition
                    && match &v.method {
                        Method::Clock { .. } => true,
                        Method::Batch { commands, .. } => {
                            commands.iter().any(|op| matches!(op, Op::Clock { .. }))
                        }
                        _ => false,
                    }
            })
        {
            return Err(self.clock_held_error().into());
        }

        if c.prepared.len() + c.rejected_clocks.len() + commands.len() > CAPACITY
            || c.prepared.iter().map(|v| v.bytes()).sum::<usize>()
                + commands.iter().map(|(v, _)| v.bytes()).sum::<usize>()
                > PENDING_BYTES
        {
            return Err("Native YM2203 pending command limit reached (4 MiB)".into());
        }
        let active_schedule = c
            .prepared
            .iter()
            .find(|command| {
                command.deferred_retirement() && !command.schedule_finished.load(Ordering::Acquire)
            })
            .cloned();
        let schedules: Vec<_> = commands
            .iter()
            .filter(|(command, _)| command.deferred_retirement())
            .collect();
        if !schedules.is_empty()
            && (!self.additional || active_schedule.is_some() || schedules.len() > 1)
        {
            return Err("Native YM2203 schedule already playing or pending".into());
        }
        for (command, _) in &schedules {
            if command
                .schedule_token()
                .is_some_and(|token| token <= c.highest_schedule)
            {
                return Err("Native YM2203 schedule token must be unique and increasing".into());
            }
            if let Method::Schedule {
                duration_samples,
                ref entries,
                ..
            } = command.method
            {
                let frames = (duration_samples as u128 * self.rate.load(Ordering::Acquire) as u128
                    + 44099)
                    / 44100;
                if frames > MAX_SAFE_INTEGER as u128 {
                    return Err("Native YM2203 schedule duration exceeds safe frame range".into());
                }
                // Bound work in every sliding 128-output-frame window, not
                // just a single timestamp. Across the shared sixteen-slot
                // rack this admits at most 4096 scheduled writes per block.
                for index in 256..entries.len() {
                    let to_frame = |sample: u64| {
                        (sample as u128 * self.rate.load(Ordering::Acquire) as u128 + 44099) / 44100
                    };
                    if to_frame(entries[index].0) - to_frame(entries[index - 256].0) < 128 {
                        return Err(
                            "Native YM2203 schedule exceeds 256 writes in 128 output frames".into(),
                        );
                    }
                }
            }
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
                return Err("Duplicate pending Native YM2203 command ID".into());
            }
        }
        let mut ordered_schedule = active_schedule;
        for (command, owner) in commands {
            if let Some(target) = &ordered_schedule {
                let matches = match command.method {
                    Method::Reset => true,
                    Method::CancelSchedule { token } => {
                        target.schedule_token() == Some(token)
                            && (command.group.is_none() || target.group == command.group)
                    }
                    Method::CancelMainOwner { target_group } => target.group == Some(target_group),
                    Method::StopVoices => command.group.is_none() || target.group == command.group,
                    _ => false,
                };
                // Source cancellation follows the requesting performance owner.
                if matches {
                    let _ = command.cancel_target.set(target.clone());
                }
            }
            if let Some(token) = command.schedule_token() {
                let _ = command.schedule_owner.set(*owner);
                c.highest_schedule = token;
                let _ = command.previous_schedule.set((
                    self.schedule_token.load(Ordering::Acquire),
                    self.schedule_phase.load(Ordering::Acquire),
                ));
                ordered_schedule = Some(command.clone());
                self.schedule_token.store(token, Ordering::Relaxed);
                self.schedule_phase.store(1, Ordering::Release);
            }
        }
        for (cmd, _) in commands {
            if let Some(id) = cmd.id {
                self.highest_id.fetch_max(id, Ordering::AcqRel);
            }
            c.prepared.push(cmd.clone());
        }
        Ok(())
    }
    pub fn wait_applied(&self, id: u64) -> Result<(), String> {
        if id == 0 || id > self.highest_id.load(Ordering::Acquire) {
            return Err("Unknown Native YM2203 command ID".into());
        }
        let cmd = {
            let mut control = self
                .control
                .lock()
                .map_err(|_| "Native YM2203 control state unavailable")?;
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
                let deadline =
                    if matches!(cmd.method, Method::Clock { .. } | Method::Schedule { .. })
                        || self.additional
                            && matches!(cmd.method, Method::Reset | Method::CancelSchedule { .. })
                    {
                        5000
                    } else {
                        100
                    };
                if start.elapsed() > std::time::Duration::from_millis(deadline) {
                    return Err(format!(
                        "Native YM2203 command was not applied within {deadline} ms"
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            cmd.outcome_observed.store(true, Ordering::Release);
            if cmd.clock_rejected.load(Ordering::Acquire) {
                self.control
                    .lock()
                    .map_err(|_| "Native YM2203 control state unavailable")?
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
        self.retire(cmd);
        if token.is_some_and(|token| self.schedule_token.load(Ordering::Acquire) == token) {
            if let Some((token, phase)) = previous {
                self.schedule_token.store(token, Ordering::Relaxed);
                self.schedule_phase.store(phase, Ordering::Release);
            }
        }
    }
    pub fn retire(&self, cmd: Arc<Prepared>) {
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
            .unwrap_or_else(|_| unreachable!("Bounded YM2203 command retirement"));
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
        self.channel_mute
            .store(state.channel_mute_mask, Ordering::Relaxed);
        self.prescaler.store(state.prescaler, Ordering::Relaxed);
        self.status.store(state.status, Ordering::Relaxed);
        self.irq.store(state.irq, Ordering::Relaxed);
        self.held.store(
            r.owners
                .iter()
                .enumerate()
                .filter(|(_, v)| v.is_some_and(|l| l.audition))
                .fold(0, |bits, (ch, _)| bits | (1 << ch)),
            Ordering::Relaxed,
        );
        self.rate.store(r.rate, Ordering::Relaxed);
        if let Some(c) = command {
            c.complete.store(true, Ordering::Release)
        }
        self.revision.fetch_add(1, Ordering::Release);
    }
    fn refresh_running_status(&self, renderer: &Renderer) {
        let state = renderer.state();
        if self.status.load(Ordering::Relaxed) == state.status
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
            .map_err(|_| "Native YM2203 control unavailable")?;
        self.collect(&mut control);
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() > std::time::Duration::from_millis(50) {
                return Err("Native YM2203 state changed while reading; retry".into());
            }
            let before = self.revision.load(Ordering::Acquire);
            if before & 1 != 0 {
                std::thread::yield_now();
                continue;
            }
            let held = self.held.load(Ordering::Relaxed);
            let out = Status {
                clock: self.clock(),
                registers: vec![self
                    .registers
                    .iter()
                    .map(|v| v.load(Ordering::Relaxed))
                    .collect()],
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
                channel_mute_mask: self.channel_mute.load(Ordering::Relaxed),
                prescaler: self.prescaler.load(Ordering::Relaxed),
                status: self.status.load(Ordering::Relaxed),
                irq: self.irq.load(Ordering::Relaxed) != 0,
                held: (0..6).filter(|ch| held & (1 << ch) != 0).collect(),
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
}
pub struct Renderer {
    additional: bool,
    current_group: Option<u64>,
    csm_owner: Option<Lease>,
    schedule: Option<Schedule>,
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
        unsafe { native_ym2203_destroy(self.ptr) }
    }
}
impl Renderer {
    pub fn new(rate: u32) -> Result<Self, String> {
        Self::new_flavor(rate, false)
    }
    pub fn new_additional(rate: u32) -> Result<Self, String> {
        Self::new_flavor(rate, true)
    }
    fn new_flavor(rate: u32, additional: bool) -> Result<Self, String> {
        let ptr = unsafe {
            if additional {
                native_ym2203_create_extra(rate)
            } else {
                native_ym2203_create(rate)
            }
        };
        if ptr.is_null() {
            return Err("Could not allocate Native YM2203 core".into());
        }
        Ok(Self {
            additional,
            current_group: None,
            csm_owner: None,
            schedule: None,
            ptr,
            rate,
            owners: [None; VOICES],
            gains: [1.; 2],
            targets: [1.; 2],
            ramp: 0,
        })
    }
    fn state(&self) -> CoreState {
        let mut state = CoreState::default();
        unsafe { native_ym2203_get_state(self.ptr, &mut state) }
        state
    }
    fn protected(&self, ch: usize, voice: Option<u64>, audition: bool) -> bool {
        self.owners[ch]
            .is_some_and(|l| l.audition && (!audition || voice.is_some() && l.voice != voice))
    }
    fn release(&mut self, ch: usize) {
        unsafe {
            if self.additional {
                native_ym2203_key_off_fm(self.ptr, ch as u8)
            } else {
                native_ym2203_key_off(self.ptr, ch as u8)
            }
        }
        self.owners[ch] = None;
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
        let csm_before = state.registers[0x27] & 0x81 == 0x81;
        let csm_write = if let Op::Write {
            register: 0x27,
            value,
            ..
        } = *op
        {
            Some(value & 0x81 == 0x81)
        } else {
            None
        };
        // CSM can sound CH3 solely from Timer A, without register 0x28.
        // Track that implicit physical voice so owner cancellation can mute it.
        let channel =
            declared_channel(op).or_else(|| csm_write.filter(|on| *on || csm_before).map(|_| 2));
        if channel.is_some_and(|ch| self.protected(ch, voice, audition)) {
            return;
        }
        if !audition {
            match *op {
                Op::Clock { .. } if self.owners.iter().flatten().any(|l| l.audition) => return,
                Op::Write {
                    register: 0x24..=0x27,
                    ..
                } if (0..3).any(|ch| self.protected(ch, voice, false)) => return,
                Op::Write {
                    register: 0x2d..=0x2f,
                    ..
                } if (0..6).any(|ch| self.protected(ch, voice, false)) => return,
                Op::Write { register: 6, .. }
                    if (3..6).any(|ch| {
                        self.protected(ch, voice, false) && state.registers[7] & (1 << (ch)) == 0
                    }) =>
                {
                    return
                }
                Op::Write {
                    register: 11..=13, ..
                } if (3..6).any(|ch| {
                    self.protected(ch, voice, false) && state.registers[8 + ch - 3] & 16 != 0
                }) =>
                {
                    return
                }
                _ => {}
            }
        }
        if self.additional {
            if let Some(csm_on) = csm_write {
                if csm_before
                    && !csm_on
                    && self.csm_owner.is_some_and(|lease| {
                        owner.is_some() && lease.owner != owner
                            || self.current_group.is_some() && lease.group != self.current_group
                            || voice.is_some()
                                && lease.voice != voice
                                && (audition || lease.audition)
                            || lease.audition && !audition
                    })
                {
                    return;
                }
                if let Op::Write {
                    port,
                    register,
                    value,
                } = *op
                {
                    unsafe {
                        native_ym2203_write(self.ptr, port, register, value);
                    }
                }
                if !csm_on {
                    self.csm_owner = None;
                } else if !preserve {
                    self.csm_owner = Some(Lease {
                        group: self.current_group,
                        owner,
                        voice,
                        audition,
                    });
                }
                return;
            }
        }
        let mut off = matches!(op, Op::KeyOff { .. });
        let mut on = matches!(op, Op::AuditionKeyOn { .. });
        if let Op::Write {
            register, value, ..
        } = *op
        {
            if register == 0x28 {
                off = value & 0xf0 == 0;
                on = !off
            } else if (8..=10).contains(&register) {
                off = value & 31 == 0;
                on = !off
            }
        }
        if !self.additional {
            if let Some(csm_on) = csm_write {
                off = csm_before && !csm_on;
                on = csm_on;
            } else if matches!(op,Op::Write{register:0x28,value,..}if value&7==2&&value&0xf0==0)
                && csm_before
            {
                // An ordinary CH3 key-off does not stop Timer A CSM. Preserve its
                // originating lease until CSM stops or a forced release cancels it.
                off = false;
                on = false;
            }
        }
        if off
            && channel.is_some_and(|ch| {
                self.owners[ch].is_some_and(|l| {
                    owner.is_some() && l.owner != owner
                        || self.current_group.is_some() && l.group != self.current_group
                        || voice.is_some()
                            && l.voice != voice
                            && (!self.additional || audition || l.audition)
                })
            })
        {
            return;
        }
        unsafe {
            match *op {
                Op::Write {
                    port,
                    register,
                    mut value,
                } => {
                    if register == 7 {
                        for ch in 3..6 {
                            if self.protected(ch, voice, audition)
                                || declared.is_some_and(|target| target != ch)
                            {
                                let bits = (1 << (ch - 3)) | (1 << ch);
                                value = (value & !bits) | (state.registers[7] & bits)
                            }
                        }
                    }
                    if self.additional && register == 7 && !preserve {
                        for ch in 3..6 {
                            let bits = (1 << (ch - 3)) | (1 << ch);
                            let was_off = state.registers[7] & bits == bits;
                            let becomes_off = value & bits == bits;
                            if !was_off
                                && becomes_off
                                && self.owners[ch].is_some_and(|lease| {
                                    owner.is_some() && lease.owner != owner
                                        || self.current_group.is_some()
                                            && lease.group != self.current_group
                                        || voice.is_some()
                                            && lease.voice != voice
                                            && (audition || lease.audition)
                                })
                            {
                                value = (value & !bits) | (state.registers[7] & bits);
                            } else if was_off
                                && !becomes_off
                                && state.registers[8 + ch - 3] & 31 != 0
                            {
                                self.owners[ch] = Some(Lease {
                                    group: self.current_group,
                                    owner,
                                    voice,
                                    audition,
                                });
                            } else if !was_off && becomes_off {
                                self.owners[ch] = None;
                            }
                        }
                    }
                    if audition && register == 0x28 && value & 0xf0 != 0 && value & 7 < 3 {
                        native_ym2203_audition_fm_key_on(self.ptr, value & 7, value >> 4);
                    } else {
                        native_ym2203_write(self.ptr, port, register, value)
                    }
                }
                Op::KeyOff { channel } => {
                    if self.additional {
                        native_ym2203_key_off_fm(self.ptr, channel)
                    } else {
                        native_ym2203_key_off(self.ptr, channel)
                    }
                }
                Op::AuditionKeyOn { channel } => native_ym2203_audition_key_on(self.ptr, channel),
                Op::SetRetainedLevel { channel, level } => {
                    native_ym2203_set_retained_ssg_level(self.ptr, channel, level)
                }
                Op::Clock { hz } => {
                    native_ym2203_set_clock(self.ptr, hz);
                }
            }
        }
        if !preserve {
            if let Some(ch) = channel {
                if off {
                    self.owners[ch] = None
                } else if on {
                    self.owners[ch] = Some(Lease {
                        group: self.current_group,
                        owner,
                        voice,
                        audition,
                    })
                }
            }
        }
    }
    pub fn apply(&mut self, c: &Arc<Prepared>, owner: Option<Owner>, shared: &Shared) {
        if !c.audition
            && self.owners.iter().flatten().any(|l| l.audition)
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
        // Scheduling is transport state: reset cancels it even if a held
        // manual voice prevents the DSP reset itself.
        if self.additional && c.invalidates_schedule() {
            let release_token = if let Method::CancelSchedule { token } = c.method {
                Some(token)
            } else {
                c.cancel_target
                    .get()
                    .and_then(|target| target.schedule_token())
            };
            self.cancel_schedule(
                c.cancel_target.get(),
                if matches!(c.method, Method::Reset) {
                    None
                } else {
                    owner
                },
                shared,
            );
            if let Some(token) = release_token {
                self.release_scheduled_voices(
                    token,
                    if matches!(c.method, Method::Reset) {
                        None
                    } else {
                        owner
                    },
                );
            }
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
            Method::AuditionKeyOn { channel } => self.op(
                &Op::AuditionKeyOn { channel: *channel },
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
                native_ym2203_set_source_mute_mask(self.ptr, *mask)
            },
            Method::ChannelMute { mask } => unsafe { native_ym2203_set_mute_mask(self.ptr, *mask) },
            Method::Reset => {
                if c.audition || !self.owners.iter().flatten().any(|l| l.audition) {
                    unsafe { native_ym2203_reset(self.ptr) }
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
                        native_ym2203_cancel_csm(self.ptr);
                    }
                    self.csm_owner = None;
                }
            }
            Method::CancelMainOwner { target_group } => {
                let matches = |lease: Lease| !lease.audition && lease.group == Some(*target_group);
                if self.csm_owner.is_some_and(matches) {
                    unsafe {
                        native_ym2203_cancel_csm(self.ptr);
                    }
                    self.csm_owner = None;
                }
                for channel in 0..6 {
                    if self.owners[channel].is_some_and(matches) {
                        self.release(channel);
                    }
                }
            }
            Method::StopVoices if self.additional => {
                if owner.is_none()
                    && c.group.is_none()
                    && (c.audition || !self.owners.iter().flatten().any(|lease| lease.audition))
                {
                    unsafe { native_ym2203_stop(self.ptr) }
                    self.owners.fill(None);
                    self.csm_owner = None;
                } else {
                    if self.csm_owner.is_some_and(|lease| {
                        (owner.is_none() || lease.owner == owner)
                            && (c.group.is_none() || lease.group == c.group)
                            && (!lease.audition || c.audition)
                    }) {
                        unsafe {
                            native_ym2203_cancel_csm(self.ptr);
                        }
                        self.csm_owner = None;
                    }
                    for ch in 0..6 {
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
            Method::StopVoices => {
                if owner.is_none() {
                    unsafe { native_ym2203_stop(self.ptr) }
                    self.owners.fill(None)
                } else {
                    for ch in 0..6 {
                        if self.owners[ch]
                            .is_some_and(|l| l.owner == owner && (!l.audition || c.audition))
                        {
                            self.release(ch)
                        }
                    }
                }
            }
            Method::Schedule { .. } => {
                self.schedule = Some(Schedule {
                    command: c.clone(),
                    owner,
                    frame: 0,
                    index: 0,
                });
                shared.schedule_phase.store(2, Ordering::Release);
            }
            Method::CancelSchedule { .. } => {}
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
                native_ym2203_cancel_csm(self.ptr);
            }
            self.csm_owner = None;
        }
        for channel in 0..6 {
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
        if let Some(mut schedule) = self.schedule.take() {
            let mut changed = false;
            if !schedule.command.schedule_finished.load(Ordering::Acquire) {
                if let Method::Schedule {
                    entries,
                    duration_samples,
                    ..
                } = &schedule.command.method
                {
                    self.current_group = schedule.command.group;
                    let position = schedule.frame as f64 * 44100.0 / self.rate as f64;
                    while let Some(&(sample, register, value)) = entries.get(schedule.index) {
                        if sample as f64 > position {
                            break;
                        }
                        self.op(
                            &Op::Write {
                                port: 0,
                                register,
                                value,
                            },
                            schedule.owner,
                            schedule.command.schedule_token(),
                            false,
                            false,
                            None,
                        );
                        schedule.index += 1;
                        changed = true;
                    }
                    if position >= *duration_samples as f64 {
                        shared.schedule_phase.store(3, Ordering::Release);
                        schedule
                            .command
                            .schedule_finished
                            .store(true, Ordering::Release);
                        shared.retire(schedule.command);
                    } else {
                        schedule.frame += 1;
                        self.schedule = Some(schedule);
                    }
                }
            } else {
                shared.retire(schedule.command);
            }
            if changed {
                shared.publish(self, None);
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
                native_ym2203_cancel_csm(self.ptr);
            }
            self.csm_owner = None;
            changed = true;
        }
        for ch in 0..6 {
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
    pub fn stop(&mut self, s: &Shared) {
        if let Some(active) = self.schedule.take() {
            s.retire(active.command);
        }
        unsafe { native_ym2203_stop(self.ptr) }
        self.owners.fill(None);
        self.csm_owner = None;
        s.publish(self, None)
    }
    pub fn mix(&mut self, pcm: &mut [f32; 2]) {
        let (mut l, mut r) = (0., 0.);
        unsafe { native_ym2203_sample(self.ptr, &mut l, &mut r) }
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    fn prep(v: Value) -> Arc<Prepared> {
        serde_json::from_value::<Message>(v)
            .unwrap()
            .prepare()
            .unwrap()
    }
    fn setup() -> (Shared, Renderer) {
        let s = Shared::default();
        let r = Renderer::new(48000).unwrap();
        r.publish(&s);
        (s, r)
    }
    fn apply(r: &mut Renderer, s: &Shared, v: Value, owner: Option<Owner>) {
        let c = prep(v);
        s.retain(&[(c.clone(), owner)]).unwrap();
        r.apply(&c, owner, s);
        s.retire(c)
    }
    fn write(register: u8, value: u8) -> Value {
        json!({"method":"write","port":0,"register":register,"value":value})
    }
    fn lease(channel: u8, voice: u64, action: &str, commands: Vec<Value>) -> Value {
        json!({"method":"batch","channel":channel,"voice":voice,"action":action,"audition":true,"commands":commands})
    }
    fn owner(n: u32) -> Option<Owner> {
        Some(Owner {
            performance: 1,
            owner: n,
        })
    }
    fn snapshot(s: &Shared) -> Status {
        s.status(0).unwrap()
    }
    fn fm(ch: u8, mask: u8) -> Vec<Value> {
        let mut out = Vec::new();
        for slot in [0, 4, 8, 12] {
            for (reg, value) in [
                (0x30 + slot + ch, 1),
                (0x40 + slot + ch, if slot == 12 { 40 } else { 127 }),
                (0x50 + slot + ch, 31),
                (0x60 + slot + ch, 0),
                (0x70 + slot + ch, 0),
                (0x80 + slot + ch, 15),
            ] {
                out.push(write(reg, value))
            }
        }
        for (reg, val) in [
            (0xb0 + ch, 7),
            (0xa4 + ch, 0x22),
            (0xa0 + ch, 0x69),
            (0x28, (mask << 4) | ch),
        ] {
            out.push(write(reg, val))
        }
        out
    }
    fn ssg(ch: u8, level: u8) -> Vec<Value> {
        vec![
            write(ch * 2, 0x80),
            write(ch * 2 + 1, 1),
            write(7, 0x3f & !(1 << ch)),
            write(8 + ch, level),
        ]
    }
    fn start(r: &mut Renderer, s: &Shared, commands: Vec<Value>) {
        for v in commands {
            apply(r, s, v, None)
        }
    }
    fn render(r: &mut Renderer, n: usize) -> Vec<[f32; 2]> {
        (0..n)
            .map(|_| {
                let mut p = [0.; 2];
                r.mix(&mut p);
                p
            })
            .collect()
    }
    #[test]
    fn ym2203_native_abi_defaults_validation_and_admission_are_bounded_atomic() {
        assert_eq!(std::mem::size_of::<CoreState>(), 280);
        let (s, r) = setup();
        assert_eq!(snapshot(&s).clock, CLOCK);
        assert_eq!(snapshot(&s).registers.len(), 1);
        assert_eq!(snapshot(&s).registers[0].len(), 256);
        assert_eq!(snapshot(&s).prescaler, 6);
        assert_eq!(snapshot(&s).ssg_levels, [0; 3]);
        assert_eq!(r.owners.len(), 6);
        for v in [
            json!({"method":"write","port":1,"register":0,"value":0}),
            json!({"method":"key-off","channel":6}),
            json!({"method":"set-retained-level","channel":3,"level":0}),
            json!({"method":"set-retained-level","channel":0,"level":32}),
            json!({"method":"source-mute","mask":4}),
            json!({"method":"channel-mute","mask":8}),
            json!({"method":"clock","hz":99999}),
            lease(0, 1, "on", fm(1, 15)),
            lease(3, 1, "on", ssg(1, 12)),
        ] {
            assert!(serde_json::from_value::<Message>(v)
                .unwrap()
                .prepare()
                .is_err())
        }
        let c = prep(json!({"id":1,"method":"write","port":0,"register":0,"value":1}));
        s.retain(&[(c.clone(), None)]).unwrap();
        assert!(s.retain(&[(c, None)]).is_err());
        assert_eq!(snapshot(&s).pending_commands, [1]);
    }
    #[test]
    fn ym2203_native_mono_combined_pcm_matches_unclipped_fm_plus_ssg_sources() {
        let (s, mut r) = setup();
        let (f, mut fr) = setup();
        let (g, mut gr) = setup();
        for (renderer, state) in [(&mut r, &s), (&mut fr, &f), (&mut gr, &g)] {
            start(renderer, state, fm(0, 15));
            start(renderer, state, ssg(0, 3))
        }
        apply(&mut fr, &f, json!({"method":"source-mute","mask":1}), None);
        apply(&mut gr, &g, json!({"method":"source-mute","mask":2}), None);
        let a = render(&mut r, 4096);
        let b = render(&mut fr, 4096);
        let c = render(&mut gr, 4096);
        assert!(b[512..].iter().any(|p| p[0] != 0.));
        assert!(c[512..].iter().any(|p| p[0] != 0.));
        for i in 512..4096 {
            assert_eq!(a[i][0], a[i][1]);
            assert!((a[i][0] - (b[i][0] + c[i][0])).abs() < 1e-7)
        }
    }
    #[test]
    fn ym2203_native_status_is_observational_and_timers_irq_follow_generated_time() {
        let (s, mut r) = setup();
        let (t, mut q) = setup();
        for (renderer, state) in [(&mut r, &s), (&mut q, &t)] {
            start(renderer, state, fm(0, 15));
            start(renderer, state, ssg(0, 6));
            apply(renderer, state, write(0x24, 255), None);
            apply(renderer, state, write(0x25, 3), None);
            apply(renderer, state, write(0x27, 5), None)
        }
        for _ in 0..30 {
            let _ = snapshot(&s);
        }
        assert_eq!(render(&mut r, 4096), render(&mut q, 4096));
        r.publish(&s);
        let a = snapshot(&s);
        assert_ne!(a.status & 1, 0);
        assert!(a.irq);
        assert_eq!(a.key_masks, [15, 0, 0]);
    }
    #[test]
    fn ym2203_native_channel_three_special_timer_and_prescaler_writes_protect_manual_fm() {
        let (s, mut r) = setup();
        apply(&mut r, &s, write(0x27, 0x40), None);
        apply(&mut r, &s, lease(2, 1, "on", fm(2, 9)), None);
        render(&mut r, 64);
        r.publish(&s);
        assert_eq!(snapshot(&s).key_masks[2], 9);
        for (reg, val) in [
            (0xa8, 0x12),
            (0xac, 0x31),
            (0x42, 127),
            (0x27, 0),
            (0x24, 255),
            (0x2f, 0),
        ] {
            apply(&mut r, &s, write(reg, val), owner(1))
        }
        apply(&mut r, &s, write(0xa0, 0x12), owner(1));
        let a = snapshot(&s);
        assert_eq!(a.registers[0][0xa8], 0);
        assert_eq!(a.registers[0][0xac], 0);
        assert_eq!(a.registers[0][0x27], 0x40);
        assert_eq!(a.registers[0][0x24], 0);
        assert_eq!(a.registers[0][0xa0], 0x12);
        assert_eq!(a.prescaler, 6);
        apply(
            &mut r,
            &s,
            lease(2, 1, "edit", vec![write(0xa8, 0x55)]),
            None,
        );
        assert_eq!(snapshot(&s).registers[0][0xa8], 0x55);
        apply(
            &mut r,
            &s,
            lease(2, 1, "off", vec![json!({"method":"key-off","channel":2})]),
            None,
        );
        for (reg, expected) in [(0x2f, 2), (0x2d, 6), (0x2e, 3), (0x2d, 6)] {
            apply(&mut r, &s, write(reg, 0), None);
            assert_eq!(snapshot(&s).prescaler, expected)
        }
    }
    #[test]
    fn ym2203_native_ssg_shared_noise_envelope_and_mixer_preserve_held_channels() {
        let (s, mut r) = setup();
        for (reg, val) in [(6, 5), (7, 0), (11, 0x78), (12, 0x56), (13, 10)] {
            apply(&mut r, &s, write(reg, val), None)
        }
        let mut commands = ssg(0, 16);
        commands[2] = write(7, 0);
        apply(&mut r, &s, lease(3, 1, "on", commands), None);
        for (reg, val) in [
            (0, 1),
            (1, 2),
            (6, 31),
            (7, 63),
            (8, 1),
            (11, 3),
            (12, 4),
            (13, 2),
        ] {
            apply(&mut r, &s, write(reg, val), owner(1))
        }
        apply(
            &mut r,
            &s,
            json!({"method":"set-retained-level","channel":0,"level":0}),
            owner(1),
        );
        let a = snapshot(&s);
        assert_eq!(a.registers[0][0], 0x80);
        assert_eq!(a.registers[0][1], 1);
        assert_eq!(a.registers[0][6], 5);
        assert_eq!(a.registers[0][7], 54);
        assert_eq!(&a.registers[0][11..14], &[0x78, 0x56, 10]);
        assert_eq!(a.ssg_levels[0], 16);
        assert_eq!(a.held, [3]);
        apply(&mut r, &s, lease(3, 1, "edit", vec![write(13, 4)]), None);
        assert_eq!(snapshot(&s).registers[0][13], 4);
    }
    #[test]
    fn ym2203_native_ssg_explicit_zero_and_preview_metadata_survive_stop_and_key_on() {
        let (s, mut r) = setup();
        start(&mut r, &s, ssg(0, 12));
        apply(
            &mut r,
            &s,
            json!({"method":"set-retained-level","channel":0,"level":0}),
            None,
        );
        apply(&mut r, &s, write(9, 16), None);
        let before = snapshot(&s);
        r.stop(&s);
        assert_eq!(snapshot(&s).ssg_levels, [0, 16, 0]);
        assert_eq!(snapshot(&s).registers[0][7], before.registers[0][7]);
        for ch in 3..6 {
            apply(
                &mut r,
                &s,
                json!({"method":"audition-key-on","channel":ch}),
                None,
            )
        }
        assert_eq!(&snapshot(&s).registers[0][8..11], &[0, 16, 0]);
    }
    #[test]
    fn ym2203_native_stale_off_and_owner_cancel_release_only_owned_physical_voices() {
        let (s, mut r) = setup();
        apply(&mut r, &s, lease(0, 1, "on", fm(0, 15)), None);
        apply(&mut r, &s, lease(0, 2, "on", fm(0, 5)), None);
        apply(
            &mut r,
            &s,
            lease(0, 1, "off", vec![json!({"method":"key-off","channel":0})]),
            None,
        );
        start(&mut r, &s, ssg(1, 9));
        for v in fm(1, 15) {
            apply(&mut r, &s, v, owner(1))
        }
        for v in ssg(2, 8) {
            apply(&mut r, &s, v, owner(2))
        }
        r.cancel_invalid(|o| o != owner(1), &s);
        render(&mut r, 64);
        r.publish(&s);
        let a = snapshot(&s);
        assert_eq!(a.key_masks[0], 5);
        assert_eq!(a.key_masks[1], 0);
        assert_eq!(a.registers[0][9], 9);
        assert_eq!(a.registers[0][10], 8);
        assert_eq!(a.held, [0]);
    }
    #[test]
    fn ym2203_native_clock_rejection_after_admission_retains_same_hz_failure_outcome() {
        let (s, mut r) = setup();
        let clock = prep(json!({"id":77,"method":"clock","hz":CLOCK}));
        s.retain(&[(clock.clone(), None)]).unwrap();
        apply(&mut r, &s, lease(0, 1, "on", fm(0, 15)), None);
        let other = prep(json!({"id":78,"method":"clock","hz":5_000_000}));
        assert_eq!(
            s.retain(&[(other, None)]).unwrap_err(),
            "Release the manually held YM2203 voices before changing the clock"
        );
        r.apply(&clock, None, &s);
        s.retire(clock);
        assert!(snapshot(&s).pending_commands.is_empty());
        assert_eq!(
            s.wait_applied(77).unwrap_err(),
            "Release the manually held YM2203 voices before changing the clock"
        );
        assert!(s.control.lock().unwrap().rejected_clocks.is_empty());
    }
    #[test]
    fn ym2203_native_normal_keyoff_keeps_release_and_forced_off_is_immediate() {
        let (s, mut r) = setup();
        start(&mut r, &s, fm(0, 15));
        apply(&mut r, &s, write(0x8c, 0), None);
        assert!(render(&mut r, 1024).iter().any(|p| p[0] != 0.));
        apply(&mut r, &s, write(0x28, 0), None);
        assert!(render(&mut r, 256).iter().any(|p| p[0] != 0.));
        apply(&mut r, &s, json!({"method":"key-off","channel":0}), None);
        // The real core caches its FM bus for one ordinary FM interval.
        let pcm = render(&mut r, 256);
        let latency = (snapshot(&s).prescaler as usize * 12 * 48000).div_ceil(CLOCK as usize) + 2;
        assert!(pcm[latency..].iter().all(|p| *p == [0.; 2]));
    }
    #[test]
    fn ym2203_native_combined_source_mixer_pan_mute_and_stop_preserve_patch() {
        let (s, mut r) = setup();
        start(&mut r, &s, fm(0, 15));
        start(&mut r, &s, ssg(0, 6));
        apply(
            &mut r,
            &s,
            json!({"method":"mix","volume":0.5,"pan":1}),
            None,
        );
        let pcm = render(&mut r, 2048);
        assert!(pcm[512..].iter().all(|p| p[0].abs() < 1e-20));
        assert!(pcm[512..].iter().any(|p| p[1] != 0.));
        apply(&mut r, &s, json!({"method":"source-mute","mask":3}), None);
        let pcm = render(&mut r, 1024);
        let latency = (snapshot(&s).prescaler as usize * 12 * 48000).div_ceil(CLOCK as usize) + 2;
        assert!(pcm[latency..].iter().all(|p| *p == [0.; 2]));
        apply(&mut r, &s, json!({"method":"clock","hz":5_000_000}), None);
        let before = snapshot(&s);
        r.stop(&s);
        let a = snapshot(&s);
        assert_eq!(a.clock, 5_000_000);
        assert_eq!(a.source_mute_mask, 3);
        assert_eq!(a.ssg_levels, before.ssg_levels);
        assert_eq!(a.registers[0][0x4c], before.registers[0][0x4c]);
        assert_eq!(a.key_masks, [0; 3]);
        assert_eq!(&a.registers[0][8..11], &[0; 3]);
    }
    fn queued(v: Value, frame: u64) -> super::super::Command {
        serde_json::from_value(json!({"type":"ym2203","frame":frame,"message":v})).unwrap()
    }
    fn engine() -> (super::super::Shared, super::super::Renderer) {
        let s = super::super::Shared::new_modes(48000, 1, false, false, false, Some("ym2203"));
        let r =
            super::super::Renderer::new_modes(48000, false, false, false, Some("ym2203")).unwrap();
        r.publish_primary(&s);
        (s, r)
    }
    fn engine_render(
        r: &mut super::super::Renderer,
        s: &super::super::Shared,
        n: usize,
    ) -> Vec<[f32; 2]> {
        r.prepare(s);
        let out = (0..n).map(|_| r.sample(s)).collect();
        s.frame.store(r.frame, Ordering::Release);
        r.publish_primary(s);
        out
    }
    fn notes(frame: u64) -> Vec<super::super::Command> {
        fm(0, 15)
            .into_iter()
            .chain(ssg(0, 6))
            .map(|v| queued(v, frame))
            .collect()
    }
    #[test]
    fn ym2203_native_primary_rejects_foreign_ports_and_has_no_hidden_chips() {
        let (s, r) = engine();
        assert!(r.core.0.is_null());
        assert!(
            r.ym2608.is_none()
                && r.ym2610b.is_none()
                && r.msx.is_none()
                && r.opl.is_none()
                && r.master_system.is_none()
        );
        assert_eq!(s.status().mode, "pc98-26");
        assert_eq!(s.status().ym2203_clock, Some(CLOCK));
        assert_eq!(s.status().ym2608_clock, None);
        assert_eq!(s.status().psg_clock, 0);
        let bad = vec![
            queued(write(0, 3), 0),
            queued(json!({"method":"write","port":1,"register":0,"value":0}), 0),
        ];
        assert!(s.enqueue(1, &bad).is_err());
        assert_eq!(s.pending.load(Ordering::Acquire), 0);
        assert!(s.queue.is_empty());
        for mode in ["ym2608", "ym2610", "ym3812", "msx-audio", "game-gear"] {
            let other = super::super::Shared::new_modes(48000, 1, false, false, false, Some(mode));
            assert!(other.enqueue(1, &notes(0)).is_err())
        }
    }
    #[test]
    fn ym2203_native_scheduled_fm_ssg_clock_and_callback_segmentation_are_exact() {
        let (s, mut r) = engine();
        let (t, mut q) = engine();
        let mut commands = notes(100);
        commands.push(queued(
            json!({"id":77,"method":"clock","hz":5_000_000}),
            1024,
        ));
        commands.push(queued(write(0x28, 0), 2000));
        commands.push(queued(write(8, 0), 3000));
        s.enqueue(1, &commands).unwrap();
        t.enqueue(1, &commands).unwrap();
        let full = engine_render(&mut r, &s, 4096);
        let mut pieces = Vec::new();
        for n in [33, 467, 1, 999, 2596] {
            pieces.extend(engine_render(&mut q, &t, n))
        }
        assert_eq!(full, pieces);
        assert!(full[..100].iter().all(|p| *p == [0.; 2]));
        assert!(full[500..1000].iter().any(|p| p[0] != 0.));
        s.ym2203.as_ref().unwrap().wait_applied(77).unwrap();
        assert_eq!(
            s.ym2203.as_ref().unwrap().status(0).unwrap().clock,
            5_000_000
        );
    }
    #[test]
    fn ym2203_native_stop_fence_cancels_future_epoch_preserves_patch_and_can_resume() {
        let (s, mut r) = engine();
        s.enqueue(1, &notes(0)).unwrap();
        s.enqueue(1, &notes(4000)).unwrap();
        s.enqueue(1, &[queued(json!({"method":"stop-voices"}), 500)])
            .unwrap();
        let out = engine_render(&mut r, &s, 5000);
        assert!(out[..500].iter().any(|p| p[0] != 0.));
        assert!(out[600..].iter().all(|p| *p == [0.; 2]));
        assert_eq!(s.pending.load(Ordering::Acquire), 0);
        s.enqueue(1, &notes(0)).unwrap();
        assert!(engine_render(&mut r, &s, 1024).iter().any(|p| p[0] != 0.));
        s.epoch.fetch_add(1, Ordering::Release);
        let out = engine_render(&mut r, &s, 1024);
        assert!(out[600..].iter().all(|p| *p == [0.; 2]));
        assert_eq!(s.stopped_epoch.load(Ordering::Acquire), 1);
        let a = s.ym2203.as_ref().unwrap().status(0).unwrap();
        assert_eq!(a.registers[0][0x4c], 40);
        assert_eq!(a.ssg_levels[0], 6);
    }
    #[test]
    fn ym2203_native_cold_csm_without_key_register_is_owned_and_cancelled() {
        let (s, mut r) = setup();
        let mut patch = fm(2, 15);
        patch.pop();
        for v in patch {
            apply(&mut r, &s, v, owner(1))
        }
        for (reg, val) in [(0x24, 255), (0x25, 3), (0x27, 0x85)] {
            apply(&mut r, &s, write(reg, val), owner(1))
        }
        assert!(render(&mut r, 2048).iter().any(|p| p[0].abs() > 0.0001));
        assert_eq!(r.owners[2].unwrap().owner, owner(1));
        apply(&mut r, &s, write(0x28, 2), owner(1));
        assert_eq!(r.owners[2].unwrap().owner, owner(1));
        r.cancel_invalid(|o| o != owner(1), &s);
        let pcm = render(&mut r, 1024);
        assert!(pcm[8..].iter().all(|p| *p == [0.; 2]));
        assert_eq!(snapshot(&s).registers[0][0x27], 0x85);
        assert!(r.owners[2].is_none());
    }
    #[test]
    fn ym2203_native_manual_ch3_csm_timer_guard_release_and_pending_cancel() {
        let (s, mut r) = setup();
        let mut patch = fm(2, 15);
        patch.pop();
        start(&mut r, &s, patch);
        for (reg, val) in [(0x24, 255), (0x25, 3), (0x27, 0x85)] {
            apply(&mut r, &s, write(reg, val), None)
        }
        assert!(render(&mut r, 512).iter().any(|p| p[0] != 0.));
        apply(
            &mut r,
            &s,
            lease(
                2,
                1,
                "on",
                vec![
                    write(0x28, 2),
                    write(0xa6, 0x22),
                    write(0xa2, 0x69),
                    write(0x28, 0x92),
                ],
            ),
            None,
        );
        apply(&mut r, &s, write(0x27, 0), owner(1));
        assert_eq!(snapshot(&s).registers[0][0x27], 0x85);
        apply(
            &mut r,
            &s,
            lease(2, 1, "off", vec![json!({"method":"key-off","channel":2})]),
            None,
        );
        let pcm = render(&mut r, 1024);
        assert!(pcm[8..].iter().all(|p| *p == [0.; 2]));
        assert!(snapshot(&s).held.is_empty());
    }
    #[test]
    fn ym2203_native_pipeline_owner_cancel_silences_csm_and_future_fm_but_keeps_other_ssg_owner() {
        let (s, mut r) = engine();
        s.active_performance.store(1, Ordering::Release);
        let mut patch = fm(2, 15);
        patch.pop();
        patch.extend([write(0x24, 255), write(0x25, 3), write(0x27, 0x85)]);
        let tagged = |v: Value, frame: u64, owner: u32| {
            let mut c = queued(v, frame);
            c.performance = Some(1);
            c.owner = Some(owner);
            c
        };
        let mut commands: Vec<_> = patch.into_iter().map(|v| tagged(v, 0, 1)).collect();
        commands.extend(ssg(0, 6).into_iter().map(|v| tagged(v, 0, 2)));
        commands.extend(fm(0, 15).into_iter().map(|v| tagged(v, 2048, 1)));
        commands.extend(ssg(1, 9).into_iter().map(|v| tagged(v, 2048, 2)));
        s.enqueue(1, &commands).unwrap();
        engine_render(&mut r, &s, 512);
        s.cancelled_owners[0].store(true, Ordering::Release);
        s.invalidation.fetch_add(1, Ordering::Release);
        engine_render(&mut r, &s, 2500);
        let state = s.ym2203.as_ref().unwrap().status(0).unwrap();
        assert_eq!(state.key_masks, [0; 3]);
        assert_eq!(state.registers[0][0x27], 0x85);
        assert_eq!(state.registers[0][8], 6);
        assert_eq!(state.registers[0][9], 9);
        s.enqueue(1, &[queued(json!({"method":"source-mute","mask":1}), 0)])
            .unwrap();
        let pcm = engine_render(&mut r, &s, 512);
        assert!(pcm[8..].iter().all(|p| *p == [0.; 2]));
        assert_eq!(s.pending.load(Ordering::Acquire), 0);
    }
    #[test]
    fn ym2203_native_combined_generator_uses_common_master_fx_and_recording_pcm() {
        let (dry, mut dr) = engine();
        let (wet, mut wr) = engine();
        dry.enqueue(1, &notes(0)).unwrap();
        wet.enqueue(1, &notes(0)).unwrap();
        let fx:super::super::FxMessage=serde_json::from_value(json!({"op":"chain","children":[{"type":"gain","slot":0}],"units":[{"type":"gain","slot":0,"values":{"gain":0.5}}],"beatSeconds":0.5})).unwrap();
        let graph = wet.fx.lock().unwrap().command(fx, 48000).unwrap().unwrap();
        wet.enqueue_special(super::super::Action::Fx(graph))
            .unwrap();
        wet.enqueue(1,&[serde_json::from_value(json!({"type":"mix","frame":0,"fmLeft":1,"fmRight":1,"psgLeft":1,"psgRight":1,"master":0.5})).unwrap()]).unwrap();
        wet.record_start(0.02).unwrap();
        let expected = engine_render(&mut dr, &dry, 2048);
        let pcm = engine_render(&mut wr, &wet, 2048);
        assert!(expected[512..].iter().any(|p| p[0].abs() > 0.0001));
        for i in 512..2048 {
            for ch in 0..2 {
                assert!((pcm[i][ch] - expected[i][ch] * 0.25).abs() < 1e-7)
            }
        }
        let recording = wet.record_stop().unwrap();
        assert_eq!(recording.frames, 960);
        for ch in 0..2 {
            assert_eq!(
                recording.channels[ch],
                pcm[..960].iter().map(|p| p[ch]).collect::<Vec<_>>()
            )
        }
    }
}
