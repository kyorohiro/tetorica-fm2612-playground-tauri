// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026, kyorohiro
//! MSM5205 PCE encoded ADPCM driver uses the same C core as the browser worklet.
//! A control-thread registry owns immutable ADPCM; the callback never frees a
//! source or allocates. Uploads are sequential raw IPC chunks, not JSON arrays.
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicU32, AtomicU64, Ordering},
    Arc, Mutex,
};

pub const BUDGET: usize = 128 * 1024;
pub const MAX_SAMPLE: usize = 64 * 1024;
pub const MAX_CHUNK: usize = 64 * 1024;
pub const CLOCK: u32 = 1_536_000;

// The C header layout is mirrored here and checked by an ABI fixture test.
#[repr(C)]
#[derive(Clone, Copy)]
struct Core {
    source: *const u8,
    source_bytes: u32,
    output_rate: u32,
    clock: u32,
    mode: u32,
    state: u32,
    offset: u32,
    length: u32,
    cursor: u32,
    looping: u32,
    rate: u32,
    phase: u64,
    output_frames: u64,
    gain: f64,
    muted: u32,
    signal: i32,
    step: i32,
    reset_line: u32,
    vclk: u32,
    prescaler: u32,
    bits: u32,
    fifo: [u8; 7],
    fifo_head: u32,
    fifo_count: u32,
}
extern "C" {
    fn msm5205_pcm_init(state: *mut Core, rate: u32, clock: u32) -> i32;
    fn msm5205_pcm_prepare(state: *mut Core, data: *const u8, bytes: u32) -> i32;
    fn msm5205_pcm_play(state: *mut Core, offset: u32, length: u32, looping: i32, rate: u32)
        -> i32;
    fn msm5205_pcm_stop(state: *mut Core) -> i32;
    fn msm5205_pcm_reset(state: *mut Core) -> i32;
    fn msm5205_pcm_write_register(state: *mut Core, register: u32, value: u32) -> i32;
    fn msm5205_pcm_set_gain(state: *mut Core, gain: f64) -> i32;
    fn msm5205_pcm_set_muted(state: *mut Core, muted: i32) -> i32;
    fn msm5205_pcm_render(state: *mut Core, left: *mut f32, right: *mut f32, frames: u32) -> i32;
}
fn default_rate() -> u32 {
    15
}
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Control {
    Play {
        #[serde(default)]
        offset: u32,
        #[serde(default)]
        length: Option<u32>,
        #[serde(default, rename = "loop")]
        looping: bool,
        #[serde(default = "default_rate")]
        rate: u32,
    },
    Stop,
    Reset,
    Unload,
    WriteRegister {
        register: u32,
        value: u32,
    },
    Mix {
        volume: f64,
        mute: bool,
        #[serde(default)]
        pan: f64,
    },
}
impl Control {
    pub fn validate(self, source: Option<&Sample>) -> Result<(), String> {
        match self {
            Self::Mix { volume, pan, .. } if !volume.is_finite() || !(0.0..=16.0).contains(&volume) || !pan.is_finite() || !(-1.0..=1.0).contains(&pan) =>
                Err("MSM5205 volume must be finite and between 0 and 16; pan must be between -1 and 1".into()),
            Self::Play { offset, length, rate, .. } => {
                let nibbles = source.ok_or("No MSM5205 sample is loaded")?.nibbles();
                let length = length.unwrap_or(nibbles.saturating_sub(offset));
                if rate > 15 || length == 0 || offset.checked_add(length).is_none_or(|end| end > nibbles) {
                    Err("MSM5205 playback requires a nonempty nibble range within the sample and rate 0 to 15".into())
                } else { Ok(()) }
            }
            Self::WriteRegister { register, value } if register > 7 || value > 15 => Err("MSM5205 raw register must be 0 to 7 and value 0 to 15".into()),
            _ => Ok(()),
        }
    }
    pub fn starts(self) -> bool {
        matches!(self, Self::Play { .. } | Self::WriteRegister { .. })
    }
    pub fn stops(self) -> bool {
        matches!(self, Self::Stop | Self::Reset | Self::Unload)
    }
}

