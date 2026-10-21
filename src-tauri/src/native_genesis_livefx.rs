//! Optional QuickJS post-FX stage. Only fixed-size Copy PCM crosses its bounded
//! queues. QuickJS, JSON, locks, destruction and reporting stay on its worker.
use super::{Owner, Shared, MAX_SAFE_INTEGER};
use crate::native_live_fx_probe::LiveFxHost;
use crossbeam_queue::ArrayQueue;
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering},
        mpsc, Arc, Mutex, Weak,
    },
    thread,
    time::{Duration, Instant},
};

pub(super) const BLOCK: usize = 128;
pub(super) const LEAD: usize = 2048;
const BLOCKS: usize = 32;
const COMMANDS: usize = 64;
const REPORTS: usize = 16;
const REPORT_BYTES: usize = 16 * 1024;
const COMMAND_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy)]
struct Block {
    generation: u64,
    frame: u64,
    terminal: bool,
    pcm: [f32; BLOCK * 2],
}
impl Default for Block {
    fn default() -> Self {
        Self {
            generation: 0,
            frame: 0,
            terminal: false,
            pcm: [0.; BLOCK * 2],
        }
    }
}

pub(super) struct Transport {
    input: ArrayQueue<Block>,
    output: ArrayQueue<Block>,
    enabled: AtomicBool,
    generation: AtomicU64,
    reset: AtomicU64,
    reset_ack: AtomicU64,
    active: AtomicU32,
    initialized: AtomicBool,
    underruns: AtomicU64,
    overruns: AtomicU64,
    calls: AtomicU64,
    queued: AtomicUsize,
    reports: Mutex<VecDeque<String>>,
    error: Mutex<Option<String>>,
}
impl Transport {
    fn new() -> Self {
        Self {
            input: ArrayQueue::new(BLOCKS),
            output: ArrayQueue::new(BLOCKS),
            enabled: AtomicBool::new(false),
            generation: AtomicU64::new(1),
            reset: AtomicU64::new(0),
            reset_ack: AtomicU64::new(0),
            active: AtomicU32::new(0),
            initialized: AtomicBool::new(false),
            underruns: AtomicU64::new(0),
            overruns: AtomicU64::new(0),
            calls: AtomicU64::new(0),
            queued: AtomicUsize::new(0),
            reports: Mutex::new(VecDeque::with_capacity(REPORTS)),
            error: Mutex::new(None),
        }
    }
    /// Callback-safe invalidation: old wet blocks can never enter a new epoch.
    pub(super) fn invalidate(&self, reset: bool) {
        if reset {
            self.enabled.store(false, Ordering::Release);
            self.reset.fetch_add(1, Ordering::AcqRel);
        }
        self.generation.fetch_add(1, Ordering::AcqRel);
    }
    fn report(&self, text: impl Into<String>) {
        let mut text = text.into();
        if text.len() > REPORT_BYTES {
            let mut end = REPORT_BYTES;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
        }
        if let Ok(mut error) = self.error.lock() {
            *error = Some(text.clone());
        }
        if let Ok(mut reports) = self.reports.lock() {
            if reports.len() == REPORTS {
                reports.pop_front();
            }
            reports.push_back(text);
        }
    }
    fn set_enabled(&self, value: bool) {
        if value && !self.enabled.load(Ordering::Acquire) {
            // A new rack must never consume a previous rack's queued audio.
            self.generation.fetch_add(1, Ordering::AcqRel);
        }
        self.enabled.store(value, Ordering::Release);
    }
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub(super) struct Status {
    active: u32,
    initialized: bool,
    latency_frames: usize,
    block_frames: usize,
    underruns: u64,
    overruns: u64,
    process_calls: u64,
    queued_commands: usize,
    error: Option<String>,
    reports: Vec<String>,
}

struct Command {
    frame: u64,
    sequence: u64,
    epoch: u64,
    reset: u64,
    owner: Option<Owner>,
    message: Value,
}
struct Worker {
    sender: mpsc::SyncSender<Command>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            if thread.thread().id() != thread::current().id() {
                let _ = thread.join();
            }
        }
    }
}
pub(super) struct Control {
    pub(super) transport: Arc<Transport>,
    worker: Mutex<Option<Worker>>,
    sequence: AtomicU64,
}
impl Control {
    pub(super) fn new() -> Self {
        Self {
            transport: Arc::new(Transport::new()),
            worker: Mutex::new(None),
            sequence: AtomicU64::new(0),
        }
    }
    pub(super) fn status(&self) -> Status {
        let transport = &self.transport;
        // The worker publishes active/initialized before releasing its queued
        // reservation. Read that reservation first so a completed first
        // registration cannot appear as active=0, queued=0 during compilation.
        let queued_commands = transport.queued.load(Ordering::Acquire);
        Status {
            active: transport.active.load(Ordering::Acquire),
            initialized: transport.initialized.load(Ordering::Acquire),
            latency_frames: if transport.enabled.load(Ordering::Acquire) {
                LEAD
            } else {
                0
            },
            block_frames: BLOCK,
            underruns: transport.underruns.load(Ordering::Relaxed),
            overruns: transport.overruns.load(Ordering::Relaxed),
            process_calls: transport.calls.load(Ordering::Relaxed),
            queued_commands,
            error: transport.error.lock().ok().and_then(|error| error.clone()),
            // Keep status observational; frontend deduplicates bounded reports.
            reports: transport
                .reports
                .lock()
                .map(|reports| reports.iter().cloned().collect())
                .unwrap_or_default(),
        }
    }
    pub(super) fn command(
        &self,
        shared: &Arc<Shared>,
        message: Value,
        frame: u64,
        owner: Option<Owner>,
    ) -> Result<(), String> {
        validate(&message)?;
        if frame > MAX_SAFE_INTEGER {
            return Err("Invalid native liveFx frame".into());
        }
        if message.get("action").and_then(Value::as_str) == Some("clear") {
            if frame != 0 {
                return Err("Native liveFx clear requires frame zero".into());
            }
            self.reset();
            return Ok(());
        }
        let mut worker = self
            .worker
            .lock()
            .map_err(|_| "Native liveFx worker state is unavailable")?;
        if worker.is_none() {
            match message.get("action").and_then(Value::as_str) {
                Some("remove") => return Ok(()),
                Some("register") => {}
                _ => return Err("Native liveFx has no registered effect".into()),
            }
            let (sender, receiver) = mpsc::sync_channel(COMMANDS);
            let stop = Arc::new(AtomicBool::new(false));
            let worker_stop = stop.clone();
            let weak = Arc::downgrade(shared);
            let transport = self.transport.clone();
            let rate = shared.rate;
            let thread = thread::Builder::new()
                .name("genesis-livefx".into())
                .spawn(move || run(weak, transport, rate, receiver, worker_stop))
                .map_err(|error| format!("Could not start native liveFx worker: {error}"))?;
            *worker = Some(Worker {
                sender,
                stop,
                thread: Some(thread),
            });
        }
        let command = Command {
            frame,
            sequence: self.sequence.fetch_add(1, Ordering::Relaxed),
            epoch: shared.epoch.load(Ordering::Acquire),
            reset: self.transport.reset.load(Ordering::Acquire),
            owner,
            message,
        };
        self.transport
            .queued
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |queued| {
                queued.checked_add(1).filter(|next| *next <= COMMANDS)
            })
            .map_err(|_| "Native liveFx command queue is full (64); command was not admitted")?;
        if let Err(error) = worker.as_ref().unwrap().sender.try_send(command) {
            self.transport.queued.fetch_sub(1, Ordering::AcqRel);
            return Err(match error {
                mpsc::TrySendError::Full(_) => {
                    "Native liveFx command queue is full (64); command was not admitted".into()
                }
                mpsc::TrySendError::Disconnected(_) => "Native liveFx worker has stopped".into(),
            });
        }
        Ok(())
    }

    pub(super) fn reset(&self) {
        self.transport.invalidate(true);
    }
    pub(super) fn wait_reset(&self) -> Result<(), String> {
        let target = self.transport.reset.load(Ordering::Acquire);
        let has_worker = self
            .worker
            .lock()
            .map_err(|_| "Native liveFx worker state is unavailable")?
            .is_some();
        if !has_worker {
            return Ok(());
        }
        let start = Instant::now();
        while self.transport.reset_ack.load(Ordering::Acquire) < target {
            if start.elapsed() > Duration::from_secs(2) {
                return Err("Native liveFx reset timed out".into());
            }
            thread::sleep(Duration::from_millis(1));
        }
        Ok(())
    }
}
fn validate(message: &Value) -> Result<(), String> {
    if !message.is_object() || message.get("op").and_then(Value::as_str) != Some("live-fx") {
        return Err("Invalid native liveFx message".into());
    }
    let action = message
        .get("action")
        .and_then(Value::as_str)
        .ok_or("Invalid native liveFx action")?;
    if !matches!(action, "register" | "context" | "remove" | "clear") {
        return Err("Unknown native liveFx action".into());
    }
    if action != "clear" {
        let name = message
            .get("name")
            .and_then(Value::as_str)
            .ok_or("Native liveFx requires a name")?;
        if name.is_empty() || name.len() > 128 {
            return Err("Native liveFx name must be 1..128 UTF-8 bytes".into());
        }
    }
    if action == "register" {
        let source = message
            .get("source")
            .and_then(Value::as_str)
            .ok_or("Native liveFx register requires source")?;
        if source.is_empty() || source.len() > 192 * 1024 {
            return Err("Native liveFx source must be 1..196608 UTF-8 bytes".into());
        }
        if message
            .get("resetState")
            .is_some_and(|value| !value.is_boolean())
        {
            return Err("Invalid native liveFx resetState".into());
        }
    }
    for key in ["contextGraph", "context"] {
        if let Some(context) = message.get(key) {
            if serde_json::to_vec(context)
                .map_err(|error| error.to_string())?
                .len()
                > 128 * 1024
            {
                return Err("Native liveFx context exceeds 128 KiB".into());
            }
        }
    }
    if serde_json::to_vec(message)
        .map_err(|error| error.to_string())?
        .len()
        > COMMAND_BYTES
    {
        return Err("Native liveFx command exceeds 256 KiB".into());
    }
    Ok(())
}
fn valid(command: &Command, shared: &Weak<Shared>) -> bool {
    shared.upgrade().is_some_and(|shared| {
        command.epoch == shared.epoch.load(Ordering::Acquire) && shared.valid_owner(command.owner)
    })
}
fn apply(command: Command, host: &mut LiveFxHost, transport: &Transport) {
    if let Err(error) = host.command(&command.message) {
        transport.report(format!("[native liveFx] {error}"));
    }
    update(host, transport);
    // Include compilation/context decoding in the global 64-command bound and
    // keep status polling alive until activation/errors have been published.
    transport.queued.fetch_sub(1, Ordering::AcqRel);
}
fn update(host: &mut LiveFxHost, transport: &Transport) {
    transport.active.store(host.active(), Ordering::Release);
    transport
        .initialized
        .store(host.initialized(), Ordering::Release);
    transport
        .calls
        .store(host.process_calls(), Ordering::Relaxed);
    let reports = host.reports();
    if !reports.is_empty() {
        transport.report(reports);
    }
    if host.active() > 0 {
        transport.set_enabled(true);
    }
}
fn run(
    shared: Weak<Shared>,
    transport: Arc<Transport>,
    rate: u32,
    receiver: mpsc::Receiver<Command>,
    stop: Arc<AtomicBool>,
) {
    let mut host = match LiveFxHost::new(rate, BLOCK as u32, 32 * 1024 * 1024, 50_000) {
        Ok(host) => host,
        Err(error) => {
            transport.report(error);
            return;
        }
    };
    let mut pending = Vec::<Command>::with_capacity(COMMANDS);
    let mut reset = 0;
    transport.reset_ack.store(0, Ordering::Release);
    while !stop.load(Ordering::Acquire) {
        let next_reset = transport.reset.load(Ordering::Acquire);
        if next_reset != reset {
            reset = next_reset;
            host.clear();
            let mut discarded = 0;
            pending.retain(|command| {
                let keep = command.reset >= reset;
                if !keep {
                    discarded += 1;
                }
                keep
            });
            while let Ok(command) = receiver.try_recv() {
                if command.reset >= reset {
                    pending.push(command);
                } else {
                    discarded += 1;
                }
            }
            transport.queued.fetch_sub(discarded, Ordering::AcqRel);
            while transport.input.pop().is_some() {}
            transport.active.store(0, Ordering::Release);
            transport.initialized.store(false, Ordering::Release);
            transport.enabled.store(false, Ordering::Release);
            if let Ok(mut error) = transport.error.lock() {
                *error = None;
            }
            if let Ok(mut reports) = transport.reports.lock() {
                reports.clear();
            }
            transport.reset_ack.store(reset, Ordering::Release);
        }
        while pending.len() < COMMANDS {
            match receiver.try_recv() {
                Ok(command) => pending.push(command),
                Err(_) => break,
            }
        }
        pending.retain(|command| {
            let keep = command.reset >= reset && valid(command, &shared);
            if !keep {
                transport.queued.fetch_sub(1, Ordering::AcqRel);
            }
            keep
        });
        pending.sort_by_key(|command| (command.frame, command.sequence));
        let mut did_work = false;
        // The first registration has no audio transport. It becomes due only
        // after the source clock reaches its frame; never activate it early.
        if !transport.enabled.load(Ordering::Acquire) {
            let now = shared
                .upgrade()
                .map(|shared| shared.frame.load(Ordering::Acquire))
                .unwrap_or(u64::MAX);
            while pending
                .first()
                .is_some_and(|command| command.frame == 0 || command.frame <= now)
            {
                if pending[0].reset != reset || transport.reset.load(Ordering::Acquire) != reset {
                    break;
                }
                let command = pending.remove(0);
                if valid(&command, &shared) {
                    apply(command, &mut host, &transport);
                } else {
                    transport.queued.fetch_sub(1, Ordering::AcqRel);
                }
                did_work = true;
            }
        }
        if transport.reset.load(Ordering::Acquire) != reset {
            continue;
        }
        while let Some(mut block) = transport.input.pop() {
            did_work = true;
            let generation = transport.generation.load(Ordering::Acquire);
            if block.generation != generation {
                continue;
            }
            // Quantize updates to source block starts, at most 127 frames late.
            while pending
                .first()
                .is_some_and(|command| command.frame == 0 || command.frame <= block.frame)
            {
                if pending[0].reset != reset || transport.reset.load(Ordering::Acquire) != reset {
                    break;
                }
                let command = pending.remove(0);
                if valid(&command, &shared) {
                    apply(command, &mut host, &transport);
                } else {
                    transport.queued.fetch_sub(1, Ordering::AcqRel);
                }
            }
            if transport.reset.load(Ordering::Acquire) != reset {
                break;
            }
            let dry = block.pcm;
            if let Err(error) = host.process(&mut block.pcm) {
                block.pcm = dry;
                transport.report(format!("[native liveFx] {error}"));
            }
            // Sanitise even if a future host implementation weakens its check.
            if block.pcm.iter().any(|sample| !sample.is_finite()) {
                block.pcm = dry;
                transport.report("[native liveFx] Non-finite PCM bypassed");
            }
            update(&mut host, &transport);
            block.terminal = host.active() == 0;
            if block.generation == transport.generation.load(Ordering::Acquire)
                && transport.reset.load(Ordering::Acquire) == reset
            {
                if transport.output.push(block).is_err() {
                    transport.overruns.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        if !did_work {
            if !transport.enabled.load(Ordering::Acquire) && pending.is_empty() {
                match receiver.recv_timeout(Duration::from_millis(100)) {
                    Ok(command) => pending.push(command),
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
            } else {
                thread::sleep(Duration::from_millis(1));
            }
        }
    }
    host.clear();
}

/// Callback-owned fixed storage. Its inactive fast path does no queue work and
/// returns the original bits. Master/fades/monitor/recording run after this stage.
pub(super) struct Renderer {
    generation: u64,
    engaged: bool,
    start: u64,
    dry: Box<[[f32; 2]; LEAD]>,
    input: Block,
    input_frames: usize,
    output: Option<Block>,
    output_ready: bool,
    mix: f32,
    last: [f32; 2],
    bridge_from: [f32; 2],
    bridge_remaining: u32,
    bridge_total: u32,
    stop_hold: Option<[f32; 2]>,
}
impl Renderer {
    pub(super) fn new() -> Self {
        Self {
            generation: 0,
            engaged: false,
            start: 0,
            dry: Box::new([[0.; 2]; LEAD]),
            input: Block::default(),
            input_frames: 0,
            output: None,
            output_ready: false,
            mix: 0.,
            last: [0.; 2],
            bridge_from: [0.; 2],
            bridge_remaining: 0,
            bridge_total: 0,
            stop_hold: None,
        }
    }
    fn reset(&mut self, generation: u64) {
        self.generation = generation;
        self.engaged = false;
        self.input_frames = 0;
        self.output = None;
        self.output_ready = false;
        self.mix = 0.;
        self.dry.fill([0.; 2]);
    }
    pub(super) fn has_output(&self) -> bool {
        self.engaged || self.bridge_remaining > 0 || self.stop_hold.is_some()
    }
    pub(super) fn begin_stop(&mut self) {
        if self.engaged {
            self.stop_hold = Some(self.last);
        }
    }
    pub(super) fn end_stop(&mut self) {
        self.reset(self.generation);
        self.stop_hold = None;
        self.last = [0.; 2];
        self.bridge_remaining = 0;
    }
    // Continue already completed wet blocks only inside the outer Stop fade.
    // No new input is submitted and no result from a new generation is used.
    // Missing completed audio holds the last delivered value as a bounded
    // fallback. end_stop discards this state before the Stop barrier is acked.
    fn stopping_sample(&mut self, pcm: [f32; 2], frame: u64, transport: &Transport) -> [f32; 2] {
        let offset = frame.saturating_sub(self.start) as usize;
        let mut result = self.stop_hold.unwrap();
        if offset >= LEAD {
            let block_offset = (offset - LEAD) % BLOCK;
            let wanted = frame - LEAD as u64 - block_offset as u64;
            if !self
                .output
                .as_ref()
                .is_some_and(|block| block.generation == self.generation && block.frame == wanted)
            {
                self.output_ready = false;
                // At most the fixed 32 queued blocks can be discarded.
                for _ in 0..=BLOCKS {
                    if let Some(block) = self.output {
                        if block.generation != self.generation || block.frame < wanted {
                            self.output = None;
                            continue;
                        }
                        self.output_ready = block.frame == wanted;
                        break;
                    }
                    self.output = transport.output.pop();
                    if self.output.is_none() {
                        break;
                    }
                }
            }
            if self.output_ready {
                let block = self.output.as_ref().unwrap();
                let wet = [block.pcm[block_offset * 2], block.pcm[block_offset * 2 + 1]];
                result = if self.mix == 1. {
                    wet
                } else {
                    [
                        pcm[0] + (wet[0] - pcm[0]) * self.mix,
                        pcm[1] + (wet[1] - pcm[1]) * self.mix,
                    ]
                };
            }
        } else {
            result = pcm;
        }
        let result = self.finish(result);
        self.stop_hold = Some(result);
        result
    }
    fn bridge(&mut self, rate: u32) {
        self.bridge_from = self.last;
        self.bridge_total = (rate / 200).max(1);
        self.bridge_remaining = self.bridge_total;
    }
    fn finish(&mut self, mut pcm: [f32; 2]) -> [f32; 2] {
        if self.bridge_remaining > 0 {
            let mix = 1. - self.bridge_remaining as f32 / self.bridge_total as f32;
            pcm = [
                self.bridge_from[0] + (pcm[0] - self.bridge_from[0]) * mix,
                self.bridge_from[1] + (pcm[1] - self.bridge_from[1]) * mix,
            ];
            self.bridge_remaining -= 1;
        }
        self.last = pcm;
        pcm
    }
    pub(super) fn process(
        &mut self,
        pcm: [f32; 2],
        frame: u64,
        rate: u32,
        transport: &Transport,
    ) -> [f32; 2] {
        if self.stop_hold.is_some() {
            return self.stopping_sample(pcm, frame, transport);
        }
        let mut enabled = transport.enabled.load(Ordering::Acquire);
        if !enabled && !self.engaged {
            return self.finish(pcm);
        }
        let generation = transport.generation.load(Ordering::Acquire);
        if generation != self.generation {
            let was_engaged = self.engaged;
            let last = self.last;
            self.reset(generation);
            if was_engaged {
                self.last = last;
                self.bridge(rate);
            }
            while transport.output.pop().is_some() {}
        }
        if !enabled && !self.engaged {
            return self.finish(pcm);
        }
        if !self.engaged {
            self.engaged = true;
            self.start = frame;
        }
        let offset = (frame - self.start) as usize;
        let delayed_dry = std::mem::replace(&mut self.dry[offset % LEAD], pcm);
        if enabled {
            if self.input_frames == 0 {
                self.input.generation = generation;
                self.input.frame = frame;
            }
            let index = self.input_frames * 2;
            self.input.pcm[index] = pcm[0];
            self.input.pcm[index + 1] = pcm[1];
            self.input_frames += 1;
            if self.input_frames == BLOCK {
                if transport.input.push(self.input).is_err() {
                    transport.overruns.fetch_add(1, Ordering::Relaxed);
                }
                self.input_frames = 0;
            }
        } else {
            self.input_frames = 0;
        }
        if offset < LEAD {
            return self.finish(pcm);
        }
        let block_offset = (offset - LEAD) % BLOCK;
        if block_offset == 0 {
            let was_ready = self.output_ready;
            let wanted = frame - LEAD as u64;
            self.output_ready = false;
            loop {
                if let Some(block) = self.output {
                    if block.generation != generation || block.frame < wanted {
                        self.output = None;
                        continue;
                    }
                    if block.frame == wanted {
                        self.output_ready = true;
                    }
                    break;
                }
                match transport.output.pop() {
                    Some(block) => self.output = Some(block),
                    None => break,
                }
            }
            if enabled && !self.output_ready {
                transport.underruns.fetch_add(1, Ordering::Relaxed);
            }
            if self.output_ready != was_ready && offset > LEAD {
                self.bridge(rate);
            }
            // Last removal becomes audible at the same delayed source frame.
            // A replacement registration prevents a stale terminal block from
            // disabling the new rack.
            if self.output_ready
                && self.output.as_ref().is_some_and(|block| block.terminal)
                && transport.active.load(Ordering::Acquire) == 0
            {
                transport.enabled.store(false, Ordering::Release);
                enabled = false;
                self.bridge(rate);
            }
        }
        let wet = if self.output_ready {
            let block = self.output.as_ref().unwrap();
            [block.pcm[block_offset * 2], block.pcm[block_offset * 2 + 1]]
        } else {
            delayed_dry
        };
        let step = 1. / (rate / 200).max(1) as f32;
        self.mix = if enabled {
            (self.mix + step).min(1.)
        } else {
            (self.mix - step).max(0.)
        };
        let result = if self.mix == 1. {
            wet
        } else if self.mix == 0. {
            pcm
        } else {
            [
                pcm[0] + (wet[0] - pcm[0]) * self.mix,
                pcm[1] + (wet[1] - pcm[1]) * self.mix,
            ]
        };
        if !enabled && self.mix == 0. {
            self.reset(generation);
        }
        self.finish(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const GAIN: &str = "process(input, output, state, context) { for (let c=0;c<2;c++) for(let i=0;i<input[c].length;i++) output[c][i]=input[c][i]*context.gain; }";
    fn register(gain: f32) -> Value {
        json!({"op":"live-fx","action":"register","name":"gain","source":GAIN,"context":{"gain":gain},"resetState":false})
    }
    fn wait(mut predicate: impl FnMut() -> bool) {
        let start = Instant::now();
        while !predicate() {
            assert!(
                start.elapsed() < Duration::from_secs(3),
                "native liveFx worker did not reach expected state"
            );
            thread::sleep(Duration::from_millis(1));
        }
    }
    fn service(transport: &Transport, gain: f32) {
        while let Some(mut block) = transport.input.pop() {
            for sample in &mut block.pcm {
                *sample *= gain;
            }
            assert!(transport.output.push(block).is_ok());
        }
    }
    fn pump(
        stage: &mut Renderer,
        shared: &Arc<Shared>,
        start: u64,
        blocks: usize,
    ) -> Vec<[f32; 2]> {
        let mut out = Vec::with_capacity(blocks * BLOCK);
        for block in 0..blocks {
            for offset in 0..BLOCK {
                let frame = start + (block * BLOCK + offset) as u64;
                out.push(stage.process([0.5, -0.25], frame, shared.rate, &shared.livefx.transport));
            }
            shared
                .frame
                .store(start + ((block + 1) * BLOCK) as u64, Ordering::Release);
            // Functional parity is driven slightly slower than 48 kHz. Queue
            // starvation and overflow have separate deterministic stress tests.
            thread::sleep(Duration::from_millis(3));
        }
        out
    }
    #[test]
    fn unused_stage_is_bit_identical_without_vm_thread_or_queue_operations() {
        let shared = Shared::new(48_000, 1);
        let transport = &shared.livefx.transport;
        transport
            .output
            .push(Block {
                frame: 99,
                ..Block::default()
            })
            .unwrap_or_else(|_| panic!());
        let mut stage = Renderer::new();
        for frame in 0..10_000 {
            let pcm = [f32::from_bits((frame * 17) as u32), -0.0];
            let result = stage.process(pcm, frame, 48_000, transport);
            assert_eq!(result.map(f32::to_bits), pcm.map(f32::to_bits));
        }
        assert!(shared.livefx.worker.lock().unwrap().is_none());
        assert_eq!(transport.input.len(), 0);
        assert_eq!(transport.output.len(), 1);
        let status = shared.livefx.status();
        assert!(!status.initialized);
        assert_eq!(status.process_calls, 0);
        assert_eq!(status.latency_frames, 0);
    }
    #[test]
    fn delayed_wet_and_missing_block_dry_have_the_same_source_frame() {
        let transport = Transport::new();
        transport.active.store(1, Ordering::Release);
        transport.set_enabled(true);
        let mut stage = Renderer::new();
        for frame in 0..(LEAD + 1024) as u64 {
            let pcm = [frame as f32 / 10_000., -(frame as f32) / 20_000.];
            let out = stage.process(pcm, frame, 48_000, &transport);
            service(&transport, 0.5);
            if frame >= (LEAD + 500) as u64 {
                let source = (frame - LEAD as u64) as f32;
                assert!((out[0] - source / 20_000.).abs() < 1e-7);
                assert!((out[1] + source / 40_000.).abs() < 1e-7);
            }
        }
        assert_eq!(transport.underruns.load(Ordering::Relaxed), 0);
        while transport.output.pop().is_some() {}
        // Stop the worker entirely; no completed block may be repeated. Every
        // missing block must use its own source frame from the dry delay.
        for frame in (LEAD + 1024) as u64..(LEAD + 3072) as u64 {
            let pcm = [frame as f32 / 10_000., 0.];
            let out = stage.process(pcm, frame, 48_000, &transport);
            if frame >= (LEAD + 2048) as u64 {
                assert!((out[0] - (frame - LEAD as u64) as f32 / 10_000.).abs() < 1e-7);
            }
        }
        assert!(transport.underruns.load(Ordering::Relaxed) > 0);
        assert_eq!(transport.input.len(), 16);
    }
    #[test]
    fn stalled_worker_has_fixed_capacity_and_epoch_flush_cannot_repeat_wet_pcm() {
        let transport = Transport::new();
        transport.active.store(1, Ordering::Release);
        transport.set_enabled(true);
        let mut stage = Renderer::new();
        for frame in 0..(BLOCK * 40) as u64 {
            assert!(stage
                .process([0.3, -0.4], frame, 48_000, &transport)
                .iter()
                .all(|sample| sample.is_finite()));
        }
        assert_eq!(transport.input.len(), BLOCKS);
        assert_eq!(transport.overruns.load(Ordering::Relaxed), 8);
        transport
            .output
            .push(Block {
                generation: transport.generation.load(Ordering::Acquire),
                pcm: [0.9; BLOCK * 2],
                ..Block::default()
            })
            .unwrap_or_else(|_| panic!());
        transport.invalidate(true);
        let dry = [-0.125, 0.25];
        let previous = stage.last;
        assert_eq!(
            stage.process(dry, (BLOCK * 40) as u64, 48_000, &transport),
            previous
        );
        for offset in 1..=240 {
            stage.process(dry, (BLOCK * 40 + offset) as u64, 48_000, &transport);
        }
        assert_eq!(stage.last, dry);
        assert_eq!(transport.output.len(), 0);
        assert!(!stage.engaged);
        assert_eq!(stage.mix, 0.);
    }
    #[test]
    fn actual_quickjs_worker_processes_and_updates_then_releases_the_vm() {
        let shared = Arc::new(Shared::new(48_000, 1));
        shared
            .livefx
            .command(&shared, register(0.5), 0, None)
            .unwrap();
        wait(|| shared.livefx.status().active == 1);
        let mut stage = Renderer::new();
        let first = pump(&mut stage, &shared, 0, 40);
        assert_eq!(first.last().copied(), Some([0.25, -0.125]));
        assert!(shared.livefx.status().process_calls > 0);
        let source_frame = (BLOCK * 44) as u64;
        shared
            .livefx
            .command(
                &shared,
                json!({"op":"live-fx","action":"context","name":"gain","context":{"gain":0.25}}),
                source_frame,
                None,
            )
            .unwrap();
        let next = pump(&mut stage, &shared, (BLOCK * 40) as u64, 32);
        let due = (source_frame + LEAD as u64 - (BLOCK * 40) as u64) as usize;
        assert_eq!(next[due - 1], [0.25, -0.125]);
        assert_eq!(next[due + BLOCK], [0.125, -0.0625]);
        shared
            .livefx
            .command(
                &shared,
                json!({"op":"live-fx","action":"remove","name":"gain"}),
                0,
                None,
            )
            .unwrap();
        let mut next_frame = (BLOCK * 72) as u64;
        // Controls are consumed at source block boundaries. Keep the audio
        // producer advancing while waiting rather than waiting with no input.
        for _ in 0..32 {
            pump(&mut stage, &shared, next_frame, 1);
            next_frame += BLOCK as u64;
            if shared.livefx.status().active == 0 {
                break;
            }
        }
        assert_eq!(shared.livefx.status().active, 0);
        assert!(!shared.livefx.status().initialized);
        pump(&mut stage, &shared, next_frame, 24);
        next_frame += (BLOCK * 24) as u64;
        assert!(!shared.livefx.transport.enabled.load(Ordering::Acquire));
        let before = shared.livefx.status().process_calls;
        assert_eq!(
            stage.process([0.5, -0.25], next_frame, 48_000, &shared.livefx.transport),
            [0.5, -0.25]
        );
        assert_eq!(shared.livefx.status().process_calls, before);
        assert_eq!(shared.livefx.status().underruns, 0);
    }
    #[test]
    fn future_first_registration_never_activates_early_and_cancelled_owner_has_no_vm() {
        let shared = Arc::new(Shared::new(48_000, 1));
        shared.performance("start", 7, None).unwrap();
        let owner = Some(Owner {
            performance: 7,
            owner: 1,
        });
        shared
            .livefx
            .command(&shared, register(0.5), 20_000, owner)
            .unwrap();
        thread::sleep(Duration::from_millis(10));
        assert!(!shared.livefx.status().initialized);
        assert_eq!(shared.livefx.status().latency_frames, 0);
        shared.performance("cancel-owner", 7, Some(1)).unwrap();
        shared.frame.store(40_000, Ordering::Release);
        wait(|| shared.livefx.status().queued_commands == 0);
        assert!(!shared.livefx.status().initialized);
        assert_eq!(shared.livefx.status().active, 0);
        assert_eq!(shared.livefx.status().latency_frames, 0);
        shared
            .livefx
            .command(&shared, register(0.5), 0, None)
            .unwrap();
        wait(|| shared.livefx.status().active == 1);
    }
    #[test]
    fn future_command_admission_is_bounded_and_reset_discards_it_off_callback() {
        let shared = Arc::new(Shared::new(48_000, 1));
        for _ in 0..COMMANDS {
            shared
                .livefx
                .command(&shared, register(0.5), 1_000_000, None)
                .unwrap();
        }
        assert!(shared
            .livefx
            .command(&shared, register(0.5), 1_000_000, None)
            .unwrap_err()
            .contains("64"));
        assert_eq!(shared.livefx.status().queued_commands, COMMANDS);
        shared.livefx.reset();
        shared.livefx.wait_reset().unwrap();
        assert_eq!(shared.livefx.status().queued_commands, 0);
        assert!(!shared.livefx.status().initialized);
        assert_eq!(shared.livefx.status().latency_frames, 0);
    }
    #[test]
    fn invalid_commands_do_not_create_a_worker_and_utf8_limits_are_bytes() {
        let shared = Arc::new(Shared::new(48_000, 1));
        let mut message = register(0.5);
        message["name"] = json!("あ".repeat(43));
        assert!(shared.livefx.command(&shared, message, 0, None).is_err());
        let mut message = register(0.5);
        message["source"] = json!("a".repeat(192 * 1024 + 1));
        assert!(shared.livefx.command(&shared, message, 0, None).is_err());
        assert!(shared
            .livefx
            .command(&shared, register(0.5), MAX_SAFE_INTEGER + 1, None)
            .is_err());
        let mut message = register(0.5);
        message["contextGraph"] = json!({"payload":"a".repeat(128 * 1024)});
        assert!(shared
            .livefx
            .command(&shared, message, 0, None)
            .unwrap_err()
            .contains("128 KiB"));
        let mut message = register(0.5);
        message["context"] = json!({"payload":"a".repeat(128 * 1024)});
        assert!(shared
            .livefx
            .command(&shared, message, 0, None)
            .unwrap_err()
            .contains("128 KiB"));
        assert!(shared.livefx.worker.lock().unwrap().is_none());
    }
    #[test]
    fn js_failure_is_a_reported_dry_bypass_without_failing_native_audio() {
        let shared = Arc::new(Shared::new(48_000, 1));
        let mut message = register(0.5);
        message["source"] = json!("process() { throw new Error('liveFx test failure'); }");
        shared.livefx.command(&shared, message, 0, None).unwrap();
        wait(|| shared.livefx.status().active == 1);
        let mut stage = Renderer::new();
        let pcm = pump(&mut stage, &shared, 0, 32);
        assert_eq!(pcm.last().copied(), Some([0.5, -0.25]));
        let status = shared.livefx.status();
        assert!(status.error.unwrap().contains("liveFx test failure"));
        assert!(!shared.failed.load(Ordering::Acquire));
        shared.livefx.reset();
        shared.livefx.wait_reset().unwrap();
        assert!(shared.livefx.status().error.is_none());
    }
    #[test]
    fn clear_followed_immediately_by_register_keeps_the_new_generation() {
        let shared = Arc::new(Shared::new(48_000, 1));
        for _ in 0..48 {
            shared
                .livefx
                .command(&shared, register(0.1), 0, None)
                .unwrap();
            shared.livefx.reset();
            let reset = shared.livefx.transport.reset.load(Ordering::Acquire);
            shared
                .livefx
                .command(&shared, register(0.25), 0, None)
                .unwrap();
            let start = Instant::now();
            loop {
                let status = shared.livefx.status();
                if shared.livefx.transport.reset_ack.load(Ordering::Acquire) >= reset
                    && status.queued_commands == 0
                    && status.active == 1
                {
                    break;
                }
                assert!(
                    start.elapsed() < Duration::from_secs(3),
                    "clear/new-register reset={reset} ack={} enabled={} status={status:?}",
                    shared.livefx.transport.reset_ack.load(Ordering::Acquire),
                    shared.livefx.transport.enabled.load(Ordering::Acquire)
                );
                thread::sleep(Duration::from_millis(1));
            }
        }
        let mut stage = Renderer::new();
        let pcm = pump(&mut stage, &shared, 0, 24);
        assert_eq!(pcm.last().copied(), Some([0.125, -0.0625]));
    }
    #[test]
    fn stop_drains_completed_wet_without_a_muted_to_dry_burst() {
        for gain in [0., 0.5] {
            let transport = Transport::new();
            transport.active.store(1, Ordering::Release);
            transport.set_enabled(true);
            let mut stage = Renderer::new();
            for frame in 0..4096u64 {
                let value = ((frame as f32) * 0.01).sin() * 0.4;
                stage.process([value, -value], frame, 48_000, &transport);
                service(&transport, gain);
            }
            stage.begin_stop();
            transport.invalidate(true);
            let inputs = transport.input.len();
            for frame in 4096..4576u64 {
                let value = ((frame as f32) * 0.01).sin() * 0.4;
                let wet = stage.process([value, -value], frame, 48_000, &transport);
                let expected = (((frame - LEAD as u64) as f32) * 0.01).sin() * 0.4 * gain;
                assert!(
                    (wet[0] - expected).abs() < 1e-7,
                    "frame {frame}: {} vs {expected}",
                    wet[0]
                );
            }
            assert_eq!(transport.input.len(), inputs);
            stage.end_stop();
            assert_eq!(stage.process([0.; 2], 4576, 48_000, &transport), [0.; 2]);
            assert!(stage.stop_hold.is_none());
        }
    }
    #[test]
    fn terminal_removal_and_explicit_reset_start_from_the_delivered_wet_value() {
        let transport = Transport::new();
        transport.active.store(1, Ordering::Release);
        transport.set_enabled(true);
        let mut stage = Renderer::new();
        for frame in 0..4096u64 {
            stage.process([0.5, -0.25], frame, 48_000, &transport);
            service(&transport, 0.);
        }
        assert_eq!(stage.last, [0.; 2]);
        // Discard completed future wet blocks and insert an explicit terminal.
        while transport.output.pop().is_some() {}
        transport.active.store(0, Ordering::Release);
        transport
            .output
            .push(Block {
                generation: stage.generation,
                frame: 2048,
                terminal: true,
                pcm: [0.5; BLOCK * 2],
            })
            .unwrap_or_else(|_| panic!());
        assert_eq!(
            stage.process([0.5, -0.25], 4096, 48_000, &transport),
            [0.; 2]
        );
        for frame in 4097..4608u64 {
            stage.process([0.5, -0.25], frame, 48_000, &transport);
        }
        assert_eq!(stage.last, [0.5, -0.25]);
        transport.active.store(1, Ordering::Release);
        transport.set_enabled(true);
        for frame in 4608..8704u64 {
            stage.process([0.5, -0.25], frame, 48_000, &transport);
            service(&transport, 0.);
        }
        assert_eq!(stage.last, [0.; 2]);
        transport.invalidate(false);
        assert_eq!(
            stage.process([0.5, -0.25], 8704, 48_000, &transport),
            [0.; 2]
        );
    }
    #[test]
    fn first_future_activation_uses_published_callback_clock_without_early_vm() {
        let shared = Arc::new(Shared::new(48_000, 1));
        shared.frame.store(512, Ordering::Release);
        shared
            .livefx
            .command(&shared, register(0.5), 600, None)
            .unwrap();
        thread::sleep(Duration::from_millis(5));
        assert_eq!(shared.livefx.status().active, 0);
        assert!(!shared.livefx.status().initialized);
        // The callback only publishes at buffer boundaries. This first activation
        // may be later than a 128-frame block, but may never precede frame600.
        shared.frame.store(1024, Ordering::Release);
        wait(|| shared.livefx.status().active == 1);
        let mut stage = Renderer::new();
        assert_eq!(
            stage.process([0.5, -0.25], 1024, 48_000, &shared.livefx.transport),
            [0.5, -0.25]
        );
        assert_eq!(stage.start, 1024);
    }
    #[test]
    fn delivered_audio_is_post_c_fx_then_js_then_master_for_recording_and_monitor() {
        use super::super::{media, monitor};
        use super::super::{
            Action, Command as NativeCommand, CommandAction, FxMessage, Renderer as NativeRenderer,
        };
        let shared = Arc::new(Shared::new(48_000, 1));
        shared
            .load_media(
                media::Kind::Sample,
                "constant".into(),
                48_000.,
                vec![vec![0.4; 64]],
            )
            .unwrap();
        let options =
            serde_json::from_value(json!({"loop":true,"pan":-1.,"fadeIn":0.,"fadeOut":0.}))
                .unwrap();
        shared
            .enqueue(
                1,
                &[
                    NativeCommand {
                        frame: 0,
                        performance: None,
                        owner: None,
                        action: CommandAction::SamplePlay {
                            name: "constant".into(),
                            voice: 1,
                            options,
                        },
                    },
                    NativeCommand {
                        frame: 0,
                        performance: None,
                        owner: None,
                        action: CommandAction::Mix {
                            fm_left: 1.,
                            fm_right: 1.,
                            psg_left: 1.,
                            psg_right: 1.,
                            master: 0.5,
                        },
                    },
                ],
            )
            .unwrap();
        let fx: FxMessage = serde_json::from_value(json!({"op":"chain","children":[{"type":"gain","slot":0}],"units":[{"type":"gain","slot":0,"values":{"gain":0.5}}],"beatSeconds":0.5})).unwrap();
        let graph = shared
            .fx
            .lock()
            .unwrap()
            .command(fx, 48_000)
            .unwrap()
            .unwrap();
        shared.enqueue_special(Action::Fx(graph)).unwrap();
        let mut message = register(0.5);
        message["source"] = json!("process(input,output) { for(let c=0;c<2;c++) for(let i=0;i<input[c].length;i++) output[c][i]=input[c][i]*input[c][i]; }");
        shared.livefx.command(&shared, message, 0, None).unwrap();
        wait(|| shared.livefx.status().active == 1);
        let mut native = NativeRenderer::new(48_000).unwrap();
        shared
            .monitor
            .read(monitor::Mode::Waveform, 0, 1, 0)
            .unwrap();
        let mut last = [0.; 2];
        for _ in 0..40 {
            native.prepare(&shared);
            for _ in 0..BLOCK {
                last = native.sample(&shared);
            }
            shared.frame.store(native.frame, Ordering::Release);
            thread::sleep(Duration::from_millis(3));
        }
        assert!(
            (last[0] - 0.02).abs() < 1e-7,
            "post-CFX-square-master output was {last:?}"
        );
        assert_eq!(last[1], 0.);
        // Owner invalidation must not reset/reprime the mixed bus or unrelated FX.
        let generation = shared.livefx.transport.generation.load(Ordering::Acquire);
        shared.performance("start", 44, None).unwrap();
        shared.performance("cancel-owner", 44, Some(1)).unwrap();
        native.prepare(&shared);
        assert_eq!(
            shared.livefx.transport.generation.load(Ordering::Acquire),
            generation
        );
        shared.record_start(0.01).unwrap();
        for _ in 0..8 {
            native.prepare(&shared);
            for _ in 0..BLOCK {
                last = native.sample(&shared);
            }
            shared.frame.store(native.frame, Ordering::Release);
            thread::sleep(Duration::from_millis(3));
        }
        let recording = shared.record_stop().unwrap();
        assert_eq!(recording.frames, 480);
        assert!(recording.channels[0]
            .iter()
            .all(|sample| (*sample - 0.02).abs() < 1e-7));
        assert!(recording.channels[1].iter().all(|sample| *sample == 0.));
        let waveform = shared
            .monitor
            .read(monitor::Mode::Waveform, 0, 1, native.frame)
            .unwrap()
            .unwrap();
        assert!(waveform
            .samples
            .unwrap()
            .iter()
            .rev()
            .take(1024)
            .all(|sample| (*sample - 0.02).abs() < 1e-7));
        // The first Stop sample must continue the delivered wet signal. The
        // fixed fade can consume only already-completed audio, never wait.
        shared.epoch.fetch_add(1, Ordering::AcqRel);
        native.prepare(&shared);
        let first_stop = native.sample(&shared);
        assert!((first_stop[0] - last[0]).abs() < 1e-7);
        for _ in 1..480 {
            native.sample(&shared);
        }
        assert_eq!(native.sample(&shared), [0.; 2]);
        assert_eq!(shared.stopped_epoch.load(Ordering::Acquire), 1);
        shared.livefx.wait_reset().unwrap();
        assert!(!shared.livefx.status().initialized);
    }
    #[test]
    fn pure_js_generator_plays_on_idle_core_and_global_stop_still_fades_to_silence() {
        use super::super::Renderer as NativeRenderer;
        let shared = Arc::new(Shared::new(48_000, 1));
        let mut native = NativeRenderer::new(48_000).unwrap();
        native.prepare(&shared);
        for _ in 0..512 {
            assert_eq!(native.sample(&shared), [0.; 2]);
        }
        let mut message = register(0.5);
        message["source"] =
            json!("process(input,output) { output[0].fill(0.25); output[1].fill(-0.125); }");
        shared.livefx.command(&shared, message, 0, None).unwrap();
        wait(|| shared.livefx.status().active == 1);
        let mut last = [0.; 2];
        for _ in 0..32 {
            native.prepare(&shared);
            for _ in 0..BLOCK {
                last = native.sample(&shared);
            }
            shared.frame.store(native.frame, Ordering::Release);
            thread::sleep(Duration::from_millis(3));
        }
        assert!(
            native.idle,
            "No chip/sample/note event was needed to wake the core"
        );
        assert_eq!(last, [0.25, -0.125]);
        shared
            .enqueue(
                1,
                &[super::super::Command {
                    frame: 0,
                    performance: None,
                    owner: None,
                    action: super::super::CommandAction::PsgWrite { value: 0x9f },
                }],
            )
            .unwrap();
        native.prepare(&shared);
        assert_eq!(
            native.sample(&shared),
            last,
            "Introducing chip input must not fade an existing JS generator to zero"
        );
        shared.epoch.fetch_add(1, Ordering::AcqRel);
        native.prepare(&shared);
        assert_eq!(native.sample(&shared), last);
        let mut previous = last;
        for _ in 1..480 {
            let sample = native.sample(&shared);
            assert!(sample[0] <= previous[0] && sample[1] >= previous[1]);
            previous = sample;
        }
        assert_eq!(native.sample(&shared), [0.; 2]);
        assert_eq!(shared.stopped_epoch.load(Ordering::Acquire), 1);
        shared.livefx.wait_reset().unwrap();
        assert!(!shared.livefx.status().initialized);
    }
}
