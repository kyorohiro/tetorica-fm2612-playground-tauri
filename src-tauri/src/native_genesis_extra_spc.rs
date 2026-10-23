//! Whole-machine SPC700 playback. Private cores are loaded/seeked on control;
//! published cores belong exclusively to one audio callback until retirement.
use super::{extra_budget, Owner, CAPACITY, MAX_SAFE_INTEGER};
use crossbeam_queue::ArrayQueue;
use serde::{Deserialize, Serialize};
use std::{
    cell::UnsafeCell,
    ffi::{c_void, CStr},
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, AtomicUsize, Ordering},
        Arc, Mutex, OnceLock,
    },
};
pub const IMAGE: usize = 0x10200;
pub const INSTANCE_BYTES: usize = extra_budget::INSTANCE_BYTES + 1024 * 1024 + 512 * 1024;
const EVENTS: usize = 8192;
const PACKET: usize = 2048;
const SCOPES: usize = 1024;
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
struct CoreState {
    native_frames: u64,
    base_native_frames: u64,
    output_frames: u64,
    playback_native_frames: f64,
    sample_rate: u32,
    loaded: u32,
    playing: u32,
    mute_mask: u32,
    failed: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
struct Event {
    clock: u64,
    reg: u32,
    value: u32,
}
extern "C" {
    fn native_spc_create(rate: u32) -> *mut c_void;
    fn native_spc_destroy(p: *mut c_void);
    fn native_spc_core_size() -> u32;
    fn native_spc_error(p: *const c_void) -> *const std::ffi::c_char;
    fn native_spc_load_control(p: *mut c_void, data: *const u8, size: u32) -> i32;
    fn native_spc_seek_control(p: *mut c_void, frames: u32) -> i32;
    fn native_spc_set_playing(p: *mut c_void, on: u32) -> i32;
    fn native_spc_set_mute_mask(p: *mut c_void, mask: u32) -> i32;
    fn native_spc_sample(p: *mut c_void, left: *mut f32, right: *mut f32);
    fn native_spc_get_state(p: *const c_void, state: *mut CoreState);
    fn native_spc_snapshot(p: *mut c_void, out: *mut u8, size: u32) -> i32;
    fn native_spc_read_dsp_registers(p: *const c_void, out: *mut u8);
    fn native_spc_observe(p: *mut c_void, on: u32, capacity: u32) -> i32;
    fn native_spc_event_data(p: *const c_void) -> *const Event;
    fn native_spc_event_count(p: *const c_void) -> u32;
    fn native_spc_events_clear(p: *mut c_void);
    fn native_spc_overflow_take(p: *mut c_void) -> u32;
}
#[derive(Clone, Copy, Default, Debug, Deserialize, PartialEq, Eq)]
pub struct Scope {
    pub session: u64,
    pub owner: u64,
}
fn valid(v: u64) -> bool {
    v > 0 && v <= MAX_SAFE_INTEGER
}
#[derive(Clone, Debug, Deserialize)]
pub struct Message {
    #[serde(default)]
    pub id: Option<u64>,
    #[serde(default, rename = "sourceOwner")]
    pub source_owner: Option<Scope>,
    #[serde(default, rename = "eventEpoch")]
    pub event_epoch: u64,
    #[serde(flatten)]
    pub method: Method,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "method", rename_all = "kebab-case")]
