//! Callback-owned 32X PWM. The fixed scheduling heap is allocated before audio
//! starts; prepared upload storage is retained and reclaimed on the control side.
use super::{Owner, CAPACITY, MAX_SAFE_INTEGER};
use crossbeam_queue::ArrayQueue;
use serde::{Deserialize, Serialize};
use std::{
    cmp::{Ordering as CmpOrdering, Reverse},
    collections::BinaryHeap,
    ffi::c_void,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        Arc, Mutex,
    },
};

pub const CLOCK: u32 = 23_011_361;
const QUEUE: usize = 1_000_000;
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
    fn native_pwm_write(ptr: *mut c_void, register: u32, value: u32);
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
#[derive(Debug, Deserialize)]
pub struct Message {
    #[serde(default)]
    pub id: Option<u64>,
    #[serde(default)]
    pub order: Option<u64>,
    #[serde(flatten)]
    pub method: Method,
}
#[derive(Debug, Deserialize)]
#[serde(tag = "method", rename_all = "kebab-case")]
pub enum Method {
    Write {
        register: u8,
        value: u16,
    },
    Reset,
    StopVoices,
    ClearSchedule,
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
    pub fn prepare(&self) -> Result<Arc<Prepared>, String> {
        if !valid_id(self.id) || !valid_id(self.order) {
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
            Method::Barrier => Method::Barrier,
        };
        Ok(Arc::new(Prepared {
            id: self.id,
            method,
            complete: AtomicBool::new(false),
            retired: AtomicBool::new(false),
            schedule,
            order: self.order,
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
        matches!(self.method, Method::StopVoices | Method::Reset)
    }
    fn entries(&self) -> usize {
        self.schedule
            .as_ref()
            .map_or(0, |schedule| schedule.entries.len())
    }
    fn bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.entries() * std::mem::size_of::<Write>()
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
    prepared: Mutex<Vec<Arc<Prepared>>>,
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
            prepared: Mutex::new(Vec::new()),
            retired: ArrayQueue::new(CAPACITY),
            retired_schedules: Arc::new(ArrayQueue::new(CAPACITY * 2)),
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
pub struct Status {
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
impl Shared {
    pub fn clock(&self) -> u32 {
        self.clock.load(Ordering::Acquire)
    }
    fn collect(&self, registry: &mut Vec<Arc<Prepared>>) {
        while self.retired.pop().is_some() {}
        while self.retired_schedules.pop().is_some() {}
        registry.retain(|command| {
            !command.retired.load(Ordering::Acquire)
                || command
                    .schedule
                    .as_ref()
                    .is_some_and(|schedule| !schedule.done.load(Ordering::Acquire))
        });
    }
    pub fn retain(&self, commands: &[Arc<Prepared>]) -> Result<(), String> {
        let mut registry = self
            .prepared
            .lock()
            .map_err(|_| "Native PWM control state is unavailable")?;
        self.collect(&mut registry);
        if commands.len() + registry.len() > CAPACITY
            || registry
                .iter()
                .chain(commands)
                .map(|c| c.bytes())
                .sum::<usize>()
                > PENDING_BYTES
        {
            return Err("Native PWM pending command limit reached (64 MiB)".into());
        }
        // A callback may consume between reads; counting completed snapshots and
        // pending uploads conservatively prevents audio-side heap growth.
        let pending: usize = registry
            .iter()
            .chain(commands)
            .filter(|c| !c.complete.load(Ordering::Acquire))
            .map(|c| c.entries())
            .sum();
        if self.queued.load(Ordering::Acquire) as usize + pending > QUEUE {
            return Err("Native PWM write queue limit reached (1000000 entries)".into());
        }
        let mut ids = std::collections::HashSet::new();
        for command in registry.iter().chain(commands) {
            if command.id.is_some_and(|id| !ids.insert(id)) {
                return Err("Duplicate pending Native PWM command ID".into());
            }
        }
        registry.extend(commands.iter().cloned());
        for command in commands {
            if let Some(id) = command.id {
                self.highest_id.fetch_max(id, Ordering::Release);
            }
        }
        Ok(())
    }
    pub fn wait_applied(&self, id: u64) -> Result<(), String> {
        if !valid_id(Some(id)) || id > self.highest_id.load(Ordering::Acquire) {
            return Err("Unknown Native PWM applied command ID".into());
        }
        let command = {
            self.prepared
                .lock()
                .map_err(|_| "Native PWM control state is unavailable")?
                .iter()
                .find(|c| c.id == Some(id))
                .cloned()
        };
        if let Some(command) = command {
            let start = std::time::Instant::now();
            while !command.complete.load(Ordering::Acquire) {
                if start.elapsed() >= std::time::Duration::from_millis(100) {
                    return Err("Native PWM command was not applied within 100 ms".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        Ok(())
    }
    pub fn retire(&self, command: Arc<Prepared>) {
        // Rejected/cancelled outer events never installed a callback cursor.
        if !command.complete.load(Ordering::Acquire) {
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
        let mut registry = self
            .prepared
            .lock()
            .map_err(|_| "Native PWM control state is unavailable")?;
        self.collect(&mut registry);
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
            let status = Status {
                model: "mame",
                output_mode: "duty",
                gain: 1.0,
                muted: false,
                clock: self.clock(),
                sample_rate: self.rate.load(Ordering::Relaxed),
                current_frame: self.current_frame.load(Ordering::Relaxed),
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
            std::sync::atomic::fence(Ordering::Acquire);
            if self.revision.load(Ordering::Relaxed) == before {
                return Ok(status);
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
pub struct Renderer {
    ptr: *mut c_void,
    rate: u32,
    frame: u64,
    order: u64,
    queue: BinaryHeap<Reverse<Entry>>,
    pending: usize,
    retired_schedules: Option<Arc<ArrayQueue<Arc<Schedule>>>>,
    owner: Option<Option<Owner>>,
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
            queue: BinaryHeap::with_capacity(CAPACITY),
            pending: 0,
            retired_schedules: None,
            owner: None,
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
    fn write(&mut self, register: u8, value: u16, owner: Option<Owner>) {
        unsafe {
            native_pwm_write(self.ptr, register as u32, value as u32);
        }
        if register >= 2 {
            self.owner = Some(owner);
        }
    }
    fn clear(&mut self, owner: Option<Owner>) {
        self.retain_schedules(|entry| owner.is_some() && entry.owner != owner);
    }
    fn clear_before(&mut self, owner: Option<Owner>, order: Option<u64>, cancel_future: bool) {
        let frame = self.frame;
        // Future Worker reservations are represented here before the browser
        // would have admitted them. An older scheduled Reset/clear must not
        // erase a later logical reservation at that future frame.
        self.retain_schedules(|entry| {
            owner.is_some() && entry.owner != owner
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
        let mut removed = 0;
        self.queue.retain(|Reverse(entry)| {
            if valid(entry) {
                return true;
            }
            removed += entry.schedule.entries.len() - entry.cursor;
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
        self.owner = None;
    }
    pub fn apply(&mut self, command: &Prepared, owner: Option<Owner>, shared: &Shared) {
        if self.retired_schedules.is_none() {
            self.retired_schedules = Some(shared.retired_schedules.clone());
        }
        match &command.method {
            Method::Write { register, value } => self.write(*register, *value, owner),
            Method::ScheduleWrites {
                entries: _,
                base_frame,
            } => {
                let base = base_frame.unwrap_or(self.frame).max(self.frame);
                let schedule = command.schedule.as_ref().expect("Prepared PWM schedule");
                let entries = &schedule.entries;
                // One cursor is admitted in O(log active batches). Neither
                // validation nor admission scans the uploaded write array here.
                let error = if self.pending + entries.len() > QUEUE || self.queue.len() >= CAPACITY
                {
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
                    self.queue.push(Reverse(Entry {
                        frame: base + entries[0].frame,
                        base,
                        order_base: command.order.unwrap_or(self.order),
                        cursor: 0,
                        schedule: schedule.clone(),
                        owner,
                    }));
                    self.order = self.order.wrapping_add(entries.len() as u64);
                    self.pending += entries.len();
                } else {
                    schedule.done.store(true, Ordering::Release);
                }
                self.last_schedule = command.id.map(|id| (id, base, error));
            }
            Method::ClearSchedule => self.clear_before(owner, command.order, false),
            Method::Reset => {
                self.clear_before(owner, command.order, true);
                if owner.is_none() || self.owner.is_none_or(|current| current == owner) {
                    unsafe {
                        native_pwm_reset(self.ptr);
                    }
                    self.owner = None;
                }
            }
            Method::StopVoices => {
                self.clear(owner);
                if owner.is_none() || self.owner == Some(owner) {
                    self.silence();
                }
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
            self.write(write.register, write.value, entry.owner);
            entry.cursor += 1;
            self.pending -= 1;
            if let Some(next) = entry.schedule.entries.get(entry.cursor) {
                entry.frame = entry.base + next.frame;
                self.queue.push(Reverse(entry));
            } else {
                self.retire_schedule(entry.schedule);
            }
        }
    }
    pub fn publish(&self, shared: &Shared) {
        shared.publish(self, None);
    }
    pub fn cancel_invalid(&mut self, valid: impl Fn(Option<Owner>) -> bool, shared: &Shared) {
        let previous = self.pending;
        self.retain_schedules(|entry| valid(entry.owner));
        let silence = self.owner.is_some_and(|owner| !valid(owner));
        if silence {
            self.silence();
        }
        if silence || previous != self.pending {
            self.publish(shared);
        }
    }
    pub fn stop(&mut self, shared: &Shared) {
        self.clear(None);
        self.silence();
        self.publish(shared);
    }
    pub fn reset(&mut self, shared: &Shared) {
        self.clear(None);
        unsafe {
            native_pwm_reset(self.ptr);
        }
        self.owner = None;
        self.publish(shared);
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    fn apply(shared: &Shared, renderer: &mut Renderer, value: Value, owner: Option<Owner>) {
        let message: Message = serde_json::from_value(value).unwrap();
        let prepared = message.prepare().unwrap();
        shared.retain(&[prepared.clone()]).unwrap();
        renderer.apply(&prepared, owner, shared);
        shared.retire(prepared);
    }
    fn setup() -> (Shared, Renderer) {
        (Shared::default(), Renderer::new(48000).unwrap())
    }
    #[test]
    fn scheduled_writes_are_stable_relative_to_the_next_output_frame() {
        let (shared, mut renderer) = setup();
        apply(
            &shared,
            &mut renderer,
            json!({"method":"schedule-writes","id":1,"entries":[{"frame":2,"register":1,"value":1},{"frame":2,"register":2,"value":100},{"frame":2,"register":2,"value":200},{"frame":2,"register":2,"value":300},{"frame":2,"register":2,"value":400}]}),
            None,
        );
        assert_eq!(
            shared.status(0).unwrap().last_schedule.unwrap().base_frame,
            0
        );
        renderer.mix(&mut [0.0; 2]);
        assert_eq!(renderer.state().left_size, 0);
        renderer.mix(&mut [0.0; 2]);
        let state = renderer.state();
        assert_eq!(state.left_size, 3);
        assert_eq!(state.left_fifo, [200, 300, 400]);
        assert_eq!(renderer.frame, 2);
        renderer.publish(&shared);
        assert_eq!(shared.status(2).unwrap().queued_writes, 0);
    }
    #[test]
    fn absolute_worker_reservations_are_admitted_immediately() {
        let (shared, mut renderer) = setup();
        apply(
            &shared,
            &mut renderer,
            json!({"method":"schedule-writes","id":1,"baseFrame":14400,"entries":[{"frame":0,"register":2,"value":99}]}),
            None,
        );
        shared.wait_applied(1).unwrap();
        let status = shared.status(0).unwrap();
        assert_eq!(status.last_schedule.unwrap().base_frame, 14400);
        assert_eq!(status.current_frame, 0);
        assert_eq!(status.queued_writes, 1);
    }
    #[test]
    fn stop_preserves_clock_routing_and_cycle_and_cannot_be_revived_by_cancelled_schedule() {
        let (shared, mut renderer) = setup();
        let owner = Some(Owner {
            performance: 1,
            owner: 1,
        });
        for (register, value) in [(0, 5), (1, 100), (4, 200)] {
            apply(
                &shared,
                &mut renderer,
                json!({"method":"write","register":register,"value":value}),
                owner,
            );
        }
        apply(
            &shared,
            &mut renderer,
            json!({"method":"clock","hz":12345678}),
            None,
        );
        apply(
            &shared,
            &mut renderer,
            json!({"method":"schedule-writes","entries":[{"frame":100,"register":4,"value":300}]}),
            owner,
        );
        renderer.stop(&shared);
        let state = shared.status(0).unwrap();
        assert_eq!(
            (state.clock, state.registers[0], state.registers[1]),
            (12345678, 5, 100)
        );
        assert!(state.left.is_none());
        assert_eq!(state.queued_writes, 0);
        for _ in 0..200 {
            let mut pcm = [0.0; 2];
            renderer.mix(&mut pcm);
            assert_eq!(pcm, [0.0; 2]);
        }
    }
    #[test]
    fn owner_cancellation_and_scoped_clear_preserve_other_reservations() {
        let (shared, mut renderer) = setup();
        let old = Some(Owner {
            performance: 1,
            owner: 1,
        });
        let new = Some(Owner {
            performance: 1,
            owner: 2,
        });
        for owner in [old, new] {
            apply(
                &shared,
                &mut renderer,
                json!({"method":"schedule-writes","entries":[{"frame":200,"register":4,"value":200}]}),
                owner,
            );
        }
        apply(
            &shared,
            &mut renderer,
            json!({"method":"clear-schedule"}),
            old,
        );
        assert_eq!(renderer.queue.len(), 1);
        renderer.cancel_invalid(|owner| owner != old, &shared);
        assert_eq!(renderer.queue.len(), 1);
        renderer.cancel_invalid(|owner| owner != new, &shared);
        assert_eq!(renderer.queue.len(), 0);
    }
    #[test]
    fn reset_preserves_output_frame_and_configured_clock() {
        let (shared, mut renderer) = setup();
        renderer.mix(&mut [0.0; 2]);
        apply(
            &shared,
            &mut renderer,
            json!({"method":"clock","hz":1000000}),
            None,
        );
        renderer.reset(&shared);
        let status = shared.status(1).unwrap();
        assert_eq!(status.current_frame, 1);
        assert_eq!(status.clock, 1000000);
        assert_eq!(status.registers[0], 0);
    }
    #[test]
    fn validates_all_uploads_before_admission() {
        for value in [
            json!({"method":"write","register":5,"value":1}),
            json!({"method":"clock","hz":0}),
            json!({"method":"schedule-writes","entries":[{"frame":MAX_SAFE_INTEGER+1,"register":2,"value":1}]}),
            json!({"method":"schedule-writes","baseFrame":MAX_SAFE_INTEGER,"entries":[{"frame":1,"register":2,"value":1}]}),
        ] {
            assert!(serde_json::from_value::<Message>(value)
                .unwrap()
                .prepare()
                .is_err());
        }
    }
    #[test]
    fn large_upload_admits_one_cursor_and_keeps_storage_until_control_reclamation() {
        let (shared, mut renderer) = setup();
        let message = Message {
            id: Some(1),
            order: None,
            method: Method::ScheduleWrites {
                entries: (0..100000)
                    .rev()
                    .map(|i| Write {
                        frame: 1000 + i,
                        register: 4,
                        value: (i % 4096) as u16,
                    })
                    .collect(),
                base_frame: None,
            },
        };
        let prepared = message.prepare().unwrap();
        let weak = Arc::downgrade(prepared.schedule.as_ref().unwrap());
        shared.retain(&[prepared.clone()]).unwrap();
        let start = std::time::Instant::now();
        renderer.apply(&prepared, None, &shared);
        let admission = start.elapsed();
        shared.retire(prepared.clone());
        drop(prepared);
        assert_eq!(renderer.queue.len(), 1);
        assert_eq!(renderer.pending, 100000);
        assert_eq!(shared.status(0).unwrap().queued_writes, 100000);
        assert_eq!(weak.strong_count(), 2); // control registry + callback cursor
        renderer.stop(&shared);
        assert!(weak.strong_count() > 0); // callback has handed the Arc to retirement
        assert_eq!(shared.status(0).unwrap().queued_writes, 0);
        assert_eq!(weak.strong_count(), 0); // final free occurred on this control read
        eprintln!(
            "Native PWM 100000-write callback admission: {:?}, one cursor",
            admission
        );
    }
    #[test]
    fn sorted_batch_cursors_preserve_equal_frame_order_across_uploads() {
        let (shared, mut renderer) = setup();
        apply(
            &shared,
            &mut renderer,
            json!({"method":"write","register":1,"value":1}),
            None,
        );
        apply(
            &shared,
            &mut renderer,
            json!({"method":"schedule-writes","entries":[{"frame":2,"register":2,"value":300},{"frame":1,"register":2,"value":100},{"frame":2,"register":2,"value":400}]}),
            None,
        );
        apply(
            &shared,
            &mut renderer,
            json!({"method":"schedule-writes","entries":[{"frame":2,"register":2,"value":500}]}),
            None,
        );
        renderer.mix(&mut [0.0; 2]);
        renderer.mix(&mut [0.0; 2]);
        assert_eq!(renderer.state().left_fifo, [300, 400, 500]);
        assert_eq!(renderer.pending, 0);
        renderer.publish(&shared);
        assert_eq!(shared.status(2).unwrap().queued_writes, 0);
    }
    #[test]
    fn cancelled_unapplied_upload_and_overflow_do_not_retain_schedule_storage() {
        let (shared, mut renderer) = setup();
        let message: Message = serde_json::from_value(
            json!({"method":"schedule-writes","entries":[{"frame":1,"register":2,"value":1}]}),
        )
        .unwrap();
        let prepared = message.prepare().unwrap();
        let weak = Arc::downgrade(prepared.schedule.as_ref().unwrap());
        shared.retain(&[prepared.clone()]).unwrap();
        shared.retire(prepared);
        shared.status(0).unwrap();
        assert_eq!(weak.strong_count(), 0);
        renderer.mix(&mut [0.0; 2]);
        apply(
            &shared,
            &mut renderer,
            json!({"method":"schedule-writes","id":1,"entries":[{"frame":MAX_SAFE_INTEGER,"register":2,"value":1}]}),
            None,
        );
        assert_eq!(
            shared.status(1).unwrap().last_schedule.unwrap().error,
            Some("Invalid Native PWM scheduled frame")
        );
        assert_eq!(renderer.pending, 0);
    }
}
