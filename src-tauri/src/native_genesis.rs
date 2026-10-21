//! Experimental Genesis output: the device callback owns both chips and PCM.
//! The control side only submits bounded timestamped register events.
use crossbeam_queue::ArrayQueue;
use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering as Compare,
    collections::BinaryHeap,
    ffi::c_void,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};

const CAPACITY: usize = 65_536;
const MAX_BATCH: usize = 4096;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);

extern "C" {
    fn native_genesis_create(rate: u32) -> *mut c_void;
    fn native_genesis_destroy(core: *mut c_void);
    fn native_genesis_reset(core: *mut c_void);
    fn native_genesis_stop_voices(core: *mut c_void);
    fn native_genesis_write(core: *mut c_void, port: u8, register: u8, value: u8);
    fn native_genesis_psg_write(core: *mut c_void, value: u8);
    fn native_genesis_psg_stereo(core: *mut c_void, value: u8);
    fn native_genesis_mix(core: *mut c_void, gains: *const f32);
    fn native_genesis_sample(core: *mut c_void, output: *mut f32);
}

struct Core(*mut c_void);
// The pointer is uniquely owned. It is moved into, and used only by, one device
// callback. C++ creates no threads and stores no callback into Rust.
unsafe impl Send for Core {}
impl Core {
    fn new(rate: u32) -> Result<Self, String> {
        if !(8000..=384000).contains(&rate) {
            return Err("Unsupported native sample rate".into());
        }
        let ptr = unsafe { native_genesis_create(rate) };
        if ptr.is_null() {
            Err("Could not allocate native Genesis sound cores".into())
        } else {
            Ok(Self(ptr))
        }
    }
    fn sample(&mut self) -> [f32; 2] {
        let mut sample = [0.0; 2];
        unsafe {
            native_genesis_sample(self.0, sample.as_mut_ptr());
        }
        sample
    }
    fn apply(&mut self, action: Action) {
        unsafe {
            match action {
                Action::Write(port, register, value) => {
                    native_genesis_write(self.0, port, register, value)
                }
                Action::PsgWrite(value) => native_genesis_psg_write(self.0, value),
                Action::PsgStereo(value) => native_genesis_psg_stereo(self.0, value),
                Action::Reset => native_genesis_reset(self.0),
                Action::Mix(gains) => native_genesis_mix(self.0, gains.as_ptr()),
            }
        }
    }
}
impl Drop for Core {
    fn drop(&mut self) {
        unsafe {
            native_genesis_destroy(self.0);
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Action {
    Write(u8, u8, u8),
    PsgWrite(u8),
    PsgStereo(u8),
    Reset,
    Mix([f32; 5]),
}
#[derive(Clone, Copy, Debug)]
struct Event {
    frame: u64,
    sequence: u64,
    epoch: u64,
    action: Action,
}
impl PartialEq for Event {
    fn eq(&self, other: &Self) -> bool {
        (self.frame, self.sequence) == (other.frame, other.sequence)
    }
}
impl Eq for Event {}
impl PartialOrd for Event {
    fn partial_cmp(&self, other: &Self) -> Option<Compare> {
        Some(self.cmp(other))
    }
}
impl Ord for Event {
    fn cmp(&self, other: &Self) -> Compare {
        (other.frame, other.sequence).cmp(&(self.frame, self.sequence))
    }
}

#[derive(Deserialize, Debug)]
pub struct Command {
    frame: u64,
    #[serde(flatten)]
    action: CommandAction,
}
#[derive(Deserialize, Debug)]
#[serde(tag = "type", rename_all = "kebab-case")]
enum CommandAction {
    Write {
        #[serde(default)]
        port: u8,
        register: u8,
        value: u8,
    },
    PsgWrite {
        value: u8,
    },
    PsgStereo {
        value: u8,
    },
    Reset,
    Mix {
        #[serde(rename = "fmLeft")]
        fm_left: f32,
        #[serde(rename = "fmRight")]
        fm_right: f32,
        #[serde(rename = "psgLeft")]
        psg_left: f32,
        #[serde(rename = "psgRight")]
        psg_right: f32,
        master: f32,
    },
}
impl Command {
    fn validate(&self) -> Result<Action, String> {
        if self.frame > MAX_SAFE_INTEGER {
            return Err("Invalid native command frame".into());
        }
        Ok(match self.action {
            CommandAction::Write {
                port,
                register,
                value,
            } => {
                if port > 1 {
                    return Err("YM2612 port must be 0 or 1".into());
                }
                if port == 0 && register == 0x2b && value & 0x80 != 0 {
                    return Err("YM2612 DAC playback is not supported by experimental native Genesis output".into());
                }
                Action::Write(port, register, value)
            }
            CommandAction::PsgWrite { value } => Action::PsgWrite(value),
            CommandAction::PsgStereo { value } => Action::PsgStereo(value),
            CommandAction::Reset => Action::Reset,
            CommandAction::Mix {
                fm_left,
                fm_right,
                psg_left,
                psg_right,
                master,
            } => {
                let gains = [fm_left, fm_right, psg_left, psg_right, master];
                if gains[..4]
                    .iter()
                    .any(|v| !v.is_finite() || !(0.0..=2.0).contains(v))
                    || !master.is_finite()
                    || !(0.0..=3.8).contains(&master)
                {
                    return Err("Native source gains must be 0–2 and master gain 0–3.8".into());
                }
                Action::Mix(gains)
            }
        })
    }
}

struct Shared {
    queue: ArrayQueue<Event>,
    producer: Mutex<u64>,
    pending: AtomicUsize,
    frame: AtomicU64,
    epoch: AtomicU64,
    stopped_epoch: AtomicU64,
    late: AtomicU64,
    overflows: AtomicU64,
    failed: AtomicBool,
    rate: u32,
    session: u64,
}
impl Shared {
    fn new(rate: u32, session: u64) -> Self {
        Self {
            queue: ArrayQueue::new(CAPACITY),
            producer: Mutex::new(0),
            pending: AtomicUsize::new(0),
            frame: AtomicU64::new(0),
            epoch: AtomicU64::new(0),
            stopped_epoch: AtomicU64::new(0),
            late: AtomicU64::new(0),
            overflows: AtomicU64::new(0),
            failed: AtomicBool::new(false),
            rate,
            session,
        }
    }
    fn enqueue(&self, session: u64, commands: &[Command]) -> Result<(), String> {
        self.check_session(session)?;
        if commands.len() > MAX_BATCH {
            return Err(format!("Native batch exceeds {MAX_BATCH} commands"));
        }
        // Validate the entire batch before reserving or mutating the queue.
        let actions: Vec<_> = commands
            .iter()
            .map(Command::validate)
            .collect::<Result<_, _>>()?;
        let mut sequence = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        if self
            .pending
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |pending| {
                pending
                    .checked_add(commands.len())
                    .filter(|next| *next <= CAPACITY)
            })
            .is_err()
        {
            self.overflows.fetch_add(1, Ordering::Relaxed);
            return Err("Native command queue is full; playback was not silently truncated".into());
        }
        let epoch = self.epoch.load(Ordering::Acquire);
        for (command, action) in commands.iter().zip(actions) {
            let event = Event {
                frame: command.frame,
                sequence: *sequence,
                epoch,
                action,
            };
            *sequence += 1;
            // Admission includes both the queue and callback heap, so capacity
            // is guaranteed while the only consumer removes queue entries.
            if self.queue.push(event).is_err() {
                unreachable!("Reserved native event capacity");
            }
        }
        Ok(())
    }
    fn check_session(&self, session: u64) -> Result<(), String> {
        if self.session != session {
            return Err("Native Genesis session has expired".into());
        }
        if self.failed.load(Ordering::Acquire) {
            return Err("Native audio device stream failed; select the output again".into());
        }
        Ok(())
    }
    fn status(&self) -> Status {
        Status {
            session: self.session,
            sample_rate: self.rate,
            frame: self.frame.load(Ordering::Acquire),
            ym_clock: 7_670_454,
            psg_clock: 3_579_545,
            running: !self.failed.load(Ordering::Acquire),
            queued: self.pending.load(Ordering::Acquire),
            late_commands: self.late.load(Ordering::Relaxed),
            queue_overflows: self.overflows.load(Ordering::Relaxed),
            error: self
                .failed
                .load(Ordering::Acquire)
                .then(|| "Native audio device stream failed".into()),
        }
    }
    fn stop(&self) -> Result<Status, String> {
        let _producer = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        let epoch = self.epoch.fetch_add(1, Ordering::AcqRel) + 1;
        let start = Instant::now();
        while self.stopped_epoch.load(Ordering::Acquire) < epoch {
            if self.failed.load(Ordering::Acquire) {
                return Err("Native audio device stream failed during stop".into());
            }
            if start.elapsed() > Duration::from_secs(2) {
                return Err("Native audio stop timed out".into());
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        Ok(self.status())
    }
}

struct Renderer {
    core: Core,
    heap: BinaryHeap<Event>,
    frame: u64,
    epoch: u64,
    fade_remaining: u32,
    fade_frames: u32,
    resume_remaining: u32,
    idle: bool,
}
impl Renderer {
    fn new(rate: u32) -> Result<Self, String> {
        Ok(Self {
            core: Core::new(rate)?,
            heap: BinaryHeap::with_capacity(CAPACITY),
            frame: 0,
            epoch: 0,
            fade_remaining: 0,
            fade_frames: (rate / 100).max(1),
            resume_remaining: 0,
            idle: true,
        })
    }
    fn prepare(&mut self, shared: &Shared) {
        let epoch = shared.epoch.load(Ordering::Acquire);
        if epoch != self.epoch {
            self.epoch = epoch;
            let mut removed = self.heap.len();
            self.heap.clear();
            while shared.queue.pop().is_some() {
                removed += 1;
            }
            shared.pending.fetch_sub(removed, Ordering::AcqRel);
            self.fade_remaining = self.fade_frames;
            self.resume_remaining = 0;
        }
        // Fixed admission bounds guarantee this never reallocates. Limit work
        // per callback; large incoming batches cannot monopolize audio forever.
        for _ in 0..MAX_BATCH {
            let Some(event) = shared.queue.pop() else {
                break;
            };
            if event.epoch == self.epoch {
                self.heap.push(event);
            } else {
                shared.pending.fetch_sub(1, Ordering::AcqRel);
            }
        }
    }
    fn sample(&mut self, shared: &Shared) -> [f32; 2] {
        while self
            .heap
            .peek()
            .is_some_and(|event| event.frame <= self.frame)
        {
            let event = self.heap.pop().unwrap();
            shared.pending.fetch_sub(1, Ordering::AcqRel);
            // Frame zero is the explicit ASAP path used by live controls.
            if event.frame != 0 && event.frame < self.frame {
                shared.late.fetch_add(1, Ordering::Relaxed);
            }
            if self.idle && !matches!(event.action, Action::Mix(_) | Action::Reset) {
                self.idle = false;
                self.resume_remaining = (shared.rate / 200).max(1);
            }
            self.core.apply(event.action);
        }
        let mut pcm = self.core.sample();
        if self.fade_remaining > 0 {
            let gain = self.fade_remaining as f32 / self.fade_frames as f32;
            pcm[0] *= gain;
            pcm[1] *= gain;
            self.fade_remaining -= 1;
            if self.fade_remaining == 0 {
                unsafe {
                    native_genesis_stop_voices(self.core.0);
                }
                self.idle = true;
                shared.stopped_epoch.store(self.epoch, Ordering::Release);
            }
        } else if self.idle {
            pcm = [0.0; 2];
        } else if self.resume_remaining > 0 {
            let total = (shared.rate / 200).max(1);
            let gain = 1.0 - self.resume_remaining as f32 / total as f32;
            pcm[0] *= gain;
            pcm[1] *= gain;
            self.resume_remaining -= 1;
        }
        self.frame += 1;
        pcm
    }
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    session: u64,
    sample_rate: u32,
    frame: u64,
    ym_clock: u32,
    psg_clock: u32,
    running: bool,
    queued: usize,
    late_commands: u64,
    queue_overflows: u64,
    error: Option<String>,
}
struct Running {
    shared: Arc<Shared>,
    stop: mpsc::Sender<()>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.shared.stop();
        let _ = self.stop.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
#[derive(Clone, Default)]
pub struct NativeGenesis {
    running: Arc<Mutex<Option<Running>>>,
}
impl NativeGenesis {
    fn start(&self) -> Result<Status, String> {
        self.replace_running(Self::launch)
    }
    fn replace_running(
        &self,
        launch: impl FnOnce(u64) -> Result<Running, String>,
    ) -> Result<Status, String> {
        let mut guard = self
            .running
            .lock()
            .map_err(|_| "Native audio state is unavailable")?;
        // An explicit start always establishes new page ownership. A delayed
        // pagehide shutdown from the old page cannot close this new session.
        drop(guard.take());
        let session = NEXT_SESSION.fetch_add(1, Ordering::Relaxed);
        let running = launch(session)?;
        let status = running.shared.status();
        *guard = Some(running);
        Ok(status)
    }
    fn launch(session: u64) -> Result<Running, String> {
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (stop, stopped) = mpsc::channel();
        let thread = std::thread::spawn(move || match start_device(session) {
            Ok((stream, shared)) => {
                let _ = ready_tx.send(Ok(shared));
                let _ = stopped.recv();
                drop(stream);
            }
            Err(error) => {
                let _ = ready_tx.send(Err(error));
            }
        });
        match ready_rx.recv().map_err(|e| e.to_string())? {
            Ok(shared) => Ok(Running {
                shared,
                stop,
                thread: Some(thread),
            }),
            Err(error) => {
                let _ = thread.join();
                Err(error)
            }
        }
    }
    fn with_session<T>(
        &self,
        session: u64,
        f: impl FnOnce(&Shared) -> Result<T, String>,
    ) -> Result<T, String> {
        let guard = self
            .running
            .lock()
            .map_err(|_| "Native audio state is unavailable")?;
        let shared = &guard
            .as_ref()
            .ok_or("Native Genesis output is not running")?
            .shared;
        shared.check_session(session)?;
        f(shared)
    }
    pub fn shutdown(&self) {
        if let Ok(mut guard) = self.running.lock() {
            guard.take();
        }
    }
    fn shutdown_session(&self, session: u64) -> Result<(), String> {
        let mut guard = self
            .running
            .lock()
            .map_err(|_| "Native audio state is unavailable")?;
        if let Some(running) = guard.as_ref() {
            if running.shared.session != session {
                return Err("Native Genesis session has expired".into());
            }
        }
        guard.take();
        Ok(())
    }
}

fn start_device(session: u64) -> Result<(cpal::Stream, Arc<Shared>), String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let device = cpal::default_host()
        .default_output_device()
        .ok_or("No default native audio output device")?;
    let supported = device
        .default_output_config()
        .map_err(|e| format!("Native audio configuration failed: {e}"))?;
    let config: cpal::StreamConfig = supported.clone().into();
    let shared = Arc::new(Shared::new(config.sample_rate.0, session));
    let stream = match supported.sample_format() {
        cpal::SampleFormat::F32 => build_stream::<f32>(&device, &config, shared.clone()),
        cpal::SampleFormat::I16 => build_stream::<i16>(&device, &config, shared.clone()),
        cpal::SampleFormat::U16 => build_stream::<u16>(&device, &config, shared.clone()),
        format => Err(format!(
            "Unsupported native output sample format: {format:?}"
        )),
    }?;
    stream
        .play()
        .map_err(|e| format!("Native audio output could not start: {e}"))?;
    Ok((stream, shared))
}
fn build_stream<T: cpal::SizedSample + cpal::FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    shared: Arc<Shared>,
) -> Result<cpal::Stream, String> {
    use cpal::traits::DeviceTrait;
    let mut renderer = Renderer::new(config.sample_rate.0)?;
    let channels = config.channels as usize;
    if channels == 0 {
        return Err("Native audio device has no output channels".into());
    }
    let errors = shared.clone();
    device
        .build_output_stream(
            config,
            move |buffer: &mut [T], _| {
                renderer.prepare(&shared);
                for frame in buffer.chunks_mut(channels) {
                    let pcm = renderer.sample(&shared);
                    for (channel, sample) in frame.iter_mut().enumerate() {
                        let value = if channels == 1 {
                            (pcm[0] + pcm[1]) * 0.5
                        } else if channel < 2 {
                            pcm[channel]
                        } else {
                            0.0
                        };
                        *sample = T::from_sample(value.clamp(-1.0, 1.0));
                    }
                }
                shared.frame.store(renderer.frame, Ordering::Release);
            },
            move |_| {
                errors.failed.store(true, Ordering::Release);
            },
            None,
        )
        .map_err(|e| format!("Native audio stream creation failed: {e}"))
}

#[tauri::command]
pub async fn native_genesis_start(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
) -> Result<Status, String> {
    crate::allowed(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.start())
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
pub fn native_genesis_status(
    window: tauri::WebviewWindow,
    state: tauri::State<NativeGenesis>,
    session: u64,
) -> Result<Status, String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| Ok(shared.status()))
}
#[tauri::command]
pub fn native_genesis_commands(
    window: tauri::WebviewWindow,
    state: tauri::State<NativeGenesis>,
    session: u64,
    commands: Vec<Command>,
) -> Result<Status, String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| {
        shared.enqueue(session, &commands)?;
        Ok(shared.status())
    })
}
#[tauri::command]
pub async fn native_genesis_stop(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
) -> Result<Status, String> {
    crate::allowed(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.with_session(session, Shared::stop))
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn native_genesis_shutdown(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
) -> Result<(), String> {
    crate::allowed(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.shutdown_session(session))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    fn command(frame: u64, action: CommandAction) -> Command {
        Command { frame, action }
    }
    fn render(renderer: &mut Renderer, shared: &Shared, count: usize) -> Vec<[f32; 2]> {
        renderer.prepare(shared);
        (0..count).map(|_| renderer.sample(shared)).collect()
    }
    fn tone(frame: u64) -> Vec<Command> {
        vec![
            command(frame, CommandAction::PsgWrite { value: 0x8e }),
            command(frame, CommandAction::PsgWrite { value: 0x0f }),
            command(frame, CommandAction::PsgWrite { value: 0x94 }),
        ]
    }
    fn fm_patch(core: &mut Core) {
        for slot in [0, 4, 8, 12] {
            core.apply(Action::Write(0, 0x30 + slot, 1));
            core.apply(Action::Write(
                0,
                0x40 + slot,
                if slot == 12 { 20 } else { 127 },
            ));
            core.apply(Action::Write(0, 0x50 + slot, 31));
            core.apply(Action::Write(0, 0x60 + slot, 0));
            core.apply(Action::Write(0, 0x70 + slot, 0));
            core.apply(Action::Write(0, 0x80 + slot, 15));
        }
        core.apply(Action::Write(0, 0xb0, 7));
        core.apply(Action::Write(0, 0xb4, 0xc0));
        core.apply(Action::Write(0, 0xa4, 0x22));
        core.apply(Action::Write(0, 0xa0, 0x84));
    }
    #[test]
    fn native_idle_signal_reset_and_stereo_are_finite() {
        for rate in [44100, 48000, 96000] {
            let shared = Shared::new(rate, 1);
            let mut renderer = Renderer::new(rate).unwrap();
            assert!(render(&mut renderer, &shared, 256)
                .iter()
                .all(|v| *v == [0.0; 2]));
            shared.enqueue(1, &tone(256)).unwrap();
            let signal = render(&mut renderer, &shared, 4096);
            assert!(signal
                .iter()
                .flatten()
                .all(|v| v.is_finite() && v.abs() <= 1.0));
            assert!(signal.iter().any(|v| v[0].abs() > 0.01));
            assert!(signal.iter().all(|v| v[0] == v[1]));
            renderer.core.apply(Action::Reset);
            assert!(render(&mut renderer, &shared, 256)
                .iter()
                .all(|v| *v == [0.0; 2]));
        }
    }
    #[test]
    fn scheduling_is_chunk_invariant_and_keeps_equal_time_order() {
        let shared_a = Shared::new(48000, 1);
        let shared_b = Shared::new(48000, 2);
        let mut a = Renderer::new(48000).unwrap();
        let mut b = Renderer::new(48000).unwrap();
        let mut commands = tone(200);
        commands.push(command(1200, CommandAction::PsgWrite { value: 0x9f }));
        // Immediate commands are allowed after already submitted future events.
        commands.push(command(0, CommandAction::PsgStereo { value: 0x10 }));
        shared_a.enqueue(1, &commands).unwrap();
        shared_b.enqueue(2, &commands).unwrap();
        let expected = render(&mut a, &shared_a, 2000);
        let mut actual = Vec::new();
        for count in [1, 127, 53, 512, 31, 1276] {
            actual.extend(render(&mut b, &shared_b, count));
        }
        assert_eq!(actual, expected);
        assert!(actual[..200].iter().all(|v| *v == [0.0; 2]));
        assert!(actual[500..1200].iter().any(|v| v[0] != 0.0));
        assert!(actual.iter().all(|v| v[1] == 0.0));
        assert_eq!(shared_a.pending.load(Ordering::Relaxed), 0);
    }
    #[test]
    fn admission_is_bounded_atomic_and_validates_before_mutation() {
        let shared = Shared::new(48000, 8);
        let batch: Vec<_> = (0..MAX_BATCH)
            .map(|_| command(48000, CommandAction::Reset))
            .collect();
        for _ in 0..(CAPACITY / MAX_BATCH) {
            shared.enqueue(8, &batch).unwrap();
        }
        assert!(shared.enqueue(8, &tone(0)).is_err());
        assert_eq!(shared.pending.load(Ordering::Relaxed), CAPACITY);
        assert_eq!(shared.queue.len(), CAPACITY);
        assert_eq!(shared.overflows.load(Ordering::Relaxed), 1);
        let mut renderer = Renderer::new(48000).unwrap();
        let capacity = renderer.heap.capacity();
        for _ in 0..(CAPACITY / MAX_BATCH) {
            renderer.prepare(&shared);
        }
        assert_eq!(renderer.heap.len(), CAPACITY);
        assert_eq!(renderer.heap.capacity(), capacity);
        assert!(shared.queue.is_empty());
        assert_eq!(shared.pending.load(Ordering::Acquire), CAPACITY);
        shared.epoch.store(1, Ordering::Release);
        renderer.prepare(&shared);
        assert!(renderer.heap.is_empty());
        assert_eq!(renderer.heap.capacity(), capacity);
        assert_eq!(shared.pending.load(Ordering::Acquire), 0);
        assert!(shared.enqueue(7, &[]).unwrap_err().contains("expired"));
        let other = Shared::new(48000, 9);
        let invalid = vec![
            command(0, CommandAction::Reset),
            command(
                0,
                CommandAction::Write {
                    port: 2,
                    register: 0,
                    value: 0,
                },
            ),
        ];
        assert!(other.enqueue(9, &invalid).is_err());
        assert_eq!(other.queue.len(), 0);
        assert_eq!(other.pending.load(Ordering::Relaxed), 0);
    }
    #[test]
    fn stop_cancels_future_notes_fades_resets_and_can_resume() {
        let shared = Shared::new(96000, 1);
        let mut renderer = Renderer::new(96000).unwrap();
        let mut commands = tone(0);
        commands.extend(tone(3000));
        shared.enqueue(1, &commands).unwrap();
        assert!(render(&mut renderer, &shared, 1000)
            .iter()
            .any(|v| v[0] != 0.0));
        shared.epoch.store(1, Ordering::Release);
        let stopped = render(&mut renderer, &shared, 3000);
        assert!(stopped[960..].iter().all(|v| *v == [0.0; 2]));
        assert_eq!(shared.stopped_epoch.load(Ordering::Acquire), 1);
        assert_eq!(shared.pending.load(Ordering::Relaxed), 0);
        shared.enqueue(1, &tone(4000)).unwrap();
        assert!(render(&mut renderer, &shared, 1000)
            .iter()
            .any(|v| v[0] != 0.0));
        assert_eq!(renderer.frame, 5000);
    }
    #[test]
    fn future_events_are_applied_in_timestamp_then_submission_order() {
        let shared = Shared::new(48000, 1);
        let mut renderer = Renderer::new(48000).unwrap();
        shared.enqueue(1, &tone(100)).unwrap();
        shared
            .enqueue(1, &[command(100, CommandAction::PsgWrite { value: 0x9f })])
            .unwrap();
        let output = render(&mut renderer, &shared, 1024);
        assert!(output.iter().all(|v| *v == [0.0; 2]));
        assert_eq!(shared.late.load(Ordering::Relaxed), 0);
    }
    #[test]
    fn ffi_core_render_matches_fresh_reset_and_mix_ramp_settles() {
        let mut core = Core::new(96000).unwrap();
        let mut fresh = Core::new(96000).unwrap();
        core.apply(Action::PsgWrite(0x90));
        for _ in 0..301 {
            core.sample();
        }
        core.apply(Action::Reset);
        for _ in 0..301 {
            assert_eq!(core.sample(), fresh.sample());
        }
        core.apply(Action::PsgWrite(0x90));
        core.apply(Action::Mix([1.0, 1.0, 1.0, 1.0, 0.0]));
        for _ in 0..480 {
            core.sample();
        }
        assert_eq!(core.sample(), [0.0; 2]);
    }
    #[test]
    fn stop_preserves_fm_raw_patch_without_retriggering_voices() {
        let mut core = Core::new(48000).unwrap();
        fm_patch(&mut core);
        // The chip's high latch is shared: A5 then A0 commits the A5 value to
        // CH1. Stop must retain the effective frequency, not replay stale A4.
        core.apply(Action::Write(0, 0xa5, 0x2a));
        core.apply(Action::Write(0, 0xa0, 0x44));
        core.apply(Action::Write(0, 0x28, 0xf0));
        let before: Vec<_> = (0..4096).map(|_| core.sample()).collect();
        assert!(before.iter().any(|v| v[0].abs() > 0.01));
        unsafe {
            native_genesis_stop_voices(core.0);
        }
        assert!((0..1024).all(|_| core.sample() == [0.0; 2]));
        core.apply(Action::Write(0, 0x28, 0xf0));
        let after: Vec<_> = (0..4096).map(|_| core.sample()).collect();
        let mut reference = Core::new(48000).unwrap();
        fm_patch(&mut reference);
        reference.apply(Action::Write(0, 0xa5, 0x2a));
        reference.apply(Action::Write(0, 0xa0, 0x44));
        for _ in 0..1024 {
            reference.sample();
        }
        reference.apply(Action::Write(0, 0x28, 0xf0));
        let expected: Vec<_> = (0..4096).map(|_| reference.sample()).collect();
        assert!(after.iter().any(|v| v[0].abs() > 0.01));
        assert!(
            after
                .iter()
                .zip(expected)
                .all(|(actual, expected)| *actual == expected),
            "Stop must preserve the patch and frequency"
        );
    }
    #[test]
    fn ipc_command_contract_and_gain_ranges_are_explicit() {
        let parsed: Command = serde_json::from_value(serde_json::json!({
            "frame": 17, "type": "mix", "fmLeft": 2, "fmRight": 1,
            "psgLeft": 0.5, "psgRight": 0, "master": 3.8,
        }))
        .unwrap();
        assert!(matches!(
            parsed.validate().unwrap(),
            Action::Mix([2.0, 1.0, 0.5, 0.0, 3.8])
        ));
        let invalid = command(
            0,
            CommandAction::Mix {
                fm_left: 2.01,
                fm_right: 1.0,
                psg_left: 1.0,
                psg_right: 1.0,
                master: 1.0,
            },
        );
        assert!(invalid.validate().is_err());
        let dac = command(
            0,
            CommandAction::Write {
                port: 0,
                register: 0x2b,
                value: 0x80,
            },
        );
        assert!(dac.validate().unwrap_err().contains("DAC"));
    }
    #[test]
    fn replacement_session_rejects_old_commands_and_delayed_pagehide_shutdown() {
        fn headless(session: u64) -> Result<Running, String> {
            let shared = Arc::new(Shared::new(48000, session));
            let callback = shared.clone();
            let (stop, stopped) = mpsc::channel();
            let mut renderer = Renderer::new(48000)?;
            let thread = std::thread::spawn(move || loop {
                if stopped.try_recv().is_ok() {
                    break;
                }
                renderer.prepare(&callback);
                for _ in 0..128 {
                    renderer.sample(&callback);
                }
                callback.frame.store(renderer.frame, Ordering::Release);
                std::thread::sleep(Duration::from_millis(1));
            });
            Ok(Running {
                shared,
                stop,
                thread: Some(thread),
            })
        }
        let rack = NativeGenesis::default();
        let first = rack.replace_running(headless).unwrap();
        rack.with_session(first.session, |shared| {
            shared.enqueue(first.session, &tone(0))
        })
        .unwrap();
        let second = rack.replace_running(headless).unwrap();
        assert_ne!(first.session, second.session);
        assert!(rack
            .with_session(first.session, |shared| shared
                .enqueue(first.session, &tone(0)))
            .unwrap_err()
            .contains("expired"));
        assert!(rack
            .shutdown_session(first.session)
            .unwrap_err()
            .contains("expired"));
        let current = rack
            .with_session(second.session, |shared| Ok(shared.status()))
            .unwrap();
        assert!(current.running);
        assert_eq!(current.session, second.session);
        rack.shutdown_session(second.session).unwrap();
        assert!(rack
            .with_session(second.session, |shared| Ok(shared.status()))
            .is_err());
    }
    #[test]
    #[ignore = "opens the system default audio output; sends silence only"]
    fn native_device_silent_smoke() {
        fn wait_for(
            rack: &NativeGenesis,
            session: u64,
            deadline: Instant,
            ready: impl Fn(&Status) -> bool,
        ) -> Status {
            loop {
                let status = rack
                    .with_session(session, |shared| Ok(shared.status()))
                    .expect("The native stream must remain available");
                assert!(status.running && status.error.is_none(), "{status:?}");
                if ready(&status) {
                    return status;
                }
                assert!(
                    Instant::now() < deadline,
                    "Native callback did not advance: {status:?}"
                );
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        let mute = || {
            command(
                0,
                CommandAction::Mix {
                    fm_left: 1.0,
                    fm_right: 1.0,
                    psg_left: 1.0,
                    psg_right: 1.0,
                    master: 0.0,
                },
            )
        };
        // Prove the startup and master-ramp path is exactly silent before
        // opening the device. No voice is keyed on and PSG volumes stay muted.
        let offline = Shared::new(48000, 1);
        let mut renderer = Renderer::new(48000).unwrap();
        assert!(render(&mut renderer, &offline, 128)
            .iter()
            .all(|pcm| *pcm == [0.0; 2]));
        offline.enqueue(1, &[mute()]).unwrap();
        assert!(render(&mut renderer, &offline, 512)
            .iter()
            .all(|pcm| *pcm == [0.0; 2]));

        let rack = NativeGenesis::default();
        let started = rack.start().expect("Open the system default native output");
        assert!(started.running && started.error.is_none(), "{started:?}");
        let deadline = Instant::now() + Duration::from_secs(2);
        rack.with_session(started.session, |shared| {
            shared.enqueue(started.session, &[mute()])
        })
        .unwrap();
        let drained = wait_for(&rack, started.session, deadline, |status| {
            status.queued == 0
        });
        // A completed callback beyond the full 5 ms ramp is the mute barrier.
        let muted = wait_for(&rack, started.session, deadline, |status| {
            status.frame > drained.frame + u64::from(status.sample_rate / 200)
        });
        rack.with_session(started.session, |shared| {
            shared.enqueue(
                started.session,
                &[command(
                    0,
                    CommandAction::Write {
                        port: 0,
                        register: 0x40,
                        value: 127,
                    },
                )],
            )
        })
        .unwrap();
        let written = wait_for(&rack, started.session, deadline, |status| {
            status.queued == 0 && status.frame > muted.frame
        });
        let stopped = rack
            .with_session(started.session, Shared::stop)
            .expect("Native stop fade barrier");
        assert!(stopped.running && stopped.error.is_none(), "{stopped:?}");
        assert_eq!(stopped.queued, 0);
        assert!(stopped.frame >= written.frame);
        wait_for(&rack, started.session, deadline, |status| {
            status.frame > stopped.frame
        });
        rack.shutdown_session(started.session)
            .expect("Close the native stream");
        assert!(rack.running.lock().unwrap().is_none());
    }
}
