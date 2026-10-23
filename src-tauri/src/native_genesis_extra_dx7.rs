//! MSFA DX7: one callback-owned, preallocated 16-voice core. All variable-sized
//! control data has a lifetime guard reclaimed on the IPC side, never in audio.
use super::{Owner, CAPACITY, MAX_SAFE_INTEGER, OWNERS};
use crossbeam_queue::ArrayQueue;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    ffi::c_void,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering},
        Arc, Mutex,
    },
};
const BYTES: usize = 155;
const SLOTS: usize = 32;
pub fn storage_bytes() -> usize {
    (unsafe { native_dx7_core_size() }) + 2 * BYTES * SLOTS
}
const PENDING_BYTES: usize = 8 * 1024 * 1024;
extern "C" {
    fn native_dx7_core_size() -> usize;
    fn native_dx7_create(rate: u32) -> *mut c_void;
    fn native_dx7_destroy(ptr: *mut c_void);
    fn native_dx7_voice(ptr: *mut c_void, voice: *const u8, id: u32, live: bool);
    fn native_dx7_note_on_mask(ptr: *mut c_void, note: u8, velocity: u8, preserve: u16) -> i32;
    fn native_dx7_kill_slot(ptr: *mut c_void, slot: u8);
    fn native_dx7_release_slot(ptr: *mut c_void, slot: u8);
    fn native_dx7_panic(ptr: *mut c_void);
    fn native_dx7_sample(ptr: *mut c_void) -> f32;
}
#[derive(Debug, Deserialize)]
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
#[derive(Debug, Deserialize)]
#[serde(tag = "method", rename_all = "kebab-case")]
pub enum Method {
    LoadBank {
        voices: Vec<Vec<u8>>,
    },
    SetPreset {
        voice: Vec<u8>,
        indices: Option<Vec<u8>>,
    },
    EditVoice {
        voice: Vec<u8>,
        indices: Option<Vec<u8>>,
    },
    SelectVoice {
        index: usize,
    },
    NoteOn {
        note: u8,
        #[serde(default = "velocity")]
        velocity: u8,
        voice: Option<u64>,
    },
    NoteOff {
        note: u8,
        voice: Option<u64>,
    },
    CancelMainOwner {
        #[serde(rename = "targetGroup")]
        target_group: u64,
    },
    AllNotesOff,
    Panic,
    Mix {
        volume: f32,
        pan: f32,
    },
}
fn velocity() -> u8 {
    100
}
#[derive(Debug)]
pub enum Edit {
    Load(Vec<[u8; BYTES]>),
    Replace([u8; BYTES], [bool; BYTES]),
    Patch([u8; BYTES], [bool; BYTES]),
    Select(usize),
    On(u8, u8, Option<u64>),
    Off(u8, Option<u64>),
    CancelMainOwner(u64),
    Release,
    Panic,
    Mix(f32, f32),
}
#[derive(Debug)]
pub struct Prepared {
    pub id: Option<u64>,
    pub audition: bool,
    pub group: Option<u64>,
    pub edit: Edit,
    retired: AtomicBool,
    cancelled: AtomicBool,
    patch_rejected: AtomicBool,
    complete: AtomicBool,
    voice_id: AtomicU64,
}
impl Prepared {
    pub fn starts(&self) -> bool {
        matches!(self.edit, Edit::On(_, 1..=127, _))
    }
}
fn voice(bytes: &[u8]) -> Result<[u8; BYTES], String> {
    const OP: [u8; 21] = [
        99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 3, 3, 7, 3, 7, 99, 1, 31, 99, 14,
    ];
    const COMMON: [u8; 19] = [
        99, 99, 99, 99, 99, 99, 99, 99, 31, 7, 1, 99, 99, 99, 99, 1, 5, 7, 48,
    ];
    let out: [u8; BYTES] = bytes
        .try_into()
        .map_err(|_| "DX7 voice must contain exactly 155 unpacked bytes")?;
    for (index, value) in out.iter().enumerate() {
        let max = if index < 126 {
            OP[index % 21]
        } else if index < 145 {
            COMMON[index - 126]
        } else {
            127
        };
        if *value > max {
            return Err(format!("DX7 voice parameter {index} must be 0–{max}"));
        }
    }
    Ok(out)
}
fn edit_mask(indices: Option<&[u8]>) -> Result<[bool; BYTES], String> {
    let mut mask = [indices.is_none(); BYTES];
    if let Some(indices) = indices {
        if indices.len() > BYTES {
            return Err("DX7 edit mask is too large".into());
        }
        for index in indices {
            let index = *index as usize;
            if index >= BYTES || mask[index] {
                return Err("DX7 edit indices must be unique and 0–154".into());
            }
            mask[index] = true;
        }
    }
    Ok(mask)
}
impl Message {
    pub fn prepared_bytes(&self) -> usize {
        std::mem::size_of::<Prepared>()
            + if let Method::LoadBank { voices } = &self.method {
                voices.len() * BYTES
            } else {
                0
            }
    }
    pub fn prepare(&self) -> Result<Arc<Prepared>, String> {
        if !valid_id(self.id) || !valid_id(self.group) {
            return Err("Invalid DX7 command ID".into());
        }
        let edit = match &self.method {
            Method::LoadBank { voices } => {
                if voices.is_empty() || voices.len() > SLOTS {
                    return Err("DX7 bank must contain 1–32 voices".into());
                }
                let mut prepared = Vec::with_capacity(voices.len());
                for bytes in voices {
                    prepared.push(voice(bytes)?);
                }
                Edit::Load(prepared)
            }
            Method::SetPreset {
                voice: bytes,
                indices,
            } => Edit::Replace(voice(bytes)?, edit_mask(indices.as_deref())?),
            Method::EditVoice {
                voice: bytes,
                indices,
            } => {
                let mask = edit_mask(indices.as_deref())?;
                Edit::Patch(voice(bytes)?, mask)
            }
            Method::SelectVoice { index } => {
                if *index >= SLOTS {
                    return Err("DX7 voice index must be 0–31".into());
                }
                Edit::Select(*index)
            }
            Method::NoteOn {
                note,
                velocity,
                voice,
            } => {
                validate_note(*note, *voice)?;
                if *velocity > 127 {
                    return Err("DX7 velocity must be 0–127".into());
                }
                if *velocity == 0 {
                    Edit::Off(*note, *voice)
                } else {
                    Edit::On(*note, *velocity, *voice)
                }
            }
            Method::NoteOff { note, voice } => {
                validate_note(*note, *voice)?;
                Edit::Off(*note, *voice)
            }
            Method::CancelMainOwner { target_group } => {
                if !valid_id(Some(*target_group)) {
                    return Err("Invalid additional DX7 Main owner group".into());
                }
                Edit::CancelMainOwner(*target_group)
            }
            Method::AllNotesOff => Edit::Release,
            Method::Panic => Edit::Panic,
            Method::Mix { volume, pan } => {
                if !volume.is_finite()
                    || !(0.0..=4.0).contains(volume)
                    || !pan.is_finite()
                    || !(-1.0..=1.0).contains(pan)
                {
                    return Err("DX7 mixer volume must be 0–4, pan −1–1".into());
                }
                Edit::Mix(*volume, *pan)
            }
        };
        Ok(Arc::new(Prepared {
            id: self.id,
            audition: self.audition,
            group: self.group,
            edit,
            retired: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
            patch_rejected: AtomicBool::new(false),
            complete: AtomicBool::new(false),
            voice_id: AtomicU64::new(0),
        }))
    }
}
fn validate_note(note: u8, voice: Option<u64>) -> Result<(), String> {
    if note > 127 || voice.is_some_and(|v| v == 0 || v > MAX_SAFE_INTEGER) {
        return Err("Invalid DX7 note or voice lease".into());
    }
    Ok(())
}
pub const PATCH_HELD: &str =
    "Release the manually held additional DX7 voices before changing the voice";