#[derive(Debug)]
pub struct Sample {
    pub id: u64,
    pub name: String,
    pub data: Vec<u8>,
}
impl Sample {
    pub fn nibbles(&self) -> u32 {
        (self.data.len() * 2) as u32
    }
}
struct Upload {
    id: u64,
    epoch: u64,
    name: String,
    data: Vec<u8>,
    written: usize,
}
#[derive(Default)]
pub struct Store {
    next: u64,
    upload: Option<Upload>,
    retained: Vec<Arc<Sample>>,
    selected: u64,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadTicket {
    pub upload: u64,
    pub max_chunk_bytes: usize,
}
impl Store {
    pub fn collect(&mut self) {
        let selected = self.selected;
        self.retained
            .retain(|sample| sample.id == selected || Arc::strong_count(sample) > 1);
    }
    pub fn begin(
        &mut self,
        epoch: u64,
        bytes: usize,
        name: String,
    ) -> Result<UploadTicket, String> {
        if bytes == 0 || bytes > MAX_SAMPLE {
            return Err("MSM5205 requires 1 to 65536 encoded ADPCM bytes".into());
        }
        if name.is_empty() || name.len() > 1024 {
            return Err("MSM5205 sample name must contain 1 to 1024 bytes".into());
        }
        if self.upload.is_some() {
            return Err("A MSM5205 upload is already active; finish or abort it first".into());
        }
        self.collect();
        let used: usize = self.retained.iter().map(|sample| sample.data.len()).sum();
        if used.checked_add(bytes).is_none_or(|total| total > BUDGET) {
            return Err(
                "MSM5205 retained source and staging budget (128 KiB) exhausted; unload and retry"
                    .into(),
            );
        }
        let id = self
            .next
            .checked_add(1)
            .filter(|id| *id <= super::MAX_SAFE_INTEGER)
            .ok_or("MSM5205 upload ID exhausted")?;
        let mut data = Vec::new();
        data.try_reserve_exact(bytes)
            .map_err(|_| "Unable to allocate MSM5205 sample storage")?;
        data.resize(bytes, 0);
        self.next = id;
        self.upload = Some(Upload {
            id,
            epoch,
            name,
            data,
            written: 0,
        });
        Ok(UploadTicket {
            upload: id,
            max_chunk_bytes: MAX_CHUNK,
        })
    }
    pub fn chunk(&mut self, epoch: u64, id: u64, offset: usize, data: &[u8]) -> Result<(), String> {
        let upload = self
            .upload
            .as_mut()
            .filter(|upload| upload.id == id && upload.epoch >= epoch)
            .ok_or("MSM5205 upload has expired")?;
        if data.is_empty() || data.len() > MAX_CHUNK || offset != upload.written {
            return Err("MSM5205 chunks must be ordered and at most 64 KiB".into());
        }
        let end = offset
            .checked_add(data.len())
            .filter(|end| *end <= upload.data.len())
            .ok_or("MSM5205 chunk exceeds the reserved sample")?;
        upload.data[offset..end].copy_from_slice(data);
        upload.written = end;
        Ok(())
    }
    pub fn finish(&mut self, epoch: u64, id: u64) -> Result<Arc<Sample>, String> {
        let upload = self
            .upload
            .as_ref()
            .filter(|upload| upload.id == id && upload.epoch >= epoch)
            .ok_or("MSM5205 upload has expired")?;
        if upload.written != upload.data.len() {
            return Err("MSM5205 sample upload is incomplete".into());
        }
        let upload = self.upload.take().unwrap();
        let sample = Arc::new(Sample {
            id,
            name: upload.name,
            data: upload.data,
        });
        self.retained.push(sample.clone());
        Ok(sample)
    }
    pub fn select(&mut self, id: u64) {
        self.selected = id;
        self.collect();
    }
    pub fn selected(&self) -> Option<&Arc<Sample>> {
        self.retained
            .iter()
            .find(|sample| sample.id == self.selected)
    }
    pub fn abort(&mut self, epoch: u64, id: u64) -> Result<(), String> {
        if self
            .upload
            .as_ref()
            .is_none_or(|upload| upload.id != id || upload.epoch < epoch)
        {
            return Err("MSM5205 upload has expired".into());
        }
        self.upload = None;
        Ok(())
    }
    pub fn invalidate(&mut self) {
        self.upload = None;
        self.selected = 0;
        self.collect();
    }
    fn discard_expired_upload(&mut self, before: u64) {
        if self
            .upload
            .as_ref()
            .is_some_and(|upload| upload.epoch < before)
        {
            self.upload = None;
        }
    }
}

#[derive(Default)]
pub struct Shared {
    pub store: Mutex<Store>,
    pub cancel_before: AtomicU64,
    revision: AtomicU64,
    source: AtomicU64,
    output: AtomicU64,
    gain: AtomicU64,
    pan: AtomicU64,
    flags: AtomicU32,
    bytes: AtomicU32,
    offset: AtomicU32,
    length: AtomicU32,
    position: AtomicU32,
    rate: AtomicU32,
    mode: AtomicU32,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub state: &'static str,
    pub loaded: bool,
    pub name: Option<String>,
    pub source: u64,
    pub source_bytes: u32,
    pub clock: u32,
    pub offset: u32,
    pub length: u32,
    pub position_nibbles: u32,
    pub rate: u32,
    pub mode: &'static str,
    pub output_frame: u64,
    pub volume: f64,
    pub pan: f64,
    pub mute: bool,
    pub budget_bytes: usize,
}
impl Shared {
    pub fn status(&self) -> Result<Status, String> {
        let store = self
            .store
            .lock()
            .map_err(|_| "MSM5205 upload state is unavailable")?;
        for _ in 0..128 {
            let revision = self.revision.load(Ordering::Acquire);
            if revision & 1 != 0 {
                std::thread::yield_now();
                continue;
            }
            let source = self.source.load(Ordering::Relaxed);
            let flags = self.flags.load(Ordering::Relaxed);
            let status = Status {
                state: match flags & 3 {
                    1 => "playing",
                    2 => "ended",
                    _ => "ready",
                },
                loaded: source != 0,
                name: store
                    .retained
                    .iter()
                    .find(|s| s.id == source)
                    .map(|s| s.name.clone()),
                source,
                source_bytes: self.bytes.load(Ordering::Relaxed),
                clock: CLOCK,
                offset: self.offset.load(Ordering::Relaxed),
                length: self.length.load(Ordering::Relaxed),
                position_nibbles: self.position.load(Ordering::Relaxed),
                rate: if revision == 0 {
                    15
                } else {
                    self.rate.load(Ordering::Relaxed)
                },
                mode: if self.mode.load(Ordering::Relaxed) == 1 {
                    "driver"
                } else {
                    "raw"
                },
                output_frame: self.output.load(Ordering::Relaxed),
                volume: if revision == 0 {
                    1.0
                } else {
                    f64::from_bits(self.gain.load(Ordering::Relaxed))
                },
                pan: f64::from_bits(self.pan.load(Ordering::Relaxed)),
                mute: flags & 4 != 0,
                budget_bytes: MAX_SAMPLE,
            };
            if revision == self.revision.load(Ordering::Acquire) {
                return Ok(status);
            }
        }
        Err("MSM5205 state changed while reading; retry".into())
    }
}

pub struct Renderer {
    core: Core,
    sample: Option<Arc<Sample>>,
    pan: f64,
}
// Only immutable sample bytes are shared; the C state belongs to one callback.
unsafe impl Send for Renderer {}
impl Renderer {
    pub fn new(rate: u32) -> Result<Self, String> {
        let mut core: Core = unsafe { std::mem::zeroed() };
        if unsafe { msm5205_pcm_init(&mut core, rate, CLOCK) } == 0 {
            return Err("Invalid Native MSM5205 output rate".into());
        }
        // The generic raw decoder starts with its legacy -2 predictor. This
        // PCE driver is silent until an explicit Play or raw-register write.
        unsafe {
            msm5205_pcm_stop(&mut core);
        }
        Ok(Self {
            core,
            sample: None,
            pan: 0.0,
        })
    }
    pub fn install(&mut self, sample: Arc<Sample>) {
        let accepted = unsafe {
            msm5205_pcm_prepare(
                &mut self.core,
                sample.data.as_ptr(),
                sample.data.len() as u32,
            )
        };
        debug_assert_eq!(accepted, 1);
        if accepted == 1 {
            self.sample = Some(sample);
        }
    }
    pub fn apply(&mut self, source: u64, action: Control) {
        let wildcard = source == 0 && action.stops();
        if !wildcard
            && !matches!(action, Control::Mix { .. })
            && self.sample.as_ref().map_or(0, |s| s.id) != source
        {
            return;
        }
        unsafe {
            match action {
                Control::Play {
                    offset,
                    length,
                    looping,
                    rate,
                } => {
                    let nibbles = self.sample.as_ref().map_or(0, |s| s.nibbles());
                    msm5205_pcm_play(
                        &mut self.core,
                        offset,
                        length.unwrap_or(nibbles.saturating_sub(offset)),
                        i32::from(looping),
                        rate,
                    );
                }
                Control::Stop => {
                    if source == 0 {
                        self.clear();
                    } else {
                        msm5205_pcm_stop(&mut self.core);
                    }
                }
                Control::Reset => {
                    if source == 0 {
                        self.clear();
                    } else {
                        msm5205_pcm_reset(&mut self.core);
                    }
                }
                Control::Unload => self.clear(),
                Control::WriteRegister { register, value } => {
                    msm5205_pcm_write_register(&mut self.core, register, value);
                }
                Control::Mix { volume, mute, pan } => {
                    msm5205_pcm_set_gain(&mut self.core, volume);
                    msm5205_pcm_set_muted(&mut self.core, i32::from(mute));
                    self.pan = pan;
                }
            }
        }
    }
    pub fn clear(&mut self) {
        let rate = self.core.output_rate;
        let gain = self.core.gain;
        let muted = self.core.muted;
        let output = self.core.output_frames;
        unsafe {
            msm5205_pcm_init(&mut self.core, rate, CLOCK);
            msm5205_pcm_stop(&mut self.core);
            msm5205_pcm_set_gain(&mut self.core, gain);
            msm5205_pcm_set_muted(&mut self.core, muted as i32);
        }
        self.core.output_frames = output;
        self.sample = None;
    }
    pub fn mix(&mut self, pcm: &mut [f32; 2]) {
        let (mut left, mut right) = (0.0, 0.0);
        unsafe {
            msm5205_pcm_render(&mut self.core, &mut left, &mut right, 1);
        }
        pcm[0] += left * (1.0 - self.pan.max(0.0)) as f32;
        pcm[1] += right * (1.0 + self.pan.min(0.0)) as f32;
    }
    pub fn publish(&self, shared: &Shared) {
        shared.revision.fetch_add(1, Ordering::AcqRel);
        shared
            .source
            .store(self.sample.as_ref().map_or(0, |s| s.id), Ordering::Relaxed);
        shared
            .bytes
            .store(self.core.source_bytes, Ordering::Relaxed);
        shared.offset.store(self.core.offset, Ordering::Relaxed);
        shared.length.store(self.core.length, Ordering::Relaxed);
        shared.position.store(self.core.cursor, Ordering::Relaxed);
        shared.rate.store(self.core.rate, Ordering::Relaxed);
        shared.mode.store(self.core.mode, Ordering::Relaxed);
        shared
            .output
            .store(self.core.output_frames, Ordering::Relaxed);
        shared
            .gain
            .store(self.core.gain.to_bits(), Ordering::Relaxed);
        shared.pan.store(self.pan.to_bits(), Ordering::Relaxed);
        shared
            .flags
            .store(self.core.state | (self.core.muted << 2), Ordering::Relaxed);
        shared.revision.fetch_add(1, Ordering::Release);
    }
}

impl super::Shared {
    fn queue_msm5205(
        &self,
        sequence: &mut u64,
        frame: u64,
        action: super::Action,
    ) -> Result<(), String> {
        if frame > super::MAX_SAFE_INTEGER {
            return Err("Invalid Native MSM5205 command frame".into());
        }
        let next = sequence
            .checked_add(1)
            .ok_or("Native event sequence has expired")?;
        self.pending
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |pending| {
                pending
                    .checked_add(1)
                    .filter(|next| *next <= super::CAPACITY)
            })
            .map_err(|_| "Native command queue is full")?;
        let event = super::Event {
            frame,
            sequence: *sequence,
            epoch: self.epoch.load(Ordering::Acquire),
            action: std::cell::RefCell::new(action),
            owner: None,
        };
        // All producers share this sequence lock. Admission covers the queue
        // plus audio heap, so the one callback consumer cannot invalidate it.
        self.queue
            .push(event)
            .unwrap_or_else(|_| unreachable!("Reserved MSM5205 queue capacity"));
        *sequence = next;
        Ok(())
    }
    fn msm5205_finalize(&self, upload: u64) -> Result<(), String> {
        let mut sequence = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        let mut store = self
            .msm5205
            .store
            .lock()
            .map_err(|_| "MSM5205 state is unavailable")?;
        let sample = store.finish(self.msm5205.cancel_before.load(Ordering::Acquire), upload)?;
        let id = sample.id;
        if let Err(error) =
            self.queue_msm5205(&mut sequence, 0, super::Action::Msm5205Install(sample))
        {
            store.collect();
            return Err(error);
        }
        store.select(id);
        Ok(())
    }
    fn msm5205_action(&self, frame: u64, action: Control) -> Result<(), String> {
        let mut sequence = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        let mut store = self
            .msm5205
            .store
            .lock()
            .map_err(|_| "MSM5205 state is unavailable")?;
        let source = store.selected();
        action.validate(source.map(Arc::as_ref))?;
        let source_id = source.map_or(0, |source| source.id);
        let cancellation = *sequence;
        self.queue_msm5205(
            &mut sequence,
            frame,
            super::Action::Msm5205(source_id, action),
        )?;
        if frame == 0 && action.stops() {
            self.msm5205
                .cancel_before
                .fetch_max(cancellation, Ordering::AcqRel);
            self.invalidation.fetch_add(1, Ordering::Release);
            store.upload = None;
        }
        if matches!(action, Control::Unload) {
            store.invalidate();
        }
        Ok(())
    }
}

