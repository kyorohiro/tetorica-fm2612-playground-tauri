//! Callback-owned 32X PWM. The fixed scheduling heap is allocated before audio
//! starts; prepared upload storage is retained and reclaimed on the control side.
use super::{Owner, CAPACITY, MAX_SAFE_INTEGER, OWNERS};
use crossbeam_queue::ArrayQueue;
use serde::{Deserialize, Serialize};
use std::{
    cmp::{Ordering as CmpOrdering, Reverse},
    collections::{BinaryHeap, HashSet},
    ffi::c_void,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        Arc, Mutex,
    },
};

pub const CLOCK: u32 = 23_011_361;
pub const QUEUE: usize = 1_000_000;
pub const CURSORS: usize = 1024;
const SCOPES: usize = CURSORS + 9;
pub const INSTANCE_BYTES: usize =
    super::extra_budget::INSTANCE_BYTES + 2 * 1024 * 1024 + 256 * 1024;
const PENDING_BYTES: usize = 64 * 1024 * 1024;
#[repr(C)]
#[derive(Default)]
struct CoreState {
    registers: [u32; 5],
    clock: u32,
    cycle: u32,
    left: u32,
    right: u32,
    left_valid: u32,
    right_valid: u32,
    left_size: u32,
    right_size: u32,
    left_fifo: [u32; 3],
    right_fifo: [u32; 3],
    timer_tick: u32,
    interrupt_count: f64,
    until_tick: f64,
}
extern "C" {
    fn native_pwm_create(rate: u32) -> *mut c_void;
    fn native_pwm_destroy(ptr: *mut c_void);
    fn native_pwm_reset(ptr: *mut c_void);
    fn native_pwm_stop(ptr: *mut c_void);
    fn native_pwm_write_tagged(ptr: *mut c_void, register: u32, value: u32, tag: u64);
    fn native_pwm_cancel_tag(ptr: *mut c_void, tag: u64);
    fn native_pwm_has_tag(ptr: *mut c_void, tag: u64) -> i32;
    fn native_pwm_set_clock(ptr: *mut c_void, hz: u32) -> i32;
    fn native_pwm_sample(ptr: *mut c_void, left: *mut f32, right: *mut f32);
    fn native_pwm_get_state(ptr: *mut c_void, out: *mut CoreState);
}
#[derive(Clone, Copy, Debug, Deserialize)]
pub struct Write {
    pub frame: u64,
    pub register: u8,
    pub value: u16,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Message {
    #[serde(default)]
    pub id: Option<u64>,
    #[serde(default)]
    pub order: Option<u64>,
    #[serde(default)]
    pub group: Option<u64>,
    #[serde(flatten)]
    pub method: Method,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "method", rename_all = "kebab-case")]
