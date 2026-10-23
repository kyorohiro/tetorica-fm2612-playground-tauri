//! Callback-owned Master System (mono PSG + OPLL) or Game Gear (stereo PSG only). All command allocation and reclamation
//! happens on the control thread; snapshots only observe applied core state.
use super::{Owner, CAPACITY, MAX_SAFE_INTEGER, OWNERS};
use crossbeam_queue::ArrayQueue;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    ffi::c_void,
    sync::{
        atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicU64, AtomicU8, Ordering},
        Arc, Mutex,
    },
};
pub const CLOCK: u32 = 3_579_545;
const VOICES: usize = 13;
const PENDING_BYTES: usize = 4 * 1024 * 1024;
#[repr(C)]
struct CoreState {
    registers: [u8; 64],
    periods: [u16; 3],
    attenuation: [u8; 4],
    levels: [u8; 4],
    psg_clock: u32,
    opll_clock: u32,
    psg_mute_mask: u32,
    opll_mute_mask: u32,
    latch: u8,
    noise_mode: u8,
    status: u8,
    irq: u8,
}
impl Default for CoreState {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}
#[repr(C)]
#[derive(Default)]
struct GameGearCoreState {
    periods: [u16; 3],
    attenuation: [u8; 4],
    levels: [u8; 4],
    clock: u32,
    mute_mask: u32,
    latch: u8,
    noise_mode: u8,
    stereo: u8,
}
extern "C" {
    fn native_game_gear_create(rate: u32) -> *mut c_void;
    fn native_game_gear_destroy(ptr: *mut c_void);
    fn native_game_gear_reset(ptr: *mut c_void);
    fn native_game_gear_stop(ptr: *mut c_void);
    fn native_game_gear_write(ptr: *mut c_void, value: u8);
    fn native_game_gear_key_off(ptr: *mut c_void, ch: u8);
    fn native_game_gear_audition_key_on(ptr: *mut c_void, ch: u8);
    fn native_game_gear_sample(ptr: *mut c_void, left: *mut f32, right: *mut f32);
    fn native_game_gear_set_clock(ptr: *mut c_void, hz: u32) -> i32;
    fn native_game_gear_set_mute_mask(ptr: *mut c_void, mask: u32);
    fn native_game_gear_set_stereo(ptr: *mut c_void, mask: u8);
    fn native_game_gear_get_state(ptr: *mut c_void, state: *mut GameGearCoreState);

    fn native_master_system_create(rate: u32) -> *mut c_void;
    fn native_master_system_destroy(ptr: *mut c_void);
    fn native_master_system_reset(ptr: *mut c_void, chip: u8);
    fn native_master_system_stop(ptr: *mut c_void, chip: u8);
    fn native_master_system_key_off(ptr: *mut c_void, voice: u8);
    fn native_master_system_audition_key_on(ptr: *mut c_void, voice: u8);
    fn native_master_system_write(ptr: *mut c_void, chip: u8, register: u8, value: u8);
    fn native_master_system_sample(
        ptr: *mut c_void,
        pl: *mut f32,
        pr: *mut f32,
        ol: *mut f32,
        or: *mut f32,
    );
    fn native_master_system_set_clock(ptr: *mut c_void, chip: u8, hz: u32) -> i32;
    fn native_master_system_set_mute_mask(ptr: *mut c_void, chip: u8, mask: u32);
    fn native_master_system_get_state(ptr: *mut c_void, state: *mut CoreState);
}
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Chip {
    Segapsg,
    Ym2413,
}
impl Chip {
    fn code(self) -> u8 {
        if self == Self::Segapsg {
            0
        } else {
            1
        }
    }
    fn channels(self) -> u8 {
        if self == Self::Segapsg {
            4
        } else {
            14
        }
    }
    fn physical(self, ch: u8) -> usize {
        if self == Self::Segapsg {
            ch as usize
        } else {
            4 + if ch < 9 {
                ch as usize
            } else {
                [6, 7, 8, 8, 7][(ch - 9) as usize]
            }
        }
    }
    fn voice(self, ch: u8) -> u8 {
        if self == Self::Segapsg {
            ch
        } else {
            4 + ch
        }
    }
    fn range(self) -> std::ops::Range<usize> {
        if self == Self::Segapsg {
            0..4
        } else {
            4..13
        }
    }
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
        #[serde(default)]
        register: u8,
        value: u8,
        #[serde(default, rename = "psgRegister")]
        psg_register: Option<u8>,
    },
    KeyOff {
        channel: u8,
    },
    AuditionKeyOn {
        channel: u8,
    },
    Clock {
        hz: u32,
    },
    Stereo {
        mask: u8,
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
        #[serde(default)]
        register: u8,
        value: u8,
        #[serde(default, rename = "psgRegister")]
        psg_register: Option<u8>,
    },
    KeyOff {
        channel: u8,
    },
    AuditionKeyOn {
        channel: u8,
    },
    Clock {
        hz: u32,
    },
    Stereo {
        mask: u8,
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
    clock_rejected: AtomicBool,
    outcome_observed: AtomicBool,
}
fn valid_id(id: Option<u64>) -> bool {
    id.is_none_or(|v| v > 0 && v <= MAX_SAFE_INTEGER)
}
fn declared_channel(chip: Chip, op: &Op) -> Option<u8> {
    match *op {
        Op::KeyOff { channel } | Op::AuditionKeyOn { channel } => Some(channel),
        Op::Write {
            value,
            psg_register,
            ..
        } if chip == Chip::Segapsg => {
            if value & 128 != 0 {
                Some((value >> 5) & 3)
            } else {
                psg_register.map(|r| r >> 1)
            }
        }
        Op::Write { register, .. }
            if chip == Chip::Ym2413 && matches!(register,0x10..=0x18|0x20..=0x28|0x30..=0x38) =>
        {
            Some(register & 15)
        }
        _ => None,
    }
}
fn validate_op(op: &Op, chip: Chip) -> Result<(), String> {
    match *op {
        Op::Stereo { .. } if chip != Chip::Segapsg => {
            Err("Stereo routing requires Sega PSG".into())
        }
        Op::KeyOff { channel } | Op::AuditionKeyOn { channel } if channel >= chip.channels() => {
            Err("Invalid Native Master System channel".into())
        }
        Op::Clock { hz } if !(100_000..=100_000_000).contains(&hz) => {
            Err("Native Master System clock must be 100000–100000000 Hz".into())
        }
        Op::Write {
            register,
            psg_register,
            ..
        } if chip == Chip::Ym2413 && (register >= 64 || psg_register.is_some()) => {
            Err("Invalid Native YM2413 register".into())
        }
        Op::Write {
            psg_register: Some(r),
            ..
        } if chip == Chip::Segapsg && r > 7 => Err("Invalid Native Sega PSG latch register".into()),
        _ => Ok(()),
    }
}
impl Message {
    pub fn prepared_bytes(&self) -> usize {
        std::mem::size_of::<Prepared>()
            + match &self.method {
                Method::Batch { commands, .. } => commands.len() * std::mem::size_of::<Op>(),
                _ => 0,
            }
    }