#[tauri::command]
pub fn native_genesis_msm5205_load_begin(
    window: tauri::WebviewWindow,
    state: tauri::State<super::NativeGenesis>,
    session: u64,
    bytes: usize,
    name: String,
) -> Result<UploadTicket, String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| {
        let mut producer = shared
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        let mut store = shared
            .msm5205
            .store
            .lock()
            .map_err(|_| "MSM5205 state is unavailable")?;
        store.discard_expired_upload(shared.msm5205.cancel_before.load(Ordering::Acquire));
        let next = producer
            .checked_add(1)
            .ok_or("Native event sequence has expired")?;
        let ticket = store.begin(*producer, bytes, name)?;
        // Upload admission receives its own sequence token. A later Stop at
        // the next sequence invalidates it even if no audio command intervenes.
        *producer = next;
        Ok(ticket)
    })
}

fn header_integer(request: &tauri::ipc::Request<'_>, name: &'static str) -> Result<u64, String> {
    let value = request
        .headers()
        .get(name)
        .ok_or_else(|| format!("Missing MSM5205 upload header {name}"))?
        .to_str()
        .map_err(|_| format!("Invalid MSM5205 upload header {name}"))?
        .parse::<u64>()
        .map_err(|_| format!("Invalid MSM5205 upload header {name}"))?;
    if value > super::MAX_SAFE_INTEGER {
        return Err(format!("Invalid MSM5205 upload header {name}"));
    }
    Ok(value)
}