pub enum Method {
    Write {
        register: u8,
        value: u16,
    },
    Reset,
    StopVoices,
    ClearSchedule,
    CancelMainOwner {
        #[serde(rename = "targetGroup")]
        target_group: u64,
    },
    Barrier,
    ScheduleWrites {
        entries: Vec<Write>,
        #[serde(default, rename = "baseFrame")]
        base_frame: Option<u64>,
    },
    Clock {
        hz: u32,
    },
    Mix {
        volume: f32,
        pan: f32,
    },
}
#[derive(Debug)]
pub struct Prepared {
    pub id: Option<u64>,
    method: Method,
    complete: AtomicBool,
    retired: AtomicBool,
    schedule: Option<Arc<Schedule>>,
    order: Option<u64>,
    group: Option<u64>,
    cancelled: AtomicBool,
    failed: AtomicU32,
    applied_base: AtomicU64,
}
#[derive(Debug)]
struct Schedule {
    entries: Vec<Write>,
    done: AtomicBool,
}
fn valid_id(value: Option<u64>) -> bool {
    value.is_none_or(|v| v > 0 && v <= MAX_SAFE_INTEGER)
}
impl Message {
    pub fn prepared_bytes(&self) -> usize {
        std::mem::size_of::<Prepared>()
            + if let Method::ScheduleWrites { entries, .. } = &self.method {
                std::mem::size_of::<Schedule>() + entries.len() * std::mem::size_of::<Write>() + 16
            } else {
                0
            }
    }
    pub fn prepare(&self) -> Result<Arc<Prepared>, String> {
        if !valid_id(self.id) || !valid_id(self.order) || !valid_id(self.group) {
            return Err("Invalid Native PWM command ID".into());
        }
        let mut schedule = None;
        let method = match &self.method {
            Method::Write { register, value } => {
                if *register > 4 {
                    return Err("Native PWM register must be 0–4".into());
                }
                Method::Write {
                    register: *register,
                    value: *value,
                }
            }
            Method::ScheduleWrites {
                entries,
                base_frame,
            } => {
                if entries.len() > QUEUE
                    || base_frame.is_some_and(|v| v > MAX_SAFE_INTEGER)
                    || entries.iter().any(|w| {
                        w.frame > MAX_SAFE_INTEGER
                            || w.register > 4
                            || base_frame.is_some_and(|base| w.frame > MAX_SAFE_INTEGER - base)
                    })
                {
                    return Err("Invalid Native PWM scheduled write queue".into());
                }
                let mut sorted = entries.clone();
                // Stable sorting and temporary allocation happen on the control
                // thread. The callback admits one cursor, irrespective of size.
                sorted.sort_by_key(|write| write.frame);
                schedule = Some(Arc::new(Schedule {
                    entries: sorted,
                    done: AtomicBool::new(entries.is_empty()),
                }));
                Method::ScheduleWrites {
                    entries: Vec::new(),
                    base_frame: *base_frame,
                }
            }
            Method::Clock { hz } => {
                if !(100_000..=100_000_000).contains(hz) {
                    return Err("Native PWM clock must be 100000–100000000 Hz".into());
                }
                Method::Clock { hz: *hz }
            }
            Method::Mix { volume, pan } => {
                if !volume.is_finite()
                    || !(0.0..=4.0).contains(volume)
                    || !pan.is_finite()
                    || !(-1.0..=1.0).contains(pan)
                {
                    return Err("Native PWM mixer volume must be 0–4, pan −1–1".into());
                }
                Method::Mix {
                    volume: *volume,
                    pan: *pan,
                }
            }
            Method::Reset => Method::Reset,
            Method::StopVoices => Method::StopVoices,
            Method::ClearSchedule => Method::ClearSchedule,
            Method::CancelMainOwner { target_group } => {
                if !valid_id(Some(*target_group)) {
                    return Err("Invalid Native additional PWM Main owner group".into());
                }
                Method::CancelMainOwner {
                    target_group: *target_group,
                }
            }
            Method::Barrier => Method::Barrier,
        };
        Ok(Arc::new(Prepared {
            id: self.id,
            method,
            complete: AtomicBool::new(false),
            retired: AtomicBool::new(false),
            schedule,
            order: self.order,
            group: self.group,
            cancelled: AtomicBool::new(false),
            failed: AtomicU32::new(0),
            applied_base: AtomicU64::new(0),
        }))
    }
}
impl Prepared {
    pub fn starts(&self) -> bool {
        matches!(
            &self.method,
            Method::Write {
                register: 2..=4,
                ..
            } | Method::ScheduleWrites { .. }
        )
    }
    pub fn stops(&self) -> bool {
        matches!(self.method, Method::StopVoices) && self.group.is_none()
    }
    fn entries(&self) -> usize {
        self.schedule
            .as_ref()
            .map_or(0, |schedule| schedule.entries.len())
    }
    pub fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.schedule.as_ref().map_or(0, |v| {
                std::mem::size_of::<Schedule>()
                    + v.entries.capacity() * std::mem::size_of::<Write>()
                    + 16
            })
    }
    pub fn is_owner_cancel(&self) -> bool {
        matches!(self.method, Method::CancelMainOwner { .. })
    }
}
#[derive(Clone, Copy, Default)]
struct Completion {
    id: u64,
    flags: u8,
    schedule: bool,
    base: u64,
}
struct Control {
    prepared: Vec<Arc<Prepared>>,
    outcomes: Box<[Completion]>,
    cursor: usize,
}
impl Default for Control {
    fn default() -> Self {
        Self {
            prepared: Vec::new(),
            outcomes: vec![Completion::default(); 512].into_boxed_slice(),
            cursor: 0,
        }
    }
}

