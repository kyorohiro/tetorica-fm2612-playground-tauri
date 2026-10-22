//! MSFA DX7: one callback-owned, preallocated 16-voice core. All variable-sized
//! control data has a lifetime guard reclaimed on the IPC side, never in audio.
use super::{Owner, CAPACITY, MAX_SAFE_INTEGER};
use crossbeam_queue::ArrayQueue;
use serde::{Deserialize, Serialize};
use std::{
    ffi::c_void,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering},
        Arc, Mutex,
    },
};
const BYTES: usize = 155;
const SLOTS: usize = 32;
const PENDING_BYTES: usize = 8 * 1024 * 1024;
extern "C" {
    fn native_dx7_create(rate: u32) -> *mut c_void;
    fn native_dx7_destroy(ptr: *mut c_void);
    fn native_dx7_voice(ptr: *mut c_void, voice: *const u8, id: u32, live: bool);
    fn native_dx7_note_on(ptr: *mut c_void, note: u8, velocity: u8) -> i32;
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
    Replace([u8; BYTES]),
    Patch([u8; BYTES], [bool; BYTES]),
    Select(usize),
    On(u8, u8, Option<u64>),
    Off(u8, Option<u64>),
    Release,
    Panic,
    Mix(f32, f32),
}
#[derive(Debug)]
pub struct Prepared {
    pub id: Option<u64>,
    pub audition: bool,
    pub edit: Edit,
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
impl Message {
    pub fn prepare(&self) -> Result<Arc<Prepared>, String> {
        if self.id.is_some_and(|id| id == 0 || id > MAX_SAFE_INTEGER) {
            return Err("Invalid DX7 command ID".into());
        }
        let edit = match &self.method {
            Method::LoadBank { voices } => {
                if voices.is_empty() || voices.len() > SLOTS {
                    return Err("DX7 bank must contain 1–32 voices".into());
                }
                Edit::Load(voices.iter().map(|v| voice(v)).collect::<Result<_, _>>()?)
            }
            Method::SetPreset { voice: bytes } => Edit::Replace(voice(bytes)?),
            Method::EditVoice {
                voice: bytes,
                indices,
            } => {
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
            edit,
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
pub struct Shared {
    revision: AtomicU64,
    voices: [[AtomicU8; BYTES]; SLOTS],
    next_voice_id: AtomicU64,
    count: AtomicU8,
    selected: AtomicU8,
    notes: [[AtomicU8; 2]; 16],
    prepared: Mutex<Vec<Arc<Prepared>>>,
    retired: ArrayQueue<Arc<Prepared>>,
}
impl Default for Shared {
    fn default() -> Self {
        let init = init_voice();
        Self {
            revision: AtomicU64::new(0),
            voices: std::array::from_fn(|slot| {
                std::array::from_fn(|i| AtomicU8::new(if slot == 0 { init[i] } else { 0 }))
            }),
            next_voice_id: AtomicU64::new(2),
            count: AtomicU8::new(1),
            selected: AtomicU8::new(0),
            notes: std::array::from_fn(|_| std::array::from_fn(|_| AtomicU8::new(0))),
            prepared: Mutex::new(Vec::new()),
            retired: ArrayQueue::new(CAPACITY),
        }
    }
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub voices: Vec<Vec<u8>>,
    pub selected_voice: u8,
    pub revision: u64,
    pub frame: u64,
    pub pending_commands: Vec<u64>,
    pub active_notes: Vec<NoteStatus>,
}
#[derive(Debug, Serialize)]
pub struct NoteStatus {
    midi: u8,
    velocity: u8,
}
impl Shared {
    pub fn retain(&self, commands: &[Arc<Prepared>]) -> Result<(), String> {
        let mut registry = self
            .prepared
            .lock()
            .map_err(|_| "Native DX7 control state is unavailable")?;
        self.collect(&mut registry);
        if registry.len() + commands.len() > CAPACITY {
            return Err("Native DX7 pending command limit reached".into());
        }
        let bytes: usize = registry
            .iter()
            .chain(commands)
            .map(|command| {
                std::mem::size_of::<Prepared>()
                    + if let Edit::Load(voices) = &command.edit {
                        voices.len() * BYTES
                    } else {
                        0
                    }
            })
            .sum();
        if bytes > PENDING_BYTES {
            return Err("Native DX7 pending command memory limit (8 MiB) reached".into());
        }
        // IDs are frontend request leases; duplicate pending IDs cannot be acknowledged safely.
        let mut ids = std::collections::HashSet::new();
        for command in registry.iter().chain(commands) {
            if command.id.is_some_and(|id| !ids.insert(id)) {
                return Err("Duplicate pending DX7 command ID".into());
            }
        }
        let identities: u64 = commands
            .iter()
            .map(|command| match &command.edit {
                Edit::Load(voices) => voices.len() as u64,
                Edit::Replace(_) => 1,
                _ => 0,
            })
            .sum();
        let mut next = self.next_voice_id.load(Ordering::Relaxed);
        if next + identities > u32::MAX as u64 + 1 {
            return Err(
                "DX7 voice identity limit reached; Stop and select the output again".into(),
            );
        }
        for command in commands {
            let count = match &command.edit {
                Edit::Load(voices) => voices.len() as u64,
                Edit::Replace(_) => 1,
                _ => 0,
            };
            if count != 0 {
                command.voice_id.store(next, Ordering::Relaxed);
                next += count;
            }
        }
        self.next_voice_id.store(next, Ordering::Relaxed);
        registry.extend(commands.iter().cloned());
        Ok(())
    }
    fn collect(&self, registry: &mut Vec<Arc<Prepared>>) {
        let mut retired = std::collections::HashSet::new();
        while let Some(command) = self.retired.pop() {
            retired.insert(Arc::as_ptr(&command) as usize);
        }
        registry.retain(|command| !retired.contains(&(Arc::as_ptr(command) as usize)));
    }
    pub fn retire(&self, command: Arc<Prepared>) {
        if !command.complete.load(Ordering::Relaxed) {
            self.revision.fetch_add(1, Ordering::AcqRel);
            command.complete.store(true, Ordering::Relaxed);
            self.revision.fetch_add(1, Ordering::Release);
        }
        self.retired
            .push(command)
            .unwrap_or_else(|_| unreachable!("Bounded DX7 control retirement"));
    }
    pub fn status(&self, frame: u64) -> Result<Status, String> {
        let mut registry = self
            .prepared
            .lock()
            .map_err(|_| "Native DX7 control state is unavailable")?;
        self.collect(&mut registry);
        // A 32-voice bank publish writes about 5 KiB of atomics. A fixed 64
        // spin budget can expire before that callback finishes, so give the
        // control thread a bounded scheduling window instead. The writer never
        // waits for this reader or takes the registry lock.
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
            let selected_voice = self.selected.load(Ordering::Relaxed);
            let voices = self.voices[..count]
                .iter()
                .map(|v| v.iter().map(|b| b.load(Ordering::Relaxed)).collect())
                .collect();
            let active_notes = self
                .notes
                .iter()
                .filter_map(|v| {
                    let note = v[0].load(Ordering::Relaxed);
                    (note != 0).then(|| NoteStatus {
                        midi: note - 1,
                        velocity: v[1].load(Ordering::Relaxed),
                    })
                })
                .collect();
            let pending_commands = registry
                .iter()
                .filter(|v| !v.complete.load(Ordering::Relaxed))
                .filter_map(|v| v.id)
                .collect();
            std::sync::atomic::fence(Ordering::Acquire);
            if self.revision.load(Ordering::Relaxed) == before {
                return Ok(Status {
                    voices,
                    selected_voice,
                    revision: before / 2,
                    frame,
                    pending_commands,
                    active_notes,
                });
            }
            std::thread::yield_now();
        }
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
        }
    }
}
#[derive(Clone, Copy)]
struct Note {
    note: u8,
    velocity: u8,
    owner: Option<Owner>,
    voice: Option<u64>,
    audition: bool,
    serial: u64,
}
pub struct Renderer {
    ptr: *mut c_void,
    voices: [[u8; BYTES]; SLOTS],
    ids: [u32; SLOTS],
    count: usize,
    selected: usize,
    notes: [Option<Note>; 16],
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
            voices: [[0; BYTES]; SLOTS],
            ids: [0; SLOTS],
            count: 1,
            selected: 0,
            notes: [None; 16],
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
        self.notes[slot] = None;
    }
    fn panic(&mut self) {
        unsafe { native_dx7_panic(self.ptr) };
        self.notes = [None; 16];
        self.apply_voice(false);
    }
    pub fn apply(&mut self, command: &Prepared, owner: Option<Owner>, shared: &Shared) {
        shared.revision.fetch_add(1, Ordering::AcqRel);
        let manual = self.notes.iter().flatten().any(|n| n.audition);
        let patch = matches!(
            command.edit,
            Edit::Load(_) | Edit::Replace(_) | Edit::Patch(..) | Edit::Select(_)
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
                Edit::Replace(voice) => {
                    self.voices[self.selected] = *voice;
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
                    let slot = unsafe { native_dx7_note_on(self.ptr, *note, *velocity) };
                    if slot >= 0 {
                        self.note_serial = self.note_serial.saturating_add(1);
                        self.notes[slot as usize] = Some(Note {
                            note: *note,
                            velocity: *velocity,
                            owner,
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
                        .filter(|n| n.note == *note && n.owner == owner)
                        .max_by_key(|n| n.serial);
                    let matches = voice.is_none() || latest.is_some_and(|n| n.voice == *voice);
                    if matches {
                        for slot in 0..16 {
                            if self.notes[slot].is_some_and(|n| {
                                n.note == *note
                                    && n.owner == owner
                                    && (!n.audition || command.audition)
                            }) {
                                self.release(slot);
                            }
                        }
                    }
                }
                Edit::Release | Edit::Panic if owner.is_some() => {
                    for slot in 0..16 {
                        if self.notes[slot]
                            .is_some_and(|n| n.owner == owner && (!n.audition || command.audition))
                        {
                            self.release(slot);
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
                            if self.notes[slot].is_some_and(|n| !n.audition) {
                                self.release(slot);
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
        command.complete.store(true, Ordering::Relaxed);
        shared.revision.fetch_add(1, Ordering::Release);
    }
    pub fn cancel_invalid(&mut self, valid: impl Fn(Option<Owner>) -> bool, shared: &Shared) {
        shared.revision.fetch_add(1, Ordering::AcqRel);
        for slot in 0..16 {
            if self.notes[slot].is_some_and(|n| !valid(n.owner)) {
                self.release(slot);
            }
        }
        shared.publish(self, false);
        shared.revision.fetch_add(1, Ordering::Release);
    }
    pub fn stop(&mut self, shared: &Shared) {
        shared.revision.fetch_add(1, Ordering::AcqRel);
        self.panic();
        shared.publish(self, false);
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

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    #[test]
    fn applied_snapshot_retries_while_bank_publication_is_in_progress() {
        let shared = Arc::new(Shared::default());
        let command = Message {
            id: Some(1),
            audition: false,
            method: Method::SelectVoice { index: 0 },
        }
        .prepare()
        .unwrap();
        shared.retain(&[command.clone()]).unwrap();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (read_tx, read_rx) = std::sync::mpsc::channel();
        let writer_shared = shared.clone();
        let writer = std::thread::spawn(move || {
            writer_shared.revision.fetch_add(1, Ordering::AcqRel);
            started_tx.send(()).unwrap();
            read_rx.recv().unwrap();
            writer_shared.voices[0][135].store(7, Ordering::Relaxed);
            writer_shared.notes[0][0].store(61, Ordering::Relaxed);
            writer_shared.notes[0][1].store(100, Ordering::Relaxed);
            command.complete.store(true, Ordering::Relaxed);
            writer_shared.revision.fetch_add(1, Ordering::Release);
        });
        started_rx.recv().unwrap();
        // Keep publication open deterministically: a bounded read must report
        // retry instead of returning an uncommitted or partially written bank.
        assert_eq!(
            shared.status(123).err().as_deref(),
            Some("Native DX7 state changed while reading; retry")
        );
        read_tx.send(()).unwrap();
        // status() has a bounded 50 ms read window. CI may deschedule the
        // writer beyond that window, so honor its retry contract instead of
        // assuming one call must succeed. Only a complete snapshot is accepted.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let state = loop {
            match shared.status(123) {
                Ok(state) => break state,
                Err(error) => {
                    assert_eq!(error, "Native DX7 state changed while reading; retry");
                    assert!(
                        std::time::Instant::now() < deadline,
                        "concurrent DX7 bank publication did not finish: {error}"
                    );
                    std::thread::yield_now();
                }
            }
        };
        writer.join().unwrap();
        assert_eq!(state.voices[0][135], 7);
        assert_eq!(state.active_notes.len(), 1);
        assert_eq!(state.active_notes[0].midi, 60);
        assert_eq!(state.active_notes[0].velocity, 100);
        assert!(state.pending_commands.is_empty());
        assert_eq!(state.revision, 1);
    }
}
