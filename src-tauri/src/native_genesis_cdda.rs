// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026, kyorohiro
//! CD-DA uses the same dependency-free PCM transport as the browser worklet.
//! A control-thread registry owns immutable PCM; the callback never frees a
//! source or allocates. Uploads are sequential raw IPC chunks, not JSON arrays.
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicU32, AtomicU64, Ordering},
    Arc, Mutex,
};

pub const BUDGET: usize = 256 * 1024 * 1024;
pub const MAX_CHUNK: usize = 256 * 1024;
pub const SOURCE_RATE: u32 = 44_100;

#[repr(C)]
#[derive(Clone, Copy)]
struct Core {
    pcm: *const u8,
    frames: u64,
    window_first_frame: u64,
    window_frames: u64,
    frame: u64,
    output_frames: u64,
    gain: f64,
    phase: u32,
    output_rate: u32,
    state: u32,
    muted: u32,
}
extern "C" {
    fn cdda_pcm_init(state: *mut Core, rate: u32) -> i32;
    fn cdda_pcm_prepare(state: *mut Core, data: *const u8, bytes: u32) -> i32;
    fn cdda_pcm_play(state: *mut Core) -> i32;
    fn cdda_pcm_pause(state: *mut Core) -> i32;
    fn cdda_pcm_resume(state: *mut Core) -> i32;
    fn cdda_pcm_stop(state: *mut Core) -> i32;
    fn cdda_pcm_seek(state: *mut Core, frame: u64) -> i32;
    fn cdda_pcm_set_gain(state: *mut Core, gain: f64) -> i32;
    fn cdda_pcm_set_muted(state: *mut Core, muted: i32) -> i32;
    fn cdda_pcm_render(state: *mut Core, left: *mut f32, right: *mut f32, frames: u32) -> i32;
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Control {
    Play {
        #[serde(default, rename = "sourceFrame")]
        source_frame: u64,
    },
    Pause,
    Resume,
    Stop,
    Unload,
    Seek {
        #[serde(rename = "sourceFrame")]
        source_frame: u64,
    },
    Mix {
        volume: f64,
        mute: bool,
        #[serde(default)]
        pan: f64,
    },
}
impl Control {
    pub fn validate(self, source: Option<&Track>) -> Result<(), String> {
        match self {
            Self::Mix { volume, pan, .. } if !volume.is_finite() || !(0.0..=16.0).contains(&volume) || !pan.is_finite() || !(-1.0..=1.0).contains(&pan) => {
                Err("CD-DA volume must be finite and between 0 and 16; pan must be between -1 and 1".into())
            }
            Self::Mix { .. } | Self::Stop | Self::Unload => Ok(()),
            Self::Seek { source_frame } | Self::Play { source_frame }
                if source.is_none_or(|track| source_frame > track.frames()) =>
            {
                Err("CD-DA seek must be within the selected track".into())
            }
            _ if source.is_none() => Err("No CD-DA track is loaded".into()),
            _ => Ok(()),
        }
    }
    pub fn starts(self) -> bool {
        matches!(self, Self::Play { .. } | Self::Resume)
    }
}

#[derive(Debug)]
pub struct Track {
    pub id: u64,
    pub name: String,
    pub data: Vec<u8>,
}
impl Track {
    pub fn frames(&self) -> u64 {
        (self.data.len() / 4) as u64
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
    retained: Vec<Arc<Track>>,
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
            .retain(|track| track.id == selected || Arc::strong_count(track) > 1);
    }
    pub fn begin(
        &mut self,
        epoch: u64,
        bytes: usize,
        name: String,
    ) -> Result<UploadTicket, String> {
        if bytes == 0 || bytes % 4 != 0 || bytes > BUDGET {
            return Err("CD-DA requires nonempty PCM16LE stereo, at most 256 MiB".into());
        }
        if name.is_empty() || name.len() > 1024 {
            return Err("CD-DA track name must contain 1 to 1024 bytes".into());
        }
        if self.upload.is_some() {
            return Err("A CD-DA upload is already active; finish or abort it first".into());
        }
        self.collect();
        let used: usize = self.retained.iter().map(|track| track.data.len()).sum();
        if used.checked_add(bytes).is_none_or(|total| total > BUDGET) {
            return Err("CD-DA source and staging budget (256 MiB) exhausted; unload the selected track before loading a larger replacement".into());
        }
        let id = self
            .next
            .checked_add(1)
            .filter(|id| *id <= super::MAX_SAFE_INTEGER)
            .ok_or("CD-DA upload ID exhausted")?;
        let mut data = Vec::new();
        data.try_reserve_exact(bytes)
            .map_err(|_| "Unable to allocate CD-DA track storage")?;
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
            .ok_or("CD-DA upload has expired")?;
        if data.is_empty()
            || data.len() > MAX_CHUNK
            || data.len() % 4 != 0
            || offset != upload.written
        {
            return Err(
                "CD-DA chunks must be ordered, stereo-frame aligned and at most 256 KiB".into(),
            );
        }
        let end = offset
            .checked_add(data.len())
            .filter(|end| *end <= upload.data.len())
            .ok_or("CD-DA chunk exceeds the reserved track")?;
        upload.data[offset..end].copy_from_slice(data);
        upload.written = end;
        Ok(())
    }
    pub fn finish(&mut self, epoch: u64, id: u64) -> Result<Arc<Track>, String> {
        let upload = self
            .upload
            .as_ref()
            .filter(|upload| upload.id == id && upload.epoch >= epoch)
            .ok_or("CD-DA upload has expired")?;
        if upload.written != upload.data.len() {
            return Err("CD-DA track upload is incomplete".into());
        }
        let upload = self.upload.take().unwrap();
        let track = Arc::new(Track {
            id,
            name: upload.name,
            data: upload.data,
        });
        self.retained.push(track.clone());
        Ok(track)
    }
    pub fn select(&mut self, id: u64) {
        self.selected = id;
        self.collect();
    }
    pub fn selected(&self) -> Option<&Arc<Track>> {
        self.retained.iter().find(|track| track.id == self.selected)
    }
    pub fn abort(&mut self, epoch: u64, id: u64) -> Result<(), String> {
        if self
            .upload
            .as_ref()
            .is_none_or(|upload| upload.id != id || upload.epoch < epoch)
        {
            return Err("CD-DA upload has expired".into());
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
    frames: AtomicU64,
    position: AtomicU64,
    output: AtomicU64,
    gain: AtomicU64,
    pan: AtomicU64,
    flags: AtomicU32,
    phase: AtomicU32,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub state: &'static str,
    pub loaded: bool,
    pub track: Option<String>,
    pub source: u64,
    pub sample_rate: u32,
    pub position_frames: u64,
    pub position_seconds: f64,
    pub duration_frames: u64,
    pub duration_seconds: f64,
    pub output_frame: u64,
    pub volume: f64,
    pub pan: f64,
    pub mute: bool,
    pub budget_bytes: usize,
}
impl Shared {
    pub fn status(&self, rate: u32) -> Result<Status, String> {
        let store = self
            .store
            .lock()
            .map_err(|_| "CD-DA upload state is unavailable")?;
        for _ in 0..128 {
            let revision = self.revision.load(Ordering::Acquire);
            if revision & 1 != 0 {
                std::thread::yield_now();
                continue;
            }
            let source = self.source.load(Ordering::Relaxed);
            let frames = self.frames.load(Ordering::Relaxed);
            let position = self.position.load(Ordering::Relaxed);
            let phase = self.phase.load(Ordering::Relaxed);
            let output = self.output.load(Ordering::Relaxed);
            let gain = f64::from_bits(self.gain.load(Ordering::Relaxed));
            let pan = f64::from_bits(self.pan.load(Ordering::Relaxed));
            let flags = self.flags.load(Ordering::Relaxed);
            if revision != self.revision.load(Ordering::Acquire) {
                continue;
            }
            return Ok(Status {
                state: match flags & 3 {
                    1 => "playing",
                    2 => "paused",
                    3 => "ended",
                    _ => "ready",
                },
                loaded: source != 0,
                track: store
                    .retained
                    .iter()
                    .find(|track| track.id == source)
                    .map(|track| track.name.clone()),
                source,
                sample_rate: SOURCE_RATE,
                position_frames: position,
                position_seconds: (position as f64 + phase as f64 / rate as f64)
                    / SOURCE_RATE as f64,
                duration_frames: frames,
                duration_seconds: frames as f64 / SOURCE_RATE as f64,
                output_frame: output,
                volume: if revision == 0 { 1.0 } else { gain },
                pan,
                mute: flags & 4 != 0,
                budget_bytes: BUDGET,
            });
        }
        Err("CD-DA state changed while reading; retry".into())
    }
}

pub struct Renderer {
    core: Core,
    track: Option<Arc<Track>>,
    pan: f64,
}
// The state and its borrowed PCM pointer are moved into one audio callback.
// Only immutable source storage is shared with the control registry.
unsafe impl Send for Renderer {}
impl Renderer {
    pub fn new(rate: u32) -> Result<Self, String> {
        let mut core: Core = unsafe { std::mem::zeroed() };
        if unsafe { cdda_pcm_init(&mut core, rate) } == 0 {
            return Err("Invalid Native CD-DA output rate".into());
        }
        Ok(Self {
            core,
            track: None,
            pan: 0.0,
        })
    }
    pub fn install(&mut self, track: Arc<Track>) {
        let accepted = unsafe {
            cdda_pcm_prepare(&mut self.core, track.data.as_ptr(), track.data.len() as u32)
        };
        debug_assert_eq!(accepted, 1);
        if accepted == 1 {
            self.track = Some(track);
        }
    }
    pub fn apply(&mut self, source: u64, action: Control) {
        // Scheduled commands for an earlier selected track cannot affect its replacement.
        let wildcard_stop = source == 0 && matches!(action, Control::Stop | Control::Unload);
        if !wildcard_stop
            && !matches!(action, Control::Mix { .. })
            && self.track.as_ref().map_or(0, |t| t.id) != source
        {
            return;
        }
        unsafe {
            match action {
                Control::Play { source_frame } => {
                    cdda_pcm_play(&mut self.core);
                    cdda_pcm_seek(&mut self.core, source_frame);
                }
                Control::Pause => {
                    cdda_pcm_pause(&mut self.core);
                }
                Control::Resume => {
                    cdda_pcm_resume(&mut self.core);
                }
                Control::Stop => {
                    if source == 0 {
                        // A preceding queued unload may have been cancelled by
                        // this newer Stop. With no selected source, finish that
                        // release rather than leaving an unreachable track.
                        self.clear();
                    } else {
                        cdda_pcm_stop(&mut self.core);
                    }
                }
                Control::Unload => {
                    self.clear();
                }
                Control::Seek { source_frame } => {
                    cdda_pcm_seek(&mut self.core, source_frame);
                }
                Control::Mix { volume, mute, pan } => {
                    cdda_pcm_set_gain(&mut self.core, volume);
                    cdda_pcm_set_muted(&mut self.core, i32::from(mute));
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
            cdda_pcm_init(&mut self.core, rate);
            cdda_pcm_set_gain(&mut self.core, gain);
            cdda_pcm_set_muted(&mut self.core, muted as i32);
        }
        self.core.output_frames = output;
        self.track = None;
    }
    pub fn mix(&mut self, pcm: &mut [f32; 2]) {
        let (mut left, mut right) = (0.0, 0.0);
        unsafe {
            cdda_pcm_render(&mut self.core, &mut left, &mut right, 1);
        }
        pcm[0] += left * (1.0 - self.pan.max(0.0)) as f32;
        pcm[1] += right * (1.0 + self.pan.min(0.0)) as f32;
    }
    pub fn publish(&self, shared: &Shared) {
        shared.revision.fetch_add(1, Ordering::AcqRel);
        shared.source.store(
            self.track.as_ref().map_or(0, |track| track.id),
            Ordering::Relaxed,
        );
        shared.frames.store(self.core.frames, Ordering::Relaxed);
        shared.position.store(self.core.frame, Ordering::Relaxed);
        shared.phase.store(self.core.phase, Ordering::Relaxed);
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
    fn queue_cdda(
        &self,
        sequence: &mut u64,
        frame: u64,
        action: super::Action,
    ) -> Result<(), String> {
        if frame > super::MAX_SAFE_INTEGER {
            return Err("Invalid Native CD-DA command frame".into());
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
            .unwrap_or_else(|_| unreachable!("Reserved CD-DA queue capacity"));
        *sequence = next;
        Ok(())
    }
    fn cdda_finalize(&self, upload: u64) -> Result<(), String> {
        let mut sequence = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        let mut store = self
            .cdda
            .store
            .lock()
            .map_err(|_| "CD-DA state is unavailable")?;
        let track = store.finish(self.cdda.cancel_before.load(Ordering::Acquire), upload)?;
        let id = track.id;
        if let Err(error) = self.queue_cdda(&mut sequence, 0, super::Action::CddaInstall(track)) {
            store.collect();
            return Err(error);
        }
        store.select(id);
        Ok(())
    }
    fn cdda_action(&self, frame: u64, action: Control) -> Result<(), String> {
        let mut sequence = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        let mut store = self
            .cdda
            .store
            .lock()
            .map_err(|_| "CD-DA state is unavailable")?;
        let source = store.selected();
        action.validate(source.map(Arc::as_ref))?;
        let source_id = source.map_or(0, |source| source.id);
        let cancellation = *sequence;
        self.queue_cdda(&mut sequence, frame, super::Action::Cdda(source_id, action))?;
        if frame == 0 && matches!(action, Control::Stop | Control::Unload) {
            self.cdda
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
pub fn native_genesis_cdda_load_begin(
    window: tauri::WebviewWindow,
    state: tauri::State<super::NativeGenesis>,
    session: u64,
    bytes: usize,
    track: String,
) -> Result<UploadTicket, String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| {
        let mut producer = shared
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        let mut store = shared
            .cdda
            .store
            .lock()
            .map_err(|_| "CD-DA state is unavailable")?;
        store.discard_expired_upload(shared.cdda.cancel_before.load(Ordering::Acquire));
        let next = producer
            .checked_add(1)
            .ok_or("Native event sequence has expired")?;
        let ticket = store.begin(*producer, bytes, track)?;
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
        .ok_or_else(|| format!("Missing CD-DA upload header {name}"))?
        .to_str()
        .map_err(|_| format!("Invalid CD-DA upload header {name}"))?
        .parse::<u64>()
        .map_err(|_| format!("Invalid CD-DA upload header {name}"))?;
    if value > super::MAX_SAFE_INTEGER {
        return Err(format!("Invalid CD-DA upload header {name}"));
    }
    Ok(value)
}

#[tauri::command]
pub fn native_genesis_cdda_load_chunk(
    window: tauri::WebviewWindow,
    state: tauri::State<super::NativeGenesis>,
    request: tauri::ipc::Request<'_>,
) -> Result<(), String> {
    crate::allowed(&window)?;
    let session = header_integer(&request, "x-cdda-session")?;
    let upload = header_integer(&request, "x-cdda-upload")?;
    let offset = usize::try_from(header_integer(&request, "x-cdda-offset")?)
        .map_err(|_| "Invalid CD-DA chunk offset")?;
    let data = match request.body() {
        tauri::ipc::InvokeBody::Raw(data) => data,
        _ => return Err("CD-DA chunks require a raw binary IPC body".into()),
    };
    // Reject oversized input before taking either control mutex or copying.
    if data.is_empty() || data.len() > MAX_CHUNK || data.len() % 4 != 0 {
        return Err("CD-DA raw chunk must contain 1 to 256 KiB of stereo-frame aligned PCM".into());
    }
    state.with_session(session, |shared| {
        let _producer = shared
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        shared
            .cdda
            .store
            .lock()
            .map_err(|_| "CD-DA state is unavailable")?
            .chunk(
                shared.cdda.cancel_before.load(Ordering::Acquire),
                upload,
                offset,
                data,
            )
    })
}

#[tauri::command]
pub fn native_genesis_cdda_load_finalize(
    window: tauri::WebviewWindow,
    state: tauri::State<super::NativeGenesis>,
    session: u64,
    upload: u64,
) -> Result<(), String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| shared.cdda_finalize(upload))
}

#[tauri::command]
pub fn native_genesis_cdda_load_abort(
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
            .cdda
            .store
            .lock()
            .map_err(|_| "CD-DA state is unavailable")?
            .abort(shared.cdda.cancel_before.load(Ordering::Acquire), upload)
    })
}

#[tauri::command]
pub fn native_genesis_cdda_action(
    window: tauri::WebviewWindow,
    state: tauri::State<super::NativeGenesis>,
    session: u64,
    frame: Option<u64>,
    action: Control,
) -> Result<(), String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| {
        shared.cdda_action(frame.unwrap_or(0), action)
    })
}

#[tauri::command]
pub fn native_genesis_cdda_state(
    window: tauri::WebviewWindow,
    state: tauri::State<super::NativeGenesis>,
    session: u64,
) -> Result<Status, String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| shared.cdda.status(shared.rate))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pcm(frames: usize) -> Vec<u8> {
        (0..frames)
            .flat_map(|i| {
                let value = if i & 1 == 0 { 16_384i16 } else { -8_192i16 };
                [value.to_le_bytes(), (-value).to_le_bytes()].concat()
            })
            .collect()
    }
    fn load(store: &mut Store, epoch: u64, name: &str, data: &[u8]) -> Arc<Track> {
        let ticket = store.begin(epoch, data.len(), name.into()).unwrap();
        for (i, data) in data.chunks(MAX_CHUNK).enumerate() {
            store
                .chunk(epoch, ticket.upload, i * MAX_CHUNK, data)
                .unwrap();
        }
        let track = store.finish(epoch, ticket.upload).unwrap();
        store.select(track.id);
        track
    }
    #[test]
    fn upload_is_ordered_bounded_atomic_and_stale_ids_do_not_abort_replacement() {
        let mut store = Store::default();
        assert!(store.begin(0, 0, "empty".into()).is_err());
        assert!(store.begin(0, 3, "alignment".into()).is_err());
        assert!(store.begin(0, BUDGET + 4, "large".into()).is_err());
        let first = load(&mut store, 0, "first", &pcm(4));
        let upload = store.begin(0, 32, "second".into()).unwrap().upload;
        assert!(store.chunk(0, upload, 4, &pcm(1)).is_err());
        assert!(store.finish(0, upload).is_err());
        assert_eq!(store.selected().unwrap().id, first.id);
        assert!(store.chunk(1, upload, 0, &pcm(1)).is_err());
        assert!(store.chunk(0, upload, 0, &vec![0; MAX_CHUNK + 4]).is_err());
        store.chunk(0, upload, 0, &pcm(8)).unwrap();
        assert!(store.chunk(0, upload, 0, &pcm(1)).is_err());
        let second = store.finish(0, upload).unwrap();
        store.select(second.id);
        assert_eq!(store.selected().unwrap().name, "second");
        let newer = store.begin(0, 4, "third".into()).unwrap().upload;
        assert!(store.abort(0, upload).is_err());
        store.chunk(0, newer, 0, &pcm(1)).unwrap();
        store.abort(0, newer).unwrap();
        store.invalidate();
        assert!(store.selected().is_none());
        drop(first);
        drop(second);
        store.collect();
        assert!(store.retained.is_empty());
    }
    #[test]
    fn transport_pcm_lifecycle_and_audio_clock_match_without_device() {
        let shared = Shared::default();
        let track = load(&mut shared.store.lock().unwrap(), 0, "stereo", &pcm(4));
        let mut renderer = Renderer::new(44_100).unwrap();
        renderer.install(track.clone());
        renderer.apply(track.id, Control::Play { source_frame: 0 });
        let mut out = [0.0; 2];
        renderer.mix(&mut out);
        assert_eq!(out, [0.5, -0.5]);
        renderer.apply(track.id, Control::Pause);
        renderer.mix(&mut [0.0; 2]);
        renderer.publish(&shared);
        let status = shared.status(44_100).unwrap();
        assert_eq!(status.state, "paused");
        assert_eq!(status.position_frames, 1);
        assert_eq!(status.output_frame, 2);
        renderer.apply(
            track.id,
            Control::Mix {
                volume: 0.5,
                mute: true,
                pan: 0.0,
            },
        );
        renderer.apply(track.id, Control::Resume);
        out = [0.0; 2];
        renderer.mix(&mut out);
        assert_eq!(out, [0.0; 2]);
        assert_eq!(renderer.core.frame, 2);
        renderer.apply(
            track.id,
            Control::Mix {
                volume: 0.5,
                mute: false,
                pan: 0.0,
            },
        );
        out = [0.0; 2];
        renderer.mix(&mut out);
        assert_eq!(out, [0.25, -0.25]);
        renderer.mix(&mut [0.0; 2]);
        renderer.publish(&shared);
        assert_eq!(shared.status(44_100).unwrap().state, "ended");
        renderer.apply(track.id, Control::Play { source_frame: 0 });
        assert_eq!(renderer.core.frame, 0);
        renderer.apply(track.id, Control::Seek { source_frame: 3 });
        assert_eq!(renderer.core.frame, 3);
        renderer.apply(track.id, Control::Stop);
        assert_eq!(renderer.core.frame, 0);
        assert_eq!(renderer.core.state, 0);
        renderer.clear();
        renderer.publish(&shared);
        assert!(!shared.status(44_100).unwrap().loaded);
    }
    #[test]
    fn replacement_invalidates_old_actions_and_source_frees_only_on_control_thread() {
        let mut store = Store::default();
        let first = load(&mut store, 0, "first", &pcm(8));
        let old_id = first.id;
        let mut renderer = Renderer::new(48_000).unwrap();
        renderer.install(first.clone());
        let weak = Arc::downgrade(&first);
        drop(first);
        let second = load(&mut store, 0, "second", &pcm(16));
        renderer.install(second.clone());
        // Even after the callback replaces it, control owns the final source ref.
        assert!(weak.upgrade().is_some());
        renderer.apply(old_id, Control::Play { source_frame: 0 });
        assert_eq!(renderer.core.state, 0);
        renderer.apply(second.id, Control::Play { source_frame: 0 });
        for _ in 0..48_000 {
            renderer.mix(&mut [0.0; 2]);
        }
        assert_eq!(renderer.core.state, 3);
        assert_eq!(renderer.core.frame, 16);
        assert_eq!(renderer.core.output_frames, 48_000);
        store.collect();
        assert!(weak.upgrade().is_none());
        assert!(Control::Mix {
            volume: f64::NAN,
            mute: false,
            pan: 0.0
        }
        .validate(Some(&second))
        .is_err());
        assert!(Control::Seek { source_frame: 17 }
            .validate(Some(&second))
            .is_err());
    }
    #[test]
    fn cdda_and_chip_commands_share_exact_renderer_frames_and_keep_independent_mix() {
        let shared = super::super::Shared::new(44_100, 9);
        let reference = super::super::Shared::new(44_100, 10);
        let mut actual = super::super::Renderer::new(44_100).unwrap();
        let mut expected = super::super::Renderer::new(44_100).unwrap();
        let chip_commands = || {
            [0x8eu8, 0x0f, 0x94]
                .into_iter()
                .map(|value| {
                    serde_json::from_value::<super::super::Command>(serde_json::json!({
                        "frame": 0, "type": "psg-write", "value": value,
                    }))
                    .unwrap()
                })
                .collect::<Vec<_>>()
        };
        shared.enqueue(9, &chip_commands()).unwrap();
        reference.enqueue(10, &chip_commands()).unwrap();
        let data = (0..2048)
            .flat_map(|_| [8192i16.to_le_bytes(), (-4096i16).to_le_bytes()].concat())
            .collect::<Vec<_>>();
        let upload = shared
            .cdda
            .store
            .lock()
            .unwrap()
            .begin(0, data.len(), "clock".into())
            .unwrap()
            .upload;
        shared
            .cdda
            .store
            .lock()
            .unwrap()
            .chunk(0, upload, 0, &data)
            .unwrap();
        shared.cdda_finalize(upload).unwrap();
        shared
            .cdda_action(300, Control::Play { source_frame: 100 })
            .unwrap();
        shared.cdda_action(400, Control::Pause).unwrap();
        shared
            .cdda_action(
                450,
                Control::Mix {
                    volume: 0.5,
                    mute: false,
                    pan: -1.0,
                },
            )
            .unwrap();
        shared.cdda_action(500, Control::Resume).unwrap();
        shared.cdda_action(600, Control::Stop).unwrap();
        actual.prepare(&shared);
        expected.prepare(&reference);
        for frame in 0..700 {
            let a = actual.sample(&shared);
            let b = expected.sample(&reference);
            let addition = if (300..400).contains(&frame) {
                [0.25, -0.125]
            } else if (500..600).contains(&frame) {
                [0.125, 0.0]
            } else {
                [0.0, 0.0]
            };
            for channel in 0..2 {
                assert!(
                    (a[channel] - b[channel] - addition[channel]).abs() < 1e-6,
                    "frame {frame}, channel {channel}: {a:?} vs {b:?}"
                );
            }
        }
        actual.publish_primary(&shared);
        let status = shared.cdda.status(44_100).unwrap();
        assert_eq!(status.position_frames, 0);
        assert_eq!(status.output_frame, 700);
        assert_eq!(status.pan, -1.0);
        assert_eq!(status.state, "ready");
        assert!(shared.check_session(8).is_err());
    }
    #[test]
    fn native_stop_invalidates_uploads_and_queued_starts_and_releases_sources_off_callback() {
        let shared = super::super::Shared::new(48_000, 1);
        let mut renderer = super::super::Renderer::new(48_000).unwrap();
        let data = pcm(96_000);
        let upload = shared
            .cdda
            .store
            .lock()
            .unwrap()
            .begin(0, data.len(), "before-stop".into())
            .unwrap()
            .upload;
        for (index, chunk) in data.chunks(MAX_CHUNK).enumerate() {
            shared
                .cdda
                .store
                .lock()
                .unwrap()
                .chunk(0, upload, index * MAX_CHUNK, chunk)
                .unwrap();
        }
        shared.cdda_finalize(upload).unwrap();
        shared
            .cdda_action(0, Control::Play { source_frame: 0 })
            .unwrap();
        shared
            .cdda_action(12_000, Control::Play { source_frame: 100 })
            .unwrap();
        renderer.prepare(&shared);
        for _ in 0..1024 {
            renderer.sample(&shared);
        }
        let pending_upload = shared
            .cdda
            .store
            .lock()
            .unwrap()
            .begin(0, 32, "cancelled".into())
            .unwrap()
            .upload;
        // Model the exact Stop epoch barrier without opening a physical device.
        shared.epoch.fetch_add(1, Ordering::AcqRel);
        shared.cdda.store.lock().unwrap().invalidate();
        renderer.prepare(&shared);
        for _ in 0..2000 {
            renderer.sample(&shared);
        }
        renderer.publish_primary(&shared);
        assert_eq!(shared.stopped_epoch.load(Ordering::Acquire), 1);
        assert!(!shared.cdda.status(48_000).unwrap().loaded);
        assert_eq!(shared.pending.load(Ordering::Acquire), 0);
        let mut store = shared.cdda.store.lock().unwrap();
        assert!(store.chunk(1, pending_upload, 0, &pcm(1)).is_err());
        assert!(store.finish(1, pending_upload).is_err());
        store.collect();
        assert!(store.retained.is_empty());
        // A fresh upload/replay succeeds in the same session after Runtime Stop.
        let track = load(&mut store, 1, "after-stop", &pcm(4));
        drop(store);
        let mut sequence = shared.producer.lock().unwrap();
        shared
            .queue_cdda(
                &mut sequence,
                0,
                super::super::Action::CddaInstall(track.clone()),
            )
            .unwrap();
        drop(sequence);
        shared
            .cdda_action(0, Control::Play { source_frame: 0 })
            .unwrap();
        renderer.prepare(&shared);
        for _ in 0..32 {
            renderer.sample(&shared);
        }
        renderer.publish_primary(&shared);
        assert_eq!(shared.cdda.status(48_000).unwrap().state, "ended");
    }
    #[test]
    fn transport_stop_and_unload_cancel_future_events_and_stale_uploads_without_stopping_psg() {
        for stop_frame in [0, 256] {
            let shared = super::super::Shared::new(44_100, 1);
            let mut renderer = super::super::Renderer::new(44_100).unwrap();
            let data = pcm(2048);
            let upload = shared
                .cdda
                .store
                .lock()
                .unwrap()
                .begin(0, data.len(), "transport-stop".into())
                .unwrap()
                .upload;
            shared
                .cdda
                .store
                .lock()
                .unwrap()
                .chunk(0, upload, 0, &data)
                .unwrap();
            shared.cdda_finalize(upload).unwrap();
            shared
                .cdda_action(0, Control::Play { source_frame: 0 })
                .unwrap();
            shared
                .cdda_action(1024, Control::Play { source_frame: 10 })
                .unwrap();
            shared.cdda_action(1200, Control::Resume).unwrap();
            let upload_sequence = *shared.producer.lock().unwrap();
            *shared.producer.lock().unwrap() += 1;
            let unfinished = shared
                .cdda
                .store
                .lock()
                .unwrap()
                .begin(upload_sequence, 32, "unfinished".into())
                .unwrap()
                .upload;
            shared.cdda_action(stop_frame, Control::Stop).unwrap();
            // Admission of unrelated PSG events is unaffected by CD-DA Stop.
            let commands = [0x8eu8, 0x0f, 0x94]
                .into_iter()
                .map(|value| {
                    serde_json::from_value::<super::super::Command>(
                        serde_json::json!({"frame": 1500, "type": "psg-write", "value": value}),
                    )
                    .unwrap()
                })
                .collect::<Vec<_>>();
            shared.enqueue(1, &commands).unwrap();
            renderer.prepare(&shared);
            let mut chip_peak = 0f32;
            for frame in 0..2500 {
                let output = renderer.sample(&shared);
                if frame >= 1500 {
                    chip_peak = chip_peak.max(output[0].abs()).max(output[1].abs());
                }
            }
            renderer.publish_primary(&shared);
            let status = shared.cdda.status(44_100).unwrap();
            assert_eq!(status.state, "ready");
            assert_eq!(status.position_frames, 0);
            assert!(status.loaded);
            assert!(chip_peak > 0.0001);
            let cutoff = shared.cdda.cancel_before.load(Ordering::Acquire);
            assert!(shared
                .cdda
                .store
                .lock()
                .unwrap()
                .chunk(cutoff, unfinished, 0, &pcm(1))
                .is_err());
            assert!(shared.cdda_finalize(unfinished).is_err());
            // Newly admitted playback after Stop remains valid.
            shared
                .cdda_action(0, Control::Play { source_frame: 100 })
                .unwrap();
            renderer.prepare(&shared);
            renderer.sample(&shared);
            renderer.publish_primary(&shared);
            assert_eq!(shared.cdda.status(44_100).unwrap().state, "playing");
            shared.cdda_action(0, Control::Unload).unwrap();
            renderer.prepare(&shared);
            renderer.sample(&shared);
            renderer.publish_primary(&shared);
            assert!(!shared.cdda.status(44_100).unwrap().loaded);
            shared.cdda.store.lock().unwrap().collect();
            assert!(shared.cdda.store.lock().unwrap().retained.is_empty());
        }
    }
    #[test]
    fn repeated_unload_and_stop_before_callback_preserve_mix_and_release_current_source() {
        let shared = super::super::Shared::new(44_100, 1);
        let mut renderer = super::super::Renderer::new(44_100).unwrap();
        let data = pcm(2048);
        let upload = shared
            .cdda
            .store
            .lock()
            .unwrap()
            .begin(0, data.len(), "repeat".into())
            .unwrap()
            .upload;
        shared
            .cdda
            .store
            .lock()
            .unwrap()
            .chunk(0, upload, 0, &data)
            .unwrap();
        shared.cdda_finalize(upload).unwrap();
        shared
            .cdda_action(
                0,
                Control::Mix {
                    volume: 0.25,
                    mute: false,
                    pan: 0.5,
                },
            )
            .unwrap();
        shared
            .cdda_action(0, Control::Play { source_frame: 0 })
            .unwrap();
        renderer.prepare(&shared);
        renderer.sample(&shared);
        shared.cdda_action(0, Control::Unload).unwrap();
        shared.cdda_action(0, Control::Unload).unwrap();
        shared.cdda_action(0, Control::Stop).unwrap();
        renderer.prepare(&shared);
        renderer.sample(&shared);
        renderer.publish_primary(&shared);
        assert_ne!(shared.cdda.status(44_100).unwrap().state, "playing");
        // A final unload with no control-side selection must still clear the
        // source after earlier queued unloads were cancelled by a newer Stop.
        shared.cdda_action(0, Control::Unload).unwrap();
        renderer.prepare(&shared);
        renderer.sample(&shared);
        renderer.publish_primary(&shared);
        let status = shared.cdda.status(44_100).unwrap();
        assert!(!status.loaded);
        assert_eq!(status.volume, 0.25);
        assert_eq!(status.pan, 0.5);
        shared.cdda.store.lock().unwrap().collect();
        assert!(shared.cdda.store.lock().unwrap().retained.is_empty());
    }
}
