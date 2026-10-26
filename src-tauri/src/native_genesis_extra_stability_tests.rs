//! Mixed additional racks through the actual Shared/Renderer, without CPAL.
//! Reservations are Native accounting units, not a heap/RSS measurement.
use super::*;
use serde_json::{json, Value};
use std::{any::Any, panic::AssertUnwindSafe, path::PathBuf};
const RATE: u32 = 48_000;
const GAIN: f32 = 0.06;
const BLOCKS: [usize; 8] = [1, 7, 31, 64, 127, 128, 257, 511];
static ID: AtomicU64 = AtomicU64::new(1);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Fm,
    Rf,
    Dx,
    Snes,
    Spc,
    Opna,
    Opnb,
    Ws,
}
impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Fm => "ym2612",
            Self::Rf => "rf5c164",
            Self::Dx => "dx7",
            Self::Snes => "snes",
            Self::Spc => "spc",
            Self::Opna => "ym2608",
            Self::Opnb => "ym2610",
            Self::Ws => "wonderswan",
        }
    }
    fn channel(self, index: usize) -> u8 {
        if self == Self::Opnb {
            [1, 2, 4][index]
        } else {
            index as u8
        }
    }
}
const ALL: [Kind; 8] = [
    Kind::Fm,
    Kind::Rf,
    Kind::Dx,
    Kind::Snes,
    Kind::Spc,
    Kind::Opna,
    Kind::Opnb,
    Kind::Ws,
];
const SCENES: [(&str, &[Kind]); 4] = [
    ("fm-pcm-dx7", &[Kind::Fm, Kind::Rf, Kind::Dx]),
    (
        "snes-spc-mixed",
        &[Kind::Fm, Kind::Rf, Kind::Dx, Kind::Snes, Kind::Spc],
    ),
    (
        "memory-heavy",
        &[
            Kind::Fm,
            Kind::Rf,
            Kind::Dx,
            Kind::Snes,
            Kind::Spc,
            Kind::Opna,
            Kind::Opnb,
        ],
    ),
    (
        "wonderswan-wave-mixed",
        &[Kind::Fm, Kind::Rf, Kind::Dx, Kind::Ws],
    ),
];
#[derive(Clone, Copy)]
struct Source {
    kind: Kind,
    id: u64,
}
fn create(s: &Shared, kind: Kind, id: u64) -> Source {
    match kind {
        Kind::Fm => {
            s.create_extra_ym2612(id, None).unwrap();
        }
        Kind::Rf => {
            s.create_extra_rf5c164(id, None).unwrap();
        }
        Kind::Dx => {
            s.create_extra_dx7(id).unwrap();
        }
        Kind::Snes => {
            s.create_extra_snes(id).unwrap();
        }
        Kind::Spc => {
            s.create_extra_spc(id).unwrap();
        }
        Kind::Opna => {
            s.create_extra_ym2608(id, None).unwrap();
        }
        Kind::Opnb => {
            s.create_extra_ym2610(id, None).unwrap();
        }
        Kind::Ws => {
            s.create_extra_wonderswan(id, None).unwrap();
        }
    }
    Source { kind, id }
}
fn cmd(source: Source, frame: u64, mut message: Value, owner: Option<u32>) -> Command {
    message["id"] = json!(ID.fetch_add(1, Ordering::Relaxed));
    if source.kind == Kind::Spc {
        if let Some(owner) = owner {
            message["sourceOwner"] = json!({"session":7,"owner":owner});
        }
    }
    let mut c:Command=serde_json::from_value(json!({"type":format!("extra-{}",source.kind.name()),"instance":source.id,"frame":frame,"message":message})).unwrap();
    if source.kind != Kind::Spc {
        c.performance = owner.map(|_| 7);
        c.owner = owner;
    }
    c
}
fn enqueue(s: &Shared, commands: Vec<Command>) {
    s.enqueue(1, &commands).unwrap();
}
fn wr(port: u8, register: u8, value: u8) -> Value {
    json!({"method":"write","port":port,"register":register,"value":value})
}
fn fm(ch: u8, opna: bool) -> Vec<Value> {
    let p = ch / 3;
    let n = ch % 3;
    let mut v = Vec::new();
    if opna {
        v.push(wr(0, 0x29, 0x80));
    }
    for slot in [0, 4, 8, 12] {
        for (base, value) in [
            (0x30, 1),
            (0x40, if slot == 12 { 48 } else { 127 }),
            (0x50, 31),
            (0x60, 0),
            (0x70, 0),
            (0x80, 15),
        ] {
            v.push(wr(p, base + slot + n, value));
        }
    }
    v.extend([
        wr(p, 0xb0 + n, 7),
        wr(p, 0xb4 + n, 0xc0),
        wr(p, 0xa4 + n, 0x22),
        wr(p, 0xa0 + n, 0x69 + n * 13),
        wr(0, 0x28, 0xf0 | n | if p == 1 { 4 } else { 0 }),
    ]);
    v
}
fn brr() -> Vec<u8> {
    vec![0xb3, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0]
}
fn dsp() -> Vec<u8> {
    let mut d = vec![0; 128];
    d[0x6c] = 0x20;
    d[0x0c] = 127;
    d[0x1c] = 127;
    d[0x5c] = 255;
    for ch in 0..3 {
        d[ch * 16] = 60;
        d[ch * 16 + 1] = 60;
        d[ch * 16 + 2] = 0;
        d[ch * 16 + 3] = 4 + ch as u8;
        d[ch * 16 + 7] = 127;
    }
    d
}
fn spc(heavy: bool) -> Vec<u8> {
    let mut b = vec![0; if heavy { 2 * 1024 * 1024 } else { 0x10200 }];
    let sig = b"SNES-SPC700 Sound File Data v0.30";
    b[..sig.len()].copy_from_slice(sig);
    b[0x21] = 0x1a;
    b[0x22] = 0x1a;
    b[0x23] = 0x1b;
    b[0x24] = 30;
    b[0x26] = 2;
    b[0x2b] = 0xef;
    // A real SPC700 loop updates voice volume while the S-DSP plays its BRR.
    b[0x300..0x30a].copy_from_slice(&[0x8f, 0, 0xf2, 0xe8, 0x20, 0xc4, 0xf3, 0xbc, 0x2f, 0xfb]);
    b[0x1101] = 0x20;
    b[0x1103] = 0x20;
    b[0x2100..0x2109].copy_from_slice(&brr());
    let mut d = dsp();
    d[0x5c] = 0;
    d[0x4c] = 1;
    d[0x5d] = 0x10;
    b[0x10100..0x10180].copy_from_slice(&d);
    b
}
fn resources(source: Source, heavy: bool, gain: f32) -> Vec<Command> {
    let mut m = vec![json!({"method":"mix","volume":gain,"pan":0})];
    match source.kind {
        Kind::Rf=>{let mut wave=(0..32).map(|i|{let x=(i as f64*std::f64::consts::TAU/32.).sin();(x.abs()*100.).round() as u8|if x>=0.{128}else{0}}).collect::<Vec<_>>();wave.push(255);m.push(json!({"method":"load-memory","offset":0,"data":wave}));},
        Kind::Dx=>m.push(json!({"method":"load-bank","voices":[dx7::init_voice().as_slice()]})),
        Kind::Snes=>m.push(json!({"method":"reset-state","dsp":dsp(),"uploads":[{"address":1024,"data":brr()},{"address":0,"data":[0,4,0,4]}]})),
        Kind::Spc=>{},
        Kind::Opna=>m.push(wr(0,0x29,0x80)),
        Kind::Opnb=>{},
        Kind::Ws=>m.push(json!({"method":"write","address":0x91,"value":8})),
        _=>{}
    }
    let mut commands: Vec<_> = m.into_iter().map(|m| cmd(source, 0, m, None)).collect();
    if source.kind == Kind::Spc {
        commands.push(Command {
            frame: 0,
            performance: None,
            owner: None,
            action: CommandAction::ExtraSpc {
                instance: source.id,
                message: extra_spc::Message {
                    id: Some(ID.fetch_add(1, Ordering::Relaxed)),
                    source_owner: None,
                    event_epoch: 0,
                    method: extra_spc::Method::LoadSpc { data: spc(heavy) },
                },
            },
        });
    }
    if source.kind == Kind::Opnb {
        for bank in 0..2 {
            commands.push(memory(
                source,
                bank,
                vec![0x11; if heavy { 2 * 1024 * 1024 } else { 64 }],
            ));
        }
    }
    commands
}
// Feed the same typed admission API without building millions of JSON Value
// cells just to represent a bounded byte buffer in this offline harness.
fn memory(source: Source, bank: u8, data: Vec<u8>) -> Command {
    Command {
        frame: 0,
        performance: None,
        owner: None,
        action: CommandAction::ExtraYm2610 {
            instance: source.id,
            message: extra_ym2610::Message {
                id: Some(ID.fetch_add(1, Ordering::Relaxed)),
                audition: false,
                group: None,
                method: extra_ym2610::Method::Memory {
                    rom_type: bank,
                    offset: 0,
                    data,
                },
            },
        },
    }
}
fn notes(source: Source, voices: usize, owners: [u32; 2]) -> Vec<Command> {
    if source.kind == Kind::Spc {
        return vec![cmd(source, 0, json!({"method":"play"}), Some(owners[1]))];
    }
    (0..voices).map(|i|{let ch=source.kind.channel(i);let owner=if i==0{None}else{Some(owners[i-1])};
        let m=match source.kind {
            Kind::Dx=>json!({"method":"note-on","note":([60,64,67][i]),"velocity":100,"voice":i+1,"audition":i==0}),
            Kind::Rf=>json!({"method":"batch","channel":ch,"voice":i+1,"action":"on","audition":i==0,"commands":[{"method":"set-channel","channel":ch,"options":{"start":0,"loopStart":0,"step":512+i*128,"volume":90,"pan":{"left":15,"right":15}}},{"method":"key-on","channel":ch}]}),
            Kind::Snes=>json!({"method":"batch","channel":ch,"voice":i+1,"action":"on","audition":i==0,"commands":[{"method":"key-on","channel":ch}]}),
            Kind::Ws=>{
                let mut commands:Vec<_>=(0..16).map(|n|json!({"method":"memory","address":ch as usize*16+n,"value":((n*2+i*3)&15)|(((n*2+1+i*3)&15)<<4),"channel":ch})).collect();
                let frequency=1737+u32::from(ch)*67;
                commands.extend([json!({"method":"write","address":0x80+ch*2,"value":frequency&255}),json!({"method":"write","address":0x81+ch*2,"value":frequency>>8}),json!({"method":"write","address":0x88+ch,"value":0xf9}),json!({"method":"write","address":0x90,"value":1u8<<ch,"controlMask":1u8<<ch})]);
                json!({"method":"batch","channel":ch,"voice":i+1,"action":"on","audition":i==0,"commands":commands})
            },
            _=>json!({"method":"batch","channel":ch,"voice":i+1,"action":"on","audition":i==0,"commands":fm(ch,false)}),
        };cmd(source,0,m,owner)
    }).collect()
}
#[derive(Default)]
struct Audio {
    frames: u64,
    callbacks: u64,
    nanos: u128,
    peak: [f64; 2],
    sum: [f64; 2],
    square: [f64; 2],
    nonzero: u64,
    nonfinite: u64,
    over: u64,
    at_limit: u64,
    hash: u64,
    pcm: Option<Vec<[u32; 2]>>,
}
impl Audio {
    fn add(&mut self, p: [f32; 2]) {
        if let Some(pcm) = &mut self.pcm {
            pcm.push(p.map(f32::to_bits));
        }
        self.frames += 1;
        let mut active = false;
        for (ch, v) in p.into_iter().enumerate() {
            self.hash = (self.hash ^ v.to_bits() as u64).wrapping_mul(1_099_511_628_211);
            if !v.is_finite() {
                self.nonfinite += 1;
                continue;
            }
            let v = v as f64;
            self.peak[ch] = self.peak[ch].max(v.abs());
            self.sum[ch] += v;
            self.square[ch] += v * v;
            active |= v.abs() > 1e-9;
            if v.abs() >= 1. {
                self.at_limit += 1;
            }
            if v.abs() > 1. {
                self.over += 1;
            }
        }
        self.nonzero += active as u64;
    }
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
            for _ in 0..n {
                self.add(r.sample(s));
            }
            self.nanos += t.elapsed().as_nanos();
            self.callbacks += 1;
            s.frame.store(r.frame, Ordering::Release);
            left -= n;
        }
        r.prepare(s);
    }
    fn ac_rms(&self) -> f64 {
        (0..2)
            .map(|ch| {
                (self.square[ch] / self.frames.max(1) as f64
                    - (self.sum[ch] / self.frames.max(1) as f64).powi(2))
                .max(0.)
                .sqrt()
            })
            .fold(0., f64::max)
    }
    fn value(&self) -> Value {
        let n = self.frames.max(1) as f64;
        let rms = self.square.map(|v| (v / n).sqrt());
        let db = |v: f64| if v > 0. { Some(20. * v.log10()) } else { None };
        json!({"frames":self.frames,"callbackBlocks":self.callbacks,"renderSeconds":self.nanos as f64/1e9,"peak":self.peak,"rms":rms,"dcMean":self.sum.map(|v|v/n),"acRmsMaximum":self.ac_rms(),"peakDbfs":self.peak.map(db),"rmsDbfs":rms.map(db),"nonzeroFrames":self.nonzero,"nonfiniteSamples":self.nonfinite,"samplesOverUnity":self.over,"samplesAtOrOverUnity":self.at_limit,"pcmHash":format!("{:016x}",self.hash)})
    }
}
#[derive(Clone, Copy, Default)]
struct Observed {
    fixed: usize,
    bytes: usize,
    pending: usize,
    metadata: usize,
}
impl Observed {
    fn record(&mut self, s: &Shared, sources: &[Source]) {
        self.bytes = self.bytes.max(s.extra_psg.memory_used());
        self.pending = self.pending.max(s.pending.load(Ordering::Acquire));
        self.metadata = self.metadata.max(
            sources
                .iter()
                .map(|source| metadata(&state(s, *source)))
                .sum(),
        );
    }
}
fn setup(
    kinds: &[Kind],
    base: u64,
    heavy: bool,
    gain: f32,
    voices: usize,
) -> (Shared, Renderer, Vec<Source>, Observed) {
    let s = Shared::new(RATE, 1);
    let mut r = Renderer::new(RATE).unwrap();
    s.performance("start", 7, None).unwrap();
    let sources = kinds
        .iter()
        .enumerate()
        .map(|(i, k)| create(&s, *k, base + i as u64))
        .collect::<Vec<_>>();
    let mut observed = Observed {
        fixed: s.extra_psg.memory_used(),
        ..Observed::default()
    };
    enqueue(
        &s,
        sources
            .iter()
            .flat_map(|source| resources(*source, heavy, gain))
            .collect(),
    );
    observed.record(&s, &sources);
    Audio::default().render(&s, &mut r, 512, false);
    enqueue(
        &s,
        sources
            .iter()
            .flat_map(|source| notes(*source, voices, [1, 2]))
            .collect(),
    );
    observed.record(&s, &sources);
    (s, r, sources, observed)
}
fn state(s: &Shared, source: Source) -> Value {
    match source.kind {
        Kind::Fm => serde_json::to_value(s.extra_ym2612_status(source.id, None).unwrap()).unwrap(),
        Kind::Rf => serde_json::to_value(s.extra_rf5c164_status(source.id, None).unwrap()).unwrap(),
        Kind::Dx => serde_json::to_value(s.extra_dx7_status(source.id, None).unwrap()).unwrap(),
        Kind::Snes => {
            serde_json::to_value(s.extra_snes_status(source.id, None, false).unwrap()).unwrap()
        }
        Kind::Spc => serde_json::to_value(s.extra_spc_status(source.id, None).unwrap()).unwrap(),
        Kind::Opna => {
            serde_json::to_value(s.extra_ym2608_status(source.id, None).unwrap()).unwrap()
        }
        Kind::Opnb => {
            serde_json::to_value(s.extra_ym2610_status(source.id, None).unwrap()).unwrap()
        }
        Kind::Ws => {
            serde_json::to_value(s.extra_wonderswan_status(source.id, None).unwrap()).unwrap()
        }
    }
}
fn metadata(v: &Value) -> usize {
    match v {
        Value::Object(m) => m
            .iter()
            .map(|(k, v)| {
                if k == "pendingCommands" {
                    v.as_array().unwrap().len()
                } else {
                    metadata(v)
                }
            })
            .sum(),
        Value::Array(a) => a.iter().map(metadata).sum(),
        _ => 0,
    }
}
fn assert_owners(s: &Shared, sources: &[Source], cancelled: bool) {
    for source in sources {
        let v = state(s, *source);
        let c = &v[source.kind.name()];
        if source.kind == Kind::Spc {
            assert_eq!(c["playing"], true);
            continue;
        }
        assert!(
            !c["held"].as_array().unwrap().is_empty(),
            "manual lease lost: {:?}",
            source.kind
        );
        match source.kind {
            Kind::Rf => {
                assert_eq!(c["channels"][0]["active"], true);
                assert_eq!(c["channels"][1]["active"], !cancelled);
                assert_eq!(c["channels"][2]["active"], true);
            }
            Kind::Dx => {
                let n = c["activeNotes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|n| n["midi"].as_u64().unwrap())
                    .collect::<Vec<_>>();
                assert!(n.contains(&60) && n.contains(&67));
                assert_eq!(n.contains(&64), !cancelled);
            }
            Kind::Snes => {
                assert_eq!(c["enabled"].as_u64().unwrap() & 1, 1);
                assert_eq!(c["enabled"].as_u64().unwrap() & 2 == 0, cancelled);
                assert_ne!(c["enabled"].as_u64().unwrap() & 4, 0);
            }
            Kind::Ws => {
                let gates = c["registers"][16].as_u64().unwrap();
                assert_eq!(gates & 1, 1);
                assert_eq!(gates & 2 == 0, cancelled);
                assert_ne!(gates & 4, 0);
            }
            _ => {
                let ch = source.kind.channel(1) as usize;
                assert_eq!(c["keyMasks"][source.kind.channel(0) as usize], 15);
                assert_eq!(c["keyMasks"][ch], if cancelled { 0 } else { 15 });
                assert_eq!(c["keyMasks"][source.kind.channel(2) as usize], 15);
            }
        }
    }
}
fn future(sources: &[Source], frame: u64) -> Vec<Command> {
    sources.iter().filter(|source|source.kind!=Kind::Spc).flat_map(|source|(1..=2).map(move|owner|{let ch=source.kind.channel(owner as usize);let m=match source.kind{Kind::Dx=>json!({"method":"note-on","note":72+owner,"velocity":90,"voice":100+owner}),Kind::Rf=>json!({"method":"set-channel","channel":ch,"options":{"step":768+owner*128}}),Kind::Snes=>json!({"method":"batch","channel":ch,"voice":owner+1,"action":"on","commands":[{"method":"key-on","channel":ch}]}),Kind::Ws=>json!({"method":"write","address":0x80+ch*2,"value":110+owner as u8}),_=>wr(ch/3,0xa0+ch%3,110+owner as u8)};cmd(*source,frame,m,Some(owner))})).collect()
}
fn guards(s: &Shared) -> Vec<std::sync::Weak<dyn Any + Send + Sync>> {
    fn weak<T: Any + Send + Sync>(v: &Arc<T>) -> std::sync::Weak<dyn Any + Send + Sync> {
        let e: Arc<dyn Any + Send + Sync> = v.clone();
        Arc::downgrade(&e)
    }
    let mut events = Vec::new();
    let mut out = Vec::new();
    while let Some(e) = s.queue.pop() {
        let w = match &*e.action.borrow() {
            Action::ExtraYm2612(_, v) => Some(weak(v)),
            Action::ExtraRf5c164(_, v) => Some(weak(v)),
            Action::ExtraDx7(_, v) => Some(weak(v)),
            Action::ExtraSnes(_, v) => Some(weak(v)),
            Action::ExtraYm2608(_, v) => Some(weak(v)),
            Action::ExtraYm2610(_, v) => Some(weak(v)),
            Action::ExtraWonderSwan(_, v) => Some(weak(v)),
            _ => None,
        };
        if let Some(w) = w {
            out.push(w);
        }
        events.push(e);
    }
    for e in events {
        s.queue.push(e).unwrap();
    }
    out
}
fn stop(s: &Shared, r: &mut Renderer) {
    let old = s.epoch.load(Ordering::Acquire);
    std::thread::scope(|scope| {
        let t = scope.spawn(|| s.stop());
        let start = Instant::now();
        while s.epoch.load(Ordering::Acquire) == old {
            assert!(start.elapsed() < Duration::from_secs(2));
            std::thread::yield_now();
        }
        Audio::default().render(s, r, r.fade_frames as usize + 512, true);
        t.join().unwrap().unwrap();
    });
    s.enqueue(1, &[]).unwrap();
    s.extra_psg.collect_retired().unwrap();
    assert_eq!(s.extra_psg.memory_used(), 0);
    assert_eq!(s.extra_psg.counts(), (0, 0));
    assert_eq!(s.pending.load(Ordering::Acquire), 0);
    assert!(r.heap.is_empty() && s.queue.is_empty());
    assert!(!s.failed.load(Ordering::Acquire));
    assert_eq!(s.overflows.load(Ordering::Acquire), 0);
    let mut silent = Audio::default();
    silent.render(s, r, 256, true);
    assert_eq!(silent.peak, [0.; 2]);
}
fn raw(kinds: &[Kind], gain: f32, voices: usize) -> Value {
    let (s, mut r, _, _) = setup(kinds, 1, false, gain, voices);
    Audio::default().render(&s, &mut r, 1024, false);
    let mut a = Audio::default();
    let t = Instant::now();
    for _ in 0..4096 {
        let mut p = [0.; 2];
        r.extra_psg.mix(&mut p);
        r.frame += 1;
        a.add(p);
    }
    a.nanos = t.elapsed().as_nanos();
    assert_eq!(a.nonfinite, 0);
    assert!(a.ac_rms() > 1e-7, "silent/DC-only fixture: {kinds:?}");
    if gain == GAIN {
        assert_eq!(a.over, 0, "safe gain raw bus clipped");
    }
    let v = a.value();
    stop(&s, &mut r);
    v
}
fn run_scene(name: &str, kinds: &[Kind], rounds: usize, frames: usize) -> Value {
    let heavy = name == "memory-heavy";
    let mut comparison = Vec::new();
    let mut reference = None;
    for variable in [false, true] {
        let (s, mut r, sources, _) = setup(kinds, 1, heavy, GAIN, 3);
        enqueue(&s, future(&sources, r.frame + 1503));
        let mut a = Audio {
            pcm: Some(Vec::with_capacity(4096)),
            ..Audio::default()
        };
        a.render(&s, &mut r, 4096, variable);
        assert!(a.ac_rms() > 1e-7);
        assert_eq!(a.nonfinite, 0);
        assert_eq!(a.over, 0);
        if let Some(expected) = &reference {
            assert_eq!(
                a.pcm.as_ref().unwrap(),
                expected,
                "{name}: callback boundaries changed exact PCM bits"
            );
        } else {
            reference = a.pcm.take();
        }
        let mut entry = a.value();
        entry["variableBlocks"] = json!(variable);
        entry["blockPattern"] = json!(if variable {
            BLOCKS.as_slice()
        } else {
            &[128][..]
        });
        comparison.push(entry);
        stop(&s, &mut r);
    }
    let mut fixed = 0;
    let mut maximum_bytes = 0;
    let mut maximum_pending = 0;
    let mut maximum_metadata = 0;
    let mut reclaimed = 0;
    let mut total = Audio::default();
    let start = Instant::now();
    let mut checkpoints = Vec::new();
    for round in 0..rounds {
        let (s, mut r, mut sources, observed) = setup(kinds, 1, heavy, GAIN, 3);
        fixed = observed.fixed;
        maximum_bytes = maximum_bytes.max(observed.bytes);
        maximum_pending = maximum_pending.max(observed.pending);
        maximum_metadata = maximum_metadata.max(observed.metadata);
        for restart in 0..2 {
            if restart == 1 {
                s.performance("start", 7, None).unwrap();
                sources = kinds
                    .iter()
                    .enumerate()
                    .map(|(i, k)| create(&s, *k, 200 + i as u64))
                    .collect();
                enqueue(
                    &s,
                    sources
                        .iter()
                        .flat_map(|src| resources(*src, heavy, GAIN))
                        .collect(),
                );
                let mut observed = Observed::default();
                observed.record(&s, &sources);
                maximum_bytes = maximum_bytes.max(observed.bytes);
                maximum_pending = maximum_pending.max(observed.pending);
                maximum_metadata = maximum_metadata.max(observed.metadata);
                Audio::default().render(&s, &mut r, 512, true);
                enqueue(
                    &s,
                    sources
                        .iter()
                        .flat_map(|src| notes(*src, 3, [1, 2]))
                        .collect(),
                );
            }
            maximum_bytes = maximum_bytes.max(s.extra_psg.memory_used());
            total.render(&s, &mut r, frames, true);
            assert_owners(&s, &sources, false);
            enqueue(&s, future(&sources, r.frame + RATE as u64 * 60));
            let weak = guards(&s);
            assert!(!weak.is_empty());
            maximum_bytes = maximum_bytes.max(s.extra_psg.memory_used());
            maximum_pending = maximum_pending.max(s.pending.load(Ordering::Acquire));
            maximum_metadata = maximum_metadata.max(
                sources
                    .iter()
                    .map(|src| metadata(&state(&s, *src)))
                    .sum::<usize>(),
            );
            s.performance("cancel-owner", 7, Some(1)).unwrap();
            for source in sources.iter().filter(|src| src.kind == Kind::Spc) {
                enqueue(
                    &s,
                    vec![cmd(
                        *source,
                        0,
                        json!({"method":"cancel-owner","session":7,"owner":1}),
                        None,
                    )],
                );
            }
            total.render(&s, &mut r, 256, true);
            s.extra_psg.collect_retired().unwrap();
            assert_owners(&s, &sources, true);
            let released = weak.iter().filter(|w| w.upgrade().is_none()).count();
            assert!(released > 0 && released < weak.len());
            let old = sources[0];
            let instance = Arc::downgrade(
                &s.extra_psg
                    .get(old.id, s.epoch.load(Ordering::Acquire))
                    .unwrap(),
            );
            s.dispose_extra_ym2612(old.id).unwrap();
            let retiring = s.extra_psg.memory_used();
            assert!(instance.upgrade().is_some());
            assert!(s.create_extra_ym2612(old.id, None).is_err());
            total.render(&s, &mut r, 256, true);
            s.extra_psg.collect_retired().unwrap();
            assert!(instance.upgrade().is_none());
            let after = s.extra_psg.memory_used();
            assert!(after < retiring);
            let replacement = create(&s, Kind::Fm, if restart == 0 { 100 } else { 300 });
            enqueue(&s, resources(replacement, false, GAIN));
            total.render(&s, &mut r, 512, true);
            enqueue(&s, notes(replacement, 3, [3, 4]));
            total.render(&s, &mut r, 512, true);
            assert!(s.enqueue(1, &[cmd(old, 0, wr(0, 0xa0, 1), None)]).is_err());
            checkpoints.push(json!({"round":round,"restart":restart,"fixedReservationBytes":fixed,"retiringReservationBytes":retiring,"afterRetireReservationBytes":after,"disposedInstance":old.id,"replacementInstance":replacement.id,"logicalSource":"ym2612","pendingBeforeStop":s.pending.load(Ordering::Acquire),"ownerCancelledGuardCount":released}));
            stop(&s, &mut r);
            assert!(weak.iter().all(|w| w.upgrade().is_none()));
            reclaimed += weak.len();
        }
    }
    assert_eq!(total.nonfinite, 0);
    assert_eq!(total.over, 0);
    assert_eq!(
        total.at_limit, 0,
        "safe delivered output reached the final clamp boundary"
    );
    assert!(total.ac_rms() > 1e-7);
    json!({"scene":name,"passed":true,"sources":kinds.iter().map(|k|k.name()).collect::<Vec<_>>(),"simultaneousVoicesPerInstrument":3,"spcFileVoices":1,"rounds":rounds,"runStopTransitions":rounds*2,"ownerCancellations":rounds*2,"segmentationExact":true,"callbackComparison":comparison,"delivered":total.value(),"wallSeconds":start.elapsed().as_secs_f64(),"fixedReservationBytes":fixed,"maximumObservedReservedBytes":maximum_bytes,"maximumPendingEvents":maximum_pending,"maximumPendingMetadata":maximum_metadata,"reclaimedFutureCommandGuards":reclaimed,"pendingAfterStop":0,"reservedBytesAfterStop":0,"retainedInstancesAfterStop":0,"checkpoints":checkpoints,"safeRawMix":raw(kinds,GAIN,3),"unityRawMixDiagnostic":raw(kinds,1.,3)})
}
fn limits() -> Value {
    let mut s = Shared::new(RATE, 1);
    s.extra_psg = extra_psg::Shared::with_memory_limit(5 * extra_budget::INSTANCE_BYTES);
    let mut r = Renderer::new(RATE).unwrap();
    create(&s, Kind::Fm, 1);
    create(&s, Kind::Rf, 2);
    create(&s, Kind::Dx, 3);
    let before = s.extra_psg.memory_used();
    let counts = s.extra_psg.counts();
    let error = s.create_extra_spc(4).unwrap_err();
    assert!(error.contains("memory budget"));
    assert_eq!(s.extra_psg.memory_used(), before);
    assert_eq!(s.extra_psg.counts(), counts);
    s.create_extra_psg(4, None).unwrap();
    stop(&s, &mut r);
    let mut s = Shared::new(RATE, 1);
    s.extra_psg = extra_psg::Shared::with_memory_limit(8 * 1024 * 1024);
    let mut r = Renderer::new(RATE).unwrap();
    let rom = create(&s, Kind::Opnb, 1);
    let fm = create(&s, Kind::Fm, 2);
    let dx = create(&s, Kind::Dx, 3);
    enqueue(&s, resources(rom, false, GAIN));
    Audio::default().render(&s, &mut r, 1, false);
    s.extra_psg.collect_retired().unwrap();
    let baseline = s.extra_psg.memory_used();
    let pending = s.pending.load(Ordering::Acquire);
    let too_large = memory(rom, 1, vec![3; 16 * 1024 * 1024]);
    assert!(s
        .enqueue(1, &[cmd(fm, 0, wr(0, 0xa0, 99), None), too_large])
        .unwrap_err()
        .contains("memory budget"));
    s.extra_psg.collect_retired().unwrap();
    assert_eq!(s.extra_psg.memory_used(), baseline);
    assert_eq!(s.pending.load(Ordering::Acquire), pending);
    for _ in 0..3 {
        let identity = ID.fetch_add(1, Ordering::Relaxed);
        let duplicate = || {
            serde_json::from_value::<Command>(json!({"type":"extra-dx7","instance":dx.id,"frame":10000,"message":{"id":identity,"method":"note-on","note":60,"velocity":90,"voice":1}})).unwrap()
        };
        assert!(s
            .enqueue(
                1,
                &[
                    cmd(
                        rom,
                        0,
                        json!({"method":"memory","romType":1,"offset":65536,"data":[2,3]}),
                        None
                    ),
                    duplicate(),
                    duplicate()
                ]
            )
            .unwrap_err()
            .contains("Duplicate"));
        s.extra_psg.collect_retired().unwrap();
        assert_eq!(s.extra_psg.memory_used(), baseline);
    }
    assert_eq!(
        s.extra_ym2612_status(2, None).unwrap().ym2612.registers[0][0xa0],
        0
    );
    stop(&s, &mut r);
    let s = Shared::new(RATE, 1);
    let mut r = Renderer::new(RATE).unwrap();
    let fm = create(&s, Kind::Fm, 1);
    Audio::default().render(&s, &mut r, 1, false);
    let base = s.extra_psg.memory_used();
    let permit = s
        .extra_psg
        .reserve_payloads(std::iter::once(
            extra_budget::DEFAULT_LIMIT - base - extra_budget::PAYLOAD_OVERHEAD,
        ))
        .unwrap();
    assert_eq!(s.extra_psg.memory_used(), extra_budget::DEFAULT_LIMIT);
    assert!(s
        .create_extra_psg(2, None)
        .unwrap_err()
        .contains("memory budget"));
    assert!(s
        .enqueue(1, &[cmd(fm, 0, wr(0, 0xa0, 99), None)])
        .unwrap_err()
        .contains("memory budget"));
    assert_eq!(s.extra_psg.counts(), (1, 1));
    assert_eq!(s.pending.load(Ordering::Acquire), 0);
    drop(permit);
    assert_eq!(s.extra_psg.memory_used(), base);
    s.create_extra_psg(2, None).unwrap();
    stop(&s, &mut r);
    json!({"passed":true,"instanceBudgetRejection":error,"customPayloadBudgetBytes":8*1024*1024,"payloadRollbackAttempts":4,"reservedBytesAfterRejection":baseline,"reservedBytesAfterStop":0,"default64MiBPolicyProbe":{"reservationOnly":true,"saturatedReservationBytes":extra_budget::DEFAULT_LIMIT,"creationAndWriteRolledBack":true,"recoveredAfterPermitDrop":true}})
}
fn slots() -> Value {
    let s = Shared::new(RATE, 1);
    let mut r = Renderer::new(RATE).unwrap();
    for i in 0..16 {
        create(&s, ALL[i % ALL.len()], i as u64 + 1);
    }
    let before = s.extra_psg.memory_used();
    assert!(before < extra_budget::DEFAULT_LIMIT);
    let error = s.create_extra_psg(17, None).unwrap_err();
    assert!(error.contains("limit"));
    assert_eq!(s.extra_psg.memory_used(), before);
    assert_eq!(s.extra_psg.counts(), (16, 16));
    let old = Source {
        kind: Kind::Fm,
        id: 1,
    };
    enqueue(&s, vec![cmd(old, RATE as u64 * 60, wr(0, 0xa0, 99), None)]);
    let weak = guards(&s);
    let instance = Arc::downgrade(&s.extra_psg.get(1, 0).unwrap());
    Audio::default().render(&s, &mut r, 1, false);
    s.dispose_extra_ym2612(1).unwrap();
    assert_eq!(s.extra_psg.counts(), (16, 15));
    assert!(s.create_extra_psg(17, None).unwrap_err().contains("limit"));
    assert!(instance.upgrade().is_some());
    Audio::default().render(&s, &mut r, 256, true);
    s.extra_psg.collect_retired().unwrap();
    assert!(instance.upgrade().is_none());
    assert!(weak.iter().all(|w| w.upgrade().is_none()));
    s.create_extra_psg(17, None).unwrap();
    assert_eq!(s.extra_psg.counts(), (16, 16));
    stop(&s, &mut r);
    json!({"passed":true,"slots":16,"reservationBytes":before,"rejectedSeventeenth":true,"retiringSlotRejectedBeforeDrain":true,"futureGuardReclaimed":true,"monotonicReplacement":17,"reservedBytesAfterStop":0})
}
fn save(v: &Value) {
    if let Some(p) = std::env::var_os("TETORICA_NATIVE_EXTRA_STABILITY_REPORT") {
        let p = PathBuf::from(p);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, serde_json::to_vec_pretty(v).unwrap()).unwrap();
    }
}
fn failure(e: Box<dyn Any + Send>) -> String {
    e.downcast_ref::<String>()
        .cloned()
        .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_else(|| "Unknown panic".into())
}
fn mixed_rack_report() {
    let long = std::env::var("TETORICA_NATIVE_EXTRA_STABILITY_PROFILE").as_deref() == Ok("long");
    let seconds = std::env::var("TETORICA_NATIVE_EXTRA_STABILITY_SECONDS")
        .ok()
        .map(|s| s.parse::<f64>().unwrap())
        .unwrap_or(if long { 30. } else { 0.125 });
    let rounds = std::env::var("TETORICA_NATIVE_EXTRA_STABILITY_ROUNDS")
        .ok()
        .map(|s| s.parse::<usize>().unwrap())
        .unwrap_or(if long { 3 } else { 2 });
    assert!(seconds.is_finite() && (0.09..=3600.).contains(&seconds));
    assert!((1..=1000).contains(&rounds));
    let mut report = json!({"schemaVersion":1,"passed":false,"deviceOpened":false,"profile":if long{"long"}else{"bounded"},"sampleRate":RATE,"secondsPerRun":seconds,"rounds":rounds,"sceneGain":GAIN,"segmentationComparisonFrames":4096,"lifecycleBlockPattern":BLOCKS,"controlHarnessStackBytes":8*1024*1024,"timingBuild":if cfg!(debug_assertions){"debug"}else{"release"},"reservationUnit":"Native budget permit bytes; not process heap or RSS","limits":{"slots":16,"reservationBytes":extra_budget::DEFAULT_LIMIT,"absolutePerformanceGate":false},"notes":["Output checks use the actual Shared/Renderer; its final finite-or-zero clamp can hide upstream faults.","Raw diagnostics separately use the actual extra rack before final FX/master/fades/clamp, after gain warmup; no DSP production tap or gain normalization was added.","Solo levels describe these patches, samples and voice counts, not universal chip calibration. Raw unity clipping is diagnostic, not a failure solely for loudness.","Maximum reservation and pending values are observed at explicit lifecycle checkpoints; this is not an exhaustive allocator leak proof."],"scenes":[],"soloSources":[]});
    save(&report);
    let start = Instant::now();
    for (name, kinds) in SCENES {
        let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
            run_scene(name, kinds, rounds, (seconds * RATE as f64).ceil() as usize)
        }));
        let entry = match result {
            Ok(v) => v,
            Err(e) => json!({"scene":name,"passed":false,"error":failure(e)}),
        };
        report["scenes"].as_array_mut().unwrap().push(entry);
        save(&report);
    }
    for kind in ALL {
        let result = std::panic::catch_unwind(AssertUnwindSafe(
            || json!({"kind":kind.name(),"passed":true,"gain":GAIN,"oneVoice":raw(&[kind],GAIN,1),"representativePolyphony":raw(&[kind],GAIN,3),"polyphonyVoices":if kind==Kind::Spc{1}else{3}}),
        ));
        let entry = match result {
            Ok(v) => v,
            Err(e) => json!({"kind":kind.name(),"passed":false,"error":failure(e)}),
        };
        report["soloSources"].as_array_mut().unwrap().push(entry);
        save(&report);
    }
    for (key, run) in [("admission", limits as fn() -> Value), ("slots", slots)] {
        let result = std::panic::catch_unwind(AssertUnwindSafe(run));
        report[key] = match result {
            Ok(v) => v,
            Err(e) => json!({"passed":false,"error":failure(e)}),
        };
    }
    report["passed"] = json!(
        report["scenes"]
            .as_array()
            .unwrap()
            .iter()
            .chain(report["soloSources"].as_array().unwrap())
            .all(|v| v["passed"] == true)
            && report["admission"]["passed"] == true
            && report["slots"]["passed"] == true
    );
    report["wallSeconds"] = json!(start.elapsed().as_secs_f64());
    save(&report);
    println!(
        "NATIVE_EXTRA_STABILITY_JSON {}",
        serde_json::to_string(&report).unwrap()
    );
    assert_eq!(
        report["passed"], true,
        "Inspect mixed extra-chip stability JSON"
    );
}
// These are control-side/offline harness threads, not simulated device stack
// limits. Large debug Shared/factory return temporaries need bounded headroom.
fn control(task: fn()) {
    std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(task)
        .unwrap()
        .join()
        .unwrap();
}
#[test]
fn native_extra_stability_mixed_rack_report() {
    control(mixed_rack_report);
}
#[test]
fn native_extra_stability_transaction_limits_and_rollback() {
    control(|| {
        limits();
    });
}
#[test]
fn native_extra_stability_sixteen_mixed_slots_are_bounded() {
    control(|| {
        slots();
    });
}