fn valid_id(id: Option<u64>) -> bool {
    id.is_none_or(|id| id > 0 && id <= MAX_SAFE_INTEGER)
}
impl Prepared {
    fn outcome(&self) -> u8 {
        self.cancelled.load(Ordering::Acquire) as u8
            | ((self.patch_rejected.load(Ordering::Acquire) as u8) << 1)
    }
    pub fn is_owner_cancel(&self) -> bool {
        matches!(self.edit, Edit::CancelMainOwner(_))
    }
    pub fn stops(&self) -> bool {
        matches!(self.edit, Edit::Release | Edit::Panic) && self.group.is_none()
    }
    pub fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + if let Edit::Load(voices) = &self.edit {
                voices.capacity() * BYTES
            } else {
                0
            }
    }
    fn patch(&self) -> bool {
        matches!(
            self.edit,
            Edit::Load(_) | Edit::Replace(..) | Edit::Patch(..) | Edit::Select(_)
        )
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
        Err("Native DX7 command was cancelled before it applied".into())
    } else if flags & 2 != 0 {
        Err(PATCH_HELD.into())
    } else {
        Ok(())
    }
}
pub struct Shared {
    revision: AtomicU64,
    voices: Box<[[AtomicU8; BYTES]; SLOTS]>,
    next_voice_id: AtomicU64,
    count: AtomicU8,
    selected: AtomicU8,
    notes: [[AtomicU8; 3]; 16],
    rate: std::sync::atomic::AtomicU32,
    held: AtomicBool,
    control: Mutex<Control>,
    retired: ArrayQueue<Arc<Prepared>>,
    highest_id: AtomicU64,
    cancel_before: AtomicU64,
    owner_cancel_before: [AtomicU64; OWNERS],
}
impl Default for Shared {
    fn default() -> Self {
        let init = init_voice();
        Self {
            revision: AtomicU64::new(0),
            voices: Box::new(std::array::from_fn(|slot| {
                std::array::from_fn(|i| AtomicU8::new(if slot == 0 { init[i] } else { 0 }))
            })),
            next_voice_id: AtomicU64::new(2),
            count: AtomicU8::new(1),
            selected: AtomicU8::new(0),
            notes: std::array::from_fn(|_| std::array::from_fn(|_| AtomicU8::new(0))),
            rate: std::sync::atomic::AtomicU32::new(0),
            held: AtomicBool::new(false),
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
    pub voices: Vec<Vec<u8>>,
    pub selected_voice: u8,
    pub pending_commands: Vec<u64>,
    pub active_notes: Vec<NoteStatus>,
    pub held: Vec<u8>,
}
#[derive(Debug, Serialize)]
pub struct NoteStatus {
    pub midi: u8,
    pub velocity: u8,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub dx7: ChipStatus,
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
            if let Some(id) = v.id {
                outcomes[*cursor] = Completion {
                    id,
                    flags: v.outcome(),
                };
                *cursor = (*cursor + 1) % outcomes.len()
            }
            false
        });
        if prepared.capacity() > 64.max(prepared.len() * 2) {
            prepared.shrink_to_fit()
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
            .map_err(|_| "Native DX7 control unavailable")?;
        self.collect(&mut c);
        if self.held.load(Ordering::Acquire)
            && commands.iter().any(|(v, _)| !v.audition && v.patch())
        {
            return Err(PATCH_HELD.into());
        }
        if c.prepared.len() + commands.len() > CAPACITY
            || c.prepared.iter().map(|v| v.retained_bytes()).sum::<usize>()
                + commands
                    .iter()
                    .map(|(v, _)| v.retained_bytes())
                    .sum::<usize>()
                > PENDING_BYTES
        {
            return Err("Native DX7 pending command limit reached (8 MiB)".into());
        }
        let mut ids = HashSet::new();
        for v in c.prepared.iter().chain(commands.iter().map(|(v, _)| v)) {
            if v.id.is_some_and(|id| !ids.insert(id)) {
                return Err("Duplicate pending Native DX7 command ID".into());
            }
        }
        let identities: u64 = commands
            .iter()
            .map(|(v, _)| match &v.edit {
                Edit::Load(voices) => voices.len() as u64,
                Edit::Replace(..) => 1,
                _ => 0,
            })
            .sum();
        let mut next = self.next_voice_id.load(Ordering::Relaxed);
        if next + identities > u32::MAX as u64 + 1 {
            return Err("Additional DX7 voice identity limit reached".into());
        }
        for (v, _) in commands {
            let count = match &v.edit {
                Edit::Load(voices) => voices.len() as u64,
                Edit::Replace(..) => 1,
                _ => 0,
            };
            if count != 0 {
                v.voice_id.store(next, Ordering::Relaxed);
                next += count;
            }
        }
        self.next_voice_id.store(next, Ordering::Relaxed);
        for (v, _) in commands {
            if let Some(id) = v.id {
                self.highest_id.fetch_max(id, Ordering::AcqRel);
            }
            c.prepared.push(v.clone())
        }
        Ok(())
    }
    pub fn commit_cancel(&self, c: &Prepared) -> bool {
        let Edit::CancelMainOwner(target) = c.edit else {
            return false;
        };
        let control = self
            .control
            .lock()
            .expect("DX7 control checked by transactional retain");
        for v in &control.prepared {
            if v.group == Some(target) && !v.audition && !v.complete.load(Ordering::Acquire) {
                v.cancelled.store(true, Ordering::Release);
            }
        }
        true
    }
    pub fn wait_applied(&self, id: u64) -> Result<(), String> {
        if id == 0 || id > self.highest_id.load(Ordering::Acquire) {
            return Err("Unknown Native DX7 command ID".into());
        }
        let cmd = {
            let c = self
                .control
                .lock()
                .map_err(|_| "Native DX7 control unavailable")?;
            if let Some(v) = c.prepared.iter().find(|v| v.id == Some(id)) {
                v.clone()
            } else {
                for offset in 0..c.outcomes.len() {
                    let result =
                        c.outcomes[(c.cursor + c.outcomes.len() - 1 - offset) % c.outcomes.len()];
                    if result.id == id {
                        return outcome(result.flags);
                    }
                }
                return Err("Native DX7 command outcome has expired".into());
            }
        };
        let start = std::time::Instant::now();
        while !cmd.complete.load(Ordering::Acquire) {
            if start.elapsed() > std::time::Duration::from_millis(5000) {
                return Err("Native DX7 command was not applied within 5000 ms".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        outcome(cmd.outcome())
    }
    pub fn rollback(&self, v: Arc<Prepared>) {
        self.retire(v)
    }
    pub fn retire(&self, v: Arc<Prepared>) {
        if !v.complete.load(Ordering::Acquire) {
            v.cancelled.store(true, Ordering::Release)
        }
        v.complete.store(true, Ordering::Release);
        v.retired.store(true, Ordering::Release);
        self.retired
            .push(v)
            .unwrap_or_else(|_| unreachable!("Bounded DX7 retirement"))
    }
    fn publish(&self, renderer: &Renderer, bank_changed: bool) {
        if bank_changed {
            for (out, bytes) in self.voices[..renderer.count]
                .iter()
                .zip(&renderer.voices[..renderer.count])
            {
                for (out, byte) in out.iter().zip(bytes) {
                    out.store(*byte, Ordering::Relaxed);
                }
            }
            self.count.store(renderer.count as u8, Ordering::Relaxed);
            self.selected
                .store(renderer.selected as u8, Ordering::Relaxed);
        }
        for (out, note) in self.notes.iter().zip(renderer.notes) {
            out[0].store(note.map_or(0, |n| n.note + 1), Ordering::Relaxed);
            out[1].store(note.map_or(0, |n| n.velocity), Ordering::Relaxed);
            out[2].store(note.is_some_and(|n| n.audition) as u8, Ordering::Relaxed);
        }
        self.rate.store(renderer.rate, Ordering::Relaxed);
        self.held.store(
            renderer.notes.iter().flatten().any(|n| n.audition),
            Ordering::Relaxed,
        );
    }
    pub fn status(&self, frame: u64) -> Result<Status, String> {
        let mut c = self
            .control
            .lock()
            .map_err(|_| "Native DX7 control unavailable")?;
        self.collect(&mut c);
        let started = std::time::Instant::now();
        loop {
            if started.elapsed() >= std::time::Duration::from_millis(50) {
                return Err("Native DX7 state changed while reading; retry".into());
            }
            let before = self.revision.load(Ordering::Acquire);
            if before & 1 != 0 {
                std::thread::yield_now();
                continue;
            }
            let count = self.count.load(Ordering::Relaxed) as usize;
            let mut active_notes = Vec::new();
            let mut held = Vec::new();
            for v in &self.notes {
                let note = v[0].load(Ordering::Relaxed);
                if note != 0 {
                    active_notes.push(NoteStatus {
                        midi: note - 1,
                        velocity: v[1].load(Ordering::Relaxed),
                    });
                    if v[2].load(Ordering::Relaxed) != 0 {
                        held.push(note - 1);
                    }
                }
            }
            let status = Status {
                frame,
                sample_rate: self.rate.load(Ordering::Relaxed),
                revision: before / 2,
                dx7: ChipStatus {
                    voices: self.voices[..count]
                        .iter()
                        .map(|v| v.iter().map(|b| b.load(Ordering::Relaxed)).collect())
                        .collect(),
                    selected_voice: self.selected.load(Ordering::Relaxed),
                    active_notes,
                    held,
                    pending_commands: c
                        .prepared
                        .iter()
                        .filter(|v| !v.complete.load(Ordering::Acquire))
                        .filter_map(|v| v.id)
                        .collect(),
                },
            };
            std::sync::atomic::fence(Ordering::Acquire);
            if self.revision.load(Ordering::Relaxed) == before {
                return Ok(status);
            }
            std::thread::yield_now();
        }
    }
}
#[derive(Clone, Copy)]
struct Note {
    note: u8,
    velocity: u8,
    owner: Option<Owner>,
    group: Option<u64>,
    voice: Option<u64>,
    audition: bool,
    serial: u64,
}
pub struct Renderer {
    ptr: *mut c_void,
    voices: Box<[[u8; BYTES]; SLOTS]>,
    ids: [u32; SLOTS],
    count: usize,
    selected: usize,
    notes: [Option<Note>; 16],
    tails: [Option<Note>; 16],
    note_serial: u64,
    gain: [f32; 2],
    target: [f32; 2],
    ramp: u32,
    rate: u32,
}
unsafe impl Send for Renderer {}
impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe { native_dx7_destroy(self.ptr) };
    }
}
impl Renderer {
    pub fn new(rate: u32) -> Result<Self, String> {
        let ptr = unsafe { native_dx7_create(rate) };
        if ptr.is_null() {
            return Err("Could not allocate Native DX7 core".into());
        }
        let mut renderer = Self {
            ptr,
            voices: Box::new([[0; BYTES]; SLOTS]),
            ids: [0; SLOTS],
            count: 1,
            selected: 0,
            notes: [None; 16],
            tails: [None; 16],
            note_serial: 0,
            gain: [1.0; 2],
            target: [1.0; 2],
            ramp: 0,
            rate,
        };
        renderer.voices[0] = init_voice();
        renderer.ids[0] = 1;
        renderer.apply_voice(false);
        Ok(renderer)
    }
    fn apply_voice(&mut self, live: bool) {
        unsafe {
            native_dx7_voice(
                self.ptr,
                self.voices[self.selected].as_ptr(),
                self.ids[self.selected],
                live,
            )
        };
    }
    fn release(&mut self, slot: usize) {
        unsafe { native_dx7_release_slot(self.ptr, slot as u8) };
        self.tails[slot] = self.notes[slot].take();
    }
    fn kill(&mut self, slot: usize) {
        unsafe { native_dx7_kill_slot(self.ptr, slot as u8) };
        self.notes[slot] = None;
        self.tails[slot] = None;
    }
    fn panic(&mut self) {
        unsafe { native_dx7_panic(self.ptr) };
        self.notes = [None; 16];
        self.tails = [None; 16];
        self.apply_voice(false);
    }
    pub fn apply(&mut self, command: &Prepared, owner: Option<Owner>, shared: &Shared) {
        shared.revision.fetch_add(1, Ordering::AcqRel);
        let manual = self.notes.iter().flatten().any(|n| n.audition);
        let patch = matches!(
            command.edit,
            Edit::Load(_) | Edit::Replace(..) | Edit::Patch(..) | Edit::Select(_)
        );
        let note = match command.edit {
            Edit::On(n, ..) | Edit::Off(n, _) => Some(n),
            _ => None,
        };
        let blocked = !command.audition
            && ((manual && patch)
                || note.is_some_and(|note| {
                    self.notes
                        .iter()
                        .flatten()
                        .any(|n| n.audition && n.note == note)
                }));
        let mut bank_changed = false;
        if manual && patch && !command.audition {
            command.patch_rejected.store(true, Ordering::Release);
        }
        if !blocked {
            match &command.edit {
                Edit::Load(voices) => {
                    self.count = voices.len();
                    self.selected = 0;
                    for (index, voice) in voices.iter().enumerate() {
                        self.voices[index] = *voice;
                        self.ids[index] =
                            (command.voice_id.load(Ordering::Relaxed) + index as u64) as u32;
                    }
                    self.apply_voice(false);
                    bank_changed = true;
                }
                Edit::Replace(voice, mask) => {
                    for i in 0..BYTES {
                        if mask[i] {
                            self.voices[self.selected][i] = voice[i];
                        }
                    }
                    self.ids[self.selected] = command.voice_id.load(Ordering::Relaxed) as u32;
                    self.apply_voice(false);
                    bank_changed = true;
                }
                Edit::Patch(voice, mask) => {
                    for i in 0..BYTES {
                        if mask[i] {
                            self.voices[self.selected][i] = voice[i];
                        }
                    }
                    self.apply_voice(true);
                    bank_changed = true;
                }
                Edit::Select(index) => {
                    if *index < self.count {
                        self.selected = *index;
                        self.apply_voice(false);
                        bank_changed = true;
                    }
                }
                Edit::On(note, velocity, voice) => {
                    // Manual audition owns this pitch until its matching lease
                    // releases it; future queued performance work is blocked.
                    if command.audition {
                        for slot in 0..16 {
                            if self.notes[slot].is_some_and(|n| n.note == *note) {
                                self.release(slot);
                            }
                        }
                    }
                    let preserve = self.notes.iter().enumerate().fold(0u16, |mask, (slot, n)| {
                        if n.is_some_and(|n| n.audition) {
                            mask | (1 << slot)
                        } else {
                            mask
                        }
                    });
                    let mut slot =
                        unsafe { native_dx7_note_on_mask(self.ptr, *note, *velocity, preserve) };
                    if slot < 0 && command.audition {
                        if let Some(oldest) = self
                            .notes
                            .iter()
                            .enumerate()
                            .filter_map(|(slot, n)| {
                                n.filter(|n| !n.audition).map(|n| (slot, n.serial))
                            })
                            .min_by_key(|(_, serial)| *serial)
                            .map(|(slot, _)| slot)
                        {
                            self.kill(oldest);
                            slot = unsafe {
                                native_dx7_note_on_mask(self.ptr, *note, *velocity, preserve)
                            };
                        }
                    }
                    if slot >= 0 {
                        self.tails[slot as usize] = None;
                        self.note_serial = self.note_serial.saturating_add(1);
                        self.notes[slot as usize] = Some(Note {
                            note: *note,
                            velocity: *velocity,
                            owner,
                            group: command.group,
                            voice: *voice,
                            audition: command.audition,
                            serial: self.note_serial,
                        });
                    }
                }
                Edit::Off(note, voice) => {
                    // Web noteOff releases layered repetitions of this pitch.
                    // A lease additionally fences stale cleanup from a newer
                    // key-down: the latest lease releases all of its older layers.
                    let latest = self
                        .notes
                        .iter()
                        .flatten()
                        .filter(|n| {
                            n.note == *note
                                && n.owner == owner
                                && n.group == command.group
                                && n.audition == command.audition
                        })
                        .max_by_key(|n| n.serial);
                    let matches = voice.is_none() || latest.is_some_and(|n| n.voice == *voice);
                    if matches {
                        for slot in 0..16 {
                            if self.notes[slot].is_some_and(|n| {
                                n.note == *note
                                    && n.owner == owner
                                    && n.group == command.group
                                    && n.audition == command.audition
                            }) {
                                self.release(slot);
                            }
                        }
                    }
                }
                Edit::CancelMainOwner(target) => {
                    for slot in 0..16 {
                        if self.notes[slot]
                            .or(self.tails[slot])
                            .is_some_and(|n| !n.audition && n.group == Some(*target))
                        {
                            self.kill(slot);
                        }
                    }
                }
                Edit::Release | Edit::Panic if owner.is_some() || command.group.is_some() => {
                    for slot in 0..16 {
                        let note = if matches!(command.edit, Edit::Panic) {
                            self.notes[slot].or(self.tails[slot])
                        } else {
                            self.notes[slot]
                        };
                        if note.is_some_and(|n| {
                            (owner.is_none() || n.owner == owner)
                                && (command.group.is_none() || n.group == command.group)
                                && (!n.audition || command.audition)
                        }) {
                            if matches!(command.edit, Edit::Panic) {
                                self.kill(slot);
                            } else {
                                self.release(slot);
                            }
                        }
                    }
                }
                Edit::Release => {
                    for slot in 0..16 {
                        if self.notes[slot].is_some_and(|n| !n.audition || command.audition) {
                            self.release(slot);
                        }
                    }
                }
                Edit::Panic => {
                    if manual && !command.audition {
                        for slot in 0..16 {
                            if self.notes[slot]
                                .or(self.tails[slot])
                                .is_some_and(|n| !n.audition)
                            {
                                self.kill(slot);
                            }
                        }
                    } else {
                        self.panic();
                    }
                }
                Edit::Mix(volume, pan) => {
                    // SoundChipMixer splits the duplicated mono stereo output
                    // and applies linear balance; center preserves unity.
                    self.target = [
                        *volume * (1.0 - pan.max(0.0)),
                        *volume * (1.0 + pan.min(0.0)),
                    ];
                    self.ramp = (self.rate / 200).max(1);
                }
            }
        }
        shared.publish(self, bank_changed);
        command.complete.store(true, Ordering::Release);
        shared.revision.fetch_add(1, Ordering::Release);
    }
    pub fn cancel_invalid(&mut self, valid: impl Fn(Option<Owner>) -> bool, shared: &Shared) {
        if !(0..16).any(|slot| {
            self.notes[slot]
                .or(self.tails[slot])
                .is_some_and(|n| !valid(n.owner))
        }) {
            return;
        }
        shared.revision.fetch_add(1, Ordering::AcqRel);
        for slot in 0..16 {
            if self.notes[slot]
                .or(self.tails[slot])
                .is_some_and(|n| !valid(n.owner))
            {
                self.kill(slot);
            }
        }
        shared.publish(self, false);
        shared.revision.fetch_add(1, Ordering::Release);
    }
    pub fn publish(&self, shared: &Shared) {
        shared.revision.fetch_add(1, Ordering::AcqRel);
        shared.publish(self, true);
        shared.revision.fetch_add(1, Ordering::Release);
    }
    pub fn mix(&mut self, pcm: &mut [f32; 2]) {
        if self.ramp != 0 {
            for ch in 0..2 {
                self.gain[ch] += (self.target[ch] - self.gain[ch]) / self.ramp as f32;
            }
            self.ramp -= 1;
        }
        let mono = unsafe { native_dx7_sample(self.ptr) };
        pcm[0] += mono * self.gain[0];
        pcm[1] += mono * self.gain[1];
    }
}
pub fn init_voice() -> [u8; BYTES] {
    let mut data = [0; BYTES];
    for op in 0..6 {
        let d = op * 21;
        data[d..d + 8].copy_from_slice(&[99, 99, 99, 70, 99, 99, 99, 0]);
        data[d + 16] = if op == 5 { 85 } else { 0 };
        data[d + 18] = 1;
        data[d + 20] = 7;
    }
    data[126..130].fill(99);
    data[130..134].fill(50);
    data[134] = 31;
    data[136] = 1;
    data[144] = 24;
    data[145..155].fill(32);
    data[145..154].copy_from_slice(b"INIT SINE");
    data
}