pub struct Shared {
    revision: AtomicU64,
    registers: [AtomicU32; 5],
    clock: AtomicU32,
    rate: AtomicU32,
    current_frame: AtomicU64,
    queued: AtomicU64,
    left: AtomicU32,
    right: AtomicU32,
    left_size: AtomicU32,
    right_size: AtomicU32,
    left_fifo: [AtomicU32; 3],
    right_fifo: [AtomicU32; 3],
    cycle: AtomicU32,
    timer_tick: AtomicU32,
    interrupt_count: AtomicU64,
    until_tick: AtomicU64,
    schedule_id: AtomicU64,
    schedule_base: AtomicU64,
    schedule_error: AtomicU32,
    control: Mutex<Control>,
    cancel_before: AtomicU64,
    owner_cancel_before: [AtomicU64; OWNERS],
    retired: ArrayQueue<Arc<Prepared>>,
    retired_schedules: Arc<ArrayQueue<Arc<Schedule>>>,
    highest_id: AtomicU64,
}
impl Default for Shared {
    fn default() -> Self {
        Self {
            revision: AtomicU64::new(0),
            registers: std::array::from_fn(|_| AtomicU32::new(0)),
            clock: AtomicU32::new(CLOCK),
            rate: AtomicU32::new(48000),
            current_frame: AtomicU64::new(0),
            queued: AtomicU64::new(0),
            left: AtomicU32::new(u32::MAX),
            right: AtomicU32::new(u32::MAX),
            left_size: AtomicU32::new(0),
            right_size: AtomicU32::new(0),
            left_fifo: std::array::from_fn(|_| AtomicU32::new(0)),
            right_fifo: std::array::from_fn(|_| AtomicU32::new(0)),
            cycle: AtomicU32::new(0),
            timer_tick: AtomicU32::new(0),
            interrupt_count: AtomicU64::new(0),
            until_tick: AtomicU64::new(0),
            schedule_id: AtomicU64::new(0),
            schedule_base: AtomicU64::new(0),
            schedule_error: AtomicU32::new(0),
            control: Mutex::new(Control::default()),
            cancel_before: AtomicU64::new(0),
            owner_cancel_before: std::array::from_fn(|_| AtomicU64::new(0)),
            retired: ArrayQueue::new(CAPACITY),
            retired_schedules: Arc::new(ArrayQueue::new(CAPACITY)),
            highest_id: AtomicU64::new(0),
        }
    }
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleResult {
    pub id: u64,
    pub base_frame: u64,
    pub error: Option<&'static str>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChipStatus {
    pub model: &'static str,
    pub output_mode: &'static str,
    pub gain: f32,
    pub muted: bool,
    pub clock: u32,
    pub sample_rate: u32,
    pub current_frame: u64,
    pub queued_writes: usize,
    pub registers: [u32; 5],
    pub left: Option<u32>,
    pub right: Option<u32>,
    pub cycle: u32,
    pub left_fifo: Vec<u32>,
    pub right_fifo: Vec<u32>,
    pub timer_tick: u32,
    pub interrupt_count: f64,
    pub until_tick: f64,
    pub last_schedule: Option<ScheduleResult>,
    pub frame: u64,
    pub revision: u64,
    pub pending_commands: Vec<u64>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub pwm: ChipStatus,
    pub frame: u64,
    pub sample_rate: u32,
    pub revision: u64,
}
impl Shared {
    pub fn prepare_message(&self, message: &Message) -> Result<Arc<Prepared>, String> {
        message.prepare()
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
    fn collect(&self, control: &mut Control) {
        while self.retired.pop().is_some() {}
        while self.retired_schedules.pop().is_some() {}
        let Control {
            prepared,
            outcomes,
            cursor,
        } = control;
        prepared.retain(|command| {
            if !command.retired.load(Ordering::Acquire)
                || command
                    .schedule
                    .as_ref()
                    .is_some_and(|v| !v.done.load(Ordering::Acquire) || Arc::strong_count(v) > 1)
            {
                return true;
            }
            if let Some(id) = command.id {
                outcomes[*cursor] = Completion {
                    id,
                    flags: command.cancelled.load(Ordering::Acquire) as u8
                        | ((command.failed.load(Ordering::Acquire) != 0) as u8) << 1,
                    schedule: command.schedule.is_some(),
                    base: command.applied_base.load(Ordering::Acquire),
                };
                *cursor = (*cursor + 1) % outcomes.len();
            }
            false
        });
        if prepared.capacity() > 64.max(prepared.len() * 2) {
            prepared.shrink_to_fit();
        }
    }
    pub fn collect_retired(&self) {
        if let Ok(mut control) = self.control.lock() {
            self.collect(&mut control);
        }
    }
    pub fn retain(&self, commands: &[(Arc<Prepared>, Option<Owner>)]) -> Result<(), String> {
        let mut control = self
            .control
            .lock()
            .map_err(|_| "Native additional PWM control state is unavailable")?;
        self.collect(&mut control);
        let registry = &control.prepared;
        if commands.len() + registry.len() > CAPACITY
            || registry.iter().map(|c| c.retained_bytes()).sum::<usize>()
                + commands
                    .iter()
                    .map(|(c, _)| c.retained_bytes())
                    .sum::<usize>()
                > PENDING_BYTES
        {
            return Err("Native additional PWM pending command limit reached (64 MiB)".into());
        }
        // Include queued, pending, and retiring schedules until control collects them.
        let schedules: Vec<_> = registry
            .iter()
            .chain(commands.iter().map(|(v, _)| v))
            .filter(|c| c.schedule.is_some())
            .collect();
        if schedules.len() > CURSORS || schedules.iter().map(|c| c.entries()).sum::<usize>() > QUEUE
        {
            return Err(
                "Native additional PWM write queue limit reached (1000000 entries / 1024 batches)"
                    .into(),
            );
        }
        let mut ids = HashSet::new();
        for c in registry.iter().chain(commands.iter().map(|(v, _)| v)) {
            if c.id.is_some_and(|id| !ids.insert(id)) {
                return Err("Duplicate pending Native additional PWM command ID".into());
            }
        }
        for (command, _) in commands {
            if let Some(id) = command.id {
                self.highest_id.fetch_max(id, Ordering::Release);
            }
            control.prepared.push(command.clone());
        }
        Ok(())
    }
    pub fn commit_cancel(&self, command: &Prepared) -> bool {
        let Method::CancelMainOwner { target_group } = command.method else {
            return false;
        };
        let control = self.control.lock().expect("PWM retain validated control");
        for c in &control.prepared {
            if c.group == Some(target_group) && !c.complete.load(Ordering::Acquire) {
                c.cancelled.store(true, Ordering::Release);
            }
        }
        true
    }
    pub fn wait_applied(&self, id: u64) -> Result<(), String> {
        if !valid_id(Some(id)) || id > self.highest_id.load(Ordering::Acquire) {
            return Err("Unknown Native additional PWM command ID".into());
        }
        let command = {
            let control = self
                .control
                .lock()
                .map_err(|_| "Native additional PWM control unavailable")?;
            if let Some(command) = control.prepared.iter().find(|c| c.id == Some(id)) {
                command.clone()
            } else {
                for offset in 0..control.outcomes.len() {
                    let result =
                        control.outcomes[(control.cursor + control.outcomes.len() - 1 - offset)
                            % control.outcomes.len()];
                    if result.id == id {
                        return completion_result(result.flags);
                    }
                }
                return Err(
                    "Native additional PWM command result expired; read current status".into(),
                );
            }
        };
        let start = std::time::Instant::now();
        while !command.complete.load(Ordering::Acquire) {
            if start.elapsed() >= std::time::Duration::from_secs(10) {
                return Err(
                    "Native additional PWM command was not applied within 10 seconds".into(),
                );
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        completion_result(
            command.cancelled.load(Ordering::Acquire) as u8
                | ((command.failed.load(Ordering::Acquire) != 0) as u8) << 1,
        )
    }
    pub fn rollback(&self, command: Arc<Prepared>) {
        self.retire(command);
    }
    pub fn retire(&self, command: Arc<Prepared>) {
        // Rejected/cancelled outer events never installed a callback cursor.
        if !command.complete.load(Ordering::Acquire) {
            command.cancelled.store(true, Ordering::Release);
            if let Some(schedule) = &command.schedule {
                schedule.done.store(true, Ordering::Release);
            }
        }
        command.complete.store(true, Ordering::Release);
        command.retired.store(true, Ordering::Release);
        self.retired
            .push(command)
            .unwrap_or_else(|_| unreachable!("Bounded PWM command retirement"));
    }
    fn publish(&self, renderer: &Renderer, command: Option<&Prepared>) {
        let state = renderer.state();
        self.revision.fetch_add(1, Ordering::AcqRel);
        for (out, value) in self.registers.iter().zip(state.registers) {
            out.store(value, Ordering::Relaxed);
        }
        self.clock.store(state.clock, Ordering::Relaxed);
        self.rate.store(renderer.rate, Ordering::Relaxed);
        self.current_frame.store(renderer.frame, Ordering::Relaxed);
        self.queued
            .store(renderer.pending as u64, Ordering::Relaxed);
        self.left.store(
            if state.left_valid != 0 {
                state.left
            } else {
                u32::MAX
            },
            Ordering::Relaxed,
        );
        self.right.store(
            if state.right_valid != 0 {
                state.right
            } else {
                u32::MAX
            },
            Ordering::Relaxed,
        );
        self.left_size.store(state.left_size, Ordering::Relaxed);
        self.right_size.store(state.right_size, Ordering::Relaxed);
        for i in 0..3 {
            self.left_fifo[i].store(state.left_fifo[i], Ordering::Relaxed);
            self.right_fifo[i].store(state.right_fifo[i], Ordering::Relaxed);
        }
        self.cycle.store(state.cycle, Ordering::Relaxed);
        self.timer_tick.store(state.timer_tick, Ordering::Relaxed);
        self.interrupt_count
            .store(state.interrupt_count.to_bits(), Ordering::Relaxed);
        self.until_tick
            .store(state.until_tick.to_bits(), Ordering::Relaxed);
        self.schedule_id
            .store(renderer.last_schedule.map_or(0, |v| v.0), Ordering::Relaxed);
        self.schedule_base
            .store(renderer.last_schedule.map_or(0, |v| v.1), Ordering::Relaxed);
        self.schedule_error
            .store(renderer.last_schedule.map_or(0, |v| v.2), Ordering::Relaxed);
        if let Some(command) = command {
            command.complete.store(true, Ordering::Relaxed);
        }
        self.revision.fetch_add(1, Ordering::Release);
    }
    pub fn status(&self, frame: u64) -> Result<Status, String> {
        self.status_after(frame, None)
    }
    pub fn status_after(&self, frame: u64, after: Option<u64>) -> Result<Status, String> {
        let mut control = self
            .control
            .lock()
            .map_err(|_| "Native PWM control state is unavailable")?;
        self.collect(&mut control);
        let registry = &control.prepared;
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() >= std::time::Duration::from_millis(50) {
                return Err("Native PWM state changed while reading; retry".into());
            }
            let before = self.revision.load(Ordering::Acquire);
            if before & 1 != 0 {
                std::thread::yield_now();
                continue;
            }
            let left = self.left.load(Ordering::Relaxed);
            let right = self.right.load(Ordering::Relaxed);
            let id = self.schedule_id.load(Ordering::Relaxed);
            let mut status = ChipStatus {
                model: "mame",
                output_mode: "duty",
                gain: 1.0,
                muted: false,
                clock: self.clock(),
                sample_rate: self.rate.load(Ordering::Relaxed),
                current_frame: frame,
                queued_writes: self.queued.load(Ordering::Relaxed) as usize,
                registers: std::array::from_fn(|i| self.registers[i].load(Ordering::Relaxed)),
                left: (left != u32::MAX).then_some(left),
                right: (right != u32::MAX).then_some(right),
                cycle: self.cycle.load(Ordering::Relaxed),
                left_fifo: self.left_fifo[..self.left_size.load(Ordering::Relaxed).min(3) as usize]
                    .iter()
                    .map(|v| v.load(Ordering::Relaxed))
                    .collect(),
                right_fifo: self.right_fifo
                    [..self.right_size.load(Ordering::Relaxed).min(3) as usize]
                    .iter()
                    .map(|v| v.load(Ordering::Relaxed))
                    .collect(),
                timer_tick: self.timer_tick.load(Ordering::Relaxed),
                interrupt_count: f64::from_bits(self.interrupt_count.load(Ordering::Relaxed)),
                until_tick: f64::from_bits(self.until_tick.load(Ordering::Relaxed)),
                last_schedule: (id != 0).then(|| ScheduleResult {
                    id,
                    base_frame: self.schedule_base.load(Ordering::Relaxed),
                    error: match self.schedule_error.load(Ordering::Relaxed) {
                        1 => Some("Native PWM write queue limit reached"),
                        2 => Some("Invalid Native PWM scheduled frame"),
                        _ => None,
                    },
                }),
                frame,
                revision: before / 2,
                pending_commands: registry
                    .iter()
                    .filter(|c| !c.complete.load(Ordering::Relaxed))
                    .filter_map(|c| c.id)
                    .collect(),
            };
            if let Some(id) = after {
                if let Some(c) = registry
                    .iter()
                    .find(|c| c.id == Some(id) && c.schedule.is_some())
                {
                    status.last_schedule = Some(ScheduleResult {
                        id,
                        base_frame: c.applied_base.load(Ordering::Acquire),
                        error: (c.failed.load(Ordering::Acquire) != 0)
                            .then_some("Native additional PWM write queue limit reached"),
                    });
                } else if let Some(result) = (0..control.outcomes.len())
                    .map(|offset| {
                        &control.outcomes[(control.cursor + control.outcomes.len() - 1 - offset)
                            % control.outcomes.len()]
                    })
                    .find(|v| v.id == id && v.schedule)
                {
                    status.last_schedule = Some(ScheduleResult {
                        id,
                        base_frame: result.base,
                        error: (result.flags & 2 != 0)
                            .then_some("Native additional PWM write queue limit reached"),
                    });
                }
            }
            std::sync::atomic::fence(Ordering::Acquire);
            if self.revision.load(Ordering::Relaxed) == before {
                return Ok(Status {
                    pwm: status,
                    frame,
                    sample_rate: self.rate.load(Ordering::Acquire),
                    revision: before / 2,
                });
            }
        }
    }
}
#[derive(Debug)]
struct Entry {
    frame: u64,
    base: u64,
    order_base: u64,
    cursor: usize,
    schedule: Arc<Schedule>,
    owner: Option<Owner>,
    group: Option<u64>,
    tag: u64,
}
impl PartialEq for Entry {
    fn eq(&self, other: &Self) -> bool {
        (self.frame, self.base, self.order_base, self.cursor)
            == (other.frame, other.base, other.order_base, other.cursor)
    }
}
impl Eq for Entry {}
impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}
impl Ord for Entry {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        (self.frame, self.base, self.order_base, self.cursor).cmp(&(
            other.frame,
            other.base,
            other.order_base,
            other.cursor,
        ))
    }
}
fn completion_result(flags: u8) -> Result<(), String> {
    if flags & 1 != 0 {
        Err("Native additional PWM command was cancelled before it applied".into())
    } else if flags & 2 != 0 {
        Err("Native additional PWM schedule admission failed".into())
    } else {
        Ok(())
    }
}
#[derive(Clone, Copy, Default)]
struct Scope {
    tag: u64,
    owner: Option<Owner>,
    group: Option<u64>,
    queued: usize,
}
pub struct Renderer {
    ptr: *mut c_void,
    rate: u32,
    frame: u64,
    order: u64,
    queue: BinaryHeap<Reverse<Entry>>,
    pending: usize,
    retired_schedules: Option<Arc<ArrayQueue<Arc<Schedule>>>>,
    scopes: Box<[Scope]>,
    next_tag: u64,
    gains: [f32; 2],
    targets: [f32; 2],
    ramp: u32,
    last_schedule: Option<(u64, u64, u32)>,
}
unsafe impl Send for Renderer {}
impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe {
            native_pwm_destroy(self.ptr);
        }
    }
}
impl Renderer {
    pub fn new(rate: u32) -> Result<Self, String> {
        let ptr = unsafe { native_pwm_create(rate) };
        if ptr.is_null() {
            return Err("Could not allocate Native PWM core".into());
        }
        Ok(Self {
            ptr,
            rate,
            frame: 0,
            order: 0,
            queue: BinaryHeap::with_capacity(CURSORS),
            pending: 0,
            retired_schedules: None,
            scopes: vec![Scope::default(); SCOPES].into_boxed_slice(),
            next_tag: 1,
            gains: [1.0; 2],
            targets: [1.0; 2],
            ramp: 0,
            last_schedule: None,
        })
    }
    fn state(&self) -> CoreState {
        let mut state = CoreState::default();
        unsafe {
            native_pwm_get_state(self.ptr, &mut state);
        }
        state
    }
    pub fn set_frame(&mut self, frame: u64) {
        self.frame = frame;
    }
    fn scope_tag(&mut self, owner: Option<Owner>, group: Option<u64>) -> u64 {
        let group = if owner.is_some() { None } else { group };
        if owner.is_none() && group.is_none() {
            return 0;
        }
        if let Some(scope) = self
            .scopes
            .iter()
            .find(|s| s.tag != 0 && s.owner == owner && s.group == group)
        {
            return scope.tag;
        }
        for scope in &mut self.scopes {
            if scope.tag != 0
                && scope.queued == 0
                && unsafe { native_pwm_has_tag(self.ptr, scope.tag) == 0 }
            {
                scope.tag = 0;
            }
        }
        let index = self
            .scopes
            .iter()
            .position(|s| s.tag == 0)
            .expect("Bounded PWM live scope tags plus one in-flight scope");
        let tag = self
            .next_tag
            .checked_mul(SCOPES as u64)
            .and_then(|v| v.checked_add(index as u64 + 1))
            .expect("PWM scope generation expired");
        self.next_tag += 1;
        self.scopes[index] = Scope {
            tag,
            owner,
            group,
            queued: 0,
        };
        tag
    }
    fn write(&mut self, register: u8, value: u16, tag: u64) {
        unsafe {
            native_pwm_write_tagged(self.ptr, register as u32, value as u32, tag);
        }
    }
    fn cancel_scopes(&mut self, valid: impl Fn(&Scope) -> bool) {
        for scope in &mut self.scopes {
            if scope.tag != 0 && !valid(scope) {
                unsafe {
                    native_pwm_cancel_tag(self.ptr, scope.tag);
                }
                if scope.queued == 0 {
                    scope.tag = 0;
                }
            }
        }
    }
    fn clear(&mut self, owner: Option<Owner>) {
        self.retain_schedules(|entry| owner.is_some() && entry.owner != owner);
    }
    fn clear_before(
        &mut self,
        owner: Option<Owner>,
        group: Option<u64>,
        order: Option<u64>,
        cancel_future: bool,
    ) {
        let frame = self.frame;
        // Future Worker reservations are represented here before the browser
        // would have admitted them. An older scheduled Reset/clear must not
        // erase a later logical reservation at that future frame.
        self.retain_schedules(|entry| {
            owner.is_some() && entry.owner != owner
                || owner.is_none()
                    && group.is_some()
                    && (entry.owner.is_some() || entry.group != group)
                || order.is_some_and(|order| {
                    entry.base > frame && !cancel_future
                        || entry.base >= frame && entry.order_base > order
                })
        });
    }
    fn retire_schedule(&self, schedule: Arc<Schedule>) {
        schedule.done.store(true, Ordering::Release);
        self.retired_schedules
            .as_ref()
            .expect("PWM callback retirement queue installed before scheduling")
            .push(schedule)
            .unwrap_or_else(|_| unreachable!("Bounded PWM schedule retirement"));
    }
    fn retain_schedules(&mut self, valid: impl Fn(&Entry) -> bool) {
        let retired = &self.retired_schedules;
        let scopes = &mut self.scopes;
        let mut removed = 0;
        self.queue.retain(|Reverse(entry)| {
            if valid(entry) {
                return true;
            }
            removed += entry.schedule.entries.len() - entry.cursor;
            if entry.tag != 0 {
                scopes[((entry.tag - 1) % SCOPES as u64) as usize].queued -= 1;
            }
            entry.schedule.done.store(true, Ordering::Release);
            retired
                .as_ref()
                .expect("PWM retirement queue installed")
                .push(entry.schedule.clone())
                .unwrap_or_else(|_| unreachable!("Bounded PWM schedule retirement"));
            false
        });
        self.pending -= removed;
    }
    fn silence(&mut self) {
        unsafe {
            native_pwm_stop(self.ptr);
        }
        self.scopes.fill(Scope::default());
    }
    pub fn apply(&mut self, command: &Prepared, owner: Option<Owner>, shared: &Shared) {
        if self.retired_schedules.is_none() {
            self.retired_schedules = Some(shared.retired_schedules.clone());
        }
        match &command.method {
            Method::Write { register, value } => {
                let tag = if *register >= 2 {
                    self.scope_tag(owner, command.group)
                } else {
                    0
                };
                self.write(*register, *value, tag);
            }
            Method::ScheduleWrites {
                entries: _,
                base_frame,
            } => {
                let base = base_frame.unwrap_or(self.frame).max(self.frame);
                let schedule = command.schedule.as_ref().expect("Prepared PWM schedule");
                let entries = &schedule.entries;
                // One cursor is admitted in O(log active batches). Neither
                // validation nor admission scans the uploaded write array here.
                let error = if self.pending + entries.len() > QUEUE || self.queue.len() >= CURSORS {
                    1
                } else if base > MAX_SAFE_INTEGER
                    || entries
                        .last()
                        .is_some_and(|entry| entry.frame > MAX_SAFE_INTEGER - base)
                {
                    2
                } else {
                    0
                };
                if error == 0 && !entries.is_empty() {
                    let tag = self.scope_tag(owner, command.group);
                    if tag != 0 {
                        self.scopes[((tag - 1) % SCOPES as u64) as usize].queued += 1;
                    }
                    self.queue.push(Reverse(Entry {
                        frame: base + entries[0].frame,
                        base,
                        order_base: command.order.unwrap_or(self.order),
                        cursor: 0,
                        schedule: schedule.clone(),
                        owner,
                        group: command.group,
                        tag,
                    }));
                    self.order = self.order.wrapping_add(entries.len() as u64);
                    self.pending += entries.len();
                } else {
                    schedule.done.store(true, Ordering::Release);
                }
                command.applied_base.store(base, Ordering::Release);
                command.failed.store(error, Ordering::Release);
                self.last_schedule = command.id.map(|id| (id, base, error));
            }
            Method::ClearSchedule => self.clear_before(owner, command.group, command.order, false),
            Method::Reset => {
                self.clear_before(owner, command.group, command.order, true);
                // An explicit public reset changes the entire hardware state.
                // Automatic owner cleanup uses tagged Stop/Cancel instead.
                unsafe {
                    native_pwm_reset(self.ptr);
                }
                for scope in &mut self.scopes {
                    if scope.queued == 0 {
                        *scope = Scope::default();
                    }
                }
            }
            Method::StopVoices => {
                if let Some(group) = command.group {
                    self.retain_schedules(|entry| {
                        entry.owner.is_some() || entry.group != Some(group)
                    });
                    self.cancel_scopes(|s| s.owner.is_some() || s.group != Some(group));
                } else if let Some(owner) = owner {
                    self.clear(Some(owner));
                    self.cancel_scopes(|s| s.owner != Some(owner));
                } else {
                    self.clear(None);
                    self.silence();
                }
            }
            Method::CancelMainOwner { target_group } => {
                self.retain_schedules(|entry| {
                    entry.owner.is_some() || entry.group != Some(*target_group)
                });
                self.cancel_scopes(|s| s.owner.is_some() || s.group != Some(*target_group));
            }
            Method::Clock { hz } => unsafe {
                native_pwm_set_clock(self.ptr, *hz);
            },
            Method::Mix { volume, pan } => {
                self.targets = [
                    *volume * if *pan > 0.0 { 1.0 - *pan } else { 1.0 },
                    *volume * if *pan < 0.0 { 1.0 + *pan } else { 1.0 },
                ];
                self.ramp = (self.rate / 200).max(1);
            }
            Method::Barrier => {}
        }
        shared.publish(self, Some(command));
    }
    fn due(&mut self, allow_admission: bool) {
        while self
            .queue
            .peek()
            .is_some_and(|entry| entry.0.frame <= self.frame)
        {
            // An ASAP-admitted future reservation becomes logically visible
            // only after ordinary outer commands (notably Reset) at its base.
            if !allow_admission
                && self
                    .queue
                    .peek()
                    .is_some_and(|entry| entry.0.base >= self.frame)
            {
                break;
            }
            let mut entry = self.queue.pop().unwrap().0;
            let write = entry.schedule.entries[entry.cursor];
            self.write(write.register, write.value, entry.tag);
            entry.cursor += 1;
            self.pending -= 1;
            if let Some(next) = entry.schedule.entries.get(entry.cursor) {
                entry.frame = entry.base + next.frame;
                self.queue.push(Reverse(entry));
            } else {
                if entry.tag != 0 {
                    self.scopes[((entry.tag - 1) % SCOPES as u64) as usize].queued -= 1;
                }
                self.retire_schedule(entry.schedule);
            }
        }
    }
    pub fn publish(&self, shared: &Shared) {
        shared.publish(self, None);
    }
    pub fn cancel_invalid(&mut self, valid: impl Fn(Option<Owner>) -> bool, shared: &Shared) {
        self.retain_schedules(|entry| valid(entry.owner));
        self.cancel_scopes(|scope| valid(scope.owner));
        self.publish(shared);
    }
    #[cfg(test)]
    pub fn layout_bytes() -> (usize, usize, usize) {
        (
            std::mem::size_of::<Entry>(),
            std::mem::size_of::<Scope>(),
            std::mem::size_of::<Completion>(),
        )
    }
    pub fn mix_additional(&mut self, pcm: &mut [f32; 2], shared: &Shared) {
        let before = self.pending;
        self.mix(pcm);
        // Publish exact due writes immediately; idle FIFO/timer observation is
        // refreshed once per render quantum, without atomics on every sample.
        if self.pending != before || self.frame % 128 == 0 {
            self.publish(shared);
        }
    }
    pub fn mix(&mut self, pcm: &mut [f32; 2]) {
        self.due(true);
        let mut sample = [0.0; 2];
        unsafe {
            native_pwm_sample(self.ptr, &mut sample[0], &mut sample[1]);
        }
        self.frame += 1;
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
        // PWM32XPlayback applies writes exactly at the right edge as well, even
        // when generateStereo(0) is used to inspect an immediate reservation.
        self.due(false);
    }
}
