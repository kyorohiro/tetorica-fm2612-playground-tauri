//! Independent YM2610 with four active physical FM voices and sparse immutable ADPCM ROM.
//! Allocation and reclamation stay on the control side of the event queue.
use super::{extra_budget, Owner, CAPACITY, MAX_SAFE_INTEGER, OWNERS};
use crossbeam_queue::ArrayQueue;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    ffi::c_void,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering},
        Arc, Mutex, OnceLock,
    },
};
pub const CLOCK: u32 = 8_000_000;
const ROM: usize = 16 * 1024 * 1024;
const PAGE: usize = 65536;
pub const MAX_BATCH_UPLOAD_BYTES: usize = 32 * 1024 * 1024;
const VOICES: usize = 16;
const PENDING_BYTES: usize = 40 * 1024 * 1024;
#[repr(C)]
struct CoreState {
    registers: [u8; 512],
    clock: u32,
    source_mute_mask: u32,
    keys: [u8; 6],
    ssg_levels: [u8; 3],
    status: u8,
    status_hi: u8,
    irq: u8,
    variant: u8,
}
impl Default for CoreState {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}
extern "C" {
    fn native_ym2610b_create_extra(rate: u32) -> *mut c_void;
    fn native_ym2610b_destroy(p: *mut c_void);
    fn native_ym2610b_reset(p: *mut c_void);
    fn native_ym2610b_stop(p: *mut c_void);
    fn native_ym2610b_write(p: *mut c_void, port: u8, register: u8, value: u8);
    fn native_ym2610b_key_off(p: *mut c_void, voice: u8);
    fn native_ym2610b_key_off_fm(p: *mut c_void, voice: u8);
    fn native_ym2610b_cancel_csm(p: *mut c_void);
    fn native_ym2610b_audition_fm_key_on(p: *mut c_void, channel: u8, mask: u8);
    fn native_ym2610b_sample(p: *mut c_void, l: *mut f32, r: *mut f32);
    fn native_ym2610b_set_clock(p: *mut c_void, hz: u32) -> i32;
    fn native_ym2610b_set_source_mute_mask(p: *mut c_void, mask: u32);
    fn native_ym2610b_set_retained_ssg_level(p: *mut c_void, ch: u8, level: u8);
    fn native_ym2610b_alloc_rom_page() -> *mut c_void;
    fn native_ym2610b_free_rom_page(p: *mut c_void);
    fn native_ym2610b_install_rom_page(
        p: *mut c_void,
        bank: u8,
        index: u16,
        page: *mut c_void,
    ) -> i32;
    fn native_ym2610b_get_extra_state(p: *mut c_void, s: *mut CoreState);
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
    Memory {
        #[serde(rename = "romType")]
        rom_type: u8,
        offset: u32,
        data: Vec<u8>,
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
struct Page {
    bank: u8,
    ptr: *mut c_void,
    _memory: extra_budget::Permit,
}
unsafe impl Send for Page {}
unsafe impl Sync for Page {}
impl std::fmt::Debug for Page {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RomPage").finish_non_exhaustive()
    }
}
impl Drop for Page {
    fn drop(&mut self) {
        unsafe { native_ym2610b_free_rom_page(self.ptr) }
    }
}
#[derive(Debug)]
struct PageRef {
    bank: u8,
    index: u16,
    page: Arc<Page>,
}
#[derive(Default)]
pub struct PagePlan {
    new: HashMap<(u64, u8, u16), Arc<Page>>,
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
    pages: OnceLock<Vec<PageRef>>,
    upload_len: usize,
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
            port: 1,
            register: 0x08..=0x0d | 0x10..=0x15 | 0x18..=0x1d | 0x20..=0x25 | 0x28..=0x2d,
            ..
        } => match op {
            Op::Write { register, .. } => Some(9 + (*register & 7) as usize),
            _ => unreachable!(),
        },
        Op::Write {
            port: 0,
            register: 0x10..=0x1b,
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
        Op::Write { port, .. } if *port > 1 => Err("Native YM2610 port must be 0 or 1".into()),
        Op::KeyOff { channel } if *channel >= VOICES as u8 => {
            Err("Native YM2610 voice must be 0–15".into())
        }
        Op::KeyOff { channel } if *channel == 0 || *channel == 3 => {
            Err("Native YM2610 FM voices 0 and 3 are unavailable".into())
        }
        Op::SetRetainedLevel { channel, level } if *channel >= 3 || *level > 31 => {
            Err("Invalid Native YM2610 SSG retained level".into())
        }
        Op::Clock { hz } if !(100_000..=100_000_000).contains(hz) => {
            Err("Native YM2610 clock must be 100000–100000000 Hz".into())
        }
        _ => Ok(()),
    }
}
impl Message {
    pub fn prepared_bytes(&self) -> usize {
        std::mem::size_of::<Prepared>()
            + match &self.method {
                Method::Batch { commands, .. } => commands.len() * std::mem::size_of::<Op>(),
                Method::Memory { offset, data, .. } => {
                    page_count(*offset, data.len()) * std::mem::size_of::<PageRef>()
                }
                _ => 0,
            }
    }
    pub fn prepare(&self) -> Result<Arc<Prepared>, String> {
        if !valid_id(self.id) || !valid_id(self.group) {
            return Err("Invalid Native YM2610 command ID".into());
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
                return Err("Native YM2610 mixer volume must be 0–4, pan −1–1".into())
            }
            Method::SourceMute { mask } if *mask > 7 => {
                return Err("Native YM2610 source mute mask must be 0–7".into())
            }
            Method::Memory {
                rom_type,
                offset,
                data,
            } if *rom_type > 1
                || data.is_empty()
                || (*offset as usize)
                    .checked_add(data.len())
                    .is_none_or(|end| end > ROM) =>
            {
                return Err("Native additional YM2610 memory exceeds 16 MiB bank bounds".into())
            }
            Method::CancelMainOwner { target_group } if !valid_id(Some(*target_group)) => {
                return Err("Invalid Native YM2610 Main owner group".into())
            }
            Method::Batch {
                commands,
                channel,
                voice,
                action,
            } => {
                if *channel >= 16
                    || *channel == 0
                    || *channel == 3
                    || commands.is_empty()
                    || commands.len() > 512
                    || !valid_id(*voice)
                    || self.audition && *action != LeaseAction::Edit && voice.is_none()
                {
                    return Err("Invalid Native YM2610 batch or voice lease".into());
                }
                for op in commands {
                    validate_op(op)?;
                    if declared_channel(op).is_some_and(|ch| ch != *channel as usize) {
                        return Err(
                            "Native YM2610 batch must target its declared physical voice".into(),
                        );
                    }
                    if let Op::Write {
                        port: 1,
                        register: 0,
                        value,
                    } = op
                    {
                        if !(9..15).contains(channel) || value & 0x3f & !(1 << (*channel - 9)) != 0
                        {
                            return Err(
                                "Native YM2610 ADPCM-A batch must target its declared voice".into(),
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
                        return Err("Native YM2610 mixer batch must target an SSG voice".into());
                    }
                }
            }
            _ => {}
        }
        Ok(Arc::new(Prepared {
            id: self.id,
            audition: self.audition,
            group: if matches!(self.method, Method::Memory { .. }) {
                None
            } else {
                self.group
            },
            method: if let Method::Memory {
                rom_type, offset, ..
            } = self.method
            {
                Method::Memory {
                    rom_type,
                    offset,
                    data: Vec::new(),
                }
            } else {
                self.method.clone()
            },
            complete: AtomicBool::new(false),
            clock_rejected: AtomicBool::new(false),
            outcome_observed: AtomicBool::new(false),
            pages: OnceLock::new(),
            upload_len: if let Method::Memory { data, .. } = &self.method {
                data.len()
            } else {
                0
            },
        }))
    }
}
fn page_count(offset: u32, len: usize) -> usize {
    if len == 0 {
        0
    } else {
        ((offset as usize % PAGE) + len).div_ceil(PAGE)
    }
}
impl Prepared {
    pub fn starts(&self) -> bool {
        match &self.method {
            Method::Write { .. } => true,
            Method::Batch { action, .. } => *action != LeaseAction::Off,
            _ => false,
        }
    }
    pub fn stops(&self) -> bool {
        matches!(self.method, Method::StopVoices) && self.group.is_none()
    }
    pub fn is_memory(&self) -> bool {
        matches!(self.method, Method::Memory { .. })
    }
    pub fn upload_bytes(&self) -> usize {
        self.upload_len
    }
    pub fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + match &self.method {
                Method::Batch { commands, .. } => commands.capacity() * std::mem::size_of::<Op>(),
                Method::Memory { data, .. } => data.capacity(),
                _ => 0,
            }
            + self
                .pages
                .get()
                .map_or(0, |p| p.capacity() * std::mem::size_of::<PageRef>())
    }
    fn bytes(&self) -> usize {
        self.retained_bytes()
    }
}
struct Control {
    prepared: Vec<Arc<Prepared>>,
    rejected_clocks: HashSet<u64>,
    pages: [Option<Arc<Page>>; 512],
    page_guards: Vec<Arc<Page>>,
}
impl Default for Control {
    fn default() -> Self {
        Self {
            prepared: Vec::new(),
            rejected_clocks: HashSet::new(),
            pages: std::array::from_fn(|_| None),
            page_guards: Vec::new(),
        }
    }
}
pub struct Shared {
    additional: bool,
    revision: AtomicU64,
    registers: [AtomicU8; 512],
    keys: [AtomicU8; 6],
    levels: [AtomicU8; 3],
    clock: AtomicU32,
    source_mute: AtomicU32,
    status: AtomicU8,
    status_hi: AtomicU8,
    rom_bytes: [AtomicU32; 2],
    budget: Arc<extra_budget::Budget>,
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
            revision: AtomicU64::new(0),
            registers: std::array::from_fn(|_| AtomicU8::new(0)),
            keys: std::array::from_fn(|_| AtomicU8::new(0)),
            levels: std::array::from_fn(|_| AtomicU8::new(0)),
            clock: AtomicU32::new(CLOCK),
            source_mute: AtomicU32::new(0),
            status: AtomicU8::new(0),
            status_hi: AtomicU8::new(0),
            rom_bytes: std::array::from_fn(|_| AtomicU32::new(0)),
            budget: Arc::new(extra_budget::Budget::default()),
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
    pub status: u8,
    pub status_hi: u8,
    pub variant: &'static str,
    pub rom_bytes: [u32; 2],
    pub rom_allocated_bytes: [u32; 2],
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
    pub ym2610: AdditionalChipStatus,
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
    pub status: u8,
    pub status_hi: u8,
    pub variant: &'static str,
    pub rom_bytes: [u32; 2],
    pub rom_allocated_bytes: [u32; 2],
    pub irq: bool,
    pub held: Vec<u8>,
    pub pending_commands: Vec<u64>,
}
impl Shared {
    fn clock_held_error(&self) -> &'static str {
        if self.additional {
            "Release the manually held additional YM2610 voices before changing the clock"
        } else {
            "Release the manually held YM2610 voices before changing the clock"
        }
    }
    pub fn additional_status(&self, frame: u64) -> Result<AdditionalStatus, String> {
        let value = self.status(frame)?;
        Ok(AdditionalStatus {
            frame: value.frame,
            sample_rate: value.sample_rate,
            revision: value.revision,
            ym2610: AdditionalChipStatus {
                clock: value.clock,
                registers: value.registers,
                key_masks: value.key_masks,
                ssg_levels: value.ssg_levels,
                ssg_gates: value.ssg_gates,
                source_mute_mask: value.source_mute_mask,
                status: value.status,
                status_hi: value.status_hi,
                variant: value.variant,
                rom_bytes: value.rom_bytes,
                rom_allocated_bytes: value.rom_allocated_bytes,
                irq: value.irq,
                held: value.held,
                pending_commands: value.pending_commands,
            },
        })
    }
    pub fn clock(&self) -> u32 {
        self.clock.load(Ordering::Acquire)
    }
    pub fn valid_event(&self, _: &Prepared, sequence: u64, owner: Option<Owner>) -> bool {
        sequence >= self.cancel_before.load(Ordering::Acquire)
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
        c.page_guards.retain(|page| Arc::strong_count(page) > 1);
        if c.page_guards.capacity() > 64.max(c.page_guards.len().saturating_mul(2)) {
            c.page_guards.shrink_to_fit();
        }
    }
    pub fn collect_retired(&self) {
        if let Ok(mut c) = self.control.lock() {
            self.collect(&mut c)
        }
    }
    pub fn with_budget(budget: Arc<extra_budget::Budget>) -> Self {
        Self {
            budget,
            ..Self::default()
        }
    }
    pub fn prepare_message(
        &self,
        message: &Message,
        instance: u64,
        plan: &mut PagePlan,
    ) -> Result<Arc<Prepared>, String> {
        let command = message.prepare()?;
        if let Method::Memory {
            rom_type,
            offset,
            data,
        } = &message.method
        {
            let c = self
                .control
                .lock()
                .map_err(|_| "Native YM2610 control state unavailable")?;
            let first = *offset as usize / PAGE;
            let count = page_count(*offset, data.len());
            let mut pages = Vec::with_capacity(count);
            for index in first..first + count {
                let key = (instance, *rom_type, index as u16);
                let previous = plan
                    .new
                    .get(&key)
                    .or_else(|| c.pages[*rom_type as usize * 256 + index].as_ref());
                let memory = self.budget.reserve(PAGE + 64)?;
                let ptr = unsafe { native_ym2610b_alloc_rom_page() };
                if ptr.is_null() {
                    return Err("Could not allocate Native YM2610 ROM page".into());
                }
                let page = Arc::new(Page {
                    bank: *rom_type,
                    ptr,
                    _memory: memory,
                });
                // YM2610 ROM pages are immutable after publication. The callback reads
                // their bytes but never writes them, so cloning the latest accepted
                // page on this control thread cannot race the audio callback.
                unsafe {
                    if let Some(old) = previous {
                        std::ptr::copy_nonoverlapping(old.ptr as *const u8, ptr as *mut u8, PAGE);
                    }
                    let start = (*offset as usize).max(index * PAGE);
                    let end = (*offset as usize + data.len()).min((index + 1) * PAGE);
                    std::ptr::copy_nonoverlapping(
                        data.as_ptr().add(start - *offset as usize),
                        (ptr as *mut u8).add(start - index * PAGE),
                        end - start,
                    );
                }
                plan.new.insert(key, page.clone());
                pages.push(PageRef {
                    bank: *rom_type,
                    index: index as u16,
                    page,
                });
            }
            command
                .pages
                .set(pages)
                .unwrap_or_else(|_| unreachable!("Fresh ROM preparation"));
        }
        Ok(command)
    }
    pub fn commit_pages(&self, command: &Prepared) {
        if let Some(pages) = command.pages.get() {
            let mut c = self.control.lock().expect("Native YM2610 page ledger");
            for p in pages {
                c.pages[p.bank as usize * 256 + p.index as usize] = Some(p.page.clone());
                // Every immutable version needs a control-side lifetime guard until
                // the callback has swapped away its last reference.
                c.page_guards.push(p.page.clone());
            }
        }
    }
    pub fn retain(&self, commands: &[(Arc<Prepared>, Option<Owner>)]) -> Result<(), String> {
        let mut c = self
            .control
            .lock()
            .map_err(|_| "Native YM2610 control state unavailable")?;
        self.collect(&mut c);
        if self.held.load(Ordering::Acquire) != 0
            && commands
                .iter()
                .any(|(v, _)| !v.audition && matches!(v.method, Method::Clock { .. }))
        {
            return Err(self.clock_held_error().into());
        }
        if c.prepared.len() + commands.len() > CAPACITY
            || c.prepared.iter().map(|v| v.bytes()).sum::<usize>()
                + commands.iter().map(|(v, _)| v.bytes()).sum::<usize>()
                > PENDING_BYTES
        {
            return Err("Native YM2610 pending command limit reached (40 MiB)".into());
        }
        let mut ids = HashSet::new();
        for id in c
            .prepared
            .iter()
            .filter_map(|v| v.id)
            .chain(commands.iter().filter_map(|(v, _)| v.id))
        {
            if !ids.insert(id) {
                return Err("Duplicate pending Native YM2610 command ID".into());
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
            return Err("Unknown Native YM2610 command ID".into());
        }
        let cmd = {
            let mut control = self
                .control
                .lock()
                .map_err(|_| "Native YM2610 control state unavailable")?;
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
                let deadline = if matches!(cmd.method, Method::Clock { .. } | Method::Reset) {
                    5000
                } else {
                    100
                };
                if start.elapsed() > std::time::Duration::from_millis(deadline) {
                    return Err(format!(
                        "Native YM2610 command was not applied within {deadline} ms"
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            cmd.outcome_observed.store(true, Ordering::Release);
            if cmd.clock_rejected.load(Ordering::Acquire) {
                self.control
                    .lock()
                    .map_err(|_| "Native YM2610 control state unavailable")?
                    .rejected_clocks
                    .remove(&id);
                return Err(self.clock_held_error().into());
            }
        }
        Ok(())
    }
    pub fn rollback(&self, cmd: Arc<Prepared>) {
        self.retire(cmd)
    }
    pub fn retire(&self, cmd: Arc<Prepared>) {
        cmd.complete.store(true, Ordering::Release);
        self.retired
            .push(cmd)
            .unwrap_or_else(|_| unreachable!("Bounded YM2610 command retirement"));
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
        for (out, value) in self.rom_bytes.iter().zip(r.rom_bytes) {
            out.store(value, Ordering::Relaxed)
        }
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
            .map_err(|_| "Native YM2610 control unavailable")?;
        self.collect(&mut control);
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() > std::time::Duration::from_millis(50) {
                return Err("Native YM2610 state changed while reading; retry".into());
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
                status: self.status.load(Ordering::Relaxed),
                irq: self.irq.load(Ordering::Relaxed) != 0,
                status_hi: self.status_hi.load(Ordering::Relaxed),
                variant: "ym2610",
                rom_bytes: std::array::from_fn(|i| self.rom_bytes[i].load(Ordering::Relaxed)),
                rom_allocated_bytes: std::array::from_fn(|bank| {
                    control
                        .page_guards
                        .iter()
                        .filter(|p| p.bank as usize == bank)
                        .count() as u32
                        * PAGE as u32
                }),
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
pub struct Renderer {
    additional: bool,
    current_group: Option<u64>,
    csm_owner: Option<Lease>,
    rom_bytes: [u32; 2],
    pages: [Option<Arc<Page>>; 512],
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
        unsafe { native_ym2610b_destroy(self.ptr) }
    }
}
impl Renderer {
    pub fn new_additional(rate: u32) -> Result<Self, String> {
        Self::new_flavor(rate, true)
    }
    fn new_flavor(rate: u32, additional: bool) -> Result<Self, String> {
        let ptr = unsafe { native_ym2610b_create_extra(rate) };
        if ptr.is_null() {
            return Err("Could not allocate Native YM2610 core".into());
        }
        Ok(Self {
            additional,
            current_group: None,
            csm_owner: None,
            ptr,
            rom_bytes: [0; 2],
            pages: std::array::from_fn(|_| None),
            rate,
            owners: [None; VOICES],
            gains: [1.; 2],
            targets: [1.; 2],
            ramp: 0,
        })
    }
    fn state(&self) -> CoreState {
        let mut state = CoreState::default();
        unsafe { native_ym2610b_get_extra_state(self.ptr, &mut state) }
        state
    }
    fn has_manual(&self) -> bool {
        self.owners.iter().flatten().any(|l| l.audition)
            || self.csm_owner.is_some_and(|l| l.audition)
    }
    fn protected(&self, ch: usize, voice: Option<u64>, audition: bool) -> bool {
        if ch == 0 || ch == 3 {
            return false;
        }
        let protected = |l: Lease| l.audition && (!audition || voice.is_some() && l.voice != voice);
        self.owners[ch].is_some_and(protected) || ch == 2 && self.csm_owner.is_some_and(protected)
    }
    fn release(&mut self, ch: usize) {
        unsafe {
            if self.additional {
                native_ym2610b_key_off_fm(self.ptr, ch as u8)
            } else {
                native_ym2610b_key_off(self.ptr, ch as u8)
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
                    register: 6 | 11..=13,
                    ..
                } => Some(6..9),
                Op::Write {
                    port: 1,
                    register: 1,
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
            unsafe { native_ym2610b_write(self.ptr, 0, 0x27, value) }
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
                port: 0,
                register: 0x10,
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
                port: 0,
                register: 0x10,
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
                Op::KeyOff { channel } => native_ym2610b_key_off_fm(self.ptr, channel),
                Op::Clock { hz } => {
                    native_ym2610b_set_clock(self.ptr, hz);
                }
                Op::SetRetainedLevel { channel, level } => {
                    native_ym2610b_set_retained_ssg_level(self.ptr, channel, level)
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
                    native_ym2610b_write(self.ptr, 0, 7, value)
                }
                Op::Write {
                    port: 1,
                    register: 0,
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
                    native_ym2610b_write(self.ptr, 1, 0, value);
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
                        native_ym2610b_audition_fm_key_on(self.ptr, ch as u8, value >> 4)
                    }
                }
                Op::Write {
                    port,
                    register,
                    value,
                } => native_ym2610b_write(self.ptr, port, register, value),
            }
        }
        if !preserve {
            if let Some(ch) = channel.filter(|ch| *ch != 0 && *ch != 3) {
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
                native_ym2610b_set_source_mute_mask(self.ptr, *mask)
            },

            Method::Reset => {
                if c.audition || !self.has_manual() {
                    unsafe { native_ym2610b_reset(self.ptr) }
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
                        native_ym2610b_cancel_csm(self.ptr);
                    }
                    self.csm_owner = None;
                }
            }
            Method::CancelMainOwner { target_group } => {
                let matches = |lease: Lease| !lease.audition && lease.group == Some(*target_group);
                if self.csm_owner.is_some_and(matches) {
                    unsafe {
                        native_ym2610b_cancel_csm(self.ptr);
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
                    unsafe { native_ym2610b_stop(self.ptr) }
                    self.owners.fill(None);
                    self.csm_owner = None;
                } else {
                    if self.csm_owner.is_some_and(|lease| {
                        (owner.is_none() || lease.owner == owner)
                            && (c.group.is_none() || lease.group == c.group)
                            && (!lease.audition || c.audition)
                    }) {
                        unsafe {
                            native_ym2610b_cancel_csm(self.ptr);
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
            Method::Memory {
                rom_type, offset, ..
            } => {
                for page in c.pages.get().expect("Prepared sparse upload") {
                    let result = unsafe {
                        native_ym2610b_install_rom_page(
                            self.ptr,
                            page.bank,
                            page.index,
                            page.page.ptr,
                        )
                    };
                    debug_assert_eq!(result, 1);
                    self.pages[page.bank as usize * 256 + page.index as usize] =
                        Some(page.page.clone());
                }
                self.rom_bytes[*rom_type as usize] =
                    self.rom_bytes[*rom_type as usize].max(*offset + c.upload_len as u32);
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
    pub fn publish(&self, s: &Shared) {
        s.publish(self, None)
    }
    pub fn cancel_invalid(&mut self, valid: impl Fn(Option<Owner>) -> bool, s: &Shared) {
        let mut changed = false;
        if self.additional && self.csm_owner.is_some_and(|lease| !valid(lease.owner)) {
            unsafe {
                native_ym2610b_cancel_csm(self.ptr);
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
        unsafe { native_ym2610b_sample(self.ptr, &mut l, &mut r) }
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
    extern "C" {
        fn native_ym2610b_copy_rom(
            ptr: *mut c_void,
            bank: u8,
            data: *mut u8,
            offset: u32,
            size: u32,
        ) -> i32;
    }
    fn upload(
        state: &Shared,
        bank: u8,
        offset: u32,
        data: Vec<u8>,
        plan: &mut PagePlan,
    ) -> Arc<Prepared> {
        state
            .prepare_message(
                &Message {
                    id: None,
                    audition: false,
                    group: None,
                    method: Method::Memory {
                        rom_type: bank,
                        offset,
                        data,
                    },
                },
                1,
                plan,
            )
            .unwrap()
    }
    fn bytes(core: &Renderer, bank: u8, offset: u32, len: usize) -> Vec<u8> {
        let mut out = vec![0; len];
        assert_eq!(
            unsafe {
                native_ym2610b_copy_rom(core.ptr, bank, out.as_mut_ptr(), offset, len as u32)
            },
            1
        );
        out
    }
    #[test]
    fn cow_partial_pages_layer_transactions_and_publish_atomically_without_modifying_old_versions()
    {
        assert_eq!(std::mem::size_of::<CoreState>(), 536);
        let budget = Arc::new(extra_budget::Budget::default());
        let state = Shared::with_budget(budget.clone());
        let mut core = Renderer::new_additional(48000).unwrap();
        let mut plan = PagePlan::default();
        let a = upload(&state, 0, 65534, vec![1, 2, 3, 4], &mut plan);
        let b = upload(&state, 0, 65535, vec![9, 8], &mut plan);
        let c = upload(&state, 1, 65535, vec![5, 6], &mut plan);
        assert_eq!(bytes(&core, 0, 65534, 4), [0, 0, 0, 0]);
        assert_eq!(a.upload_bytes(), 4);
        assert!(a.retained_bytes() < 1024);
        assert!(matches!(&a.method,Method::Memory{data,..}if data.is_empty()));
        for command in [&a, &b, &c] {
            state.commit_pages(command);
            state.retain(&[(command.clone(), None)]).unwrap();
        }
        core.apply(&a, None, &state);
        assert_eq!(bytes(&core, 0, 65534, 4), [1, 2, 3, 4]);
        core.apply(&b, None, &state);
        assert_eq!(bytes(&core, 0, 65534, 4), [1, 9, 8, 4]);
        assert_eq!(bytes(&core, 1, 65534, 4), [0, 0, 0, 0]);
        core.apply(&c, None, &state);
        assert_eq!(bytes(&core, 1, 65534, 4), [0, 5, 6, 0]);
        // A subsequent partial update starts from the latest accepted immutable page.
        let d = upload(&state, 0, 65536, vec![7], &mut PagePlan::default());
        state.commit_pages(&d);
        core.apply(&d, None, &state);
        assert_eq!(bytes(&core, 0, 65534, 4), [1, 9, 7, 4]);
        for cmd in [a, b, c] {
            state.retire(cmd)
        }
        drop(plan);
        state.collect_retired();
        assert!(budget.used() > 0);
    }
    #[test]
    fn unpublished_page_rollback_releases_all_permits_and_does_not_change_committed_rom() {
        let budget = Arc::new(extra_budget::Budget::new(4 * (PAGE + 64)));
        let state = Shared::with_budget(budget.clone());
        let mut core = Renderer::new_additional(48000).unwrap();
        let a = upload(&state, 0, 0, vec![1], &mut PagePlan::default());
        state.commit_pages(&a);
        core.apply(&a, None, &state);
        drop(a);
        let baseline = budget.used();
        {
            let mut plan = PagePlan::default();
            let b = upload(&state, 0, 0, vec![2], &mut plan);
            let c = upload(&state, 1, 0, vec![3], &mut plan);
            assert!(budget.used() > baseline);
            assert_eq!(bytes(&core, 0, 0, 1), [1]);
            drop((b, c));
        }
        assert_eq!(budget.used(), baseline);
        assert_eq!(bytes(&core, 0, 0, 1), [1]);
        assert_eq!(bytes(&core, 1, 0, 1), [0]);
    }
    #[test]
    fn old_page_versions_survive_stalled_callback_and_release_after_pointer_swap() {
        let budget = Arc::new(extra_budget::Budget::new(4 * (PAGE + 64)));
        let state = Shared::with_budget(budget.clone());
        let mut core = Renderer::new_additional(48000).unwrap();
        let a = upload(&state, 0, 0, vec![1], &mut PagePlan::default());
        state.commit_pages(&a);
        core.apply(&a, None, &state);
        drop(a);
        let b = upload(&state, 0, 0, vec![2], &mut PagePlan::default());
        state.commit_pages(&b);
        state.collect_retired();
        assert_eq!(budget.used(), 2 * (PAGE + 64));
        assert_eq!(bytes(&core, 0, 0, 1), [1]);
        core.apply(&b, None, &state);
        drop(b);
        state.collect_retired();
        assert_eq!(budget.used(), PAGE + 64);
        assert_eq!(bytes(&core, 0, 0, 1), [2]);
    }
}
