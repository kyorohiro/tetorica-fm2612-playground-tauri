//! Full rack tests use real standalone C++ DSP without an audio device.
use super::*;
use serde_json::{json, Value};
fn cmd(id: u64, frame: u64, m: Value) -> Command {
    serde_json::from_value(json!({"type":"extra-okim6258","instance":id,"frame":frame,"message":m}))
        .unwrap()
}
fn send(s: &Shared, id: u64, ms: Vec<Value>) {
    s.enqueue(
        1,
        &ms.into_iter().map(|m| cmd(id, 0, m)).collect::<Vec<_>>(),
    )
    .unwrap();
}
fn render(r: &mut Renderer, s: &Shared, n: usize) -> Vec<[f32; 2]> {
    let out = (0..n)
        .map(|i| {
            if i % 128 == 0 {
                r.prepare(s)
            }
            r.sample(s)
        })
        .collect();
    r.prepare(s);
    out
}
fn new() -> (Shared, Renderer) {
    let s = Shared::new(48000, 1);
    let r = Renderer::new(48000).unwrap();
    s.create_extra_okim6258(1, None).unwrap();
    (s, r)
}
fn state(s: &Shared, id: u64) -> extra_okim6258::ChipStatus {
    s.extra_okim6258_status(id, None).unwrap().okim6258
}
fn wave(id: u64, len: usize) -> Value {
    json!({"method":"load-memory","sampleId":id,"data":vec![0x17u8;len],"metadata":{"frames":len*2,"sampleRate":15625,"source":"raw","name":format!("sample-{id}"),"waveform":[0.,0.2,-0.1]}})
}
fn audition(s: &Shared, voice: u64, action: &str, commands: Vec<Value>) {
    send(
        s,
        1,
        vec![
            json!({"method":"batch","channel":0,"voice":voice,"audition":true,"action":action,"commands":commands}),
        ],
    )
}
fn group(mut m: Value, id: u64) -> Value {
    m["group"] = json!(id);
    m
}
fn energy(p: &[[f32; 2]], ch: usize) -> f32 {
    p.iter().map(|p| p[ch].abs()).sum()
}
#[test]
fn extra_oki_layout_and_fourteen_kinds_share_sixteen_slots() {
    let (s, mut r) = new();
    s.create_extra_okim6258(2, Some(4_000_000)).unwrap();
    s.create_extra_psg(3, None).unwrap();
    s.create_extra_ym2612(4, None).unwrap();
    s.create_extra_ym2203(5, None).unwrap();
    s.create_extra_ym2608(6, None).unwrap();
    s.create_extra_ym2610(7, None).unwrap();
    s.create_extra_ym2151(8, None).unwrap();
    s.create_extra_ym2413(9, None).unwrap();
    s.create_extra_ay8910(10, None).unwrap();
    s.create_extra_ym3526(11, None).unwrap();
    s.create_extra_ym3812(12, None).unwrap();
    s.create_extra_y8950(13, None).unwrap();
    s.create_extra_ymf262(14, None).unwrap();
    s.create_extra_rf5c164(15, None).unwrap();
    s.create_extra_okim6258(16, None).unwrap();
    assert!(s
        .create_extra_okim6258(17, None)
        .unwrap_err()
        .contains("limit"));
    send(
        &s,
        1,
        vec![
            wave(1, 512),
            json!({"method":"write","register":2,"value":1}),
            json!({"method":"play","repeat":true}),
        ],
    );
    send(
        &s,
        2,
        vec![
            wave(1, 1024),
            json!({"method":"write","register":2,"value":2}),
            json!({"method":"play","repeat":true}),
        ],
    );
    let p = render(&mut r, &s, 1000);
    assert!(energy(&p, 0) > 1. && energy(&p, 1) > 1.);
    assert_eq!(state(&s, 1).clock, 8_000_000);
    assert_eq!(state(&s, 2).clock, 4_000_000);
    assert_eq!(state(&s, 1).sample.unwrap().bytes, 512);
    assert_eq!(state(&s, 2).sample.unwrap().bytes, 1024);
    assert!(r.x68000.is_none());
    assert_eq!(std::mem::size_of::<extra_okim6258::CoreState>(), 28);
    println!(
        "OKI layout Instance={} Shared={} Renderer={} session={}",
        std::mem::size_of::<extra_psg::Instance>(),
        std::mem::size_of::<extra_okim6258::Shared>(),
        std::mem::size_of::<extra_okim6258::Renderer>(),
        std::mem::size_of::<Renderer>()
    );
}
#[test]
fn extra_oki_reset_retains_sample_and_clock_and_does_not_retrigger() {
    let (s, mut r) = new();
    send(
        &s,
        1,
        vec![
            wave(1, 128),
            json!({"method":"clock","hz":4_000_000}),
            json!({"method":"write","register":12,"value":0}),
            json!({"method":"write","register":2,"value":1}),
            json!({"method":"play","repeat":true}),
        ],
    );
    assert!(energy(&render(&mut r, &s, 2000), 0) > 1.);
    send(&s, 1, vec![json!({"method":"reset"})]);
    let p = render(&mut r, &s, 1000);
    assert!(energy(&p, 0) < 1e-6);
    let v = state(&s, 1);
    assert!(!v.playing);
    assert_eq!((v.clock, v.divider, v.pan), (4_000_000, 512, 0));
    assert_eq!(v.sample_id, 1);
    send(&s, 1, vec![json!({"method":"play"})]);
    assert!(energy(&render(&mut r, &s, 1000), 0) > 1.);
}
#[test]
fn extra_oki_nonrepeat_exhausts_with_end_silence_and_keeps_metadata() {
    let (s, mut r) = new();
    send(&s, 1, vec![wave(1, 16), json!({"method":"play"})]);
    let p = render(&mut r, &s, 1000);
    assert!(energy(&p, 0) > 0.01);
    assert!(energy(&p[500..], 0) < 1e-6);
    let v = state(&s, 1);
    assert!(!v.playing && !v.streaming);
    assert_eq!(v.sample.unwrap().name.as_deref(), Some("sample-1"));
}
#[test]
fn extra_oki_manual_lease_stale_off_and_foreign_release_are_ignored() {
    let (s, mut r) = new();
    send(&s, 1, vec![wave(1, 128)]);
    render(&mut r, &s, 1);
    audition(&s, 1, "on", vec![json!({"method":"play","repeat":true})]);
    render(&mut r, &s, 100);
    audition(&s, 2, "on", vec![json!({"method":"play","repeat":true})]);
    render(&mut r, &s, 100);
    audition(&s, 1, "off", vec![json!({"method":"stop"})]);
    send(
        &s,
        1,
        vec![
            group(json!({"method":"stop"}), 1),
            json!({"method":"stop-voices"}),
            json!({"method":"reset"}),
            json!({"method":"write","register":0,"value":1}),
        ],
    );
    render(&mut r, &s, 100);
    assert!(state(&s, 1).playing);
    assert_eq!(state(&s, 1).held, [0]);
    audition(&s, 2, "off", vec![json!({"method":"stop"})]);
    render(&mut r, &s, 100);
    assert!(!state(&s, 1).playing);
}
#[test]
fn extra_oki_held_memory_and_clock_admission_rejections_preserve_sample() {
    let (s, mut r) = new();
    send(&s, 1, vec![wave(1, 128)]);
    render(&mut r, &s, 1);
    audition(&s, 1, "on", vec![json!({"method":"play","repeat":true})]);
    render(&mut r, &s, 1);
    assert!(s
        .enqueue(1, &[cmd(1, 0, wave(2, 256))])
        .unwrap_err()
        .contains("replacing their sample"));
    assert!(s
        .enqueue(1, &[cmd(1, 0, json!({"method":"clock","hz":4_000_000}))])
        .unwrap_err()
        .contains("manually held"));
    assert_eq!(state(&s, 1).sample_id, 1);
    assert_eq!(state(&s, 1).memory_revision, 1);
    assert_eq!(state(&s, 1).clock, 8_000_000);
}
#[test]
fn extra_oki_scheduled_memory_and_clock_recheck_manual_hold_at_application() {
    let (s, mut r) = new();
    send(&s, 1, vec![wave(1, 128)]);
    let mut load = wave(2, 256);
    load["id"] = json!(10);
    s.enqueue(
        1,
        &[
            cmd(1, 1000, load),
            cmd(1, 1000, json!({"method":"clock","hz":4_000_000,"id":11})),
        ],
    )
    .unwrap();
    audition(&s, 1, "on", vec![json!({"method":"play","repeat":true})]);
    render(&mut r, &s, 1200);
    assert!(s
        .extra_okim6258_status(1, Some(10))
        .unwrap_err()
        .contains("replacing their sample"));
    assert!(s
        .extra_okim6258_status(1, Some(11))
        .unwrap_err()
        .contains("manually held"));
    let v = state(&s, 1);
    assert_eq!((v.sample_id, v.memory_revision, v.clock), (1, 1, 8_000_000));
    assert_eq!(v.sample.unwrap().bytes, 128);
}
#[test]
fn extra_oki_full_batch_rejects_invalid_sample_without_partial_writes() {
    let (s, mut r) = new();
    for invalid in [
        json!({"method":"write","register":13,"value":1}),
        json!({"method":"load-memory","sampleId":1,"data":[],"metadata":{"frames":0,"sampleRate":15625,"source":"raw"}}),
        json!({"method":"load-memory","sampleId":1,"data":[1],"metadata":{"frames":7,"sampleRate":15625,"source":"raw"}}),
        json!({"method":"batch","channel":1,"voice":1,"action":"on","commands":[{"method":"play"}]}),
    ] {
        assert!(s
            .enqueue(
                1,
                &[
                    cmd(1, 0, json!({"method":"write","register":2,"value":3})),
                    cmd(1, 0, invalid)
                ]
            )
            .is_err());
        render(&mut r, &s, 10);
        assert_eq!(state(&s, 1).pan, 0);
        assert_eq!(state(&s, 1).sample_id, 0);
    }
}
#[test]
fn extra_oki_cancel_main_future_upload_completes_wait_and_reclaims_payload() {
    let (s, mut r) = new();
    render(&mut r, &s, 1);
    let baseline = s.extra_psg.memory_used();
    let mut load = group(wave(1, 1024 * 1024), 42);
    load["id"] = json!(1);
    s.enqueue(1, &[cmd(1, 100_000, load)]).unwrap();
    assert!(s.extra_psg.memory_used() > baseline + 1024 * 1024);
    send(
        &s,
        1,
        vec![json!({"method":"cancel-main-owner","targetGroup":42})],
    );
    render(&mut r, &s, 128);
    state(&s, 1);
    s.extra_psg.reserve_payloads(std::iter::empty()).unwrap();
    assert_eq!(state(&s, 1).sample_id, 0);
    assert_eq!(s.extra_psg.memory_used(), baseline);
    assert!(s
        .extra_okim6258_status(1, Some(1))
        .unwrap_err()
        .contains("cancelled"));
}
#[test]
fn extra_oki_loaded_sample_accounted_until_replacement_control_collection() {
    let (s, mut r) = new();
    render(&mut r, &s, 1);
    let base = s.extra_psg.memory_used();
    send(&s, 1, vec![wave(1, 1024 * 1024)]);
    render(&mut r, &s, 1);
    state(&s, 1);
    s.extra_psg.reserve_payloads(std::iter::empty()).unwrap();
    let loaded = s.extra_psg.memory_used();
    assert!(loaded > base + 1024 * 1024);
    send(&s, 1, vec![wave(2, 128)]);
    assert!(s.extra_psg.memory_used() > loaded);
    render(&mut r, &s, 1);
    state(&s, 1);
    s.extra_psg.reserve_payloads(std::iter::empty()).unwrap();
    assert!(s.extra_psg.memory_used() < base + 4096);
    assert_eq!(state(&s, 1).sample.unwrap().bytes, 128);
}
#[test]
fn extra_oki_all_family_retention_failure_does_not_cancel_prior_lookahead() {
    let (s, mut r) = new();
    s.create_extra_rf5c164(2, None).unwrap();
    let rf = |m: Value| {
        serde_json::from_value::<Command>(
            json!({"type":"extra-rf5c164","instance":2,"frame":0,"message":m}),
        )
        .unwrap()
    };
    s.enqueue(1,&[rf(json!({"method":"batch","channel":0,"voice":1,"action":"on","audition":true,"commands":[{"method":"key-on","channel":0}]}))]).unwrap();
    s.enqueue(1, &[cmd(1, 1000, group(wave(1, 128), 42))])
        .unwrap();
    render(&mut r, &s, 1);
    assert!(s
        .enqueue(
            1,
            &[
                cmd(1, 0, json!({"method":"cancel-main-owner","targetGroup":42})),
                rf(json!({"method":"clock","hz":10_000_000}))
            ]
        )
        .unwrap_err()
        .contains("manually held"));
    render(&mut r, &s, 1200);
    assert_eq!(state(&s, 1).sample_id, 1);
}
#[test]
fn extra_oki_old_and_new_samples_share_the_global_budget() {
    let (s, mut r) = new();
    render(&mut r, &s, 1);
    for i in 1..=3 {
        send(&s, 1, vec![wave(i, 16 * 1024 * 1024)]);
        if i == 1 {
            render(&mut r, &s, 1);
        }
    }
    assert!(s
        .enqueue(1, &[cmd(1, 0, wave(4, 16 * 1024 * 1024))])
        .unwrap_err()
        .contains("budget"));
    render(&mut r, &s, 1);
    state(&s, 1);
    s.extra_psg.reserve_payloads(std::iter::empty()).unwrap();
    assert!(s.extra_psg.memory_used() < 18 * 1024 * 1024);
    assert_eq!(state(&s, 1).sample_id, 3);
}