pub enum Method {
    LoadSpc {
        data: Vec<u8>,
    },
    Play,
    Pause,
    Stop,
    Mute {
        mask: u8,
    },
    Barrier,
    Snapshot,
    SeekBegin {
        #[serde(rename = "seekId")]
        seek_id: u64,
    },
    SeekEnd {
        #[serde(rename = "seekId")]
        seek_id: u64,
        resume: bool,
    },
    Observe {
        enabled: bool,
        epoch: u64,
        #[serde(rename = "intervalMs")]
        interval_ms: f64,
    },
    EventAck {
        epoch: u64,
        sequence: u64,
    },
    CancelOwner {
        session: u64,
        owner: u64,
    },
    CancelPerformance {
        session: u64,
    },
    Mix {
        volume: f32,
        pan: f32,
    },
}
struct File {
    data: Vec<u8>,
    _memory: extra_budget::Permit,
}
struct Core {
    ptr: *mut c_void,
    file: Arc<File>,
    _memory: extra_budget::Permit,
}
unsafe impl Send for Core {}
unsafe impl Sync for Core {}
impl Drop for Core {
    fn drop(&mut self) {
        unsafe { native_spc_destroy(self.ptr) }
    }
}
impl Core {
    fn state(&self) -> CoreState {
        let mut s = CoreState::default();
        unsafe { native_spc_get_state(self.ptr, &mut s) }
        s
    }
    fn error(&self) -> String {
        unsafe { CStr::from_ptr(native_spc_error(self.ptr)) }
            .to_string_lossy()
            .into_owned()
    }
}
struct Job {
    id: u64,
    scope: Option<Scope>,
    basis: Option<Arc<Prepared>>,
    core: Mutex<Arc<Core>>,
    cancelled: AtomicBool,
    sealed: AtomicBool,
    epoch: AtomicU64,
}
struct Capture {
    data: UnsafeCell<Box<[u8]>>,
    state: UnsafeCell<CoreState>,
    ready: AtomicBool,
    delivered: AtomicBool,
    created: std::time::Instant,
    _memory: extra_budget::Permit,
}
// One writer, private control or callback, followed by immutable control reads
// after Prepared completion Release/Acquire. Registry guards final destruction.
unsafe impl Sync for Capture {}
impl Capture {
    fn record(&self, core: &Core) {
        unsafe {
            native_spc_snapshot(core.ptr, (&mut *self.data.get()).as_mut_ptr(), IMAGE as u32);
            *self.state.get() = core.state();
        }
        self.ready.store(true, Ordering::Release);
    }
    fn snapshot(&self) -> Snapshot {
        assert!(self.ready.load(Ordering::Acquire));
        let bytes = unsafe { &*self.data.get() };
        let s = unsafe { *self.state.get() };
        Snapshot {
            ram: bytes[0x100..0x10100].to_vec(),
            dsp: bytes[0x10100..0x10180].to_vec(),
            native_frames: s.native_frames,
            position_seconds: s.playback_native_frames / 32000.,
            playback_native_frames: s.playback_native_frames,
            native_sample_rate: 32000,
            mute_mask: s.mute_mask as u8,
        }
    }
}
enum Edit {
    Install(Arc<Core>),
    Play,
    Pause,
    Stop(Option<Arc<Core>>),
    Mute(u8),
    Barrier,
    Snapshot(Arc<Capture>),
    SeekBegin(Arc<Job>, Arc<Core>),
    SeekEnd(Arc<Job>, bool),
    Observe(bool, u64),
    Ack(u64, u64),
    CancelOwner(Scope),
    CancelPerformance(u64),
    Mix(f32, f32),
}
pub struct Prepared {
    pub id: Option<u64>,
    scope: Option<Scope>,
    event_epoch: u64,
    edit: Edit,
    install: OnceLock<Arc<Core>>,
    basis: OnceLock<Arc<Prepared>>,
    applied: AtomicBool,
    complete: AtomicBool,
    retired: AtomicBool,
    cancelled: AtomicBool,
    rejected: AtomicBool,
}
impl std::fmt::Debug for Prepared {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.debug_struct("PreparedSpc").field("id", &self.id).finish()
    }
}
impl Message {
    pub fn prepared_bytes(&self) -> usize {
        std::mem::size_of::<Prepared>()
    }
    pub fn capture_bytes(&self) -> usize {
        if matches!(self.method, Method::Snapshot) {
            IMAGE
        } else {
            0
        }
    }
}
impl Prepared {
    pub fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
    }
    pub fn capture_bytes(&self) -> usize {
        if matches!(self.edit, Edit::Snapshot(_)) {
            IMAGE
        } else {
            0
        }
    }
    pub fn starts(&self) -> bool {
        matches!(self.edit, Edit::Play | Edit::SeekEnd(_, true))
    }
    pub fn stops(&self) -> bool {
        false
    }
    pub fn is_owner_cancel(&self) -> bool {
        matches!(self.edit, Edit::CancelOwner(_) | Edit::CancelPerformance(_))
    }
}
#[derive(Clone, Copy, Default)]
struct Completion {
    id: u64,
    flags: u8,
}
struct Control {
    prepared: Vec<Arc<Prepared>>,
    outcomes: Box<[Completion]>,
    cursor: usize,
    cores: Vec<Arc<Core>>,
    highest: u64,
}
impl Default for Control {
    fn default() -> Self {
        Self {
            prepared: Vec::new(),
            outcomes: vec![Completion::default(); 512].into_boxed_slice(),
            cursor: 0,
            cores: Vec::new(),
            highest: 0,
        }
    }
}
#[derive(Clone, Copy, Default)]
struct Queued {
    generation: u64,
    event: Event,
}
#[derive(Default)]
struct Delivery {
    generation: u64,
    epoch: u64,
    sequence: u64,
    inflight: Vec<u64>,
    reset_seen: u64,
    initial: bool,
}
pub struct Shared {
    rate: u32,
    budget: Arc<extra_budget::Budget>,
    control: Mutex<Control>,
    job: Mutex<Option<Arc<Job>>>,
    seek_epoch: AtomicU64,
    scope_session: AtomicU64,
    stopped_session: AtomicU64,
    cancelled_owners: Box<[AtomicU64]>,
    cancelled_count: AtomicU32,
    retired: ArrayQueue<Arc<Prepared>>,
    retired_cores: ArrayQueue<Arc<Core>>,
    revision: AtomicU64,
    active_core: AtomicUsize,
    loaded: AtomicBool,
    playing: AtomicBool,
    mute: AtomicU8,
    failed: AtomicBool,
    native_frames: AtomicU64,
    playback: AtomicU64,
    dsp: [AtomicU8; 128],
    event_write_revision: AtomicU64,
    event_enabled: AtomicBool,
    event_epoch: AtomicU64,
    event_generation: AtomicU64,
    event_reset: AtomicU64,
    event_suspended: AtomicBool,
    event_final: AtomicBool,
    event_clock: AtomicU64,
    event_lost: AtomicU64,
    reset_clock: AtomicU64,
    reset_dsp: [AtomicU8; 128],
    events: ArrayQueue<Queued>,
    delivery: Mutex<Delivery>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerState {
    pub ready: bool,
    pub loaded: bool,
    pub playing: bool,
    pub paused: bool,
    pub mute_mask: u8,
    pub sample_rate: u32,
    pub native_sample_rate: u32,
    pub playback_native_frames: f64,
    pub position_seconds: f64,
    pub rendered_native_frames: u64,
    pub failed: bool,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub ram: Vec<u8>,
    pub dsp: Vec<u8>,
    pub native_frames: u64,
    pub position_seconds: f64,
    pub playback_native_frames: f64,
    pub native_sample_rate: u32,
    pub mute_mask: u8,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub sample_rate: u32,
    pub frame: u64,
    pub revision: u64,
    pub spc: PlayerState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<Snapshot>,
}
#[derive(Debug, Serialize)]
pub struct Reset {
    pub clock: u64,
    pub dsp: Vec<u8>,
    pub reason: &'static str,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Packet {
    pub sequence: u64,
    pub clock: u64,
    pub writes: Vec<f64>,
    pub count: usize,
    pub dropped_writes: u64,
    pub r#final: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dsp: Option<Vec<u8>>,
}
#[derive(Debug, Serialize)]
pub struct EventResult {
    pub epoch: u64,
    pub failed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset: Option<Reset>,
    pub suspended: bool,
    pub batches: Vec<Packet>,
}
impl Shared {
    pub fn new(rate: u32, budget: Arc<extra_budget::Budget>) -> Self {
        Self {
            rate,
            budget,
            control: Mutex::new(Control::default()),
            job: Mutex::new(None),
            seek_epoch: AtomicU64::new(0),
            scope_session: AtomicU64::new(0),
            stopped_session: AtomicU64::new(0),
            cancelled_owners: (0..SCOPES)
                .map(|_| AtomicU64::new(0))
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            cancelled_count: AtomicU32::new(0),
            retired: ArrayQueue::new(CAPACITY),
            retired_cores: ArrayQueue::new(CAPACITY),
            revision: AtomicU64::new(0),
            active_core: AtomicUsize::new(0),
            loaded: AtomicBool::new(false),
            playing: AtomicBool::new(false),
            mute: AtomicU8::new(0),
            failed: AtomicBool::new(false),
            native_frames: AtomicU64::new(0),
            playback: AtomicU64::new(0f64.to_bits()),
            dsp: std::array::from_fn(|_| AtomicU8::new(0)),
            event_write_revision: AtomicU64::new(0),
            event_enabled: AtomicBool::new(false),
            event_epoch: AtomicU64::new(0),
            event_generation: AtomicU64::new(0),
            event_reset: AtomicU64::new(0),
            event_suspended: AtomicBool::new(false),
            event_final: AtomicBool::new(false),
            event_clock: AtomicU64::new(0),
            event_lost: AtomicU64::new(0),
            reset_clock: AtomicU64::new(0),
            reset_dsp: std::array::from_fn(|_| AtomicU8::new(0)),
            events: ArrayQueue::new(EVENTS),
            delivery: Mutex::new(Delivery::default()),
        }
    }
    fn valid_scope(&self, s: Option<Scope>) -> bool {
        s.is_none_or(|s| {
            s.session > self.stopped_session.load(Ordering::Acquire)
                && (s.session > self.scope_session.load(Ordering::Acquire)
                    || s.session == self.scope_session.load(Ordering::Acquire)
                        && !self.cancelled_owners
                            [..self.cancelled_count.load(Ordering::Acquire) as usize]
                            .iter()
                            .any(|v| v.load(Ordering::Acquire) == s.owner))
        })
    }
    fn collect(&self, c: &mut Control) {
        while self.retired.pop().is_some() {}
        while self.retired_cores.pop().is_some() {}
        let Control {
            prepared,
            outcomes,
            cursor,
            ..
        } = c;
        prepared.retain(|v| {
            if !v.retired.load(Ordering::Acquire) {
                return true;
            }
            if let Edit::Snapshot(cap) = &v.edit {
                if !cap.delivered.load(Ordering::Acquire)
                    && cap.created.elapsed() < std::time::Duration::from_secs(30)
                {
                    return true;
                }
            }
            if let Some(id) = v.id {
                outcomes[*cursor] = Completion {
                    id,
                    flags: v.cancelled.load(Ordering::Acquire) as u8
                        | ((v.rejected.load(Ordering::Acquire) as u8) << 1),
                };
                *cursor = (*cursor + 1) % outcomes.len();
            }
            false
        });
        c.cores.retain(|v| Arc::strong_count(v) > 1);
        if c.prepared.capacity() > 64.max(c.prepared.len() * 2) {
            c.prepared.shrink_to_fit();
        }
    }
    pub fn collect_retired(&self) {
        if let Ok(mut c) = self.control.lock() {
            self.collect(&mut c)
        }
    }
    fn create_core(&self, file: Arc<File>) -> Result<Arc<Core>, String> {
        let memory = self
            .budget
            .reserve(unsafe { native_spc_core_size() } as usize + 256)?;
        let ptr = unsafe { native_spc_create(self.rate) };
        if ptr.is_null() {
            return Err("Native SPC core allocation failed".into());
        }
        let core = Arc::new(Core {
            ptr,
            file,
            _memory: memory,
        });
        if unsafe {
            native_spc_load_control(ptr, core.file.data.as_ptr(), core.file.data.len() as u32)
        } == 0
        {
            return Err(format!("Native SPC load rejected: {}", core.error()));
        }
        unsafe {
            native_spc_set_mute_mask(ptr, self.mute.load(Ordering::Acquire) as u32);
        }
        let mut c = self
            .control
            .lock()
            .map_err(|_| "Native SPC control unavailable")?;
        self.collect(&mut c);
        c.cores.push(core.clone());
        Ok(core)
    }
    fn current_file(&self) -> Result<Arc<File>, String> {
        let c = self
            .control
            .lock()
            .map_err(|_| "Native SPC control unavailable")?;
        c.cores
            .iter()
            .find(|v| v.ptr as usize == self.active_core.load(Ordering::Acquire))
            .map(|v| v.file.clone())
            .ok_or("Load an SPC file before playback".into())
    }
    fn basis_valid(&self, p: &Prepared) -> bool {
        p.applied.load(Ordering::Acquire)
            || !p.cancelled.load(Ordering::Acquire) && self.valid_scope(p.scope)
    }
    fn planned_file(
        &self,
        prior: &[Arc<Prepared>],
    ) -> Result<(Arc<File>, Option<Arc<Prepared>>), String> {
        let c = self
            .control
            .lock()
            .map_err(|_| "Native SPC control unavailable")?;
        for p in prior.iter().rev().chain(c.prepared.iter().rev()) {
            if !self.basis_valid(p) || p.basis.get().is_some_and(|v| !self.basis_valid(v)) {
                continue;
            }
            let core = match &p.edit {
                Edit::Install(core) | Edit::Stop(Some(core)) | Edit::SeekBegin(_, core) => {
                    Some(core)
                }
                _ => None,
            };
            if let Some(core) = core {
                let basis = if p.applied.load(Ordering::Acquire) {
                    None
                } else {
                    Some(p.basis.get().cloned().unwrap_or_else(|| p.clone()))
                };
                return Ok((core.file.clone(), basis));
            }
        }
        drop(c);
        self.current_file().map(|file| (file, None))
    }
    fn planned_mask(&self, prior: &[Arc<Prepared>]) -> u8 {
        if let Ok(c) = self.control.lock() {
            for p in prior.iter().rev().chain(c.prepared.iter().rev()) {
                if self.valid_event(p, 0, None) {
                    if let Edit::Mute(mask) = p.edit {
                        return mask;
                    }
                }
            }
        }
        self.mute.load(Ordering::Acquire)
    }
    pub fn prepare_message(
        &self,
        m: &Message,
        prior: &[Arc<Prepared>],
    ) -> Result<Arc<Prepared>, String> {
        if m.id.is_some_and(|v| !valid(v))
            || m.event_epoch > MAX_SAFE_INTEGER
            || m.source_owner
                .is_some_and(|s| !valid(s.session) || !valid(s.owner))
        {
            return Err("Invalid Native SPC command identity".into());
        }
        if !self.valid_scope(m.source_owner) {
            return Err("Native SPC operation cancelled".into());
        }
        let mut basis = None;
        let edit = match &m.method {
            Method::LoadSpc { data } => {
                if !(0x10180..=16 * 1024 * 1024).contains(&data.len()) {
                    return Err("Invalid SPC file size".into());
                }
                let memory = self.budget.reserve(data.len() + 128)?;
                let file = Arc::new(File {
                    data: data.clone(),
                    _memory: memory,
                });
                Edit::Install(self.create_core(file)?)
            }
            Method::Play => Edit::Play,
            Method::Pause => Edit::Pause,
            Method::Stop => Edit::Stop(match self.planned_file(prior) {
                Ok((file, dependency)) => {
                    basis = dependency;
                    Some(self.create_core(file)?)
                }
                Err(_) if !self.loaded.load(Ordering::Acquire) => None,
                Err(e) => return Err(e),
            }),
            Method::Mute { mask } => Edit::Mute(*mask),
            Method::Barrier => Edit::Barrier,
            Method::Snapshot => {
                if !self.loaded.load(Ordering::Acquire) && self.planned_file(prior).is_err() {
                    return Err("Load an SPC file before playback".into());
                }
                if m.id.is_none() {
                    return Err("Native SPC snapshot requires a command ID".into());
                }
                let memory = self.budget.reserve(IMAGE + 512)?;
                let cap = Arc::new(Capture {
                    data: UnsafeCell::new(vec![0; IMAGE].into_boxed_slice()),
                    state: UnsafeCell::new(CoreState::default()),
                    ready: AtomicBool::new(false),
                    delivered: AtomicBool::new(false),
                    created: std::time::Instant::now(),
                    _memory: memory,
                });
                if let Some(job) = self
                    .job
                    .lock()
                    .map_err(|_| "Native SPC seek unavailable")?
                    .clone()
                {
                    if !prior.iter().any(|v| {
                        matches!(
                            v.edit,
                            Edit::Install(_)
                                | Edit::Stop(_)
                                | Edit::SeekBegin(..)
                                | Edit::Play
                                | Edit::Pause
                        )
                    }) && !job.cancelled.load(Ordering::Acquire)
                        && !job.sealed.load(Ordering::Acquire)
                    {
                        let core = job.core.lock().map_err(|_| "Native SPC seek unavailable")?;
                        if self.job_valid(&job) {
                            basis = job.basis.clone();
                            unsafe {
                                native_spc_set_mute_mask(core.ptr, self.planned_mask(prior) as u32);
                            }
                            cap.record(&core);
                        }
                    }
                }
                Edit::Snapshot(cap)
            }
            Method::SeekBegin { seek_id } => {
                if !valid(*seek_id) {
                    return Err("Invalid Native SPC seek ID".into());
                }
                let (file, dependency) = self.planned_file(prior)?;
                basis = dependency;
                let visible = self.create_core(file.clone())?;
                let private = self.create_core(file)?;
                Edit::SeekBegin(
                    Arc::new(Job {
                        id: *seek_id,
                        scope: m.source_owner,
                        basis: basis.clone(),
                        core: Mutex::new(private),
                        cancelled: AtomicBool::new(false),
                        sealed: AtomicBool::new(false),
                        epoch: AtomicU64::new(0),
                    }),
                    visible,
                )
            }
            Method::SeekEnd { seek_id, resume } => {
                let job = self
                    .job
                    .lock()
                    .map_err(|_| "Native SPC seek unavailable")?
                    .clone()
                    .filter(|j| {
                        j.id == *seek_id
                            && j.scope == m.source_owner
                            && !j.cancelled.load(Ordering::Acquire)
                            && !j.sealed.load(Ordering::Acquire)
                    })
                    .ok_or("Native SPC seek cancelled")?;
                basis = job.basis.clone();
                Edit::SeekEnd(job, *resume)
            }
            Method::Observe {
                enabled,
                epoch,
                interval_ms,
            } => {
                if *epoch > MAX_SAFE_INTEGER
                    || !interval_ms.is_finite()
                    || !(16.0..=1000.).contains(interval_ms)
                {
                    return Err("Invalid Native SPC observation options".into());
                }
                Edit::Observe(*enabled, *epoch)
            }
            Method::EventAck { epoch, sequence } => Edit::Ack(*epoch, *sequence),
            Method::CancelOwner { session, owner } => {
                if !valid(*session) || !valid(*owner) {
                    return Err("Invalid Native SPC owner".into());
                }
                Edit::CancelOwner(Scope {
                    session: *session,
                    owner: *owner,
                })
            }
            Method::CancelPerformance { session } => {
                if !valid(*session) {
                    return Err("Invalid Native SPC performance".into());
                }
                Edit::CancelPerformance(*session)
            }
            Method::Mix { volume, pan } => {
                if !volume.is_finite()
                    || !(0.0..=4.0).contains(volume)
                    || !pan.is_finite()
                    || !(-1.0..=1.0).contains(pan)
                {
                    return Err("Invalid Native SPC mixer gain/pan".into());
                }
                Edit::Mix(*volume, *pan)
            }
        };
        Ok(Arc::new(Prepared {
            id: m.id,
            scope: m.source_owner,
            event_epoch: m.event_epoch,
            edit,
            install: OnceLock::new(),
            basis: basis.map_or_else(OnceLock::new, OnceLock::from),
            applied: AtomicBool::new(false),
            complete: AtomicBool::new(false),
            retired: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
            rejected: AtomicBool::new(false),
        }))
    }
    pub fn retain(&self, commands: &[(Arc<Prepared>, Option<Owner>)]) -> Result<(), String> {
        let mut c = self
            .control
            .lock()
            .map_err(|_| "Native SPC control unavailable")?;
        self.collect(&mut c);
        if c.prepared.len() + commands.len() > CAPACITY {
            return Err("Native SPC pending command limit reached".into());
        }
        if c.prepared
            .iter()
            .chain(commands.iter().map(|(v, _)| v))
            .filter(
                |v| matches!(&v.edit,Edit::Snapshot(cap)if !cap.delivered.load(Ordering::Acquire)),
            )
            .count()
            > 16
        {
            return Err("Native SPC pending RAM snapshot limit reached (16)".into());
        }
        let mut ids = std::collections::HashSet::new();
        for v in c.prepared.iter().chain(commands.iter().map(|(v, _)| v)) {
            if v.id.is_some_and(|id| !ids.insert(id)) {
                return Err("Duplicate pending Native SPC command ID".into());
            }
        }
        let mut projected_session = self.scope_session.load(Ordering::Acquire);
        let mut cancelled: std::collections::HashSet<u64> = self.cancelled_owners
            [..self.cancelled_count.load(Ordering::Acquire) as usize]
            .iter()
            .map(|v| v.load(Ordering::Acquire))
            .collect();
        for (v, _) in commands {
            if !self.valid_scope(v.scope) {
                return Err("Native SPC operation cancelled".into());
            }
            if let Some(scope) = v.scope {
                if scope.session > projected_session {
                    projected_session = scope.session;
                    cancelled.clear();
                }
            }
            if let Edit::CancelOwner(scope) = v.edit {
                if scope.session >= projected_session {
                    if scope.session > projected_session {
                        projected_session = scope.session;
                        cancelled.clear();
                    }
                    cancelled.insert(scope.owner);
                    if cancelled.len() > SCOPES {
                        return Err("Native SPC owner limit reached (1024)".into());
                    }
                }
            }
        }
        for (v, _) in commands {
            if let Some(id) = v.id {
                c.highest = c.highest.max(id);
            }
            c.prepared.push(v.clone());
        }

        Ok(())
    }
    fn job_valid(&self, job: &Job) -> bool {
        !job.cancelled.load(Ordering::Acquire)
            && !job.sealed.load(Ordering::Acquire)
            && self.valid_scope(job.scope)
            && job.basis.as_ref().is_none_or(|v| self.basis_valid(v))
    }
    fn cancel_job(&self) {
        self.seek_epoch.fetch_add(1, Ordering::AcqRel);
        if let Ok(mut j) = self.job.lock() {
            if let Some(job) = j.take() {
                job.cancelled.store(true, Ordering::Release);
            }
        }
    }
    fn take_seek_core(&self, playing: bool) -> Option<(Arc<Core>, Option<Arc<Prepared>>)> {
        self.seek_epoch.fetch_add(1, Ordering::AcqRel);
        let job = self.job.lock().ok()?.take()?;
        let core = job.core.lock().ok()?;
        let available = !job.sealed.load(Ordering::Acquire)
            && !job.cancelled.load(Ordering::Acquire)
            && job.basis.as_ref().is_none_or(|v| self.basis_valid(v));
        job.sealed.store(true, Ordering::Release);
        job.cancelled.store(true, Ordering::Release);
        if !available {
            return None;
        }
        unsafe {
            native_spc_set_mute_mask(core.ptr, self.planned_mask(&[]) as u32);
            native_spc_set_playing(core.ptr, playing as u32);
        }
        Some((core.clone(), job.basis.clone()))
    }
    pub fn commit_cancel(&self, p: &Prepared) -> bool {
        if let Some(s) = p.scope {
            if s.session > self.scope_session.load(Ordering::Acquire) {
                for v in self.cancelled_owners.iter() {
                    v.store(0, Ordering::Relaxed);
                }
                self.cancelled_count.store(0, Ordering::Relaxed);
                self.scope_session.store(s.session, Ordering::Release);
            }
        }
        let mut changed = false;
        match &p.edit {
            Edit::CancelOwner(s) => {
                if s.session >= self.scope_session.load(Ordering::Acquire) {
                    if s.session > self.scope_session.load(Ordering::Acquire) {
                        for v in self.cancelled_owners.iter() {
                            v.store(0, Ordering::Relaxed);
                        }
                        self.cancelled_count.store(0, Ordering::Relaxed);
                        self.scope_session.store(s.session, Ordering::Release);
                    }
                    let count = self.cancelled_count.load(Ordering::Acquire) as usize;
                    if !self.cancelled_owners[..count]
                        .iter()
                        .any(|v| v.load(Ordering::Acquire) == s.owner)
                    {
                        self.cancelled_owners[count].store(s.owner, Ordering::Release);
                        self.cancelled_count
                            .store((count + 1) as u32, Ordering::Release);
                    }
                }
                changed = true;
            }
            Edit::CancelPerformance(session) => {
                self.stopped_session.fetch_max(*session, Ordering::AcqRel);
                changed = true;
            }
            Edit::SeekBegin(job, _) => {
                self.cancel_job();
                job.epoch
                    .store(self.seek_epoch.load(Ordering::Acquire), Ordering::Release);
                *self.job.lock().expect("SPC seek control checked") = Some(job.clone());
            }
            Edit::SeekEnd(job, resume) => {
                let core = job.core.lock().expect("SPC private seek core");
                if !job.sealed.load(Ordering::Acquire)
                    && !job.cancelled.load(Ordering::Acquire)
                    && job.epoch.load(Ordering::Acquire) == self.seek_epoch.load(Ordering::Acquire)
                {
                    job.sealed.store(true, Ordering::Release);
                    unsafe {
                        native_spc_set_mute_mask(core.ptr, self.planned_mask(&[]) as u32);
                        native_spc_set_playing(core.ptr, *resume as u32);
                    }
                    let _ = p.install.set(core.clone());
                } else {
                    p.cancelled.store(true, Ordering::Release);
                }
            }
            Edit::Play | Edit::Pause => {
                if let Some((core, basis)) = self.take_seek_core(matches!(p.edit, Edit::Play)) {
                    if let Some(basis) = basis {
                        let _ = p.basis.set(basis);
                    }
                    let _ = p.install.set(core);
                }
            }
            Edit::Install(_) | Edit::Stop(_) => self.cancel_job(),
            Edit::Ack(epoch, sequence) => {
                if let Ok(mut d) = self.delivery.lock() {
                    if d.epoch == *epoch {
                        d.inflight.retain(|v| v != sequence);
                    }
                }
            }
            _ => {}
        }
        if changed {
            let c = self.control.lock().expect("SPC control checked");
            for v in &c.prepared {
                if !self.valid_scope(v.scope) {
                    v.cancelled.store(true, Ordering::Release);
                    if let Edit::Snapshot(cap) = &v.edit {
                        cap.delivered.store(true, Ordering::Release);
                    }
                }
            }
            drop(c);
            let matching = self
                .job
                .lock()
                .ok()
                .and_then(|j| j.clone())
                .is_some_and(|j| {
                    !self.valid_scope(j.scope)
                        || j.basis.as_ref().is_some_and(|v| !self.basis_valid(v))
                });
            if matching {
                if let Some((core, basis)) = self.take_seek_core(false) {
                    if let Some(basis) = basis {
                        let _ = p.basis.set(basis);
                    }
                    let _ = p.install.set(core);
                }
            }
        }
        changed
    }
    pub fn valid_event(&self, p: &Prepared, _seq: u64, _owner: Option<Owner>) -> bool {
        !p.cancelled.load(Ordering::Acquire)
            && self.valid_scope(p.scope)
            && p.basis.get().is_none_or(|v| self.basis_valid(v))
    }
    pub fn cancel_before(&self, _: &Prepared, _: u64, _: Option<Owner>) {}
    pub fn rollback(&self, p: Arc<Prepared>) {
        self.retire(p)
    }
    pub fn retire(&self, p: Arc<Prepared>) {
        if !p.complete.load(Ordering::Acquire) {
            p.cancelled.store(true, Ordering::Release);
            if let Edit::Snapshot(c) = &p.edit {
                c.delivered.store(true, Ordering::Release);
            }
        }
        p.complete.store(true, Ordering::Release);
        p.retired.store(true, Ordering::Release);
        self.retired
            .push(p)
            .unwrap_or_else(|_| unreachable!("Bounded SPC retirement"));
    }
    pub fn wait_applied(&self, id: u64) -> Result<(), String> {
        let p = {
            let c = self
                .control
                .lock()
                .map_err(|_| "Native SPC control unavailable")?;
            if let Some(p) = c.prepared.iter().find(|v| v.id == Some(id)) {
                p.clone()
            } else {
                for off in 0..512 {
                    let v = c.outcomes[(c.cursor + 511 - off) % 512];
                    if v.id == id {
                        return result(v.flags);
                    }
                }
                return Err("Native SPC command result expired or unknown".into());
            }
        };
        let start = std::time::Instant::now();
        while !p.complete.load(Ordering::Acquire) {
            if start.elapsed() > std::time::Duration::from_secs(10) {
                return Err("Native SPC command was not applied within 10 seconds".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        result(
            p.cancelled.load(Ordering::Acquire) as u8
                | ((p.rejected.load(Ordering::Acquire) as u8) << 1),
        )
    }
    pub fn seek_step(
        &self,
        seek_id: u64,
        frames: u32,
        scope: Option<Scope>,
    ) -> Result<Status, String> {
        if frames > 16000 {
            return Err("Invalid SPC seek block".into());
        }
        if !self.valid_scope(scope) {
            return Err("Native SPC seek cancelled".into());
        }
        let job = self
            .job
            .lock()
            .map_err(|_| "Native SPC seek unavailable")?
            .clone()
            .filter(|j| j.id == seek_id && j.scope == scope)
            .ok_or("Native SPC seek cancelled")?;
        let core = job.core.lock().map_err(|_| "Native SPC seek unavailable")?;
        if !self.job_valid(&job) {
            return Err("Native SPC seek cancelled".into());
        }
        unsafe {
            native_spc_set_mute_mask(core.ptr, self.planned_mask(&[]) as u32);
        }
        if unsafe { native_spc_seek_control(core.ptr, frames) } == 0 {
            return Err(core.error());
        }
        if !self.job_valid(&job) {
            return Err("Native SPC seek cancelled".into());
        }
        drop(core);
        self.status(0)
    }
    fn player(&self, s: CoreState) -> PlayerState {
        PlayerState {
            ready: true,
            loaded: s.loaded != 0,
            playing: s.playing != 0,
            paused: s.loaded != 0 && s.playing == 0,
            mute_mask: self.planned_mask(&[]),
            sample_rate: self.rate,
            native_sample_rate: 32000,
            playback_native_frames: s.playback_native_frames,
            position_seconds: s.playback_native_frames / 32000.,
            rendered_native_frames: s.native_frames,
            failed: s.failed != 0,
        }
    }
    pub fn status(&self, frame: u64) -> Result<Status, String> {
        self.status_after(frame, None)
    }
    pub fn status_after(&self, frame: u64, after: Option<u64>) -> Result<Status, String> {
        let mut snapshot = None;
        {
            let mut c = self
                .control
                .lock()
                .map_err(|_| "Native SPC control unavailable")?;
            if let Some(id) = after {
                if let Some(p) = c.prepared.iter().find(|v| v.id == Some(id)) {
                    if let Edit::Snapshot(cap) = &p.edit {
                        if !p.complete.load(Ordering::Acquire) {
                            return Err("Native SPC snapshot has not applied".into());
                        }
                        result(
                            p.cancelled.load(Ordering::Acquire) as u8
                                | ((p.rejected.load(Ordering::Acquire) as u8) << 1),
                        )?;
                        snapshot = Some(cap.snapshot());
                        cap.delivered.store(true, Ordering::Release);
                    }
                }
            }
            self.collect(&mut c);
        }
        let job = self
            .job
            .lock()
            .map_err(|_| "Native SPC seek unavailable")?
            .clone();
        if let Some(job) = job {
            if self.job_valid(&job) {
                let core = job.core.lock().map_err(|_| "Native SPC seek unavailable")?;
                if self.job_valid(&job) {
                    return Ok(Status {
                        sample_rate: self.rate,
                        frame,
                        revision: self.revision.load(Ordering::Acquire) / 2,
                        spc: self.player(core.state()),
                        snapshot,
                    });
                }
            }
        }
        for _ in 0..10000 {
            let before = self.revision.load(Ordering::Acquire);
            if before & 1 != 0 {
                std::thread::yield_now();
                continue;
            }
            let loaded = self.loaded.load(Ordering::Relaxed);
            let playing = self.playing.load(Ordering::Relaxed);
            let playback = f64::from_bits(self.playback.load(Ordering::Relaxed));
            let status = Status {
                sample_rate: self.rate,
                frame,
                revision: before / 2,
                spc: PlayerState {
                    ready: true,
                    loaded,
                    playing,
                    paused: loaded && !playing,
                    mute_mask: self.mute.load(Ordering::Relaxed),
                    sample_rate: self.rate,
                    native_sample_rate: 32000,
                    playback_native_frames: playback,
                    position_seconds: playback / 32000.,
                    rendered_native_frames: self.native_frames.load(Ordering::Relaxed),
                    failed: self.failed.load(Ordering::Relaxed),
                },
                snapshot,
            };
            std::sync::atomic::fence(Ordering::Acquire);
            if before == self.revision.load(Ordering::Relaxed) {
                return Ok(status);
            }
            snapshot = status.snapshot;
        }
        Err("Native SPC state changed while reading; retry".into())
    }
}
fn result(flags: u8) -> Result<(), String> {
    if flags & 1 != 0 {
        Err("Native SPC operation cancelled".into())
    } else if flags & 2 != 0 {
        Err("Load an SPC file before playback".into())
    } else {
        Ok(())
    }
}
pub struct Renderer {
    active: Option<Arc<Core>>,
    scope: Option<Scope>,
    rate: u32,
    frame: u64,
    gains: [f32; 2],
    targets: [f32; 2],
    ramp: u32,
    observe: bool,
    epoch: u64,
    generation: u64,
    suspended: bool,
}
impl Renderer {
    pub fn new(rate: u32) -> Result<Self, String> {
        Ok(Self {
            active: None,
            scope: None,
            rate,
            frame: 0,
            gains: [1.; 2],
            targets: [1.; 2],
            ramp: 0,
            observe: false,
            epoch: 0,
            generation: 0,
            suspended: false,
        })
    }
    pub fn set_frame(&mut self, frame: u64) {
        self.frame = frame;
    }
    fn reset_events(&mut self, s: &Shared, epoch: u64, suspended: bool) {
        self.epoch = epoch;
        self.generation += 1;
        self.suspended = suspended;
        s.event_reset.store(0, Ordering::Release);
        s.event_enabled.store(self.observe, Ordering::Relaxed);
        s.event_epoch.store(epoch, Ordering::Relaxed);
        s.event_lost.store(0, Ordering::Relaxed);
        s.event_final.store(false, Ordering::Relaxed);
        s.event_suspended.store(suspended, Ordering::Relaxed);
        if let Some(core) = &self.active {
            unsafe {
                native_spc_observe(core.ptr, (self.observe && !suspended) as u32, PACKET as u32);
            }
            let state = core.state();
            let mut dsp = [0; 128];
            unsafe {
                native_spc_read_dsp_registers(core.ptr, dsp.as_mut_ptr());
            }
            for (o, v) in s.reset_dsp.iter().zip(dsp) {
                o.store(v, Ordering::Relaxed);
            }
            s.reset_clock
                .store(state.native_frames * 32, Ordering::Relaxed);
            s.event_clock
                .store(state.native_frames * 32, Ordering::Release);
        } else {
            s.reset_clock.store(0, Ordering::Relaxed);
            s.event_clock.store(0, Ordering::Release);
        }
        s.event_generation.store(self.generation, Ordering::Release);
        s.event_reset.store(self.generation, Ordering::Release);
    }
    fn install(&mut self, core: Arc<Core>, s: &Shared, epoch: u64) {
        s.active_core.store(core.ptr as usize, Ordering::Release);
        if let Some(old) = self.active.replace(core) {
            s.retired_cores
                .push(old)
                .unwrap_or_else(|_| unreachable!("Bounded SPC core retirement"));
        }
        if let Some(core) = &self.active {
            unsafe {
                native_spc_set_mute_mask(core.ptr, s.mute.load(Ordering::Acquire) as u32);
            }
        }
        self.reset_events(s, epoch, false);
    }
    pub fn apply(&mut self, p: &Prepared, _owner: Option<Owner>, s: &Shared) {
        if !s.valid_event(p, 0, None) {
            p.cancelled.store(true, Ordering::Release);
            s.publish(self, Some(p));
            return;
        }
        match &p.edit {
            Edit::Install(core) => {
                self.scope = p.scope;
                self.install(core.clone(), s, p.event_epoch);
            }
            Edit::Play => {
                if let Some(core) = p.install.get() {
                    self.install(core.clone(), s, p.event_epoch);
                }
                if let Some(core) = &self.active {
                    unsafe {
                        native_spc_set_playing(core.ptr, 1);
                    }
                    self.scope = p.scope;
                    if self.suspended {
                        self.reset_events(s, p.event_epoch, false);
                    }
                } else {
                    p.rejected.store(true, Ordering::Release);
                }
            }
            Edit::Pause => {
                if let Some(core) = p.install.get() {
                    self.install(core.clone(), s, p.event_epoch);
                }
                self.collect_events(s);
                if let Some(core) = &self.active {
                    unsafe {
                        native_spc_set_playing(core.ptr, 0);
                    }
                }
                s.event_final.store(true, Ordering::Release);
            }
            Edit::Stop(core) => {
                self.scope = p.scope;
                if let Some(core) = core {
                    self.install(core.clone(), s, p.event_epoch);
                } else {
                    self.reset_events(s, p.event_epoch, false);
                }
            }
            Edit::Mute(mask) => {
                s.mute.store(*mask, Ordering::Release);
                if let Some(core) = &self.active {
                    unsafe {
                        native_spc_set_mute_mask(core.ptr, *mask as u32);
                    }
                }
            }
            Edit::Snapshot(cap) => {
                if !cap.ready.load(Ordering::Acquire) {
                    if let Some(core) = &self.active {
                        cap.record(core);
                    } else {
                        p.rejected.store(true, Ordering::Release);
                    }
                }
            }
            Edit::SeekBegin(_, visible) => {
                self.scope = p.scope;
                self.install(visible.clone(), s, p.event_epoch);
                self.reset_events(s, p.event_epoch, true);
            }
            Edit::SeekEnd(job, _) => {
                if !job.cancelled.load(Ordering::Acquire)
                    && job.epoch.load(Ordering::Acquire) == s.seek_epoch.load(Ordering::Acquire)
                {
                    if let Some(core) = p.install.get() {
                        self.scope = p.scope;
                        self.install(core.clone(), s, p.event_epoch);
                    } else {
                        p.cancelled.store(true, Ordering::Release);
                    }
                } else {
                    p.cancelled.store(true, Ordering::Release);
                }
            }
            Edit::Observe(on, epoch) => {
                if self.observe != *on || self.epoch != *epoch {
                    self.observe = *on;
                    self.reset_events(s, *epoch, self.suspended);
                }
            }
            Edit::CancelOwner(scope) => {
                if self.scope == Some(*scope) || self.scope.is_none() && p.install.get().is_some() {
                    if let Some(core) = p.install.get() {
                        self.install(core.clone(), s, self.epoch);
                    }
                    self.pause_cancel(s, self.epoch);
                }
            }
            Edit::CancelPerformance(session) => {
                if self.scope.is_some_and(|v| v.session <= *session)
                    || self.scope.is_none() && p.install.get().is_some()
                {
                    if let Some(core) = p.install.get() {
                        self.install(core.clone(), s, self.epoch);
                    }
                    self.pause_cancel(s, self.epoch);
                }
            }
            Edit::Mix(volume, pan) => {
                self.targets = [
                    *volume * if *pan > 0. { 1. - *pan } else { 1. },
                    *volume * if *pan < 0. { 1. + *pan } else { 1. },
                ];
                self.ramp = (self.rate / 200).max(1);
            }
            Edit::Barrier | Edit::Ack(..) => {}
        }
        p.applied.store(true, Ordering::Release);
        s.publish(self, Some(p));
    }
    fn pause_cancel(&mut self, s: &Shared, epoch: u64) {
        if let Some(core) = &self.active {
            unsafe {
                native_spc_set_playing(core.ptr, 0);
            }
        }
        self.reset_events(s, epoch, true);
        self.scope = None;
    }
    pub fn cancel_invalid(&mut self, _: impl Fn(Option<Owner>) -> bool, s: &Shared) {
        if !s.valid_scope(self.scope) {
            self.pause_cancel(s, self.epoch);
        }
        self.publish(s);
    }
    fn collect_events(&mut self, s: &Shared) {
        if !self.observe || self.suspended {
            return;
        }
        if let Some(core) = &self.active {
            s.event_write_revision.fetch_add(1, Ordering::AcqRel);
            let count = unsafe { native_spc_event_count(core.ptr) } as usize;
            let events =
                unsafe { std::slice::from_raw_parts(native_spc_event_data(core.ptr), count) };
            let mut lost = unsafe { native_spc_overflow_take(core.ptr) } as u64;
            for event in events {
                if s.events
                    .push(Queued {
                        generation: self.generation,
                        event: *event,
                    })
                    .is_err()
                {
                    lost += 1;
                }
            }
            unsafe {
                native_spc_events_clear(core.ptr);
            }
            s.event_clock
                .store(core.state().native_frames * 32, Ordering::Release);
            self.publish(s);
            if lost != 0 {
                s.event_lost.fetch_add(lost, Ordering::AcqRel);
            }
            s.event_write_revision.fetch_add(1, Ordering::Release);
        }
    }
    pub fn publish(&self, s: &Shared) {
        s.publish(self, None)
    }
    pub fn mix_additional(&mut self, pcm: &mut [f32; 2], s: &Shared) {
        self.frame += 1;
        if let Some(core) = &self.active {
            let mut sample = [0.; 2];
            unsafe {
                native_spc_sample(core.ptr, &mut sample[0], &mut sample[1]);
            }
            if self.ramp != 0 {
                for ch in 0..2 {
                    self.gains[ch] += (self.targets[ch] - self.gains[ch]) / self.ramp as f32;
                }
                self.ramp -= 1;
            }
            for ch in 0..2 {
                pcm[ch] += sample[ch] * self.gains[ch];
            }
        }
        if self.frame % 128 == 0 {
            self.collect_events(s);
            if !self.observe || self.suspended {
                self.publish(s);
            }
        }
    }
}
impl Shared {
    fn publish(&self, r: &Renderer, p: Option<&Prepared>) {
        let state = r.active.as_ref().map(|v| v.state()).unwrap_or_default();
        self.active_core.store(
            r.active.as_ref().map_or(0, |v| v.ptr as usize),
            Ordering::Release,
        );
        self.revision.fetch_add(1, Ordering::AcqRel);
        self.loaded.store(state.loaded != 0, Ordering::Relaxed);
        self.playing.store(state.playing != 0, Ordering::Relaxed);
        self.native_frames
            .store(state.native_frames, Ordering::Relaxed);
        self.playback
            .store(state.playback_native_frames.to_bits(), Ordering::Relaxed);
        self.failed.store(state.failed != 0, Ordering::Relaxed);
        if let Some(core) = &r.active {
            let mut dsp = [0; 128];
            unsafe {
                native_spc_read_dsp_registers(core.ptr, dsp.as_mut_ptr());
            }
            for (o, v) in self.dsp.iter().zip(dsp) {
                o.store(v, Ordering::Relaxed);
            }
        }
        if let Some(p) = p {
            p.complete.store(true, Ordering::Release);
        }
        self.revision.fetch_add(1, Ordering::Release);
    }
    fn event_header(&self) -> Option<(u64, Vec<u8>)> {
        for _ in 0..1000 {
            let before = self.revision.load(Ordering::Acquire);
            if before & 1 != 0 {
                std::thread::yield_now();
                continue;
            }
            let clock = self.native_frames.load(Ordering::Relaxed) * 32;
            let dsp = self.dsp.iter().map(|v| v.load(Ordering::Relaxed)).collect();
            std::sync::atomic::fence(Ordering::Acquire);
            if before == self.revision.load(Ordering::Relaxed) {
                return Some((clock, dsp));
            }
        }
        None
    }
    pub fn drain_events(&self, epoch: u64) -> Result<EventResult, String> {
        let mut d = self
            .delivery
            .lock()
            .map_err(|_| "Native SPC event delivery unavailable")?;
        let generation = self.event_generation.load(Ordering::Acquire);
        let actual_epoch = self.event_epoch.load(Ordering::Acquire);
        let empty = || EventResult {
            epoch: self.event_epoch.load(Ordering::Acquire),
            failed: self.failed.load(Ordering::Acquire),
            reset: None,
            suspended: false,
            batches: Vec::new(),
        };
        if epoch != actual_epoch
            || self.event_reset.load(Ordering::Acquire) != generation
            || !self.event_enabled.load(Ordering::Acquire)
        {
            return Ok(empty());
        }
        let production = self.event_write_revision.load(Ordering::Acquire);
        if production & 1 != 0 {
            return Ok(empty());
        }
        if d.generation != generation {
            d.generation = generation;
            d.epoch = actual_epoch;
            d.inflight.clear();
            d.initial = true;
            d.reset_seen = 0;
        }
        let original_inflight = d.inflight.len();
        let mut popped = 0u64;
        let mut new_generation_lost = 0u64;
        let mut consumed_lost = 0u64;
        let mut result = EventResult {
            epoch: actual_epoch,
            failed: self.failed.load(Ordering::Acquire),
            reset: None,
            suspended: false,
            batches: Vec::new(),
        };
        if d.reset_seen != generation {
            let clock = self.reset_clock.load(Ordering::Relaxed);
            let dsp = self
                .reset_dsp
                .iter()
                .map(|v| v.load(Ordering::Relaxed))
                .collect();
            if self.event_reset.load(Ordering::Acquire) != generation
                || self.event_generation.load(Ordering::Acquire) != generation
            {
                return Ok(empty());
            }
            d.reset_seen = generation;
            result.suspended = self.event_suspended.load(Ordering::Acquire);
            if !result.suspended {
                result.reset = Some(Reset {
                    clock,
                    dsp,
                    reason: "native",
                });
            }
        }
        if !result.suspended && d.inflight.len() < 4 {
            consumed_lost = self.event_lost.swap(0, Ordering::AcqRel);
            if consumed_lost != 0 {
                let mut dropped = consumed_lost;
                for _ in 0..EVENTS {
                    match self.events.pop() {
                        Some(e) => {
                            if e.generation == generation {
                                dropped += 1;
                                popped += 1;
                            } else if e.generation > generation {
                                new_generation_lost += 1;
                            }
                        }
                        None => break,
                    }
                }
                if let Some((clock, dsp)) = self.event_header() {
                    d.sequence += 1;
                    let sequence = d.sequence;
                    d.inflight.push(sequence);
                    result.batches.push(Packet {
                        sequence,
                        clock,
                        writes: Vec::new(),
                        count: 0,
                        dropped_writes: dropped,
                        r#final: false,
                        dsp: Some(dsp),
                    });
                } else {
                    self.event_lost.fetch_add(dropped, Ordering::AcqRel);
                    return Ok(empty());
                }
            } else {
                while d.inflight.len() < 4 {
                    let mut writes = Vec::with_capacity(PACKET * 3);
                    let mut exhausted = false;
                    // Old-generation entries do not consume a current packet's
                    // capacity or incorrectly advance its finite watermark.
                    for _ in 0..EVENTS {
                        if writes.len() / 3 == PACKET {
                            break;
                        }
                        match self.events.pop() {
                            Some(e) if e.generation == generation => {
                                writes.extend_from_slice(&[
                                    e.event.clock as f64,
                                    e.event.reg as f64,
                                    e.event.value as f64,
                                ]);
                                popped += 1;
                            }
                            Some(e) => {
                                if e.generation > generation {
                                    new_generation_lost += 1;
                                }
                            }
                            None => {
                                exhausted = true;
                                break;
                            }
                        }
                    }
                    let final_flag = self.event_final.load(Ordering::Acquire);
                    if writes.is_empty() && !d.initial && !final_flag {
                        break;
                    }
                    d.initial = false;
                    d.sequence += 1;
                    let sequence = d.sequence;
                    d.inflight.push(sequence);
                    let count = writes.len() / 3;
                    let last = if count != 0 {
                        writes[(count - 1) * 3] as u64
                    } else {
                        0
                    };
                    let clock = if exhausted {
                        self.event_clock.load(Ordering::Acquire).max(last)
                    } else {
                        last
                    };
                    let final_packet = final_flag && exhausted;
                    result.batches.push(Packet {
                        sequence,
                        clock,
                        writes,
                        count,
                        dropped_writes: 0,
                        r#final: final_packet,
                        dsp: None,
                    });
                    if exhausted {
                        break;
                    }
                }
            }
        }
        let same_generation = self.event_generation.load(Ordering::Acquire) == generation
            && self.event_reset.load(Ordering::Acquire) == generation;
        if !same_generation || self.event_write_revision.load(Ordering::Acquire) != production {
            d.inflight.truncate(original_inflight);
            d.reset_seen = 0;
            d.initial = true;
            if same_generation {
                self.event_lost
                    .fetch_add(consumed_lost + popped, Ordering::AcqRel);
            } else if new_generation_lost != 0 {
                self.event_lost
                    .fetch_add(new_generation_lost, Ordering::AcqRel);
            }
            return Ok(empty());
        }
        if result.batches.iter().any(|p| p.r#final) {
            self.event_final.store(false, Ordering::Release);
        }
        Ok(result)
    }
}

#[cfg(test)]
pub(super) fn layout() -> (usize, usize, usize, usize, usize) {
    (
        std::mem::size_of::<Shared>(),
        std::mem::size_of::<Renderer>(),
        std::mem::size_of::<Prepared>(),
        unsafe { native_spc_core_size() } as usize,
        2 * CAPACITY * (std::mem::size_of::<Arc<Prepared>>() + std::mem::size_of::<usize>())
            + EVENTS * (std::mem::size_of::<Queued>() + std::mem::size_of::<usize>())
            + SCOPES * std::mem::size_of::<AtomicU64>()
            + 512 * std::mem::size_of::<Completion>(),
    )
}
