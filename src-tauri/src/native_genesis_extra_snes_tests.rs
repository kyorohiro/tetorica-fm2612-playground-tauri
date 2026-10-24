//! Additional CPU-free S-DSP integration without opening an audio device.
use super::*;
use serde_json::{json, Value};
fn cmd(instance: u64, frame: u64, message: Value) -> Command {
    serde_json::from_value(
        json!({"type":"extra-snes","instance":instance,"frame":frame,"message":message}),
    )
    .unwrap()
}
fn send(s: &Shared, id: u64, messages: Vec<Value>) {
    s.enqueue(
        1,
        &messages
            .into_iter()
            .map(|m| cmd(id, 0, m))
            .collect::<Vec<_>>(),
    )
    .unwrap();
}
fn render(s: &Shared, r: &mut Renderer, count: usize) -> Vec<[f32; 2]> {
    let out = (0..count)
        .map(|i| {
            if i % 128 == 0 {
                r.prepare(s)
            }
            r.sample(s)
        })
        .collect();
    r.prepare(s);
    s.frame.store(r.frame, Ordering::Release);
    out
}
fn new() -> (Shared, Renderer) {
    let s = Shared::new(48000, 1);
    let r = Renderer::new(48000).unwrap();
    s.create_extra_snes(1).unwrap();
    (s, r)
}
fn state(s: &Shared, id: u64) -> extra_snes::ChipStatus {
    s.extra_snes_status(id, None, false).unwrap().snes
}
fn write(register: u8, value: u8) -> Value {
    json!({"method":"write","register":register,"value":value})
}
fn group(mut m: Value, id: u64) -> Value {
    m["group"] = json!(id);
    m
}
fn batch(ch: u8, voice: u64, action: &str, manual: bool, ops: Vec<Value>) -> Value {
    json!({"method":"batch","channel":ch,"voice":voice,"action":action,"audition":manual,"commands":ops})
}
fn on(ch: u8, voice: u64, manual: bool) -> Value {
    batch(
        ch,
        voice,
        "on",
        manual,
        vec![json!({"method":"key-on","channel":ch})],
    )
}
fn off(ch: u8, voice: u64, manual: bool) -> Value {
    batch(
        ch,
        voice,
        "off",
        manual,
        vec![json!({"method":"key-off","channel":ch})],
    )
}
fn dsp() -> Vec<u8> {
    let mut d = vec![0; 128];
    for (reg, v) in [(0x6c, 0x20), (0x0c, 127), (0x1c, 127), (0x5c, 255)] {
        d[reg] = v;
    }
    for ch in 0..8 {
        d[ch * 16] = 100;
        d[ch * 16 + 1] = 100;
        d[ch * 16 + 3] = 16;
        d[ch * 16 + 7] = 127;
    }
    d
}
fn brr() -> Vec<u8> {
    vec![0xb3, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0]
}
fn reset_state() -> Value {
    json!({"method":"reset-state","dsp":dsp(),"uploads":[{"address":0x400,"data":brr()},{"address":0,"data":[0,4,0,4]}]})
}
fn configure(s: &Shared, id: u64) {
    send(s, id, vec![reset_state()]);
}
fn energy(v: &[[f32; 2]]) -> f32 {
    v.iter().flatten().map(|v| v.abs()).sum()
}
fn capture(s: &Shared, r: &mut Renderer, id: u64, command: u64) -> extra_snes::ChipStatus {
    send(
        s,
        id,
        vec![json!({"method":"barrier","ram":true,"id":command})],
    );
    render(s, r, 1);
    s.extra_snes_status(id, Some(command), true).unwrap().snes
}
fn timeline() -> Value {
    let mut ram = vec![0u8; 65536];
    ram[..4].copy_from_slice(&[0, 4, 0, 4]);
    ram[0x400..0x409].copy_from_slice(&brr());
    json!({"method":"prepare-timeline","ram":ram,"dsp":dsp(),"events":[0,0,0x5c,0,0,0,0x4c,1,1000,1,0x500,42],"durationClocks":32000})
}
#[test]
fn extra_snes_twenty_kinds_share_sixteen_slots_and_bounded_layout() {
    let (s, mut r) = new();
    s.create_extra_pwm(2).unwrap();
    s.create_extra_dx7(3).unwrap();
    s.create_extra_huc6280(4, None).unwrap();
    s.create_extra_nes(5, None, true).unwrap();
    s.create_extra_gameboy(6, None).unwrap();
    s.create_extra_psg(7, None).unwrap();
    s.create_extra_ym2612(8, None).unwrap();
    s.create_extra_ym2203(9, None).unwrap();
    s.create_extra_ym2608(10, None).unwrap();
    s.create_extra_ym2610(11, None).unwrap();
    s.create_extra_ym2151(12, None).unwrap();
    s.create_extra_ym2413(13, None).unwrap();
    s.create_extra_ay8910(14, None).unwrap();
    s.create_extra_ym3526(15, None).unwrap();
    s.create_extra_ym3812(16, None).unwrap();
    assert!(s
        .create_extra_y8950(17, None)
        .unwrap_err()
        .contains("limit"));
    render(&s, &mut r, 1);
    s.dispose_extra_snes(1).unwrap();
    render(&s, &mut r, 1);
    s.extra_psg.collect_retired().unwrap();
    s.create_extra_y8950(17, None).unwrap();
    println!(
        "SNES Instance={} Shared={} Renderer={} Session={} Fixed={}",
        std::mem::size_of::<extra_psg::Instance>(),
        std::mem::size_of::<extra_snes::Shared>(),
        std::mem::size_of::<extra_snes::Renderer>(),
        std::mem::size_of::<Renderer>(),
        extra_snes::INSTANCE_BYTES
    );
    assert!(std::mem::size_of::<extra_psg::Instance>() + 27040 < 65536);
}
#[test]
fn extra_snes_sixteen_independent_rams_fit_shared_memory() {
    let (s, mut r) = new();
    for id in 2..=16 {
        s.create_extra_snes(id).unwrap();
    }
    assert_eq!(s.extra_psg.memory_used(), 16 * extra_snes::INSTANCE_BYTES);
    assert!(s.extra_psg.memory_used() < extra_budget::DEFAULT_LIMIT);
    render(&s, &mut r, 1);
    s.extra_psg.deactivate_all().unwrap();
    render(&s, &mut r, 1);
    s.extra_psg.collect_retired().unwrap();
    assert_eq!(s.extra_psg.memory_used(), 0);
}
#[test]
fn extra_snes_ram_brr_registers_and_audio_are_independent() {
    let (s, mut r) = new();
    s.create_extra_snes(2).unwrap();
    configure(&s, 1);
    configure(&s, 2);
    send(
        &s,
        1,
        vec![
            write(0, 60),
            json!({"method":"load-memory","address":0x500,"data":[42]}),
            on(0, 1, false),
        ],
    );
    assert!(energy(&render(&s, &mut r, 4096)) > 1.);
    assert_eq!(state(&s, 1).registers[0], 60);
    assert_eq!(state(&s, 2).registers[0], 100);
    assert_eq!(capture(&s, &mut r, 1, 1).ram.unwrap()[0x500], 42);
    assert_eq!(capture(&s, &mut r, 2, 1).ram.unwrap()[0x500], 0);
    assert_eq!(state(&s, 2).enabled, 0);
}
#[test]
fn extra_snes_pcm_matches_existing_primary_at_three_rates() {
    for rate in [44100, 48000, 96000] {
        let s = Shared::new(rate, 1);
        let mut r = Renderer::new(rate).unwrap();
        s.create_extra_snes(1).unwrap();
        let p = Shared::new_modes(rate, 1, false, false, false, Some("snes"));
        let mut pr = Renderer::new_modes(rate, false, false, false, Some("snes")).unwrap();
        let mut messages = vec![
            json!({"method":"reset"}),
            json!({"method":"load-memory","address":0x400,"data":brr()}),
            json!({"method":"load-memory","address":0,"data":[0,4,0,4]}),
        ];
        messages.extend(
            dsp()
                .iter()
                .enumerate()
                .map(|(reg, v)| write(reg as u8, *v)),
        );
        messages.push(on(0, 1, false));
        s.enqueue(
            1,
            &messages
                .iter()
                .map(|m| cmd(1, 0, m.clone()))
                .collect::<Vec<_>>(),
        )
        .unwrap();
        p.enqueue(
            1,
            &messages
                .iter()
                .map(|m| {
                    serde_json::from_value(json!({"type":"snes","frame":0,"message":m})).unwrap()
                })
                .collect::<Vec<_>>(),
        )
        .unwrap();
        let a = render(&s, &mut r, 4096);
        let b = render(&p, &mut pr, 4096);
        assert_eq!(a, b, "rate {rate}");
        assert!(a.iter().flatten().all(|v| v.is_finite()));
    }
}
#[test]
fn extra_snes_manual_voice_blocks_ram_and_state_admission_atomically() {
    let (s, mut r) = new();
    configure(&s, 1);
    send(&s, 1, vec![on(0, 1, true)]);
    render(&s, &mut r, 1);
    for m in [
        json!({"method":"load-memory-batch","uploads":[{"address":0x500,"data":[99]},{"address":0,"data":[99]}]}),
        reset_state(),
        timeline(),
        json!({"method":"play-timeline"}),
    ] {
        assert!(s
            .enqueue(1, &[cmd(1, 0, m)])
            .unwrap_err()
            .contains("manually held"));
    }
    let a = capture(&s, &mut r, 1, 1);
    assert_eq!(a.ram.unwrap()[0x500], 0);
    assert_eq!(a.held, [0]);
    assert!(!a.timeline.loaded);
}
#[test]
fn extra_snes_future_ram_and_state_apply_recheck_protects_later_manual_voice() {
    let (s, mut r) = new();
    configure(&s, 1);
    let mut reset = reset_state();
    reset["id"] = json!(2);
    s.enqueue(1,&[cmd(1,1000,json!({"method":"load-memory-batch","id":1,"uploads":[{"address":0x500,"data":[99]},{"address":0,"data":[99]}]})),cmd(1,1000,reset)]).unwrap();
    send(&s, 1, vec![on(0, 5, true)]);
    render(&s, &mut r, 1100);
    assert!(s
        .extra_snes_status(1, Some(1), false)
        .unwrap_err()
        .contains("sample memory"));
    assert!(s
        .extra_snes_status(1, Some(2), false)
        .unwrap_err()
        .contains("DSP state"));
    let a = capture(&s, &mut r, 1, 3);
    assert_eq!(a.ram.unwrap()[0], 0);
    assert_eq!(a.held, [0]);
}
#[test]
fn extra_snes_manual_lease_and_shared_bitfield_edits_preserve_other_voices() {
    let (s, mut r) = new();
    configure(&s, 1);
    send(&s, 1, vec![group(on(0, 1, false), 7), on(1, 2, true)]);
    render(&s, &mut r, 1);
    send(
        &s,
        1,
        vec![
            group(off(1, 1, false), 7),
            write(0x4c, 0),
            write(0x5c, 255),
            write(0x10, 1),
            write(0x0c, 0),
            batch(0, 1, "edit", false, vec![write(0x2d, 1)]),
        ],
    );
    render(&s, &mut r, 1);
    let a = state(&s, 1);
    assert_eq!(a.held, [1]);
    assert_ne!(a.enabled & 2, 0);
    assert_eq!(a.registers[0x10], 100);
    assert_eq!(a.registers[0x0c], 127);
    send(&s, 1, vec![off(1, 2, true)]);
    render(&s, &mut r, 1);
    assert!(state(&s, 1).held.is_empty());
}
#[test]
fn extra_snes_stale_group_release_and_cancel_preserve_foreign_and_manual() {
    let (s, mut r) = new();
    configure(&s, 1);
    send(
        &s,
        1,
        vec![
            group(on(0, 1, false), 1),
            group(on(0, 2, false), 2),
            group(on(1, 3, false), 1),
            on(2, 4, true),
        ],
    );
    render(&s, &mut r, 1);
    send(
        &s,
        1,
        vec![
            group(off(0, 1, false), 1),
            json!({"method":"cancel-main-owner","targetGroup":1}),
        ],
    );
    render(&s, &mut r, 1);
    let a = state(&s, 1);
    assert_ne!(a.enabled & 1, 0);
    assert_eq!(a.enabled & 2, 0);
    assert_eq!(a.held, [2]);
    assert_ne!(a.enabled & 4, 0);
}
#[test]
fn extra_snes_worker_cancel_preserves_manual_and_releases_owned_future() {
    let (s, mut r) = new();
    configure(&s, 1);
    s.performance("start", 7, None).unwrap();
    send(&s, 1, vec![on(2, 4, true)]);
    let mut owned = cmd(1, 0, on(0, 1, false));
    owned.performance = Some(7);
    owned.owner = Some(1);
    let mut future = cmd(
        1,
        1000,
        json!({"method":"write","register":1,"value":7,"id":1}),
    );
    future.performance = Some(7);
    future.owner = Some(1);
    s.enqueue(1, &[owned, future]).unwrap();
    render(&s, &mut r, 1);
    s.performance("cancel-owner", 7, Some(1)).unwrap();
    render(&s, &mut r, 1);
    let a = state(&s, 1);
    assert_eq!(a.enabled & 1, 0);
    assert_eq!(a.held, [2]);
    assert!(s
        .extra_snes_status(1, Some(1), false)
        .unwrap_err()
        .contains("cancelled"));
}
#[test]
fn extra_snes_snapshot_is_one_shot_coherent_and_exact_for_requested_barrier() {
    let (s, mut r) = new();
    configure(&s, 1);
    send(
        &s,
        1,
        vec![
            json!({"method":"load-memory","address":0x500,"data":[42]}),
            write(0, 60),
            json!({"method":"barrier","ram":true,"id":1}),
            json!({"method":"load-memory","address":0x500,"data":[99]}),
            write(0, 90),
            json!({"method":"barrier","ram":true,"id":2}),
        ],
    );
    render(&s, &mut r, 1);
    let a = s.extra_snes_status(1, Some(1), true).unwrap();
    let b = s.extra_snes_status(1, Some(2), true).unwrap();
    assert_eq!(a.snes.ram.unwrap()[0x500], 42);
    assert_eq!(a.snes.registers[0], 60);
    assert_eq!(b.snes.ram.unwrap()[0x500], 99);
    assert_eq!(b.snes.registers[0], 90);
    assert!(a.revision < b.revision);
    assert_eq!(a.snes.native_frames, 0);
    assert!(state(&s, 1).ram.is_none());
}
#[test]
fn extra_snes_atomic_reset_retains_brr_and_restores_instrument_mode_after_timeline() {
    let (s, mut r) = new();
    configure(&s, 1);
    send(
        &s,
        1,
        vec![timeline(), json!({"method":"play-timeline","id":1})],
    );
    render(&s, &mut r, 100);
    assert!(state(&s, 1).timeline.loaded);
    send(&s, 1, vec![reset_state(), on(0, 2, false)]);
    assert!(energy(&render(&s, &mut r, 1000)) > 1.);
    let a = capture(&s, &mut r, 1, 2);
    assert!(!a.timeline.loaded);
    assert_eq!(&a.ram.unwrap()[0x400..0x409], brr().as_slice());
}
#[test]
fn extra_snes_timeline_natural_end_replay_stop_and_generation() {
    let (s, mut r) = new();
    send(
        &s,
        1,
        vec![timeline(), json!({"method":"play-timeline","id":1})],
    );
    assert!(energy(&render(&s, &mut r, 1600)) > 0.);
    let a = state(&s, 1).timeline;
    assert!(a.loaded && a.ended && !a.playing);
    assert_eq!(a.play_command_id, 1);
    send(&s, 1, vec![json!({"method":"play-timeline","id":2})]);
    render(&s, &mut r, 10);
    let b = state(&s, 1).timeline;
    assert!(b.playing);
    assert!(b.generation > a.generation);
    assert_eq!(b.play_command_id, 2);
    send(&s, 1, vec![json!({"method":"stop-timeline","id":3})]);
    render(&s, &mut r, 1);
    let c = state(&s, 1).timeline;
    assert!(c.loaded && !c.playing);
    assert_eq!(c.play_command_id, 0);
    assert!(c.generation > b.generation);
}
#[test]
fn extra_snes_timeline_and_instrument_edit_exclusivity_is_applied_atomically() {
    let (s, mut r) = new();
    send(
        &s,
        1,
        vec![timeline(), json!({"method":"play-timeline","id":1})],
    );
    render(&s, &mut r, 10);
    let initial = state(&s, 1).registers;
    assert!(s
        .enqueue(1, &[cmd(1, 0, write(0, 1))])
        .unwrap_err()
        .contains("exclusive"));
    send(&s, 1, vec![json!({"method":"stop-timeline"})]);
    render(&s, &mut r, 1);
    assert!(state(&s, 1).timeline.loaded);
    assert_eq!(state(&s, 1).registers[0], initial[0]);
}
#[test]
fn extra_snes_timeline_binding_and_immutable_payload_budget_are_independent() {
    let (s, mut r) = new();
    s.create_extra_snes(2).unwrap();
    let fixed = 2 * extra_snes::INSTANCE_BYTES;
    send(&s, 1, vec![timeline()]);
    render(&s, &mut r, 1);
    s.extra_psg.collect_retired().unwrap();
    assert!(s.extra_psg.memory_used() > fixed + 65536);
    assert!(!state(&s, 2).timeline.loaded);
    send(&s, 1, vec![json!({"method":"reset"})]);
    render(&s, &mut r, 1);
    s.extra_psg.collect_retired().unwrap();
    assert_eq!(s.extra_psg.memory_used(), fixed);
}
#[test]
fn extra_snes_timeline_cancel_is_scoped_to_its_main_group() {
    let (s, mut r) = new();
    send(
        &s,
        1,
        vec![
            group(timeline(), 1),
            group(json!({"method":"play-timeline","id":1}), 1),
        ],
    );
    render(&s, &mut r, 10);
    send(
        &s,
        1,
        vec![json!({"method":"cancel-main-owner","targetGroup":2})],
    );
    render(&s, &mut r, 1);
    assert!(state(&s, 1).timeline.playing);
    send(
        &s,
        1,
        vec![json!({"method":"cancel-main-owner","targetGroup":1})],
    );
    render(&s, &mut r, 1);
    assert!(!state(&s, 1).timeline.playing);
}
#[test]
fn extra_snes_main_and_worker_cancellation_reclaim_applied_unread_captures() {
    let (s, mut r) = new();
    configure(&s, 1);
    send(
        &s,
        1,
        vec![group(json!({"method":"barrier","ram":true,"id":1}), 1)],
    );
    render(&s, &mut r, 1);
    send(
        &s,
        1,
        vec![json!({"method":"cancel-main-owner","targetGroup":1})],
    );
    render(&s, &mut r, 1);
    s.extra_psg.collect_retired().unwrap();
    assert!(s
        .extra_snes_status(1, Some(1), true)
        .unwrap_err()
        .contains("cancelled"));
    assert_eq!(s.extra_psg.memory_used(), extra_snes::INSTANCE_BYTES);
    s.performance("start", 7, None).unwrap();
    let mut c = cmd(1, 0, json!({"method":"barrier","ram":true,"id":2}));
    c.performance = Some(7);
    c.owner = Some(1);
    s.enqueue(1, &[c]).unwrap();
    render(&s, &mut r, 1);
    s.performance("cancel-owner", 7, Some(1)).unwrap();
    s.extra_psg.collect_retired().unwrap();
    assert!(s
        .extra_snes_status(1, Some(2), true)
        .unwrap_err()
        .contains("cancelled"));
    assert_eq!(s.extra_psg.memory_used(), extra_snes::INSTANCE_BYTES);
}
#[test]
fn extra_snes_global_capture_cap_reclaims_consumed_payloads() {
    let (s, mut r) = new();
    s.create_extra_snes(2).unwrap();
    let commands = (1..=16)
        .map(|id| {
            cmd(
                if id < 9 { 1 } else { 2 },
                0,
                json!({"method":"barrier","ram":true,"id":id}),
            )
        })
        .collect::<Vec<_>>();
    s.enqueue(1, &commands).unwrap();
    assert!(s
        .enqueue(
            1,
            &[cmd(2, 0, json!({"method":"barrier","ram":true,"id":17}))]
        )
        .unwrap_err()
        .contains("16 across shared rack"));
    render(&s, &mut r, 1);
    for id in 1..=16 {
        s.extra_snes_status(if id < 9 { 1 } else { 2 }, Some(id), true)
            .unwrap();
    }
    s.extra_psg.collect_retired().unwrap();
    assert_eq!(s.extra_psg.memory_used(), 2 * extra_snes::INSTANCE_BYTES);
    send(&s, 2, vec![json!({"method":"barrier","ram":true,"id":17})]);
    render(&s, &mut r, 1);
    assert!(s.extra_snes_status(2, Some(17), true).is_ok());
}
#[test]
fn extra_snes_invalid_atomic_upload_and_duplicate_ids_do_not_mutate() {
    let (s, mut r) = new();
    configure(&s, 1);
    render(&s, &mut r, 1);
    for m in [
        json!({"method":"load-memory-batch","uploads":[{"address":0x500,"data":[99]},{"address":65536,"data":[1]}]}),
        json!({"method":"reset-state","dsp":[0],"uploads":[]}),
        json!({"method":"barrier","ram":true}),
        json!({"method":"cancel-main-owner","targetGroup":0}),
        batch(0, 1, "on", false, vec![write(0x10, 1)]),
    ] {
        assert!(s.enqueue(1, &[cmd(1, 0, m)]).is_err());
    }
    let c = || cmd(1, 1000, json!({"method":"barrier","id":1}));
    assert!(s.enqueue(1, &[c(), c()]).unwrap_err().contains("Duplicate"));
    assert_eq!(capture(&s, &mut r, 1, 2).ram.unwrap()[0x500], 0);
}
#[test]
fn extra_snes_pending_payload_budget_counts_capture_and_timeline() {
    let mut s = Shared::new(48000, 1);
    s.extra_psg = extra_psg::Shared::with_memory_limit(extra_snes::INSTANCE_BYTES + 1024);
    let mut r = Renderer::new(48000).unwrap();
    s.create_extra_snes(1).unwrap();
    assert!(s
        .enqueue(
            1,
            &[cmd(1, 0, json!({"method":"barrier","ram":true,"id":1}))]
        )
        .unwrap_err()
        .contains("budget"));
    assert!(s
        .enqueue(1, &[cmd(1, 0, timeline())])
        .unwrap_err()
        .contains("budget"));
    render(&s, &mut r, 1);
    assert_eq!(s.extra_psg.memory_used(), extra_snes::INSTANCE_BYTES);
}
#[test]
fn extra_snes_dispose_rejects_stale_generation_and_new_instance_is_clean() {
    let (s, mut r) = new();
    configure(&s, 1);
    send(
        &s,
        1,
        vec![
            timeline(),
            json!({"method":"play-timeline","id":1}),
            json!({"method":"barrier","ram":true,"id":2}),
        ],
    );
    render(&s, &mut r, 10);
    s.dispose_extra_snes(1).unwrap();
    render(&s, &mut r, 1);
    s.extra_psg.collect_retired().unwrap();
    assert_eq!(s.extra_psg.memory_used(), 0);
    assert!(s.create_extra_snes(1).is_err());
    assert!(s.extra_snes_status(1, None, false).is_err());
    s.create_extra_snes(2).unwrap();
    let a = capture(&s, &mut r, 2, 1);
    assert!(a.ram.unwrap().iter().all(|v| *v == 0));
    assert!(!a.timeline.loaded);
}
#[test]
fn extra_snes_completion_wait_releases_control_and_history_expires() {
    use std::{sync::mpsc, time::Duration};
    let (s, mut r) = new();
    let s = Arc::new(s);
    s.enqueue(
        1,
        &[cmd(
            1,
            1000,
            json!({"method":"write","register":0,"value":77,"id":1}),
        )],
    )
    .unwrap();
    send(
        &s,
        1,
        (2..=601)
            .map(|id| json!({"method":"barrier","id":id}))
            .collect(),
    );
    render(&s, &mut r, 1);
    state(&s, 1);
    assert!(s
        .extra_snes_status(1, Some(2), false)
        .unwrap_err()
        .contains("expired"));
    let (tx, rx) = mpsc::channel();
    let waiting = s.clone();
    let task = std::thread::spawn(move || {
        // A completed command can still overlap snapshot publication. Honor
        // the bounded reader's retry contract if CI deschedules the writer;
        // every other error must still reach the assertion below.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let retry = "Native additional SNES state changed while reading; retry";
        let result = loop {
            let result = waiting.extra_snes_status(1, Some(1), false);
            if !matches!(&result, Err(error) if error == retry) {
                break result;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "concurrent SNES state publication did not finish"
            );
            std::thread::yield_now();
        };
        tx.send(result).unwrap()
    });
    assert!(rx.recv_timeout(Duration::from_millis(20)).is_err());
    send(&s, 1, vec![write(1, 66)]);
    render(&s, &mut r, 1100);
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap()
            .snes
            .registers[0],
        77
    );
    task.join().unwrap();
}
#[test]
fn extra_snes_failed_other_family_retain_does_not_commit_cancel() {
    let (s, mut r) = new();
    s.create_extra_okim6258(2, None).unwrap();
    s.enqueue(
        1,
        &[cmd(
            1,
            1000,
            group(json!({"method":"write","register":0,"value":77,"id":1}), 42),
        )],
    )
    .unwrap();
    let oki = |m| {
        serde_json::from_value::<Command>(
            json!({"type":"extra-okim6258","instance":2,"frame":0,"message":m}),
        )
        .unwrap()
    };
    s.enqueue(1,&[oki(json!({"method":"batch","channel":0,"voice":1,"audition":true,"action":"on","commands":[{"method":"write","register":0,"value":2}]}))]).unwrap();
    send(
        &s,
        1,
        vec![group(json!({"method":"barrier","ram":true,"id":2}), 42)],
    );
    render(&s, &mut r, 1);
    assert!(s
        .enqueue(
            1,
            &[
                cmd(1, 0, json!({"method":"cancel-main-owner","targetGroup":42})),
                oki(json!({"method":"clock","hz":4000000}))
            ]
        )
        .unwrap_err()
        .contains("manually held"));
    assert!(
        s.extra_snes_status(1, Some(2), true).is_ok(),
        "Rejected mixed-family transaction consumed existing RAM capture"
    );
    render(&s, &mut r, 1100);
    assert_eq!(
        s.extra_snes_status(1, Some(1), false)
            .unwrap()
            .snes
            .registers[0],
        77
    );
}
#[test]
fn extra_snes_timeline_pcm_matches_existing_primary_at_three_rates() {
    for rate in [44100, 48000, 96000] {
        let s = Shared::new(rate, 1);
        let mut r = Renderer::new(rate).unwrap();
        s.create_extra_snes(1).unwrap();
        let p = Shared::new_modes(rate, 1, false, false, false, Some("snes"));
        let mut pr = Renderer::new_modes(rate, false, false, false, Some("snes")).unwrap();
        let messages = [timeline(), json!({"method":"play-timeline","id":1})];
        s.enqueue(
            1,
            &messages
                .iter()
                .map(|m| cmd(1, 0, m.clone()))
                .collect::<Vec<_>>(),
        )
        .unwrap();
        p.enqueue(
            1,
            &messages
                .iter()
                .map(|m| {
                    serde_json::from_value(json!({"type":"snes","frame":0,"message":m})).unwrap()
                })
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert_eq!(
            render(&s, &mut r, 4096),
            render(&p, &mut pr, 4096),
            "rate {rate}"
        );
    }
}
#[test]
fn extra_snes_cancel_races_do_not_permanently_fill_snapshot_slots() {
    let (s, mut r) = new();
    s.performance("start", 7, None).unwrap();
    for cycle in 1..=20 {
        let group = cycle as u64;
        send(&s, 1, vec![grouped_capture(group, group)]);
        render(&s, &mut r, 1);
        send(
            &s,
            1,
            vec![json!({"method":"cancel-main-owner","targetGroup":group})],
        );
        render(&s, &mut r, 1);
        s.extra_psg.collect_retired().unwrap();
        assert_eq!(s.extra_psg.memory_used(), extra_snes::INSTANCE_BYTES);
    }
    fn grouped_capture(id: u64, group_id: u64) -> Value {
        group(json!({"method":"barrier","ram":true,"id":id}), group_id)
    }
}
#[test]
fn extra_snes_upload_batch_limits_count_pending_and_retiring_storage() {
    let (s, mut r) = new();
    let large = json!({"method":"load-memory","address":0,"data":vec![0x5a;65536]});
    assert!(s
        .enqueue(
            1,
            &[cmd(1, 100000, large.clone()), cmd(1, 100000, large.clone())]
        )
        .unwrap_err()
        .contains("64 KiB"));
    for _ in 0..16 {
        s.enqueue(1, &[cmd(1, 100000, large.clone())]).unwrap();
    }
    assert!(s
        .enqueue(1, &[cmd(1, 100000, large)])
        .unwrap_err()
        .contains("1 MiB"));
    render(&s, &mut r, 1);
    s.dispose_extra_snes(1).unwrap();
    render(&s, &mut r, 1);
    s.extra_psg.collect_retired().unwrap();
    assert_eq!(s.extra_psg.memory_used(), 0);
}
#[test]
fn extra_snes_callback_capture_and_timeline_fit_audio_stack() {
    const ENV: &str = "TETORICA_EXTRA_SNES_SMALL_STACK";
    if std::env::var_os(ENV).is_none() {
        let result=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","native_genesis::extra_snes_tests::extra_snes_callback_capture_and_timeline_fit_audio_stack","--nocapture"]).env(ENV,"1").output().unwrap();
        assert!(
            result.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        return;
    }
    let (s, r) = new();
    let s = Arc::new(s);
    configure(&s, 1);
    send(
        &s,
        1,
        vec![
            on(0, 1, false),
            json!({"method":"barrier","ram":true,"id":1}),
        ],
    );
    let shared = s.clone();
    let mut r = Box::new(r);
    r = std::thread::Builder::new()
        .stack_size(64 * 1024)
        .spawn(move || {
            r.prepare(&shared);
            for _ in 0..256 {
                r.sample(&shared);
            }
            r.prepare(&shared);
            r
        })
        .unwrap()
        .join()
        .unwrap();
    assert!(s.extra_snes_status(1, Some(1), true).is_ok());
    send(
        &s,
        1,
        vec![
            timeline(),
            json!({"method":"play-timeline","id":2}),
            json!({"method":"barrier","ram":true,"id":3}),
        ],
    );
    let shared = s.clone();
    r = std::thread::Builder::new()
        .stack_size(64 * 1024)
        .spawn(move || {
            r.prepare(&shared);
            for _ in 0..256 {
                r.sample(&shared);
            }
            r.prepare(&shared);
            r
        })
        .unwrap()
        .join()
        .unwrap();
    assert!(s.extra_snes_status(1, Some(3), true).is_ok());
    drop(r);
}
#[test]
fn extra_snes_snapshot_transfer_cost_and_actual_fixed_heap_are_bounded() {
    let (s, mut r) = new();
    configure(&s, 1);
    render(&s, &mut r, 1);
    let actual = std::mem::size_of::<extra_psg::Instance>() + 67280 + 2 * CAPACITY * 16 + 512 * 16;
    println!(
        "SNES fixedactualupper={} reserved={}",
        actual,
        extra_snes::INSTANCE_BYTES
    );
    assert!(actual < extra_snes::INSTANCE_BYTES);
    let mut one = [0u128; 32];
    let mut burst = [0u128; 32];
    let mut next = 1u64;
    for count in [1, 16] {
        for sample in if count == 1 { &mut one } else { &mut burst } {
            let ids = (next..next + count).collect::<Vec<_>>();
            next += count;
            send(
                &s,
                1,
                ids.iter()
                    .map(|id| json!({"method":"barrier","ram":true,"id":id}))
                    .collect(),
            );
            r.prepare(&s);
            let start = std::time::Instant::now();
            r.sample(&s);
            *sample = start.elapsed().as_nanos();
            for id in ids {
                s.extra_snes_status(1, Some(id), true).unwrap();
            }
        }
    }
    one.sort_unstable();
    burst.sort_unstable();
    println!("SNES debug one-shot capture 32trials:1x64KiB median={}ns p97={}ns max={}ns;16x64KiB median={}ns p97={}ns max={}ns",one[16],one[30],one[31],burst[16],burst[30],burst[31]);
}