#[test]
fn extra_oki_worker_cancel_future_upload_preserves_manual_voice_and_reclaims() {
    let (s, mut r) = new();
    s.performance("start", 7, None).unwrap();
    send(&s, 1, vec![wave(1, 128)]);
    render(&mut r, &s, 1);
    audition(&s, 1, "on", vec![json!({"method":"play","repeat":true})]);
    let mut upload = wave(2, 1024 * 1024);
    upload["id"] = json!(1);
    let mut c = cmd(1, 100_000, upload);
    c.performance = Some(7);
    c.owner = Some(1);
    s.enqueue(1, &[c]).unwrap();
    render(&mut r, &s, 1);
    s.performance("cancel-owner", 7, Some(1)).unwrap();
    render(&mut r, &s, 1);
    assert!(s
        .extra_okim6258_status(1, Some(1))
        .unwrap_err()
        .contains("cancelled"));
    assert!(state(&s, 1).playing);
    assert_eq!(state(&s, 1).sample_id, 1);
    assert_eq!(state(&s, 1).held, [0]);
    state(&s, 1);
    s.extra_psg.reserve_payloads(std::iter::empty()).unwrap();
    assert!(s.extra_psg.memory_used() < extra_budget::INSTANCE_BYTES + 4096);
}
#[test]
fn extra_oki_clock_boundary_at_eight_khz_is_safe_after_reset() {
    let s = Shared::new(8000, 1);
    let mut r = Renderer::new(8000).unwrap();
    assert!(s
        .create_extra_okim6258(1, Some(57_344_001))
        .unwrap_err()
        .contains("FIFO"));
    s.create_extra_okim6258(1, Some(57_344_000)).unwrap();
    render(&mut r, &s, 1);
    assert_eq!(state(&s, 1).clock, 57_344_000);
    assert!(s
        .enqueue(1, &[cmd(1, 0, json!({"method":"clock","hz":57_344_001}))])
        .unwrap_err()
        .contains("FIFO"));
    send(
        &s,
        1,
        vec![json!({"method":"write","register":12,"value":0})],
    );
    let bytes = 57_344_001u32.to_le_bytes();
    send(
        &s,
        1,
        (0..4)
            .map(|i| json!({"method":"write","register":8+i,"value":bytes[i]}))
            .collect(),
    );
    send(&s, 1, vec![json!({"method":"reset"})]);
    render(&mut r, &s, 1);
    assert_eq!(state(&s, 1).clock, 57_344_000);
    assert_eq!(state(&s, 1).divider, 512);
}
#[test]
fn extra_oki_after_wait_runs_outside_control_queue_and_observes_actual_sample() {
    use std::{sync::mpsc, time::Duration};
    let (s, mut r) = new();
    let s = Arc::new(s);
    let mut load = wave(1, 128);
    load["id"] = json!(1);
    s.enqueue(1, &[cmd(1, 1000, load)]).unwrap();
    let (tx, rx) = mpsc::channel();
    let waiting = s.clone();
    let task =
        std::thread::spawn(move || tx.send(waiting.extra_okim6258_status(1, Some(1))).unwrap());
    assert!(rx.recv_timeout(Duration::from_millis(20)).is_err());
    send(
        &s,
        1,
        vec![json!({"method":"write","register":2,"value":1})],
    );
    render(&mut r, &s, 1200);
    assert_eq!(
        rx.recv_timeout(Duration::from_millis(200))
            .unwrap()
            .unwrap()
            .okim6258
            .sample_id,
        1
    );
    task.join().unwrap();
}
#[test]
fn extra_oki_dispose_replacement_isolated_and_sample_budget_reclaimed_on_control() {
    let (s, mut r) = new();
    send(
        &s,
        1,
        vec![wave(1, 1024 * 1024), json!({"method":"play","repeat":true})],
    );
    render(&mut r, &s, 100);
    s.dispose_extra_okim6258(1).unwrap();
    render(&mut r, &s, 1);
    s.extra_psg.collect_retired().unwrap();
    assert_eq!(s.extra_psg.counts(), (0, 0));
    assert_eq!(s.extra_psg.memory_used(), 0);
    assert!(s.create_extra_okim6258(1, None).is_err());
    s.create_extra_okim6258(2, None).unwrap();
    send(
        &s,
        2,
        vec![wave(1, 128), json!({"method":"play","repeat":true})],
    );
    render(&mut r, &s, 100);
    assert!(state(&s, 2).playing);
    assert!(s.extra_okim6258_status(1, None).is_err());
}
#[test]
fn extra_oki_finite_preview_releases_manual_hold_and_admits_new_sample_clock() {
    let (s, mut r) = new();
    send(&s, 1, vec![wave(1, 16)]);
    render(&mut r, &s, 1);
    audition(&s, 1, "on", vec![json!({"method":"play"})]);
    render(&mut r, &s, 1000);
    assert!(!state(&s, 1).playing);
    assert!(state(&s, 1).held.is_empty());
    send(
        &s,
        1,
        vec![wave(2, 32), json!({"method":"clock","hz":4_000_000})],
    );
    render(&mut r, &s, 1);
    assert_eq!(state(&s, 1).sample_id, 2);
    assert_eq!(state(&s, 1).clock, 4_000_000);
}
#[test]
fn extra_oki_foreign_raw_zero_stop_cannot_release_another_main_group() {
    let (s, mut r) = new();
    send(
        &s,
        1,
        vec![
            wave(1, 128),
            group(json!({"method":"play","repeat":true}), 1),
        ],
    );
    render(&mut r, &s, 10);
    send(
        &s,
        1,
        vec![group(json!({"method":"write","register":0,"value":0}), 2)],
    );
    render(&mut r, &s, 10);
    assert!(state(&s, 1).playing);
    send(
        &s,
        1,
        vec![group(json!({"method":"write","register":0,"value":0}), 1)],
    );
    render(&mut r, &s, 10);
    assert!(!state(&s, 1).playing);
}
#[test]
fn extra_oki_rejected_retention_upload_with_id_reclaims_without_after_wait() {
    let (s, mut r) = new();
    s.create_extra_rf5c164(2, None).unwrap();
    let rf = |m: Value| {
        serde_json::from_value::<Command>(
            json!({"type":"extra-rf5c164","instance":2,"frame":0,"message":m}),
        )
        .unwrap()
    };
    s.enqueue(1,&[rf(json!({"method":"batch","channel":0,"voice":1,"audition":true,"action":"on","commands":[{"method":"key-on","channel":0}]}))]).unwrap();
    render(&mut r, &s, 1);
    s.extra_psg.reserve_payloads(std::iter::empty()).unwrap();
    let base = s.extra_psg.memory_used();
    let mut load = wave(1, 1024 * 1024);
    load["id"] = json!(1);
    assert!(s
        .enqueue(
            1,
            &[
                cmd(1, 0, load),
                rf(json!({"method":"clock","hz":10_000_000}))
            ]
        )
        .is_err());
    state(&s, 1);
    s.extra_psg.reserve_payloads(std::iter::empty()).unwrap();
    assert_eq!(state(&s, 1).sample_id, 0);
    assert_eq!(s.extra_psg.memory_used(), base);
}