#[tauri::command]
pub fn native_genesis_msm5205_load_chunk(
    window: tauri::WebviewWindow,
    state: tauri::State<super::NativeGenesis>,
    request: tauri::ipc::Request<'_>,
) -> Result<(), String> {
    crate::allowed(&window)?;
    let session = header_integer(&request, "x-msm5205-session")?;
    let upload = header_integer(&request, "x-msm5205-upload")?;
    let offset = usize::try_from(header_integer(&request, "x-msm5205-offset")?)
        .map_err(|_| "Invalid MSM5205 chunk offset")?;
    let data = match request.body() {
        tauri::ipc::InvokeBody::Raw(data) => data,
        _ => return Err("MSM5205 chunks require a raw binary IPC body".into()),
    };
    // Reject oversized input before taking either control mutex or copying.
    if data.is_empty() || data.len() > MAX_CHUNK {
        return Err("MSM5205 raw chunk must contain 1 to 65536 ADPCM bytes".into());
    }
    state.with_session(session, |shared| {
        let _producer = shared
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        shared
            .msm5205
            .store
            .lock()
            .map_err(|_| "MSM5205 state is unavailable")?
            .chunk(
                shared.msm5205.cancel_before.load(Ordering::Acquire),
                upload,
                offset,
                data,
            )
    })
}