    pub fn prepare(&self) -> Result<Arc<Prepared>, String> {
        if !valid_id(self.id) {
            return Err("Invalid Native Master System command ID".into());
        }
        match &self.method {
            Method::Write {
                register,
                value,
                psg_register,
            } => validate_op(
                &Op::Write {
                    register: *register,
                    value: *value,
                    psg_register: *psg_register,
                },
                self.chip,
            )?,
            Method::KeyOff { channel } | Method::AuditionKeyOn { channel } => {
                validate_op(&Op::KeyOff { channel: *channel }, self.chip)?
            }
            Method::Clock { hz } => validate_op(&Op::Clock { hz: *hz }, self.chip)?,
            Method::Stereo { mask } => validate_op(&Op::Stereo { mask: *mask }, self.chip)?,
            Method::Mix { volume, pan }
                if !volume.is_finite()
                    || !(0.0..=4.0).contains(volume)
                    || !pan.is_finite()
                    || !(-1.0..=1.0).contains(pan) =>
            {
                return Err("Native Master System mixer volume must be 0–4, pan −1–1".into())
            }
            Method::SourceMute { mask }
                if *mask > if self.chip == Chip::Segapsg { 15 } else { 511 } =>
            {
                return Err("Invalid Native Master System source mute mask".into())
            }
            Method::Batch {
                commands,
                channel,
                voice,
                action,
            } => {
                if *channel >= self.chip.channels()
                    || commands.is_empty()
                    || commands.len() > 512
                    || !valid_id(*voice)
                    || self.audition && *action != LeaseAction::Edit && voice.is_none()
                {
                    return Err("Invalid Native Master System batch or voice lease".into());
                }
                let mut psg_latch = None;
                for op in commands {
                    validate_op(op, self.chip)?;
                    let target = if self.chip == Chip::Segapsg {
                        match *op {
                            Op::Write {
                                value,
                                psg_register,
                                ..
                            } => {
                                let register = if value & 128 != 0 {
                                    (value >> 4) & 7
                                } else {
                                    psg_register.or(psg_latch).ok_or(
                                        "Native Sega PSG batch continuations require a latch register",
                                    )?
                                };
                                psg_latch = Some(register);
                                Some(register >> 1)
                            }
                            Op::KeyOff { channel } | Op::AuditionKeyOn { channel } => {
                                psg_latch = Some(channel * 2 + 1);
                                Some(channel)
                            }
                            _ => None,
                        }
                    } else {
                        declared_channel(self.chip, op)
                    };
                    if target
                        .is_some_and(|ch| self.chip.physical(ch) != self.chip.physical(*channel))
                    {
                        return Err(
                            "Native Master System batch must target its declared physical voice"
                                .into(),
                        );
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
            clock_rejected: AtomicBool::new(false),
            outcome_observed: AtomicBool::new(false),
        }))
    }
}
impl Prepared {
    pub fn starts(&self) -> bool {
        match &self.method {
            Method::KeyOff { .. }
            | Method::Clock { .. }
            | Method::Stereo { .. }
            | Method::Mix { .. }
            | Method::SourceMute { .. }
            | Method::Reset
            | Method::StopVoices => false,
            Method::Batch { action, .. } => {
                *action == LeaseAction::On || *action == LeaseAction::Edit
            }
            _ => true,
        }
    }
    pub fn stops(&self) -> bool {
        matches!(self.method, Method::StopVoices)
    }
    fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + if let Method::Batch { commands, .. } = &self.method {
                commands.len() * std::mem::size_of::<Op>()
            } else {
                0
            }
    }
    pub fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + if let Method::Batch { commands, .. } = &self.method {
                commands.capacity() * std::mem::size_of::<Op>()
            } else {
                0
            }
    }
}
#[derive(Default)]
struct Control {
    prepared: Vec<Arc<Prepared>>,
    rejected_clocks: HashSet<u64>,
}
pub struct Shared {
    game_gear: bool,
    stereo: AtomicU8,
    revision: AtomicU64,
    registers: [AtomicU8; 64],
    periods: [AtomicU16; 3],
    attenuation: [AtomicU8; 4],
    levels: [AtomicU8; 4],
    clock: [AtomicU32; 2],
    mute: [AtomicU32; 2],
    latch: AtomicU8,
    noise_mode: AtomicU8,
    status: AtomicU8,
    irq: AtomicU8,
    held: AtomicU32,
    rate: AtomicU32,
    control: Mutex<Control>,
    retired: ArrayQueue<Arc<Prepared>>,
    highest_id: AtomicU64,
    cancel_before: [AtomicU64; 2],
    owner_cancel_before: [[AtomicU64; OWNERS]; 2],
}
impl Default for Shared {
    fn default() -> Self {
        Self {
            game_gear: false,
            stereo: AtomicU8::new(255),
            revision: AtomicU64::new(0),
            registers: std::array::from_fn(|_| AtomicU8::new(0)),
            periods: std::array::from_fn(|_| AtomicU16::new(0)),
            attenuation: std::array::from_fn(|_| AtomicU8::new(15)),
            levels: std::array::from_fn(|_| AtomicU8::new(15)),
            clock: std::array::from_fn(|_| AtomicU32::new(CLOCK)),
            mute: std::array::from_fn(|_| AtomicU32::new(0)),
            latch: AtomicU8::new(0),
            noise_mode: AtomicU8::new(0),
            status: AtomicU8::new(0),
            irq: AtomicU8::new(0),
            held: AtomicU32::new(0),
            rate: AtomicU32::new(0),
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
pub struct PsgStatus {
    pub stereo: u8,
    pub clock: u32,
    pub periods: Vec<u16>,
    pub attenuations: Vec<u8>,
    pub last_volumes: Vec<u8>,
    pub latched: u8,
    pub mode: u8,
    pub source_mute_mask: u32,
    pub held: Vec<u8>,
    pub pending_commands: Vec<u64>,
    pub sample_rate: u32,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpllStatus {
    pub clock: u32,
    pub registers: Vec<u8>,
    pub status: u8,
    pub irq: bool,
    pub source_mute_mask: u32,
    pub held: Vec<u8>,
    pub pending_commands: Vec<u64>,
    pub sample_rate: u32,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub segapsg: PsgStatus,
    pub ym2413: OpllStatus,
    pub sample_rate: u32,
    pub frame: u64,
    pub revision: u64,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameGearStatus {
    pub segapsg: PsgStatus,
    pub sample_rate: u32,
    pub frame: u64,
    pub revision: u64,
}
impl Shared {
    pub fn game_gear() -> Self {
        Self {
            game_gear: true,
            ..Self::default()
        }
    }
    pub fn is_game_gear(&self) -> bool {
        self.game_gear
    }
    pub fn mode(&self) -> &'static str {
        if self.game_gear {
            "game-gear"
        } else {
            "master-system"
        }
    }
    pub fn accepts(&self, command: &Prepared) -> bool {
        if self.game_gear {
            command.chip == Chip::Segapsg
        } else {
            !matches!(command.method, Method::Stereo { .. })
                && !matches!(&command.method,Method::Batch{commands,..} if commands.iter().any(|op|matches!(op,Op::Stereo{..})))
        }
    }
    pub fn game_gear_status(&self, frame: u64) -> Result<GameGearStatus, String> {
        if !self.game_gear {
            return Err("Native Game Gear status requires Game Gear mode".into());
        }
        let state = self.status(frame)?;
        Ok(GameGearStatus {
            segapsg: state.segapsg,
            sample_rate: state.sample_rate,
            frame: state.frame,
            revision: state.revision,
        })
    }
    pub fn clock(&self) -> u32 {
        self.clock[0].load(Ordering::Acquire)
    }
    pub fn opll_clock(&self) -> u32 {
        self.clock[1].load(Ordering::Acquire)
    }
    pub fn valid_event(&self, command: &Prepared, sequence: u64, owner: Option<Owner>) -> bool {
        let chip = command.chip.code() as usize;
        sequence >= self.cancel_before[chip].load(Ordering::Acquire)
            && owner.is_none_or(|o| {
                sequence
                    >= self.owner_cancel_before[chip][(o.owner - 1) as usize]
                        .load(Ordering::Acquire)
            })
    }
    pub fn cancel_before(&self, command: &Prepared, sequence: u64, owner: Option<Owner>) {
        let chip = command.chip.code() as usize;
        if let Some(o) = owner {
            self.owner_cancel_before[chip][(o.owner - 1) as usize]
                .fetch_max(sequence, Ordering::AcqRel);
        } else {
            self.cancel_before[chip].fetch_max(sequence, Ordering::AcqRel);
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
            .map_err(|_| "Native Master System control state unavailable")?;
        self.collect(&mut c);
        if commands.iter().any(|(command, _)| !self.accepts(command)) {
            return Err("Native PSG command is unavailable in the selected machine mode".into());
        }
        if self.game_gear
            && self.held.load(Ordering::Acquire) & 15 != 0
            && commands.iter().any(|(cmd, _)| {
                !cmd.audition
                    && match &cmd.method {
                        Method::Clock { .. } => true,
                        Method::Batch { commands, .. } => {
                            commands.iter().any(|op| matches!(op, Op::Clock { .. }))
                        }
                        _ => false,
                    }
            })
        {
            return Err(
                "Release the manually held Game Gear voices before changing the clock".into(),
            );
        }
        if c.prepared.len() + c.rejected_clocks.len() + commands.len() > CAPACITY
            || c.prepared.iter().map(|v| v.bytes()).sum::<usize>()
                + commands.iter().map(|(v, _)| v.bytes()).sum::<usize>()
                > PENDING_BYTES
        {
            return Err("Native Master System pending command limit reached (4 MiB)".into());
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
                return Err("Duplicate pending Native Master System command ID".into());
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
            return Err("Unknown Native Master System command ID".into());
        }
        let cmd = {
            let mut control = self
                .control
                .lock()
                .map_err(|_| "Native PSG control state unavailable")?;
            if control.rejected_clocks.remove(&id) {
                return Err(
                    "Release the manually held Game Gear voices before changing the clock".into(),
                );
            }
            control.prepared.iter().find(|v| v.id == Some(id)).cloned()
        };
        if let Some(cmd) = cmd {
            let start = std::time::Instant::now();
            while !cmd.complete.load(Ordering::Acquire) {
                let deadline = if self.game_gear && matches!(cmd.method, Method::Clock { .. }) {
                    5000
                } else {
                    100
                };
                if start.elapsed() > std::time::Duration::from_millis(deadline) {
                    return Err(format!(
                        "Native PSG command was not applied within {deadline} ms"
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            cmd.outcome_observed.store(true, Ordering::Release);
            if cmd.clock_rejected.load(Ordering::Acquire) {
                self.control
                    .lock()
                    .map_err(|_| "Native PSG control state unavailable")?
                    .rejected_clocks
                    .remove(&id);
                return Err(
                    "Release the manually held Game Gear voices before changing the clock".into(),
                );
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
            .unwrap_or_else(|_| unreachable!("Bounded Master System command retirement"));
    }
    fn publish(&self, r: &Renderer, command: Option<&Prepared>) {
        let s = r.state();
        self.revision.fetch_add(1, Ordering::AcqRel);
        for (o, v) in self.registers.iter().zip(s.registers) {
            o.store(v, Ordering::Relaxed)
        }
        for (o, v) in self.periods.iter().zip(s.periods) {
            o.store(v, Ordering::Relaxed)
        }
        for (o, v) in self.attenuation.iter().zip(s.attenuation) {
            o.store(v, Ordering::Relaxed)
        }
        for (o, v) in self.levels.iter().zip(s.levels) {
            o.store(v, Ordering::Relaxed)
        }
        self.clock[0].store(s.psg_clock, Ordering::Relaxed);
        self.clock[1].store(s.opll_clock, Ordering::Relaxed);
        self.mute[0].store(s.psg_mute_mask, Ordering::Relaxed);
        self.mute[1].store(s.opll_mute_mask, Ordering::Relaxed);
        self.latch.store(s.latch, Ordering::Relaxed);
        self.noise_mode.store(s.noise_mode, Ordering::Relaxed);
        self.stereo.store(r.stereo(), Ordering::Relaxed);
        self.status.store(s.status, Ordering::Relaxed);
        self.irq.store(s.irq, Ordering::Relaxed);
        self.held.store(
            r.owners
                .iter()
                .flatten()
                .filter(|l| l.audition)
                .fold(0, |bits, l| bits | (1 << l.wire_voice())),
            Ordering::Relaxed,
        );
        self.rate.store(r.rate, Ordering::Relaxed);
        if let Some(cmd) = command {
            cmd.complete.store(true, Ordering::Relaxed)
        }
        self.revision.fetch_add(1, Ordering::Release);
    }
    pub fn status(&self, frame: u64) -> Result<Status, String> {
        let mut c = self
            .control
            .lock()
            .map_err(|_| "Native Master System control state unavailable")?;
        self.collect(&mut c);
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() > std::time::Duration::from_millis(50) {
                return Err("Native Master System state changed while reading; retry".into());
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
            let out = Status {
                segapsg: PsgStatus {
                    stereo: self.stereo.load(Ordering::Relaxed),
                    clock: self.clock(),
                    periods: self
                        .periods
                        .iter()
                        .map(|v| v.load(Ordering::Relaxed))
                        .collect(),
                    attenuations: self
                        .attenuation
                        .iter()
                        .map(|v| v.load(Ordering::Relaxed))
                        .collect(),
                    last_volumes: self
                        .levels
                        .iter()
                        .map(|v| v.load(Ordering::Relaxed))
                        .collect(),
                    latched: self.latch.load(Ordering::Relaxed),
                    mode: self.noise_mode.load(Ordering::Relaxed),
                    source_mute_mask: self.mute[0].load(Ordering::Relaxed),
                    held: (0..4).filter(|ch| held & (1 << ch) != 0).collect(),
                    pending_commands: pending(Chip::Segapsg),
                    sample_rate: rate,
                },
                ym2413: OpllStatus {
                    clock: self.opll_clock(),
                    registers: self
                        .registers
                        .iter()
                        .map(|v| v.load(Ordering::Relaxed))
                        .collect(),
                    status: self.status.load(Ordering::Relaxed),
                    irq: self.irq.load(Ordering::Relaxed) != 0,
                    source_mute_mask: self.mute[1].load(Ordering::Relaxed),
                    held: (0..14).filter(|ch| held & (1 << (4 + ch)) != 0).collect(),
                    pending_commands: pending(Chip::Ym2413),
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
    chip: Chip,
    channel: u8,
}
impl Lease {
    fn wire_voice(self) -> u8 {
        self.chip.voice(self.channel)
    }
}
pub struct Renderer {
    game_gear: bool,
    ptr: *mut c_void,
    rate: u32,
    owners: [Option<Lease>; VOICES],
    gains: [[f32; 2]; 2],
    targets: [[f32; 2]; 2],
    ramps: [u32; 2],
}
unsafe impl Send for Renderer {}
impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe {
            if self.game_gear {
                native_game_gear_destroy(self.ptr)
            } else {
                native_master_system_destroy(self.ptr)
            }
        }
    }
}
impl Renderer {
    pub fn new(rate: u32) -> Result<Self, String> {
        Self::new_profile(rate, false)
    }
    pub fn new_game_gear(rate: u32) -> Result<Self, String> {
        Self::new_profile(rate, true)
    }
    fn new_profile(rate: u32, game_gear: bool) -> Result<Self, String> {
        let ptr = unsafe {
            if game_gear {
                native_game_gear_create(rate)
            } else {
                native_master_system_create(rate)
            }
        };
        if ptr.is_null() {
            return Err("Could not allocate Native Master System core".into());
        }
        Ok(Self {
            game_gear,
            ptr,
            rate,
            owners: [None; VOICES],
            gains: [[1.; 2]; 2],
            targets: [[1.; 2]; 2],
            ramps: [0; 2],
        })
    }
    fn state(&self) -> CoreState {
        let mut s = CoreState::default();
        unsafe {
            if self.game_gear {
                let mut p = GameGearCoreState::default();
                native_game_gear_get_state(self.ptr, &mut p);
                s.periods = p.periods;
                s.attenuation = p.attenuation;
                s.levels = p.levels;
                s.psg_clock = p.clock;
                s.psg_mute_mask = p.mute_mask;
                s.latch = p.latch;
                s.noise_mode = p.noise_mode;
            } else {
                native_master_system_get_state(self.ptr, &mut s)
            }
        }
        s
    }
    fn stereo(&self) -> u8 {
        if !self.game_gear {
            return 255;
        }
        let mut s = GameGearCoreState::default();
        unsafe { native_game_gear_get_state(self.ptr, &mut s) }
        s.stereo
    }
    unsafe fn core_write(&mut self, chip: u8, register: u8, value: u8) {
        if self.game_gear {
            debug_assert_eq!(chip, 0);
            native_game_gear_write(self.ptr, value)
        } else {
            native_master_system_write(self.ptr, chip, register, value)
        }
    }
    unsafe fn core_off(&mut self, voice: u8) {
        if self.game_gear {
            native_game_gear_key_off(self.ptr, voice)
        } else {
            native_master_system_key_off(self.ptr, voice)
        }
    }
    unsafe fn core_on(&mut self, voice: u8) {
        if self.game_gear {
            native_game_gear_audition_key_on(self.ptr, voice)
        } else {
            native_master_system_audition_key_on(self.ptr, voice)
        }
    }
    unsafe fn core_clock(&mut self, chip: u8, hz: u32) {
        if self.game_gear {
            debug_assert_eq!(chip, 0);
            native_game_gear_set_clock(self.ptr, hz);
        } else {
            native_master_system_set_clock(self.ptr, chip, hz);
        }
    }
    unsafe fn core_reset(&mut self, chip: u8) {
        if self.game_gear {
            debug_assert_eq!(chip, 0);
            native_game_gear_reset(self.ptr)
        } else {
            native_master_system_reset(self.ptr, chip)
        }
    }
    unsafe fn core_stop(&mut self, chip: u8) {
        if self.game_gear {
            native_game_gear_stop(self.ptr)
        } else {
            native_master_system_stop(self.ptr, chip)
        }
    }
    unsafe fn core_mute(&mut self, chip: u8, mask: u32) {
        if self.game_gear {
            debug_assert_eq!(chip, 0);
            native_game_gear_set_mute_mask(self.ptr, mask)
        } else {
            native_master_system_set_mute_mask(self.ptr, chip, mask)
        }
    }

    fn protected(&self, ch: usize, voice: Option<u64>, audition: bool) -> bool {
        self.owners[ch]
            .is_some_and(|l| l.audition && (!audition || voice.is_some() && l.voice != voice))
    }
    fn release(&mut self, ch: usize) {
        if let Some(l) = self.owners[ch] {
            unsafe { self.core_off(l.wire_voice()) }
            if ch >= 10 {
                // Rhythm siblings share a physical OPLL channel. Cancellation
                // releases the whole leased oscillator, including pending gates.
                let melodic = (ch - 4) as u8;
                unsafe {
                    self.core_off(4 + melodic);
                    for drum in 0..5 {
                        if Chip::Ym2413.physical(9 + drum) == ch {
                            self.core_off(13 + drum);
                        }
                    }
                }
            }
        }
        self.owners[ch] = None;
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
        let state = self.state();
        let ch = match op {
            Op::Write {
                value,
                psg_register,
                ..
            } if chip == Chip::Segapsg => Some(if value & 128 != 0 {
                (value >> 5) & 3
            } else {
                psg_register.unwrap_or(state.latch) >> 1
            }),
            _ => declared_channel(chip, op),
        };
        let target = ch.map(|ch| chip.physical(ch));
        if target.is_some_and(|p| self.protected(p, voice, audition)) {
            return;
        }
        if !audition {
            if matches!(op, Op::Clock { .. })
                && chip.range().any(|p| self.protected(p, voice, false))
            {
                return;
            }
            if chip == Chip::Segapsg
                && target == Some(2)
                && state.noise_mode & 3 == 3
                && self.protected(3, voice, false)
            {
                return;
            }
            if chip == Chip::Ym2413
                && matches!(
                    op,
                    Op::Write {
                        register: 0..=7,
                        ..
                    }
                )
                && (4..13).any(|p| self.protected(p, voice, false))
            {
                return;
            }
        }
        let mut off = matches!(op, Op::KeyOff { .. });
        let mut on = matches!(op, Op::AuditionKeyOn { .. });
        if let Op::Write {
            register, value, ..
        } = *op
        {
            if chip == Chip::Segapsg {
                let r = if value & 128 != 0 {
                    (value >> 4) & 7
                } else {
                    if let Op::Write { psg_register, .. } = op {
                        psg_register.unwrap_or(state.latch)
                    } else {
                        state.latch
                    }
                };
                if r & 1 != 0 {
                    off = value & 15 == 15;
                    on = !off;
                }
            } else if (0x20..=0x28).contains(&register) {
                off = value & 16 == 0;
                on = !off;
            }
        }
        if off
            && target.is_some_and(|p| {
                self.owners[p].is_some_and(|l| {
                    owner.is_some() && l.owner != owner || voice.is_some() && l.voice != voice
                })
            })
        {
            return;
        }
        unsafe {
            match *op {
                Op::Write {
                    register,
                    mut value,
                    psg_register,
                } => {
                    if chip == Chip::Segapsg {
                        if value & 128 == 0 {
                            if let Some(r) = psg_register {
                                if r != state.latch {
                                    let low = if r & 1 != 0 {
                                        value & 15
                                    } else if r == 6 {
                                        state.noise_mode
                                    } else {
                                        state.periods[(r >> 1) as usize] as u8 & 15
                                    };
                                    self.core_write(0, 0, 128 | (r << 4) | low);
                                }
                            }
                        }
                        self.core_write(0, 0, value);
                    } else {
                        if register == 14 {
                            if !audition
                                && (value ^ state.registers[14]) & 32 != 0
                                && (4..13).any(|p| self.protected(p, voice, false))
                            {
                                return;
                            }
                            // The five drums share three physical channels; never alter a held sibling gate.
                            for drum in 0..5 {
                                let logical = 9 + drum;
                                let p = chip.physical(logical);
                                let bit = [16, 8, 4, 2, 1][drum as usize];
                                let stale = self.owners[p].is_some_and(|l| {
                                    l.channel == logical && owner.is_some() && l.owner != owner
                                });
                                if self.protected(p, voice, audition) || value & bit == 0 && stale {
                                    value = (value & !bit) | (state.registers[14] & bit);
                                }
                            }
                            self.core_write(1, register, value);
                            if !preserve {
                                for drum in 0..5 {
                                    let logical = 9 + drum;
                                    let p = chip.physical(logical);
                                    let bit = [16, 8, 4, 2, 1][drum as usize];
                                    if value & bit != 0 && state.registers[14] & bit == 0 {
                                        self.owners[p] = Some(Lease {
                                            owner,
                                            voice,
                                            audition,
                                            chip,
                                            channel: logical,
                                        })
                                    } else if value & bit == 0
                                        && state.registers[14] & bit != 0
                                        && self.owners[p].is_some_and(|l| l.channel == logical)
                                    {
                                        self.owners[p] = None;
                                    }
                                }
                            }
                        } else {
                            self.core_write(1, register, value);
                        }
                    }
                }
                Op::KeyOff { channel } => self.core_off(chip.voice(channel)),
                Op::AuditionKeyOn { channel } => self.core_on(chip.voice(channel)),
                Op::Clock { hz } => {
                    self.core_clock(chip.code(), hz);
                }
                Op::Stereo { mut mask } => {
                    if self.game_gear {
                        if !audition {
                            let before = self.stereo();
                            for ch in 0..4 {
                                if self.protected(ch, voice, false) {
                                    let bits = (1 << ch) | (1 << (ch + 4));
                                    mask = (mask & !bits) | (before & bits)
                                }
                            }
                        }
                        native_game_gear_set_stereo(self.ptr, mask);
                    }
                }
            }
        }
        if !preserve {
            if let (Some(ch), Some(p)) = (ch, target) {
                if off {
                    self.owners[p] = None
                } else if on {
                    self.owners[p] = Some(Lease {
                        owner,
                        voice,
                        audition,
                        chip,
                        channel: ch,
                    })
                }
            }
        }
    }
    pub fn apply(&mut self, cmd: &Prepared, owner: Option<Owner>, shared: &Shared) {
        let chip = cmd.chip;
        if !shared.accepts(cmd) {
            return;
        }
        if self.game_gear
            && !cmd.audition
            && self.owners[..4].iter().flatten().any(|l| l.audition)
            && match &cmd.method {
                Method::Clock { .. } => true,
                Method::Batch { commands, .. } => {
                    commands.iter().any(|op| matches!(op, Op::Clock { .. }))
                }
                _ => false,
            }
        {
            cmd.clock_rejected.store(true, Ordering::Release);
            shared.publish(self, Some(cmd));
            return;
        }
        match &cmd.method {
            Method::Write {
                register,
                value,
                psg_register,
            } => self.op(
                chip,
                &Op::Write {
                    register: *register,
                    value: *value,
                    psg_register: *psg_register,
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
            Method::AuditionKeyOn { channel } => self.op(
                chip,
                &Op::AuditionKeyOn { channel: *channel },
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
            Method::Stereo { mask } => self.op(
                chip,
                &Op::Stereo { mask: *mask },
                owner,
                None,
                cmd.audition,
                false,
            ),
            Method::Mix { volume, pan } => {
                let i = chip.code() as usize;
                self.targets[i] = [
                    *volume * if *pan > 0. { 1. - *pan } else { 1. },
                    *volume * if *pan < 0. { 1. + *pan } else { 1. },
                ];
                self.ramps[i] = (self.rate / 200).max(1);
            }
            Method::SourceMute { mask } => unsafe { self.core_mute(chip.code(), *mask) },
            Method::Reset => {
                if cmd.audition || !chip.range().any(|p| self.protected(p, None, false)) {
                    unsafe { self.core_reset(chip.code()) }
                    for p in chip.range() {
                        self.owners[p] = None
                    }
                }
            }
            Method::StopVoices => {
                if owner.is_none() {
                    unsafe { self.core_stop(chip.code()) }
                    for p in chip.range() {
                        self.owners[p] = None
                    }
                } else {
                    for p in chip.range() {
                        if self.owners[p]
                            .is_some_and(|l| l.owner == owner && (!l.audition || cmd.audition))
                        {
                            self.release(p)
                        }
                    }
                }
            }
            Method::Batch {
                commands,
                channel,
                voice,
                action,
            } => {
                let p = chip.physical(*channel);
                let current = self.owners[p];
                let allowed = match action {
                    LeaseAction::On => cmd.audition || current.is_none_or(|l| !l.audition),
                    LeaseAction::Off => current.is_some_and(|l| {
                        l.channel == *channel
                            && l.owner == owner
                            && (voice.is_none() || l.voice == *voice)
                            && (!l.audition || cmd.audition)
                    }),
                    LeaseAction::Edit => current.is_none_or(|l| {
                        !l.audition || cmd.audition && (voice.is_none() || l.voice == *voice)
                    }),
                };
                if allowed {
                    if *action == LeaseAction::On {
                        if current.is_some_and(|l| l.channel != *channel) {
                            self.release(p)
                        }
                        self.owners[p] = Some(Lease {
                            owner,
                            voice: *voice,
                            audition: cmd.audition,
                            chip,
                            channel: *channel,
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
                        )
                    }
                    if *action == LeaseAction::On {
                        self.owners[p] = Some(Lease {
                            owner,
                            voice: *voice,
                            audition: cmd.audition,
                            chip,
                            channel: *channel,
                        })
                    } else if *action == LeaseAction::Off {
                        self.owners[p] = None;
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
        for p in 0..VOICES {
            if self.owners[p].is_some_and(|l| !valid(l.owner)) {
                self.release(p);
                changed = true
            }
        }
        if changed {
            shared.publish(self, None)
        }
    }
    pub fn stop(&mut self, shared: &Shared) {
        unsafe { self.core_stop(2) }
        self.owners.fill(None);
        shared.publish(self, None)
    }
    pub fn mix(&mut self, pcm: &mut [f32; 2]) {
        let (mut pl, mut pr, mut ol, mut or) = (0., 0., 0., 0.);
        unsafe {
            if self.game_gear {
                native_game_gear_sample(self.ptr, &mut pl, &mut pr)
            } else {
                native_master_system_sample(self.ptr, &mut pl, &mut pr, &mut ol, &mut or)
            }
        }
        for i in 0..2 {
            if self.ramps[i] > 0 {
                for ch in 0..2 {
                    self.gains[i][ch] +=
                        (self.targets[i][ch] - self.gains[i][ch]) / self.ramps[i] as f32;
                }
                self.ramps[i] -= 1;
            }
        }
        pcm[0] += pl * self.gains[0][0] + ol * self.gains[1][0];
        pcm[1] += pr * self.gains[0][1] + or * self.gains[1][1];
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
    fn apply(r: &mut Renderer, s: &Shared, v: Value, owner: Option<Owner>) {
        let c = prep(v);
        s.retain(&[(c.clone(), owner)]).unwrap();
        r.apply(&c, owner, s);
        s.retire(c)
    }
    fn write(chip: &str, register: u8, value: u8) -> Value {
        json!({"chip":chip,"method":"write","register":register,"value":value})
    }
    fn lease(chip: &str, channel: u8, voice: u64, action: &str, commands: Vec<Value>) -> Value {
        json!({"chip":chip,"method":"batch","channel":channel,"voice":voice,"action":action,"audition":true,"commands":commands})
    }
    fn owner(n: u32) -> Option<Owner> {
        Some(Owner {
            performance: 1,
            owner: n,
        })
    }
    fn setup() -> (Shared, Renderer) {
        let s = Shared::default();
        let r = Renderer::new(48000).unwrap();
        r.publish(&s);
        (s, r)
    }
    fn snapshot(s: &Shared) -> Status {
        s.status(0).unwrap()
    }
    fn note(ch: u8) -> Vec<Value> {
        vec![
            json!({"method":"write","register":0x30+ch,"value":0x15}),
            json!({"method":"write","register":0x10+ch,"value":0x90}),
            json!({"method":"write","register":0x20+ch,"value":0x17}),
        ]
    }
    #[test]
    fn master_system_native_validates_chip_channels_registers_clocks_and_atomic_batches() {
        for v in [
            json!({"chip":"ym2413","method":"write","register":64,"value":0}),
            json!({"chip":"segapsg","method":"write","value":1,"psgRegister":8}),
            json!({"chip":"segapsg","method":"key-off","channel":4}),
            json!({"chip":"ym2413","method":"audition-key-on","channel":14}),
            json!({"chip":"ym2413","method":"clock","hz":99999}),
            json!({"chip":"segapsg","method":"source-mute","mask":16}),
            json!({"chip":"ym2413","method":"source-mute","mask":512}),
            json!({"chip":"segapsg","method":"mix","volume":5,"pan":0}),
            lease("ym2413", 0, 1, "on", note(1)),
            lease(
                "segapsg",
                0,
                1,
                "on",
                vec![json!({"method":"write","value":0x12})],
            ),
            lease(
                "segapsg",
                0,
                1,
                "on",
                vec![json!({"method":"write","value":0x12,"psgRegister":2})],
            ),
        ] {
            assert!(serde_json::from_value::<Message>(v)
                .unwrap()
                .prepare()
                .is_err());
        }
        assert!(serde_json::from_value::<Message>(lease(
            "ym2413",
            10,
            1,
            "on",
            vec![json!({"method":"write","register":0x17,"value":0x80})]
        ))
        .unwrap()
        .prepare()
        .is_ok());
    }
    #[test]
    fn master_system_native_snapshot_is_observational_and_default_mono_clock_correct() {
        let (s, mut r) = setup();
        let a = snapshot(&s);
        assert_eq!(a.segapsg.clock, CLOCK);
        assert_eq!(a.ym2413.clock, CLOCK);
        assert_eq!(a.ym2413.registers.len(), 64);
        assert_eq!(a.segapsg.attenuations, vec![15; 4]);
        for _ in 0..256 {
            let mut p = [0.; 2];
            r.mix(&mut p);
            assert_eq!(p[0], p[1]);
        }
        assert_eq!(snapshot(&s).revision, a.revision);
    }
    #[test]
    fn master_system_native_psg_continuations_restore_their_original_latch() {
        let (s, mut r) = setup();
        apply(&mut r, &s, write("segapsg", 0, 0x85), None);
        apply(&mut r, &s, write("segapsg", 0, 0xc9), None);
        apply(
            &mut r,
            &s,
            json!({"chip":"segapsg","method":"write","value":0x12,"psgRegister":0}),
            None,
        );
        let out = snapshot(&s);
        assert_eq!(out.segapsg.periods, [0x125, 0, 9]);
        assert_eq!(out.segapsg.latched, 0);
    }
    #[test]
    fn master_system_native_psg_manual_noise_tone3_and_clock_are_protected() {
        let (s, mut r) = setup();
        apply(
            &mut r,
            &s,
            lease(
                "segapsg",
                3,
                1,
                "on",
                vec![
                    json!({"method":"write","value":0xe7}),
                    json!({"method":"write","value":0xf8}),
                ],
            ),
            None,
        );
        for v in [
            write("segapsg", 0, 0xe4),
            write("segapsg", 0, 0xc6),
            write("segapsg", 0, 0xff),
            json!({"chip":"segapsg","method":"clock","hz":4_000_000}),
        ] {
            apply(&mut r, &s, v, owner(1))
        }
        let out = snapshot(&s);
        assert_eq!(out.segapsg.mode, 7);
        assert_eq!(out.segapsg.periods[2], 0);
        assert_eq!(out.segapsg.attenuations[3], 8);
        assert_eq!(out.segapsg.clock, CLOCK);
        assert_eq!(out.segapsg.held, [3]);
        apply(
            &mut r,
            &s,
            lease(
                "segapsg",
                3,
                2,
                "off",
                vec![json!({"method":"key-off","channel":3})],
            ),
            None,
        );
        assert_eq!(snapshot(&s).segapsg.held, [3]);
        apply(
            &mut r,
            &s,
            lease(
                "segapsg",
                3,
                1,
                "off",
                vec![json!({"method":"key-off","channel":3})],
            ),
            None,
        );
        assert!(snapshot(&s).segapsg.held.is_empty());
    }
    #[test]
    fn master_system_native_all_opll_channels_protect_authored_patch_and_stale_off() {
        let (s, mut r) = setup();
        for ch in 0..9 {
            apply(
                &mut r,
                &s,
                lease("ym2413", ch, ch as u64 + 1, "on", note(ch)),
                None,
            )
        }
        let before = snapshot(&s);
        for ch in 0..9 {
            apply(&mut r, &s, write("ym2413", 0x30 + ch, 0xff), owner(1));
            apply(
                &mut r,
                &s,
                json!({"chip":"ym2413","method":"key-off","channel":ch}),
                owner(1),
            );
        }
        for reg in 0..8 {
            apply(&mut r, &s, write("ym2413", reg, 0xff), owner(1));
        }
        apply(&mut r, &s, write("ym2413", 14, 0x3f), owner(1));
        assert_eq!(snapshot(&s).ym2413.registers, before.ym2413.registers);
        assert_eq!(snapshot(&s).ym2413.held, (0..9).collect::<Vec<_>>());
        apply(
            &mut r,
            &s,
            lease(
                "ym2413",
                2,
                3,
                "edit",
                vec![json!({"method":"write","register":0x32,"value":0x42})],
            ),
            None,
        );
        assert_eq!(snapshot(&s).ym2413.registers[0x32], 0x42);
    }
    #[test]
    fn master_system_native_rhythm_siblings_share_a_lease_and_stale_release_cannot_stop_replacement(
    ) {
        let (s, mut r) = setup();
        apply(&mut r, &s, write("ym2413", 14, 0x20), None);
        apply(
            &mut r,
            &s,
            lease(
                "ym2413",
                10,
                1,
                "on",
                vec![json!({"method":"audition-key-on","channel":10})],
            ),
            None,
        );
        for _ in 0..16 {
            r.mix(&mut [0.; 2]);
        }
        r.publish(&s);
        assert_eq!(snapshot(&s).ym2413.held, [10]);
        apply(
            &mut r,
            &s,
            lease(
                "ym2413",
                13,
                2,
                "on",
                vec![json!({"method":"audition-key-on","channel":13})],
            ),
            None,
        );
        apply(
            &mut r,
            &s,
            lease(
                "ym2413",
                10,
                1,
                "off",
                vec![json!({"method":"key-off","channel":10})],
            ),
            None,
        );
        for _ in 0..16 {
            r.mix(&mut [0.; 2]);
        }
        r.publish(&s);
        let out = snapshot(&s);
        assert_eq!(out.ym2413.held, [13]);
        assert_eq!(out.ym2413.registers[14] & 9, 1);
        apply(&mut r, &s, write("ym2413", 14, 0x20), owner(1));
        assert_eq!(snapshot(&s).ym2413.registers[14] & 1, 1);
    }
    #[test]
    fn master_system_native_stop_retains_authored_levels_patch_periods_and_clock() {
        let (s, mut r) = setup();
        for v in [
            write("segapsg", 0, 0x87),
            write("segapsg", 0, 0x22),
            write("segapsg", 0, 0x96),
            write("ym2413", 0, 0x71),
            write("ym2413", 0x30, 0x34),
            write("ym2413", 0x20, 0x16),
            json!({"chip":"segapsg","method":"clock","hz":4_000_000}),
        ] {
            apply(&mut r, &s, v, None)
        }
        let before = snapshot(&s);
        r.stop(&s);
        let after = snapshot(&s);
        assert_eq!(after.segapsg.periods, before.segapsg.periods);
        assert_eq!(after.segapsg.last_volumes[0], 6);
        assert_eq!(after.segapsg.attenuations, [15; 4]);
        assert_eq!(after.segapsg.clock, 4_000_000);
        assert_eq!(after.ym2413.registers[0], 0x71);
        assert_eq!(after.ym2413.registers[0x30], 0x34);
        assert_eq!(after.ym2413.registers[0x20] & 16, 0);
        apply(
            &mut r,
            &s,
            json!({"chip":"segapsg","method":"audition-key-on","channel":0}),
            None,
        );
        assert_eq!(snapshot(&s).segapsg.attenuations[0], 6);
    }
    #[test]
    fn master_system_native_owner_cancellation_releases_only_matching_voices_and_pending_drums() {
        let (s, mut r) = setup();
        apply(&mut r, &s, write("ym2413", 14, 0x20), None);
        apply(
            &mut r,
            &s,
            json!({"chip":"ym2413","method":"batch","channel":13,"voice":1,"action":"on","commands":[{"method":"audition-key-on","channel":13}]}),
            owner(1),
        );
        apply(
            &mut r,
            &s,
            json!({"chip":"segapsg","method":"write","value":0x94}),
            owner(2),
        );
        r.cancel_invalid(|o| o != owner(1), &s);
        for _ in 0..128 {
            r.mix(&mut [0.; 2]);
        }
        r.publish(&s);
        let out = snapshot(&s);
        assert_eq!(out.ym2413.registers[14] & 31, 0);
        assert_eq!(out.segapsg.attenuations[0], 4);
    }
    #[test]
    fn master_system_native_retained_admission_is_bounded_atomic_and_reclaimed_off_callback() {
        let (s, mut r) = setup();
        let a = prep(json!({"chip":"segapsg","method":"write","value":0x94,"id":1}));
        s.retain(&[(a.clone(), None)]).unwrap();
        assert!(s.retain(&[(a.clone(), None)]).is_err());
        assert_eq!(snapshot(&s).segapsg.pending_commands, [1]);
        r.apply(&a, None, &s);
        s.retire(a);
        let out = snapshot(&s);
        assert!(out.segapsg.pending_commands.is_empty());
        assert!(s.control.lock().unwrap().prepared.is_empty());
        assert!(s.wait_applied(1).is_ok());
        assert!(s.wait_applied(2).is_err());
    }
}

#[cfg(test)]
mod pipeline_tests {
    use super::super::*;
    use serde_json::{json, Value};
    fn command(frame: u64, message: Value) -> Command {
        serde_json::from_value(json!({"frame":frame,"type":"master-system","message":message}))
            .unwrap()
    }
    fn tagged(frame: u64, message: Value, owner: u32) -> Command {
        let mut c = command(frame, message);
        c.performance = Some(1);
        c.owner = Some(owner);
        c
    }
    fn setup() -> (Shared, Renderer) {
        let s = Shared::new_modes(48000, 1, false, false, false, Some("master-system"));
        let r = Renderer::new_modes(48000, false, false, false, Some("master-system")).unwrap();
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
    fn note(ch: u8) -> Value {
        json!({"chip":"ym2413","method":"batch","channel":ch,"action":"on","voice":ch+1,"commands":[{"method":"write","register":0x30+ch,"value":0x15},{"method":"write","register":0x10+ch,"value":0x90},{"method":"write","register":0x20+ch,"value":0x17}]})
    }
    fn psg() -> Value {
        json!({"chip":"segapsg","method":"batch","channel":0,"action":"on","voice":10,"commands":[{"method":"write","value":0x85},{"method":"write","value":0x10,"psgRegister":0},{"method":"write","value":0x95}]})
    }
    fn snapshot(s: &Shared) -> master_system::Status {
        s.master_system
            .as_ref()
            .unwrap()
            .status(s.frame.load(Ordering::Acquire))
            .unwrap()
    }
    #[test]
    fn master_system_native_primary_has_no_hidden_genesis_and_rejects_foreign_commands_atomically()
    {
        let (s, mut r) = setup();
        assert!(r.core.0.is_null());
        assert!(r.master_system.is_some());
        assert!(r.dx7.is_none() && r.x68000.is_none() && r.rf5c164.is_none());
        assert_eq!(s.status().mode, "master-system");
        assert_eq!(s.status().ym2413_clock, Some(master_system::CLOCK));
        assert_eq!(s.status().psg_clock, master_system::CLOCK);
        for value in [
            json!({"frame":0,"type":"write","register":40,"value":0}),
            json!({"frame":0,"type":"psg-write","value":128}),
            json!({"frame":0,"type":"rf5c164-reset"}),
        ] {
            let invalid: Command = serde_json::from_value(value).unwrap();
            assert!(s.enqueue(1, &[command(0, note(0)), invalid]).is_err());
            assert_eq!(s.pending.load(Ordering::Acquire), 0)
        }
        assert!(render(&s, &mut r, 128).iter().all(|v| *v == [0.; 2]));
        let genesis = Shared::new(48000, 2);
        assert!(genesis.enqueue(2, &[command(0, note(0))]).is_err());
    }
    #[test]
    fn master_system_native_callback_segmentation_and_scheduled_frames_are_exact() {
        let (a, mut left) = setup();
        let (b, mut right) = setup();
        let cmds = [command(100, note(0)), command(175, psg())];
        a.enqueue(1, &cmds).unwrap();
        b.enqueue(1, &cmds).unwrap();
        let all = render(&a, &mut left, 4096);
        let mut split = Vec::new();
        for n in [1, 31, 128, 384, 1024, 2528] {
            split.extend(render(&b, &mut right, n));
        }
        assert_eq!(all, split);
        assert!(all[..100].iter().all(|v| *v == [0.; 2]));
        assert!(all[500..].iter().any(|v| v[0].abs() > 0.0001));
        assert!(all.iter().all(|v| v[0] == v[1]));
    }
    #[test]
    fn master_system_native_chip_scoped_stop_cancels_only_matching_chip_and_owner_future() {
        let (s, mut r) = setup();
        s.performance("start", 1, None).unwrap();
        s.enqueue(
            1,
            &[
                tagged(0, note(0), 1),
                tagged(0, note(1), 2),
                tagged(1000, note(0), 1),
                tagged(1200, note(1), 2),
                tagged(1100, psg(), 1),
            ],
        )
        .unwrap();
        render(&s, &mut r, 256);
        s.enqueue(
            1,
            &[tagged(
                0,
                json!({"chip":"ym2413","method":"stop-voices"}),
                1,
            )],
        )
        .unwrap();
        render(&s, &mut r, 1500);
        let out = snapshot(&s);
        assert_eq!(out.ym2413.registers[0x20] & 16, 0);
        assert_ne!(out.ym2413.registers[0x21] & 16, 0);
        assert_eq!(out.segapsg.attenuations[0], 5);
        assert!(out.ym2413.pending_commands.is_empty());
        assert!(out.segapsg.pending_commands.is_empty());
    }
    #[test]
    fn master_system_native_epoch_stop_cancels_future_and_releases_both_chips_preserving_authored_data(
    ) {
        let (s, mut r) = setup();
        s.enqueue(
            1,
            &[
                command(0, note(0)),
                command(0, psg()),
                command(5000, note(1)),
            ],
        )
        .unwrap();
        render(&s, &mut r, 256);
        s.epoch.fetch_add(1, Ordering::AcqRel);
        render(&s, &mut r, 6000);
        let out = snapshot(&s);
        assert_eq!(out.segapsg.attenuations, [15; 4]);
        assert_eq!(out.segapsg.periods[0], 0x105);
        assert_eq!(out.ym2413.registers[0x30], 0x15);
        assert_eq!(out.ym2413.registers[0x20] & 16, 0);
        assert_eq!(out.ym2413.registers[0x21] & 16, 0);
        assert_eq!(s.pending.load(Ordering::Acquire), 0);
    }
    #[test]
    fn master_system_native_independent_psg_opll_mixers_reach_the_same_final_pcm() {
        let (s, mut r) = setup();
        s.enqueue(
            1,
            &[
                command(0, note(0)),
                command(0, psg()),
                command(
                    0,
                    json!({"chip":"segapsg","method":"mix","volume":1,"pan":-1}),
                ),
                command(
                    0,
                    json!({"chip":"ym2413","method":"mix","volume":1,"pan":1}),
                ),
            ],
        )
        .unwrap();
        let out = render(&s, &mut r, 4096);
        assert!(out[512..].iter().any(|v| v[0].abs() > 0.0001));
        assert!(out[512..].iter().any(|v| v[1].abs() > 0.0001));
        assert!(out[512..].iter().any(|v| v[0] != v[1]));
        s.enqueue(
            1,
            &[command(
                0,
                json!({"chip":"segapsg","method":"mix","volume":0,"pan":0}),
            )],
        )
        .unwrap();
        let out = render(&s, &mut r, 1024);
        assert!(out[512..].iter().all(|v| v[0] == 0.));
        assert!(out[512..].iter().any(|v| v[1].abs() > 0.0001));
    }
}

#[cfg(test)]
mod game_gear_tests {
    use super::*;
    use serde_json::{json, Value};
    fn prep(v: Value) -> Arc<Prepared> {
        serde_json::from_value::<Message>(v)
            .unwrap()
            .prepare()
            .unwrap()
    }
    fn setup() -> (Shared, Renderer) {
        let s = Shared::game_gear();
        let r = Renderer::new_game_gear(48000).unwrap();
        r.publish(&s);
        (s, r)
    }
    fn apply(r: &mut Renderer, s: &Shared, v: Value, owner: Option<Owner>) {
        let c = prep(v);
        s.retain(&[(c.clone(), owner)]).unwrap();
        r.apply(&c, owner, s);
        s.retire(c)
    }
    fn write(value: u8) -> Value {
        json!({"chip":"segapsg","method":"write","value":value})
    }
    fn stereo(mask: u8) -> Value {
        json!({"chip":"segapsg","method":"stereo","mask":mask})
    }
    fn note(ch: u8) -> Vec<Value> {
        let mut data = if ch < 3 {
            vec![write(0x80 | (ch << 5) | 5), write(0x10)]
        } else {
            vec![write(0xe4)]
        };
        data.push(write(0x90 | (ch << 5) | 6));
        data
    }
    fn start(r: &mut Renderer, s: &Shared, ch: u8) {
        for op in note(ch) {
            apply(r, s, op, None)
        }
    }
    fn snapshot(s: &Shared) -> PsgStatus {
        s.game_gear_status(0).unwrap().segapsg
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
    fn owner(n: u32) -> Option<Owner> {
        Some(Owner {
            performance: 1,
            owner: n,
        })
    }
    fn lease(ch: u8, voice: u64, action: &str, commands: Vec<Value>) -> Value {
        json!({"chip":"segapsg","method":"batch","channel":ch,"voice":voice,"action":action,"audition":true,"commands":commands})
    }
    #[test]
    fn game_gear_native_state_abi_profile_and_status_only_expose_stereo_psg() {
        assert_eq!(std::mem::size_of::<GameGearCoreState>(), 28);
        let (s, mut r) = setup();
        assert!(r.game_gear);
        let data = s.game_gear_status(7).unwrap();
        assert_eq!(data.frame, 7);
        assert_eq!(data.segapsg.clock, CLOCK);
        assert_eq!(data.segapsg.stereo, 255);
        assert_eq!(data.segapsg.attenuations, [15; 4]);
        assert_eq!(r.state().opll_clock, 0);
        let encoded = serde_json::to_value(data).unwrap();
        assert!(encoded.get("ym2413").is_none());
        start(&mut r, &s, 0);
        let before = serde_json::to_value(s.game_gear_status(0).unwrap()).unwrap();
        for _ in 0..20 {
            assert_eq!(
                serde_json::to_value(s.game_gear_status(0).unwrap()).unwrap(),
                before
            )
        }
        assert!(render(&mut r, 512).iter().any(|v| v[0] != 0.));
    }
    #[test]
    fn game_gear_native_all_256_stereo_masks_route_each_physical_voice() {
        for ch in 0..4 {
            let (s, mut r) = setup();
            start(&mut r, &s, ch);
            render(&mut r, 4096);
            for mask in 0..=255u8 {
                apply(&mut r, &s, stereo(mask), None);
                let pcm = render(&mut r, 256);
                for side in 0..2 {
                    let bit = if side == 0 { ch + 4 } else { ch };
                    let energy: f32 = pcm.iter().map(|v| v[side].abs()).sum();
                    if mask & (1 << bit) == 0 {
                        assert_eq!(energy, 0., "ch{ch} mask{mask:02x} side{side}")
                    } else {
                        assert!(energy > 0., "ch{ch} mask{mask:02x} side{side}")
                    }
                }
                assert_eq!(snapshot(&s).stereo, mask)
            }
        }
    }
    #[test]
    fn game_gear_native_manual_route_bits_are_protected_and_ui_edits_apply() {
        let (s, mut r) = setup();
        let commands = note(1)
            .into_iter()
            .map(|mut v| {
                v.as_object_mut().unwrap().remove("chip");
                v
            })
            .collect();
        apply(&mut r, &s, lease(1, 1, "on", commands), None);
        apply(&mut r, &s, stereo(0), owner(1));
        assert_eq!(snapshot(&s).stereo, 0x22);
        apply(
            &mut r,
            &s,
            lease(1, 1, "edit", vec![json!({"method":"stereo","mask":4})]),
            None,
        );
        assert_eq!(snapshot(&s).stereo, 4);
        assert_eq!(snapshot(&s).held, [1]);
        apply(&mut r, &s, stereo(255), owner(1));
        assert_eq!(snapshot(&s).stereo, 0xdd);
        apply(
            &mut r,
            &s,
            lease(1, 1, "off", vec![json!({"method":"key-off","channel":1})]),
            None,
        );
        apply(&mut r, &s, stereo(255), owner(1));
        assert_eq!(snapshot(&s).stereo, 255);
    }
    #[test]
    fn game_gear_native_latch_continuation_noise_tone3_and_stale_owner_are_protected() {
        let (s, mut r) = setup();
        apply(&mut r, &s, write(0x85), None);
        apply(&mut r, &s, write(0xc9), None);
        apply(
            &mut r,
            &s,
            json!({"chip":"segapsg","method":"write","value":0x12,"psgRegister":0}),
            None,
        );
        assert_eq!(snapshot(&s).periods, [0x125, 0, 9]);
        apply(
            &mut r,
            &s,
            lease(
                3,
                1,
                "on",
                vec![
                    json!({"method":"write","value":0xe7}),
                    json!({"method":"write","value":0xf8}),
                ],
            ),
            None,
        );
        for value in [0xc6, 0xe4, 0xff] {
            apply(&mut r, &s, write(value), owner(1))
        }
        assert_eq!(snapshot(&s).periods[2], 9);
        assert_eq!(snapshot(&s).mode, 7);
        assert_eq!(snapshot(&s).attenuations[3], 8);
        apply(
            &mut r,
            &s,
            lease(3, 2, "off", vec![json!({"method":"key-off","channel":3})]),
            None,
        );
        assert_eq!(snapshot(&s).held, [3]);
        apply(
            &mut r,
            &s,
            lease(3, 1, "off", vec![json!({"method":"key-off","channel":3})]),
            None,
        );
        assert!(snapshot(&s).held.is_empty());
    }
    #[test]
    fn game_gear_native_stop_retains_tone_noise_authored_levels_clock_mask_and_mute() {
        let (s, mut r) = setup();
        start(&mut r, &s, 0);
        start(&mut r, &s, 3);
        apply(&mut r, &s, stereo(0x81), None);
        apply(
            &mut r,
            &s,
            json!({"chip":"segapsg","method":"clock","hz":4_000_000}),
            None,
        );
        apply(
            &mut r,
            &s,
            json!({"chip":"segapsg","method":"source-mute","mask":8}),
            None,
        );
        let before = snapshot(&s);
        r.stop(&s);
        assert!(render(&mut r, 1024).iter().all(|p| *p == [0.; 2]));
        let after = snapshot(&s);
        assert_eq!(after.periods, before.periods);
        assert_eq!(after.mode, before.mode);
        assert_eq!(after.last_volumes, before.last_volumes);
        assert_eq!(after.stereo, 0x81);
        assert_eq!(after.clock, 4_000_000);
        assert_eq!(after.source_mute_mask, 8);
        assert_eq!(after.attenuations, [15; 4]);
        apply(
            &mut r,
            &s,
            json!({"chip":"segapsg","method":"audition-key-on","channel":0}),
            None,
        );
        let pcm = render(&mut r, 512);
        assert!(pcm.iter().any(|v| v[1] != 0.));
        assert!(pcm.iter().all(|v| v[0] == 0.));
        apply(
            &mut r,
            &s,
            json!({"chip":"segapsg","method":"reset","audition":true}),
            None,
        );
        assert_eq!(snapshot(&s).stereo, 255);
        assert_eq!(snapshot(&s).clock, 4_000_000);
    }
    #[test]
    fn game_gear_native_clock_barrier_rejects_manual_hold_after_admission_even_same_hz() {
        let (s, mut r) = setup();
        let clock = prep(json!({"chip":"segapsg","id":77,"method":"clock","hz":CLOCK}));
        s.retain(&[(clock.clone(), None)]).unwrap();
        apply(
            &mut r,
            &s,
            lease(
                0,
                1,
                "on",
                vec![
                    json!({"method":"write","value":0x85}),
                    json!({"method":"write","value":0x10}),
                    json!({"method":"write","value":0x96}),
                ],
            ),
            None,
        );
        let next = prep(json!({"chip":"segapsg","id":78,"method":"clock","hz":4_000_000}));
        assert_eq!(
            s.retain(&[(next, owner(1))]).unwrap_err(),
            "Release the manually held Game Gear voices before changing the clock"
        );
        r.apply(&clock, None, &s);
        s.retire(clock);
        assert!(snapshot(&s).pending_commands.is_empty());
        assert_eq!(
            s.wait_applied(77).unwrap_err(),
            "Release the manually held Game Gear voices before changing the clock"
        );
        assert!(s.control.lock().unwrap().rejected_clocks.is_empty());
        assert_eq!(snapshot(&s).clock, CLOCK);
    }
    #[test]
    fn game_gear_native_profile_rejects_opll_and_master_system_rejects_stereo_atomically() {
        let (s, _) = setup();
        let good = prep(write(0x85));
        let bad = prep(json!({"chip":"ym2413","method":"write","register":0x20,"value":0x17}));
        assert!(s.retain(&[(good, None), (bad, None)]).is_err());
        assert!(s.control.lock().unwrap().prepared.is_empty());
        let master = Shared::default();
        assert!(master.retain(&[(prep(stereo(0)), None)]).is_err());
        let batch = prep(lease(
            0,
            1,
            "edit",
            vec![json!({"method":"stereo","mask":0})],
        ));
        assert!(master.retain(&[(batch, None)]).is_err());
        assert!(serde_json::from_value::<Message>(
            json!({"chip":"segapsg","method":"stereo","mask":256})
        )
        .is_err());
        assert!(serde_json::from_value::<Message>(
            json!({"chip":"ym2413","method":"stereo","mask":0})
        )
        .unwrap()
        .prepare()
        .is_err());
    }
    fn queued(v: Value, frame: u64) -> super::super::Command {
        serde_json::from_value(json!({"type":"master-system","message":v,"frame":frame})).unwrap()
    }
    fn engine() -> (super::super::Shared, super::super::Renderer) {
        (
            super::super::Shared::new_modes(48000, 1, false, false, false, Some("game-gear")),
            super::super::Renderer::new_modes(48000, false, false, false, Some("game-gear"))
                .unwrap(),
        )
    }
    fn engine_render(
        r: &mut super::super::Renderer,
        s: &super::super::Shared,
        n: usize,
    ) -> Vec<[f32; 2]> {
        r.prepare(s);
        (0..n).map(|_| r.sample(s)).collect()
    }
    #[test]
    fn game_gear_native_queue_has_no_hidden_chips_and_routing_frames_are_segmentation_exact() {
        let (s, mut r) = engine();
        let (t, mut q) = engine();
        assert!(r.core.0.is_null());
        assert!(r.msx.is_none() && r.opl.is_none() && r.ym2608.is_none());
        let info = s.status();
        assert_eq!(info.mode, "game-gear");
        assert_eq!(info.ym2413_clock, None);
        assert_eq!(info.psg_clock, CLOCK);
        let mut commands: Vec<_> = note(0).into_iter().map(|v| queued(v, 0)).collect();
        for (frame, mask) in [(512, 1), (1024, 16), (1536, 0)] {
            commands.push(queued(stereo(mask), frame))
        }
        s.enqueue(1, &commands).unwrap();
        t.enqueue(1, &commands).unwrap();
        let pcm = engine_render(&mut r, &s, 2048);
        let mut segmented = Vec::new();
        for n in [7, 300, 205, 512, 31, 993] {
            segmented.extend(engine_render(&mut q, &t, n))
        }
        assert_eq!(pcm, segmented);
        assert!(pcm[600..1024].iter().all(|p| p[0] == 0.));
        assert!(pcm[600..1024].iter().any(|p| p[1] != 0.));
        assert!(pcm[1100..1536].iter().all(|p| p[1] == 0.));
        assert!(pcm[1100..1536].iter().any(|p| p[0] != 0.));
        assert!(pcm[1536..].iter().all(|p| *p == [0.; 2]));
        let invalid = vec![
            queued(write(0x90), 0),
            queued(
                json!({"chip":"ym2413","method":"write","register":0x20,"value":0x17}),
                0,
            ),
        ];
        assert!(s.enqueue(1, &invalid).is_err());
        assert_eq!(s.pending.load(Ordering::Acquire), 0);
    }
    #[test]
    fn game_gear_native_stop_fence_and_epoch_stop_retain_stereo() {
        let (s, mut r) = engine();
        let mut owned: Vec<_> = note(0).into_iter().map(|v| queued(v, 0)).collect();
        let owned2: Vec<_> = note(1).into_iter().map(|v| queued(v, 0)).collect();
        owned.extend(owned2);
        s.enqueue(1, &owned).unwrap();
        s.enqueue(1, &[queued(stereo(0x81), 0)]).unwrap();
        engine_render(&mut r, &s, 512);
        s.enqueue(
            1,
            &note(2)
                .into_iter()
                .map(|v| queued(v, 3000))
                .collect::<Vec<_>>(),
        )
        .unwrap();
        s.enqueue(
            1,
            &[queued(
                json!({"chip":"segapsg","method":"stop-voices"}),
                600,
            )],
        )
        .unwrap();
        let pcm = engine_render(&mut r, &s, 4096);
        assert!(pcm[200..].iter().all(|p| *p == [0.; 2]));
        assert_eq!(s.pending.load(Ordering::Acquire), 0);
        assert_eq!(snapshot(s.master_system.as_ref().unwrap()).stereo, 0x81);
        s.enqueue(
            1,
            &note(0)
                .into_iter()
                .map(|v| queued(v, 0))
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert!(engine_render(&mut r, &s, 512).iter().any(|p| p[1] != 0.));
        s.epoch.fetch_add(1, Ordering::Release);
        engine_render(&mut r, &s, 1024);
        assert_eq!(s.stopped_epoch.load(Ordering::Acquire), 1);
        assert_eq!(snapshot(s.master_system.as_ref().unwrap()).stereo, 0x81);
    }
    #[test]
    fn game_gear_native_owner_cancellation_releases_only_owned_voice_and_keeps_routing() {
        let (s, mut r) = setup();
        for (ch, n) in [(0, 1), (1, 2)] {
            for v in note(ch) {
                apply(&mut r, &s, v, owner(n))
            }
        }
        apply(&mut r, &s, stereo(0x12), None);
        apply(
            &mut r,
            &s,
            lease(
                3,
                1,
                "on",
                vec![
                    json!({"method":"write","value":0xe4}),
                    json!({"method":"write","value":0xf8}),
                ],
            ),
            None,
        );
        r.cancel_invalid(|o| o != owner(1), &s);
        let a = snapshot(&s);
        assert_eq!(a.attenuations[0], 15);
        assert_eq!(a.attenuations[1], 6);
        assert_eq!(a.attenuations[3], 8);
        assert_eq!(a.held, [3]);
        assert_eq!(a.stereo, 0x12);
        let pcm = render(&mut r, 1024);
        assert!(pcm.iter().all(|v| v[0] == 0.));
        assert!(pcm.iter().any(|v| v[1] != 0.));
    }
    #[test]
    fn game_gear_native_future_clock_waits_for_its_audio_frame_before_acknowledging() {
        let (s, mut r) = engine();
        s.enqueue(
            1,
            &[queued(
                json!({"chip":"segapsg","id":77,"method":"clock","hz":4_000_000}),
                1024,
            )],
        )
        .unwrap();
        engine_render(&mut r, &s, 512);
        let state = snapshot(s.master_system.as_ref().unwrap());
        assert_eq!(state.clock, CLOCK);
        assert_eq!(state.pending_commands, [77]);
        engine_render(&mut r, &s, 1024);
        let state = snapshot(s.master_system.as_ref().unwrap());
        assert_eq!(state.clock, 4_000_000);
        assert!(state.pending_commands.is_empty());
        s.master_system.as_ref().unwrap().wait_applied(77).unwrap();
    }
}
