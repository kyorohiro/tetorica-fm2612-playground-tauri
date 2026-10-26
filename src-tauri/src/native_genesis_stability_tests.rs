//! Offline lifecycle and render stress. Never creates CPAL/OS output streams.
//! Times describe this test build and host; they are measurements, not CI gates.
use super::*;
use serde_json::{json, Value};
use std::{any::Any, panic::AssertUnwindSafe, path::PathBuf};

const MODES: [&str; 23] = [
    "genesis",
    "mega-cd",
    "32x",
    "genesis-full-nuked",
    "genesis-nuked",
    "dx7",
    "nes",
    "gameboy",
    "snes",
    "pc-engine",
    "wonderswan",
    "pc98",
    "pc98-26",
    "neogeo",
    "neogeo-b",
    "x68000",
    "master-system",
    "game-gear",
    "msx",
    "msx-audio",
    "ym3526",
    "ym3812",
    "ymf262",
];
const RATE: u32 = 48000;
static NEXT_MESSAGE: AtomicU64 = AtomicU64::new(1);
const BLOCKS: [usize; 8] = [1, 7, 31, 64, 127, 128, 257, 511];

fn command(frame: u64, mut value: Value) -> Command {
    value["frame"] = json!(frame);
    serde_json::from_value(value).unwrap()
}
fn message(kind: &str, frame: u64, mut value: Value) -> Command {
    value["id"] = json!(NEXT_MESSAGE.fetch_add(1, Ordering::Relaxed));
    command(frame, json!({"type":kind,"message":value}))
}
fn owned(mut c: Command, performance: u64) -> Command {
    c.performance = Some(performance);
    c.owner = Some(1);
    c
}
fn write(register: u16, value: u8) -> Value {
    json!({"method":"write","register":register,"value":value})
}
fn batch(chip: &str, channel: u8, ops: Vec<Value>) -> Value {
    json!({"chip":chip,"method":"batch","channel":channel,"voice":1,"action":"on","commands":ops})
}
fn fm_ops() -> Vec<Value> {
    let mut out = Vec::new();
    for slot in [0, 4, 8, 12] {
        for (register, value) in [
            (0x30 + slot, 1),
            (0x40 + slot, if slot == 12 { 40 } else { 127 }),
            (0x50 + slot, 31),
            (0x60 + slot, 0),
            (0x70 + slot, 0),
            (0x80 + slot, 15),
        ] {
            out.push(write(register, value));
        }
    }
    for (register, value) in [
        (0xb0, 7),
        (0xb4, 0xc0),
        (0xa4, 0x22),
        (0xa0, 0x69),
        (0x28, 0xf0),
    ] {
        out.push(write(register, value));
    }
    out
}
fn opl_ops() -> Vec<Value> {
    let mut ops = Vec::new();
    for (slot, tl) in [(0, 63), (3, 0)] {
        for (reg, value) in [
            (0x20 + slot, 0x21),
            (0x40 + slot, tl),
            (0x60 + slot, 0xf0),
            (0x80 + slot, 15),
        ] {
            ops.push(write(reg, value));
        }
    }
    for (reg, value) in [(0xa0, 0x98), (0xc0, 0x31), (0xb0, 0x31)] {
        ops.push(write(reg, value));
    }
    ops
}
fn opll() -> Value {
    batch(
        "ym2413",
        0,
        vec![write(0x30, 0x15), write(0x10, 0x90), write(0x20, 0x17)],
    )
}
fn scene(mode: &str, frame: u64) -> Vec<Command> {
    let mut out = Vec::new();
    match mode {
        "genesis" | "mega-cd" | "32x" | "genesis-full-nuked" | "genesis-nuked" => {
            for op in fm_ops() {
                out.push(command(frame,json!({"type":"write","port":0,"register":op["register"],"value":op["value"]})));
            }
            for value in [0x85,0x10,0x9a] { out.push(command(frame,json!({"type":"psg-write","value":value}))); }
            if matches!(mode,"mega-cd"|"32x"|"genesis-full-nuked") {
                out.push(command(frame,json!({"type":"rf5c164-load-memory","offset":0,"data":[128,208,224,208,128,80,96,80,255]})));
                for (register,value) in [(7,0xc0),(0,80),(1,0xff),(2,0),(3,8),(4,0),(5,0),(6,0)] {
                    out.push(command(frame,json!({"type":"rf5c164-write","register":register,"value":value})));
                }
                out.push(command(frame,json!({"type":"rf5c164-key-on","channel":0})));
            }
            if matches!(mode,"32x"|"genesis-full-nuked") {
                for (register,value) in [(0,5),(1,4095),(2,2147),(3,2147)] {
                    out.push(message("pwm",frame,json!({"method":"write","register":register,"value":value})));
                }
            }
        }
        "dx7" => {
            let mut patch = dx7::init_voice();
            patch[121] = 78;
            out.push(message("dx7",frame,json!({"method":"load-bank","voices":[patch.as_slice(),dx7::init_voice().as_slice()]})));
            out.push(message("dx7",frame,json!({"method":"note-on","note":60,"velocity":100,"voice":1})));
        }
        "nes" => out.push(message("nes",frame,json!({"method":"batch","channel":0,"voice":1,"action":"on","commands":[
            {"method":"write","address":0x4000,"value":0xbf},{"method":"write","address":0x4001,"value":8},
            {"method":"write","address":0x4002,"value":253},{"method":"enable","channel":0,"enabled":true},
            {"method":"write","address":0x4003,"value":8}]}))),
        "gameboy" => {
            for (address,value) in [(22,128),(20,0x77),(21,0xff)] {
                out.push(message("gameboy",frame,json!({"method":"write","address":address,"value":value})));
            }
            out.push(message("gameboy",frame,json!({"method":"batch","channel":0,"voice":1,"action":"on","commands":[
                {"method":"write","address":1,"value":128},{"method":"write","address":2,"value":160},
                {"method":"write","address":3,"value":34},{"method":"write","address":4,"value":135}]})));
        }
        "snes" => {
            out.push(message("snes",frame,json!({"method":"load-memory","address":0x400,"data":[0xb3,0x12,0x34,0x56,0x78,0x9a,0xbc,0xde,0xf0]})));
            out.push(message("snes",frame,json!({"method":"load-memory","address":0,"data":[0,4,0,4]})));
            for (register,value) in [(0x6c,0x20),(0x0c,127),(0x1c,127),(0x5d,0),(0x5c,255),(0x2c,0),(0x3c,0),(0x2d,0),(0x3d,0),(0x4d,0),
                (0,100),(1,100),(2,0),(3,16),(4,0),(5,0),(6,0),(7,127)] {
                out.push(message("snes",frame,write(register,value)));
            }
            out.push(message("snes",frame,json!({"method":"batch","channel":0,"voice":1,"action":"on","commands":[{"method":"key-on","channel":0}]})));
        }
        "pc-engine" => {
            out.push(message("huc6280",frame,write(1,255)));
            out.push(message("huc6280",frame,json!({"method":"waveform","channel":0,"samples":(0..32).collect::<Vec<_>>()})));
            out.push(message("huc6280",frame,json!({"method":"write","channel":0,"register":5,"value":255})));
            out.push(message("huc6280",frame,json!({"method":"batch","channel":0,"voice":1,"action":"on","commands":[
                {"method":"write","register":2,"value":254},{"method":"write","register":3,"value":0},{"method":"write","register":4,"value":152}]})));
        }
        "wonderswan" => {
            out.push(message("wonderswan",frame,json!({"method":"write","address":0x91,"value":8})));
            let mut commands: Vec<_> = (0..16).map(|i|json!({"method":"memory","address":i,"value":((i*2)&15)|(((i*2+1)&15)<<4),"channel":0})).collect();
            commands.extend([
                json!({"method":"write","address":0x80,"value":0x61}),
                json!({"method":"write","address":0x81,"value":7}),
                json!({"method":"write","address":0x88,"value":0xf9}),
                json!({"method":"write","address":0x90,"value":1,"controlMask":1}),
            ]);
            out.push(message("wonderswan",frame,json!({"method":"batch","channel":0,"voice":1,"action":"on","commands":commands})));
        }
        "pc98" | "pc98-26" | "neogeo" | "neogeo-b" => {
            let kind = match mode { "pc98"=>"ym2608","pc98-26"=>"ym2203",_=>"ym2610b" };
            for mut op in fm_ops() { op["port"]=json!(0); out.push(message(kind,frame,op)); }
            let base = if mode=="pc98-26" {3} else {6};
            let mut ops=vec![write(0,0x80),write(1,1),write(7,0x3e),write(8,10)];
            for op in &mut ops { op["port"]=json!(0); }
            out.push(message(kind,frame,json!({"method":"batch","channel":base,"voice":2,"action":"on","commands":ops})));
        }
        "x68000" => {
            let mut ops=vec![write(0x20,0xc7),write(0x28,0x4a),write(0x30,0)];
            for slot in [0,8,16,24] { for (base,value) in [(0x40,1),(0x60,0x20),(0x80,31),(0xa0,0),(0xc0,0),(0xe0,15)] {ops.push(write(base+slot,value));} }
            ops.push(write(8,0x78));
            out.push(message("x68000",frame,batch("ym2151",0,ops)));
        }
        "master-system" | "game-gear" => {
            out.push(message("master-system",frame,batch("segapsg",0,vec![json!({"method":"write","value":0x85}),json!({"method":"write","value":0x10,"psgRegister":0}),json!({"method":"write","value":0x95})])));
            if mode=="master-system" {out.push(message("master-system",frame,opll()));}
            else {out.push(message("master-system",frame,json!({"chip":"segapsg","method":"stereo","mask":0xff})));}
        }
        "msx" | "msx-audio" => {
            out.push(message("msx",frame,opll()));
            out.push(message("msx",frame,batch("ay8910",0,vec![write(0,127),write(1,1),write(7,62),write(8,9)])));
            if mode=="msx-audio" {out.push(message("opl",frame,batch("y8950",0,opl_ops())));}
        }
        "ym3526" | "ym3812" | "ymf262" => {
            if mode=="ymf262" {out.push(message("opl",frame,json!({"chip":mode,"method":"write","register":0x105,"value":1})));}
            out.push(message("opl",frame,batch(mode,0,opl_ops())));
        }
        _ => unreachable!("Uncovered Native mode {mode}"),
    }
    out
}
fn setup(mode: &str) -> (Shared, Renderer) {
    let (cd, dx, nes, console) = NativeGenesis::mode_config(Some(mode)).unwrap();
    let s = Shared::new_modes(RATE, 1, cd, dx, nes, console);
    let r = Renderer::new_modes(RATE, cd, dx, nes, console).unwrap();
    r.publish_primary(&s);
    (s, r)
}
struct Audio {
    frames: u64,
    callbacks: u64,
    nanos: u128,
    hash: u64,
    peak: f32,
    min: f32,
    max: f32,
}
impl Default for Audio {
    fn default() -> Self {
        Self {
            frames: 0,
            callbacks: 0,
            nanos: 0,
            hash: 0,
            peak: 0.,
            min: f32::INFINITY,
            max: f32::NEG_INFINITY,
        }
    }
}
impl Audio {
    fn render(&mut self, s: &Shared, r: &mut Renderer, frames: usize, variable: bool) {
        let mut left = frames;
        while left > 0 {
            let n = left.min(if variable {
                BLOCKS[self.callbacks as usize % BLOCKS.len()]
            } else {
                128
            });
            let t = Instant::now();
            r.prepare(s);
            for index in 0..n {
                let pcm = r.sample(s);
                assert!(
                    pcm.iter().all(|v| v.is_finite() && v.abs() <= 1.0),
                    "nonfinite or unbounded PCM"
                );
                for sample in pcm {
                    self.hash =
                        (self.hash ^ sample.to_bits() as u64).wrapping_mul(1_099_511_628_211);
                    self.peak = self.peak.max(sample.abs());
                    // Ignore initial wake/ramp transitions when checking that
                    // a scene produces a signal rather than only idle DC.
                    if self.frames + index as u64 >= 512 {
                        self.min = self.min.min(sample);
                        self.max = self.max.max(sample);
                    }
                }
            }
            s.frame.store(r.frame, Ordering::Release);
            r.publish_primary(s);
            self.nanos += t.elapsed().as_nanos();
            self.frames += n as u64;
            self.callbacks += 1;
            left -= n;
        }
    }
    fn audible(&self) -> bool {
        self.peak > 0.00001 && self.max - self.min > 0.00001
    }
}
fn pending_metadata(s: &Shared) -> usize {
    let frame = s.frame.load(Ordering::Acquire);
    let mut states = Vec::new();
    macro_rules! snapshot {
        ($field:ident) => {
            if let Some(chip) = &s.$field {
                states.push(serde_json::to_value(chip.status(frame).unwrap()).unwrap());
            }
        };
    }
    snapshot!(dx7);
    snapshot!(nes);
    snapshot!(gameboy);
    snapshot!(snes);
    snapshot!(huc6280);
    snapshot!(wonderswan);
    snapshot!(ym2608);
    snapshot!(ym2203);
    snapshot!(ym2610b);
    snapshot!(x68000);
    snapshot!(master_system);
    snapshot!(msx);
    snapshot!(opl);
    snapshot!(pwm);
    fn count(v: &Value) -> usize {
        match v {
            Value::Object(map) => map
                .iter()
                .map(|(k, v)| {
                    if k == "pendingCommands" {
                        v.as_array().unwrap().len()
                    } else {
                        count(v)
                    }
                })
                .sum(),
            Value::Array(a) => a.iter().map(count).sum(),
            _ => 0,
        }
    }
    states.iter().map(count).sum()
}
// Observe the exact admitted allocations, without becoming a lifetime guard.
// Queue order/sequence is restored before the offline callback begins.
fn accepted_guards(s: &Shared) -> Vec<std::sync::Weak<dyn Any + Send + Sync>> {
    fn weak<T: Any + Send + Sync>(value: &Arc<T>) -> std::sync::Weak<dyn Any + Send + Sync> {
        let erased: Arc<dyn Any + Send + Sync> = value.clone();
        Arc::downgrade(&erased)
    }
    let mut events = Vec::with_capacity(s.queue.len());
    let mut guards = Vec::new();
    while let Some(event) = s.queue.pop() {
        let guard = match &*event.action.borrow() {
            Action::Dx7(v) => Some(weak(v)),
            Action::Nes(v) => Some(weak(v)),
            Action::Gameboy(v) => Some(weak(v)),
            Action::Snes(v) => Some(weak(v)),
            Action::Huc6280(v) => Some(weak(v)),
            Action::WonderSwan(v) => Some(weak(v)),
            Action::Ym2608(v) => Some(weak(v)),
            Action::Ym2203(v) => Some(weak(v)),
            Action::Ym2610b(v) => Some(weak(v)),
            Action::X68000(v) => Some(weak(v)),
            Action::MasterSystem(v) => Some(weak(v)),
            Action::Msx(v) => Some(weak(v)),
            Action::Opl(v) => Some(weak(v)),
            Action::Pwm(v) => Some(weak(v)),
            Action::RfLoad(v) => Some(weak(v)),
            Action::SamplePlay(v) => Some(weak(v)),
            _ => None,
        };
        if let Some(guard) = guard {
            guards.push(guard);
        }
        events.push(event);
    }
    for event in events {
        s.queue.push(event).unwrap();
    }
    guards
}
fn drained(s: &Shared, r: &Renderer) {
    s.enqueue(1, &[]).unwrap(); // All prepared objects and bank guards retire on control.
    assert_eq!(s.pending.load(Ordering::Acquire), 0);
    assert!(s.queue.is_empty() && r.heap.is_empty());
    assert_eq!(pending_metadata(s), 0);
    assert!(s.retired.is_empty() && s.retired_stops.is_empty() && s.retired_rf.is_empty());
    assert!(s.rf_memory.lock().unwrap().is_empty());
    let store = s.media.lock().unwrap();
    assert!(store.reservations.is_empty() && store.stops.is_empty());
    assert_eq!(s.overflows.load(Ordering::Acquire), 0);
    assert!(!s.failed.load(Ordering::Acquire));
}
fn error_text(e: Box<dyn Any + Send>) -> String {
    if let Some(v) = e.downcast_ref::<String>() {
        v.clone()
    } else if let Some(v) = e.downcast_ref::<&str>() {
        v.to_string()
    } else {
        "Unknown panic".into()
    }
}
fn save(report: &Value) {
    if let Some(path) = std::env::var_os("TETORICA_NATIVE_STABILITY_REPORT") {
        let path = PathBuf::from(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, serde_json::to_vec_pretty(report).unwrap()).unwrap();
    }
}
fn run_mode(mode: &str, rounds: usize, active_frames: usize) -> Value {
    let mut construct = 0.;
    let mut drop_time = 0.;
    let mut render_nanos = 0u128;
    let mut frames = 0u64;
    let mut callbacks = 0;
    let mut events = 0;
    let mut max_pending = 0;
    let mut max_metadata = 0;
    let mut reclaimed_banks = 0;
    let mut reclaimed_commands = 0;
    let mut stopped_events = 0;
    let mut peak = 0f32;
    // A separate fixed-block render checks that callback segmentation is immaterial.
    let mut hashes = Vec::new();
    let mut comparison = Vec::new();
    for variable in [false, true] {
        let (s, mut r) = setup(mode);
        s.enqueue(1, &scene(mode, 64)).unwrap();
        let mut audio = Audio::default();
        audio.render(&s, &mut r, 4096, variable);
        assert!(
            audio.audible(),
            "{mode}: setup failed to produce a varying signal"
        );
        hashes.push(audio.hash);
        comparison.push(json!({"variableBlocks":variable,"frames":audio.frames,"callbackBlocks":audio.callbacks,"renderSeconds":audio.nanos as f64/1e9}));
    }
    assert_eq!(
        hashes[0], hashes[1],
        "{mode}: callback segmentation changed PCM"
    );
    for round in 0..rounds {
        let t = Instant::now();
        let (s, mut r) = setup(mode);
        construct += t.elapsed().as_secs_f64();
        let heap_capacity = r.heap.capacity();
        for restart in 0..2 {
            let performance = (restart + 1) as u64;
            s.performance("start", performance, None).unwrap();
            let start = r.frame + 32;
            let future = start + active_frames as u64 + RATE as u64 * 60;
            let mut commands: Vec<_> = scene(mode, start)
                .into_iter()
                .map(|c| owned(c, performance))
                .collect();
            // Enough queued work to exercise cancellation and reference retirement
            // without a long test depending on an unbounded event backlog.
            for burst in 0..8 {
                commands.extend(
                    scene(mode, future + burst * 64)
                        .into_iter()
                        .map(|c| owned(c, performance)),
                );
            }
            let bank_name = format!("stability-{round}-{restart}");
            s.load_media(
                media::Kind::Sample,
                bank_name.clone(),
                RATE as f64,
                vec![vec![0.2; 128]],
            )
            .unwrap();
            let bank = s
                .media
                .lock()
                .unwrap()
                .get(media::Kind::Sample, &bank_name)
                .unwrap();
            let weak = Arc::downgrade(&bank);
            drop(bank);
            commands.push(owned(command(future,json!({"type":"sample-play","name":bank_name,"voice":900,"options":{"loop":true}})),performance));
            events += commands.len();
            s.enqueue(1, &commands).unwrap();
            let guards = accepted_guards(&s);
            assert!(!guards.is_empty());
            s.remove_media(media::Kind::Sample, &bank_name).unwrap();
            assert!(weak.upgrade().is_some());
            max_pending = max_pending.max(s.pending.load(Ordering::Acquire));
            assert!(max_pending <= MAX_BATCH);
            max_metadata = max_metadata.max(pending_metadata(&s));
            assert!(max_metadata <= MAX_BATCH);
            let mut audio = Audio::default();
            audio.render(&s, &mut r, active_frames, true);
            assert!(
                audio.audible(),
                "{mode}: round {round} restart {restart} did not sound"
            );
            peak = peak.max(audio.peak);
            s.performance("cancel-owner", performance, Some(1)).unwrap();
            audio.render(&s, &mut r, 64, true);
            drained(&s, &r);
            assert!(
                weak.upgrade().is_none(),
                "{mode}: cancelled bank still retained after control reclaim"
            );
            reclaimed_banks += 1;
            assert!(
                guards.iter().all(|guard| guard.upgrade().is_none()),
                "{mode}: accepted command guard retained after cancellation/reclaim"
            );
            reclaimed_commands += guards.len();
            // Queue a different, still-valid owner after cancellation. Global
            // Stop must discard that work too, rather than passing on an empty queue.
            let stop_commands: Vec<_> = scene(mode, r.frame + RATE as u64 * 60)
                .into_iter()
                .map(|mut c| {
                    c.performance = Some(performance);
                    c.owner = Some(2);
                    c
                })
                .collect();
            events += stop_commands.len();
            stopped_events += stop_commands.len();
            s.enqueue(1, &stop_commands).unwrap();
            let stop_guards = accepted_guards(&s);
            assert_eq!(s.pending.load(Ordering::Acquire), stop_commands.len());
            max_pending = max_pending.max(stop_commands.len());
            max_metadata = max_metadata.max(pending_metadata(&s));
            // Same epoch/fade barrier used by IPC Stop, driven offline by this renderer.
            s.active_performance.store(0, Ordering::Release);
            let epoch = s.epoch.fetch_add(1, Ordering::AcqRel) + 1;
            let fade_frames = r.fade_frames as usize + 64;
            audio.render(&s, &mut r, fade_frames, true);
            let mut silence = Audio::default();
            silence.render(&s, &mut r, 256, true);
            assert_eq!(silence.peak, 0., "{mode}: Stop left output active");
            assert_eq!(s.stopped_epoch.load(Ordering::Acquire), epoch);
            assert!(r.idle);
            if let Some(chip) = &s.dx7 {
                let bank = chip.status(r.frame).unwrap();
                assert_eq!(bank.voices.len(), 2);
                assert_eq!(
                    bank.voices[0][121], 78,
                    "Stop changed the authored DX7 bank"
                );
            }
            drained(&s, &r);
            assert!(
                stop_guards.iter().all(|guard| guard.upgrade().is_none()),
                "{mode}: Stop retained future command guards"
            );
            reclaimed_commands += stop_guards.len();
            assert_eq!(r.heap.capacity(), heap_capacity);
            render_nanos += audio.nanos + silence.nanos;
            frames += audio.frames + silence.frames;
            callbacks += audio.callbacks + silence.callbacks;
        }
        let t = Instant::now();
        drop(r);
        drop(s);
        drop_time += t.elapsed().as_secs_f64();
    }
    let render_seconds = render_nanos as f64 / 1e9;
    let simulated_seconds = frames as f64 / RATE as f64;
    json!({"mode":mode,"passed":true,"rounds":rounds,"constructorCount":rounds+2,
        "lifecycleConstructorCount":rounds,"segmentationConstructorCount":2,
        "runStopTransitions":rounds*2,"ownerCancellations":rounds*2,"events":events,
        "renderedFrames":frames,"callbackBlocks":callbacks,"simulatedSeconds":simulated_seconds,
        "renderSeconds":render_seconds,"realtimeFactor":simulated_seconds/render_seconds.max(1e-12),
        "lifecycleConstructSeconds":construct,"lifecycleDropSeconds":drop_time,"peak":peak,"segmentationExact":true,
        "maximumPendingEvents":max_pending,"maximumPendingMetadata":max_metadata,"reclaimedBankGuards":reclaimed_banks,
        "reclaimedCommandGuards":reclaimed_commands,"retainedCommandGuardsAfterReclaim":0,"futureEventsDiscardedByStop":stopped_events,
        "callbackComparison":comparison,
        "pendingEventsAfterStop":0,"pendingMetadataAfterStop":0,"retainedTestBanksAfterReclaim":0,
        "heapCapacityStable":true})
}
#[test]
fn native_stability_all_modes_offline_lifecycle_and_load_report() {
    let long = std::env::var("TETORICA_NATIVE_STABILITY_PROFILE").as_deref() == Ok("long");
    let seconds = std::env::var("TETORICA_NATIVE_STABILITY_SECONDS")
        .ok()
        .map(|v| v.parse::<f64>().unwrap())
        .unwrap_or(if long { 30. } else { 0.125 });
    let rounds = std::env::var("TETORICA_NATIVE_STABILITY_ROUNDS")
        .ok()
        .map(|v| v.parse::<usize>().unwrap())
        .unwrap_or(if long { 3 } else { 2 });
    assert!(seconds.is_finite() && (0.09..=3600.).contains(&seconds));
    assert!((1..=1000).contains(&rounds));
    let mut report = json!({"schemaVersion":1,"profile":if long {"long"} else {"bounded"},"passed":false,
        "deviceOpened":false,"sampleRate":RATE,"rounds":rounds,"secondsPerRun":seconds,
        "timingBuild":if cfg!(debug_assertions) {"debug"} else {"release"},
        "limits":{"maximumBatch":MAX_BATCH,"queueCapacity":CAPACITY,"mediaBudgetBytes":media::BANK_BUDGET,
            "nonFinitePcm":0,"stoppedPeak":0,"pendingAfterStop":0,"testBankGuardsAfterReclaim":0,
            "absolutePerformanceGate":false},
        "notes":["Offline callback/control timings include assertion and snapshot overhead; no OS audio device or scheduling is measured.",
            "Bounded queue, metadata, and weak command/bank guards are verified; this is not a general heap or RSS leak proof."],"modes":[]});
    save(&report);
    let start = Instant::now();
    for mode in MODES {
        let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
            run_mode(mode, rounds, (seconds * RATE as f64).ceil() as usize)
        }));
        let entry = match result {
            Ok(v) => v,
            Err(e) => json!({"mode":mode,"passed":false,"error":error_text(e)}),
        };
        report["modes"].as_array_mut().unwrap().push(entry);
        save(&report);
    }
    report["passed"] = json!(report["modes"]
        .as_array()
        .unwrap()
        .iter()
        .all(|m| m["passed"] == true));
    let modes = report["modes"].as_array().unwrap();
    let sum_u64 = |key: &str| modes.iter().filter_map(|m| m[key].as_u64()).sum::<u64>();
    let sum_f64 = |key: &str| modes.iter().filter_map(|m| m[key].as_f64()).sum::<f64>();
    let totals = json!({"modes":modes.len(),"constructorCount":sum_u64("constructorCount"),
        "runStopTransitions":sum_u64("runStopTransitions"),"ownerCancellations":sum_u64("ownerCancellations"),
        "events":sum_u64("events"),"futureEventsDiscardedByStop":sum_u64("futureEventsDiscardedByStop"),"renderedFrames":sum_u64("renderedFrames"),
        "simulatedSeconds":sum_f64("simulatedSeconds"),"renderSeconds":sum_f64("renderSeconds"),
        "reclaimedBankGuards":sum_u64("reclaimedBankGuards"),"reclaimedCommandGuards":sum_u64("reclaimedCommandGuards")});
    report["totals"] = totals;
    report["wallSeconds"] = json!(start.elapsed().as_secs_f64());
    save(&report);
    println!(
        "NATIVE_STABILITY_JSON {}",
        serde_json::to_string(&report).unwrap()
    );
    assert_eq!(
        report["passed"], true,
        "Native stability failed; inspect per-mode JSON report"
    );
}