#[tauri::command]
pub fn native_genesis_msm5205_load_finalize(
    window: tauri::WebviewWindow,
    state: tauri::State<super::NativeGenesis>,
    session: u64,
    upload: u64,
) -> Result<(), String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| shared.msm5205_finalize(upload))
}

#[tauri::command]
pub fn native_genesis_msm5205_load_abort(
    window: tauri::WebviewWindow,
    state: tauri::State<super::NativeGenesis>,
    session: u64,
    upload: u64,
) -> Result<(), String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| {
        let _producer = shared
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        shared
            .msm5205
            .store
            .lock()
            .map_err(|_| "MSM5205 state is unavailable")?
            .abort(shared.msm5205.cancel_before.load(Ordering::Acquire), upload)
    })
}

#[tauri::command]
pub fn native_genesis_msm5205_action(
    window: tauri::WebviewWindow,
    state: tauri::State<super::NativeGenesis>,
    session: u64,
    frame: Option<u64>,
    action: Control,
) -> Result<(), String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| {
        shared.msm5205_action(frame.unwrap_or(0), action)
    })
}

#[tauri::command]
pub fn native_genesis_msm5205_state(
    window: tauri::WebviewWindow,
    state: tauri::State<super::NativeGenesis>,
    session: u64,
) -> Result<Status, String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| shared.msm5205.status())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bytes() -> Vec<u8> {
        (0..256).map(|n| n as u8).collect()
    }
    fn load(store: &mut Store, epoch: u64, name: &str, data: &[u8]) -> Arc<Sample> {
        let ticket = store.begin(epoch, data.len(), name.into()).unwrap();
        for (i, chunk) in data.chunks(MAX_CHUNK).enumerate() {
            store
                .chunk(epoch, ticket.upload, i * MAX_CHUNK, chunk)
                .unwrap();
        }
        let sample = store.finish(epoch, ticket.upload).unwrap();
        store.select(sample.id);
        sample
    }
    fn play(offset: u32, length: Option<u32>, looping: bool, rate: u32) -> Control {
        Control::Play {
            offset,
            length,
            looping,
            rate,
        }
    }
    fn install(shared: &super::super::Shared, name: &str, data: &[u8]) -> u64 {
        let upload = shared
            .msm5205
            .store
            .lock()
            .unwrap()
            .begin(*shared.producer.lock().unwrap(), data.len(), name.into())
            .unwrap()
            .upload;
        shared
            .msm5205
            .store
            .lock()
            .unwrap()
            .chunk(0, upload, 0, data)
            .unwrap();
        shared.msm5205_finalize(upload).unwrap();
        upload
    }
    #[test]
    fn upload_is_bounded_ordered_atomic_and_preserves_previous_source_on_error() {
        let mut store = Store::default();
        for size in [0, MAX_SAMPLE + 1] {
            assert!(store.begin(0, size, "invalid".into()).is_err());
        }
        let first = load(&mut store, 0, "old", &bytes());
        let upload = store.begin(1, 17, "new".into()).unwrap().upload;
        assert!(store.begin(1, 1, "overlap".into()).is_err());
        assert!(store.chunk(1, upload, 1, &[1]).is_err());
        assert!(store.finish(1, upload).is_err());
        assert!(store.chunk(2, upload, 0, &[1]).is_err());
        assert_eq!(store.selected().unwrap().id, first.id);
        store.chunk(1, upload, 0, &[0x12; 17]).unwrap();
        let second = store.finish(1, upload).unwrap();
        store.select(second.id);
        let next = store.begin(2, 1, "next".into()).unwrap().upload;
        assert!(store.abort(2, upload).is_err());
        store.abort(2, next).unwrap();
        assert!(play(34, Some(1), false, 15)
            .validate(Some(&second))
            .is_err());
        assert!(play(0, Some(1), false, 16).validate(Some(&second)).is_err());
        assert!(play(0, Some(0), false, 15).validate(Some(&second)).is_err());
        assert!(play(1, Some(33), false, 15).validate(Some(&second)).is_ok());
        store.invalidate();
        drop(first);
        drop(second);
        store.collect();
        assert!(store.retained.is_empty());
    }
    #[test]
    fn replacement_and_unload_free_encoded_bytes_only_on_control_thread() {
        let mut store = Store::default();
        let first = load(&mut store, 0, "first", &bytes());
        let old = first.id;
        let weak = Arc::downgrade(&first);
        let mut renderer = Renderer::new(48_000).unwrap();
        renderer.install(first.clone());
        drop(first);
        let second = load(&mut store, 0, "second", &[0x12; 20]);
        renderer.install(second.clone());
        assert!(weak.upgrade().is_some());
        renderer.apply(old, play(0, None, false, 15));
        assert_eq!(renderer.core.state, 0);
        renderer.apply(second.id, play(1, Some(37), false, 12));
        for _ in 0..48_000 {
            renderer.mix(&mut [0.0; 2]);
        }
        assert_eq!(renderer.core.state, 2);
        assert_eq!(renderer.core.cursor, 38);
        store.collect();
        assert!(weak.upgrade().is_none());
        renderer.apply(second.id, Control::Unload);
        store.invalidate();
        drop(second);
        store.collect();
        assert!(store.retained.is_empty());
    }
    #[test]
    fn stop_cancels_future_play_and_upload_without_stopping_other_sources() {
        for stop_frame in [0, 100] {
            let shared = super::super::Shared::new(44_100, 1);
            let mut renderer = super::super::Renderer::new(44_100).unwrap();
            install(&shared, "test", &bytes());
            shared.msm5205_action(0, play(0, None, true, 15)).unwrap();
            shared.msm5205_action(500, play(0, None, true, 15)).unwrap();
            let epoch = *shared.producer.lock().unwrap();
            *shared.producer.lock().unwrap() += 1;
            let pending = shared
                .msm5205
                .store
                .lock()
                .unwrap()
                .begin(epoch, 2, "pending".into())
                .unwrap()
                .upload;
            shared.msm5205_action(stop_frame, Control::Stop).unwrap();
            renderer.prepare(&shared);
            for _ in 0..1000 {
                renderer.sample(&shared);
            }
            renderer.publish_primary(&shared);
            let status = shared.msm5205.status().unwrap();
            assert_eq!(status.state, "ready");
            assert!(status.loaded);
            assert!(shared.msm5205_finalize(pending).is_err());
            shared.msm5205_action(0, play(0, None, true, 15)).unwrap();
            renderer.prepare(&shared);
            renderer.sample(&shared);
            renderer.publish_primary(&shared);
            assert_eq!(shared.msm5205.status().unwrap().state, "playing");
            shared.msm5205_action(0, Control::Unload).unwrap();
            shared.msm5205_action(0, Control::Stop).unwrap();
            shared.msm5205_action(0, Control::Reset).unwrap();
            renderer.prepare(&shared);
            renderer.sample(&shared);
            renderer.publish_primary(&shared);
            assert!(!shared.msm5205.status().unwrap().loaded);
            shared.msm5205.store.lock().unwrap().collect();
            assert!(shared.msm5205.store.lock().unwrap().retained.is_empty());
        }
    }
    #[test]
    fn stale_scheduled_raw_writes_cannot_switch_a_replacement_sample_to_raw_mode() {
        let shared = super::super::Shared::new(48_000, 1);
        let mut renderer = super::super::Renderer::new(48_000).unwrap();
        let first = install(&shared, "first", &bytes());
        shared
            .msm5205_action(
                400,
                Control::WriteRegister {
                    register: 1,
                    value: 7,
                },
            )
            .unwrap();
        let second = install(&shared, "second", &[0x12; 64]);
        assert_ne!(first, second);
        shared
            .msm5205_action(0, play(1, Some(100), true, 12))
            .unwrap();
        renderer.prepare(&shared);
        for _ in 0..1000 {
            renderer.sample(&shared);
        }
        renderer.publish_primary(&shared);
        let status = shared.msm5205.status().unwrap();
        assert_eq!(status.source, second);
        assert_eq!(status.state, "playing");
        assert_eq!(status.mode, "driver");
        // Raw writes are still valid before any source is prepared. Once a
        // source is installed, a queued source-zero raw write is stale too.
        let mut direct = Renderer::new(48_000).unwrap();
        direct.apply(
            0,
            Control::WriteRegister {
                register: 1,
                value: 7,
            },
        );
        assert_eq!(direct.core.mode, 0);
        assert_eq!(direct.core.fifo_count, 1);
        let sample = shared
            .msm5205
            .store
            .lock()
            .unwrap()
            .selected()
            .unwrap()
            .clone();
        direct.install(sample.clone());
        direct.apply(sample.id, play(0, None, true, 15));
        direct.apply(
            0,
            Control::WriteRegister {
                register: 1,
                value: 7,
            },
        );
        assert_eq!(direct.core.mode, 1);
        assert_eq!(direct.core.state, 1);
        assert_eq!(direct.core.fifo_count, 0);
    }
    #[test]
    fn runtime_stop_invalidates_pending_upload_future_events_and_source() {
        let shared = super::super::Shared::new(48_000, 1);
        let mut renderer = super::super::Renderer::new(48_000).unwrap();
        install(&shared, "test", &bytes());
        shared.msm5205_action(0, play(0, None, true, 15)).unwrap();
        shared
            .msm5205_action(12_000, play(0, None, true, 15))
            .unwrap();
        renderer.prepare(&shared);
        for _ in 0..512 {
            renderer.sample(&shared);
        }
        let pending = shared
            .msm5205
            .store
            .lock()
            .unwrap()
            .begin(0, 2, "pending".into())
            .unwrap()
            .upload;
        shared.epoch.fetch_add(1, Ordering::AcqRel);
        shared.msm5205.store.lock().unwrap().invalidate();
        renderer.prepare(&shared);
        for _ in 0..2000 {
            renderer.sample(&shared);
        }
        renderer.publish_primary(&shared);
        assert!(!shared.msm5205.status().unwrap().loaded);
        assert_eq!(shared.pending.load(Ordering::Acquire), 0);
        assert!(shared.msm5205_finalize(pending).is_err());
        shared.msm5205.store.lock().unwrap().collect();
        assert!(shared.msm5205.store.lock().unwrap().retained.is_empty());
    }
    #[test]
    fn unprepared_driver_and_reset_never_add_dc_to_existing_modes() {
        let mut renderer = Renderer::new(48_000).unwrap();
        for action in [
            None,
            Some(Control::Reset),
            Some(Control::Stop),
            Some(Control::Unload),
        ] {
            if let Some(action) = action {
                renderer.apply(0, action);
            }
            for _ in 0..64 {
                let mut out = [0.0; 2];
                renderer.mix(&mut out);
                assert_eq!(out, [0.0; 2]);
            }
        }
        assert_eq!(renderer.core.output_frames, 256);
    }
    #[test]
    fn native_abi_and_pcm_match_direct_c_at_44100_and_48000() {
        extern "C" {
            fn native_msm5205_fixture_size() -> usize;
            fn native_msm5205_fixture_offset(index: u32) -> usize;
            fn native_msm5205_fixture_render(
                rate: u32,
                data: *const u8,
                bytes: u32,
                left: *mut f32,
                right: *mut f32,
                frames: u32,
            ) -> i32;
        }
        unsafe {
            assert_eq!(native_msm5205_fixture_size(), std::mem::size_of::<Core>());
            let offsets = [
                std::mem::offset_of!(Core, source),
                std::mem::offset_of!(Core, phase),
                std::mem::offset_of!(Core, gain),
                std::mem::offset_of!(Core, fifo),
                std::mem::offset_of!(Core, fifo_count),
            ];
            for (i, offset) in offsets.into_iter().enumerate() {
                assert_eq!(native_msm5205_fixture_offset(i as u32), offset);
            }
        }
        for rate in [44_100, 48_000] {
            let data = bytes();
            let frames = rate as usize / 10;
            let (mut left, mut right) = (vec![0.0; frames], vec![0.0; frames]);
            assert_eq!(
                unsafe {
                    native_msm5205_fixture_render(
                        rate,
                        data.as_ptr(),
                        data.len() as u32,
                        left.as_mut_ptr(),
                        right.as_mut_ptr(),
                        frames as u32,
                    )
                },
                1
            );
            let mut store = Store::default();
            let sample = load(&mut store, 0, "C reference", &data);
            let mut renderer = Renderer::new(rate).unwrap();
            renderer.install(sample.clone());
            renderer.apply(
                sample.id,
                play(1, Some(data.len() as u32 * 2 - 2), false, 12),
            );
            for i in 0..frames {
                let mut out = [0.0; 2];
                renderer.mix(&mut out);
                assert_eq!(out, [left[i], right[i]], "rate {rate} frame {i}");
            }
            assert!(left.iter().any(|v| v.abs() > 0.01));
            assert!(left[frames - 100..].iter().all(|v| *v == 0.0));
            renderer.apply(sample.id, play(0, None, true, 15));
            renderer.apply(
                sample.id,
                Control::Mix {
                    volume: 0.5,
                    mute: true,
                    pan: 1.0,
                },
            );
            for _ in 0..50 {
                let mut out = [0.0; 2];
                renderer.mix(&mut out);
                assert_eq!(out, [0.0; 2]);
            }
            let cursor = renderer.core.cursor;
            renderer.apply(
                sample.id,
                Control::Mix {
                    volume: 0.5,
                    mute: false,
                    pan: 1.0,
                },
            );
            let mut heard = false;
            for _ in 0..50 {
                let mut out = [0.0; 2];
                renderer.mix(&mut out);
                assert_eq!(out[0], 0.0);
                heard |= out[1] != 0.0;
            }
            assert!(heard);
            assert_ne!(renderer.core.cursor, cursor);
            renderer.apply(sample.id, Control::Stop);
            let mut out = [0.0; 2];
            renderer.mix(&mut out);
            assert_eq!(out, [0.0; 2]);
        }
    }
    #[test]
    fn pce_psg_cdda_and_adpcm_share_callback_clock_and_independent_mix() {
        for rate in [44_100, 48_000] {
            let shared =
                super::super::Shared::new_modes(rate, 1, false, false, false, Some("huc6280"));
            let reference =
                super::super::Shared::new_modes(rate, 2, false, false, false, Some("huc6280"));
            let mut actual =
                super::super::Renderer::new_modes(rate, false, false, false, Some("huc6280"))
                    .unwrap();
            let mut expected =
                super::super::Renderer::new_modes(rate, false, false, false, Some("huc6280"))
                    .unwrap();
            let commands = [serde_json::json!({"method":"write","register":1,"value":255}),
                serde_json::json!({"method":"waveform","channel":0,"samples":(0..32).collect::<Vec<_>>()}),
                serde_json::json!({"method":"write","channel":0,"register":5,"value":255}),
                serde_json::json!({"method":"write","channel":0,"register":2,"value":254}),
                serde_json::json!({"method":"write","channel":0,"register":3,"value":0}),
                serde_json::json!({"method":"write","channel":0,"register":4,"value":152})]
                .into_iter().map(|message| serde_json::from_value(serde_json::json!({"frame":0,"type":"huc6280","message":message})).unwrap()).collect::<Vec<super::super::Command>>();
            shared.enqueue(1, &commands).unwrap();
            reference.enqueue(2, &commands).unwrap();
            for target in [&shared, &reference] {
                let pcm = (0..2048)
                    .flat_map(|_| [8192i16.to_le_bytes(), (-4096i16).to_le_bytes()].concat())
                    .collect::<Vec<_>>();
                let mut store = target.cdda.store.lock().unwrap();
                let id = store.begin(0, pcm.len(), "disc".into()).unwrap().upload;
                store.chunk(0, id, 0, &pcm).unwrap();
                let track = store.finish(0, id).unwrap();
                store.select(id);
                drop(store);
                let mut sequence = target.producer.lock().unwrap();
                target
                    .queue_msm5205(&mut sequence, 0, super::super::Action::CddaInstall(track))
                    .unwrap();
                target
                    .queue_msm5205(
                        &mut sequence,
                        0,
                        super::super::Action::Cdda(
                            id,
                            super::super::cdda::Control::Play { source_frame: 0 },
                        ),
                    )
                    .unwrap();
            }
            install(&shared, "ADPCM", &bytes());
            shared
                .msm5205_action(300, play(1, Some(100), true, 12))
                .unwrap();
            shared
                .msm5205_action(
                    450,
                    Control::Mix {
                        volume: 0.5,
                        mute: false,
                        pan: -1.0,
                    },
                )
                .unwrap();
            shared.msm5205_action(600, Control::Stop).unwrap();
            let mut source_store = Store::default();
            let source = load(&mut source_store, 0, "direct", &bytes());
            let mut direct = Renderer::new(rate).unwrap();
            direct.install(source.clone());
            actual.prepare(&shared);
            expected.prepare(&reference);
            let mut peak = 0.0f32;
            for frame in 0..1000 {
                if frame == 300 {
                    direct.apply(source.id, play(1, Some(100), true, 12));
                }
                if frame == 450 {
                    direct.apply(
                        source.id,
                        Control::Mix {
                            volume: 0.5,
                            mute: false,
                            pan: -1.0,
                        },
                    );
                }
                if frame == 600 {
                    direct.apply(source.id, Control::Stop);
                }
                let a = actual.sample(&shared);
                let b = expected.sample(&reference);
                let mut addition = [0.0; 2];
                direct.mix(&mut addition);
                peak = peak.max(b[0].abs());
                for ch in 0..2 {
                    assert!(
                        (a[ch] - b[ch] - addition[ch]).abs() < 1e-6,
                        "rate {rate}, frame {frame}, channel {ch}"
                    );
                }
            }
            assert!(peak > 0.0001);
            actual.publish_primary(&shared);
            assert_eq!(shared.msm5205.status().unwrap().output_frame, 1000);
            assert_eq!(shared.cdda.status(rate).unwrap().output_frame, 1000);
            assert_eq!(shared.msm5205.status().unwrap().state, "ready");
            assert_eq!(shared.cdda.status(rate).unwrap().state, "playing");
        }
    }
    #[test]
    fn pc_engine_cd_selects_same_huc6280_core_and_state() {
        let a = super::super::NativeGenesis::mode_config(Some("pc-engine")).unwrap();
        let b = super::super::NativeGenesis::mode_config(Some("pc-engine-cd")).unwrap();
        assert_eq!(a, b);
        let (mega_cd, dx7, nes, console) = b;
        let shared = super::super::Shared::new_modes(48_000, 1, mega_cd, dx7, nes, console);
        assert!(shared.huc6280.is_some());
        assert_eq!(shared.status().mode, "huc6280");
    }
}