#[test]
fn extra_oki_completion_ring_expires_old_outcomes_but_keeps_future_waiters_correct() {
    use std::{sync::mpsc, time::Duration};
    let (s, mut r) = new();
    let s = Arc::new(s);
    let mut future = wave(1, 128);
    future["id"] = json!(1);
    s.enqueue(1, &[cmd(1, 1000, future)]).unwrap();
    let writes = (2..=601)
        .map(|id| {
            cmd(
                1,
                0,
                json!({"method":"write","register":2,"value":1,"id":id}),
            )
        })
        .collect::<Vec<_>>();
    s.enqueue(1, &writes).unwrap();
    render(&mut r, &s, 1);
    state(&s, 1);
    s.extra_psg.reserve_payloads(std::iter::empty()).unwrap();
    assert!(s
        .extra_okim6258_status(1, Some(2))
        .unwrap_err()
        .contains("expired"));
    assert!(s.extra_okim6258_status(1, Some(601)).is_ok());
    let (tx, rx) = mpsc::channel();
    let waiting = s.clone();
    let task =
        std::thread::spawn(move || tx.send(waiting.extra_okim6258_status(1, Some(1))).unwrap());
    assert!(rx.recv_timeout(Duration::from_millis(20)).is_err());
    render(&mut r, &s, 1200);
    assert_eq!(
        rx.recv_timeout(Duration::from_millis(200))
            .unwrap()
            .unwrap()
            .okim6258
            .sample_id,
        1
    );
    task.join().unwrap();
}
#[test]
fn extra_oki_inspector_clock_is_a_standalone_audition_command_preserving_lease() {
    let (s, mut r) = new();
    send(&s, 1, vec![wave(1, 128)]);
    render(&mut r, &s, 1);
    audition(&s, 7, "on", vec![json!({"method":"play","repeat":true})]);
    render(&mut r, &s, 1);
    send(
        &s,
        1,
        vec![json!({"method":"clock","hz":4_000_000,"audition":true,"id":100})],
    );
    render(&mut r, &s, 1);
    let v = s.extra_okim6258_status(1, Some(100)).unwrap().okim6258;
    assert_eq!((v.clock, v.sample_id), (4_000_000, 1));
    assert_eq!(v.held, [0]);
    assert!(v.playing);
    assert!(s
        .enqueue(
            1,
            &[cmd(1, 0, json!({"method":"clock","hz":8_000_000,"id":101}))]
        )
        .unwrap_err()
        .contains("manually held"));
    audition(&s, 7, "off", vec![json!({"method":"stop"})]);
    render(&mut r, &s, 1);
    assert!(!state(&s, 1).playing);
}
