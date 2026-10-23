//! Offline full-rack tests: independent PCM RAM, exact control semantics,
//! scoped voice ownership, upload accounting and callback-safe retirement.
use super::*;
use serde_json::{json, Value};
fn command(id: u64, frame: u64, m: Value) -> Command {
    serde_json::from_value(json!({"type":"extra-rf5c164","instance":id,"frame":frame,"message":m}))
        .unwrap()
}
fn send(s: &Shared, id: u64, ms: Vec<Value>) {
    s.enqueue(
        1,
        &ms.into_iter()
            .map(|m| command(id, 0, m))
            .collect::<Vec<_>>(),
    )
    .unwrap();
}
fn render(r: &mut Renderer, s: &Shared, n: usize) -> Vec<[f32; 2]> {
    let p = (0..n)
        .map(|i| {
            if i % 128 == 0 {
                r.prepare(s)
            }
            r.sample(s)
        })
        .collect();
    r.prepare(s);
    p
}
fn stop(s: &Shared, r: &mut Renderer) {
    let epoch = s.epoch.load(Ordering::Acquire);
    std::thread::scope(|scope| {
        let task = scope.spawn(|| s.stop());
        while s.epoch.load(Ordering::Acquire) == epoch {
            std::thread::yield_now()
        }
        render(r, s, 1024);
        task.join().unwrap().unwrap();
    });
}
fn state(s: &Shared, id: u64) -> extra_rf5c164::ChipStatus {
    s.extra_rf5c164_status(id, None).unwrap().rf5c164
}
fn new(clock: Option<u32>) -> (Shared, Renderer) {
    let s = Shared::new(48000, 1);
    let r = Renderer::new(48000).unwrap();
    s.create_extra_rf5c164(1, clock).unwrap();
    (s, r)
}
fn patch(ch: u8, start: u32, pan: [u8; 2]) -> Value {
    json!({"method":"set-channel","channel":ch,"options":{"start":start,"loopStart":start,"step":2048,"volume":100,"pan":{"left":pan[0],"right":pan[1]}}})
}
fn wave(offset: u32, value: u8) -> Value {
    let mut data = vec![value; 32];
    data.push(255);
    json!({"method":"load-memory","offset":offset,"data":data})
}
fn group(mut m: Value, g: u64) -> Value {
    m["group"] = json!(g);
    m
}
fn audition(s: &Shared, ch: u8, v: u64, action: &str, commands: Vec<Value>) {
    send(
        s,
        1,
        vec![
            json!({"method":"batch","audition":true,"channel":ch,"voice":v,"action":action,"commands":commands}),
        ],
    );
}
fn note(ch: u8, start: u32, pan: [u8; 2]) -> Vec<Value> {
    vec![
        patch(ch, start, pan),
        json!({"method":"key-on","channel":ch}),
    ]
}
fn energy(p: &[[f32; 2]], ch: usize) -> f32 {
    p.iter().map(|p| p[ch].abs()).sum()
}
#[test]
fn extra_rf_layout() {
    assert_eq!(std::mem::size_of::<extra_rf5c164::CoreState>(), 100);
    println!(
        "RF layout Instance={} Shared={} Renderer={} session={}",
        std::mem::size_of::<extra_psg::Instance>(),
        std::mem::size_of::<extra_rf5c164::Shared>(),
        std::mem::size_of::<extra_rf5c164::Renderer>(),
        std::mem::size_of::<Renderer>()
    );
}
#[test]
fn extra_rf_thirteen_kinds_share_sixteen_slots_and_two_pcm_rams() {
    let (s, mut r) = new(None);
    s.create_extra_rf5c164(2, Some(10_000_000)).unwrap();
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
    s.create_extra_rf5c164(16, None).unwrap();
    assert!(s
        .create_extra_rf5c164(17, None)
        .unwrap_err()
        .contains("limit"));
    send(&s, 1, vec![wave(0, 0xc0)]);
    send(&s, 2, vec![wave(0, 0x40)]);
    send(&s, 1, note(0, 0, [15, 0]));
    send(&s, 2, note(7, 0, [0, 15]));
    let pcm = render(&mut r, &s, 1000);
    assert!(pcm.iter().any(|p| p[0] > 0. && p[1] < 0.));
    assert_eq!(state(&s, 1).clock, 12_500_000);
    assert_eq!(state(&s, 2).clock, 10_000_000);
    assert_eq!(s.extra_psg.counts(), (16, 16));
    assert!(r.rf5c164.is_none());
}
#[test]
fn extra_rf_high_select_and_raw_bank_applied_exactly() {
    let (s, mut r) = new(None);
    send(&s, 1, vec![patch(3, 256, [2, 9])]);
    render(&mut r, &s, 1);
    let v = state(&s, 1);
    assert!(v.enabled);
    assert_eq!(v.selected, 3);
    assert_eq!(v.channels[3].step, 2048);
    send(
        &s,
        1,
        vec![
            json!({"method":"key-on","channel":5}),
            json!({"method":"write","register":0,"value":88}),
            json!({"method":"write","register":7,"value":0x82}),
            json!({"method":"write-memory","offset":5,"value":255}),
        ],
    );
    render(&mut r, &s, 1);
    let v = state(&s, 1);
    assert_eq!(v.selected, 5);
    assert_eq!(v.channels[5].volume, 88);
    assert_eq!(v.bank, 8192);
    assert!(v
        .known_memory_ranges
        .iter()
        .any(|r| r.start == 8197 && r.end == 8198));
}
#[test]
fn extra_rf_reset_retains_ram_and_clock_without_retrigger() {
    let (s, mut r) = new(Some(10_000_000));
    send(&s, 1, vec![wave(0, 0xc0)]);
    send(&s, 1, note(0, 0, [15, 15]));
    assert!(energy(&render(&mut r, &s, 1000), 0) > 1.);
    send(&s, 1, vec![json!({"method":"reset"})]);
    let pcm = render(&mut r, &s, 1000);
    assert!(energy(&pcm[500..], 0) < 1e-6);
    let v = state(&s, 1);
    assert_eq!(v.clock, 10_000_000);
    assert_eq!(v.mask, 255);
    assert_eq!(v.channels[0].configured_step, 0);
    assert_eq!(v.known_memory_ranges.len(), 1);
    send(&s, 1, note(0, 0, [15, 15]));
    assert!(energy(&render(&mut r, &s, 1000), 0) > 1.);
}
#[test]
fn extra_rf_preview_replacement_stale_off_and_configured_pitch() {
    let (s, mut r) = new(None);
    send(&s, 1, vec![wave(0, 0xc0), patch(0, 0, [15, 15])]);
    render(&mut r, &s, 1);
    audition(
        &s,
        0,
        1,
        "on",
        vec![
            json!({"method":"set-channel","channel":0,"options":{"step":4096}}),
            json!({"method":"key-on","channel":0}),
        ],
    );
    render(&mut r, &s, 100);
    assert_eq!(state(&s, 1).channels[0].configured_step, 2048);
    audition(&s, 0, 2, "on", vec![json!({"method":"key-on","channel":0})]);
    audition(
        &s,
        0,
        1,
        "off",
        vec![json!({"method":"key-off","channel":0})],
    );
    render(&mut r, &s, 100);
    assert_eq!(state(&s, 1).held, [0]);
    assert!(state(&s, 1).channels[0].active);
    audition(
        &s,
        0,
        2,
        "off",
        vec![
            json!({"method":"key-off","channel":0}),
            json!({"method":"set-channel","channel":0,"options":{"step":2048}}),
        ],
    );
    render(&mut r, &s, 100);
    let v = state(&s, 1);
    assert!(v.held.is_empty());
    assert_eq!(v.channels[0].step, 2048);
}
#[test]
fn extra_rf_raw_mask_same_on_does_not_steal_foreign_source() {
    let (s, mut r) = new(None);
    send(
        &s,
        1,
        vec![
            wave(0, 0xc0),
            patch(0, 0, [15, 0]),
            patch(1, 0, [0, 15]),
            group(json!({"method":"write","register":8,"value":254}), 1),
            group(json!({"method":"write","register":8,"value":252}), 2),
        ],
    );
    render(&mut r, &s, 100);
    send(
        &s,
        1,
        vec![json!({"method":"cancel-main-owner","targetGroup":1})],
    );
    let p = render(&mut r, &s, 100);
    let v = state(&s, 1);
    assert!(!v.channels[0].active);
    assert!(v.channels[1].active);
    assert!(p.last().unwrap()[0].abs() < 1e-6);
    assert!(p.last().unwrap()[1] > 0.);
}
#[test]
fn extra_rf_raw_mask_and_global_disable_merge_foreign_voices() {
    let (s, mut r) = new(None);
    send(
        &s,
        1,
        vec![
            wave(0, 0xc0),
            patch(0, 0, [15, 0]),
            group(json!({"method":"key-on","channel":0}), 1),
        ],
    );
    render(&mut r, &s, 1);
    send(
        &s,
        1,
        vec![
            group(json!({"method":"write","register":8,"value":255}), 2),
            group(json!({"method":"write","register":7,"value":0x40}), 2),
        ],
    );
    render(&mut r, &s, 10);
    assert!(state(&s, 1).channels[0].active);
    send(
        &s,
        1,
        vec![group(json!({"method":"write","register":8,"value":255}), 1)],
    );
    render(&mut r, &s, 10);
    assert!(!state(&s, 1).channels[0].active);
}
#[test]
fn extra_rf_manual_voice_guards_only_its_sample_extent() {
    let (s, mut r) = new(None);
    send(
        &s,
        1,
        vec![wave(0, 0xc0), wave(512, 0xc0), patch(0, 0, [15, 15])],
    );
    render(&mut r, &s, 1);
    audition(&s, 0, 1, "on", vec![json!({"method":"key-on","channel":0})]);
    render(&mut r, &s, 100);
    let before = state(&s, 1).memory_revision;
    send(
        &s,
        1,
        vec![json!({"method":"load-memory","offset":512,"data":[0],"id":1})],
    );
    render(&mut r, &s, 1);
    s.extra_rf5c164_status(1, Some(1)).unwrap();
    assert_eq!(state(&s, 1).memory_revision, before + 1);
    send(
        &s,
        1,
        vec![json!({"method":"load-memory","offset":3,"data":[0],"id":2})],
    );
    render(&mut r, &s, 1);
    assert!(s
        .extra_rf5c164_status(1, Some(2))
        .unwrap_err()
        .contains("sample RAM"));
    assert_eq!(state(&s, 1).memory_revision, before + 1);
    assert_eq!(state(&s, 1).held, [0]);
}
#[test]
fn extra_rf_manual_guard_includes_current_address_after_live_start_change() {
    let (s, mut r) = new(None);
    send(
        &s,
        1,
        vec![wave(0, 0xc0), wave(512, 0xc0), patch(0, 0, [15, 15])],
    );
    render(&mut r, &s, 1);
    audition(&s, 0, 1, "on", vec![json!({"method":"key-on","channel":0})]);
    render(&mut r, &s, 1);
    audition(
        &s,
        0,
        1,
        "edit",
        vec![json!({"method":"set-channel","channel":0,"options":{"start":512,"loopStart":512}})],
    );
    render(&mut r, &s, 1);
    send(
        &s,
        1,
        vec![json!({"method":"load-memory","offset":16,"data":[1],"id":1})],
    );
    render(&mut r, &s, 1);
    assert!(s
        .extra_rf5c164_status(1, Some(1))
        .unwrap_err()
        .contains("sample RAM"));
}
#[test]
fn extra_rf_high_step_short_marker_conservatively_guards_wide_ram() {
    let (s, mut r) = new(None);
    send(&s, 1, vec![wave(0, 0xc0), patch(0, 0, [15, 15])]);
    render(&mut r, &s, 1);
    audition(
        &s,
        0,
        1,
        "on",
        vec![
            json!({"method":"set-channel","channel":0,"options":{"step":65535}}),
            json!({"method":"key-on","channel":0}),
        ],
    );
    render(&mut r, &s, 1);
    send(
        &s,
        1,
        vec![json!({"method":"load-memory","offset":60000,"data":[1],"id":1})],
    );
    render(&mut r, &s, 1);
    assert!(s.extra_rf5c164_status(1, Some(1)).is_err());
}
#[test]
fn extra_rf_clock_completion_rejects_manual_hold_race_without_false_clock() {
    let (s, mut r) = new(None);
    send(&s, 1, vec![wave(0, 0xc0), patch(0, 0, [15, 15])]);
    s.enqueue(
        1,
        &[command(
            1,
            1000,
            json!({"method":"clock","hz":10_000_000,"id":1}),
        )],
    )
    .unwrap();
    audition(&s, 0, 1, "on", vec![json!({"method":"key-on","channel":0})]);
    render(&mut r, &s, 1200);
    assert!(s
        .extra_rf5c164_status(1, Some(1))
        .unwrap_err()
        .contains("manually held"));
    assert_eq!(state(&s, 1).clock, 12_500_000);
    assert!(s
        .enqueue(
            1,
            &[command(
                1,
                0,
                json!({"method":"clock","hz":10_000_000,"id":2})
            )]
        )
        .is_err());
}
#[test]
fn extra_rf_invalid_batch_or_memory_rolls_back_without_partial_writes() {
    let (s, mut r) = new(None);
    let bad = [
        json!({"method":"set-channel","channel":8,"options":{}}),
        json!({"method":"set-channel","channel":0,"options":{"start":1}}),
        json!({"method":"set-channel","channel":0,"options":{"pan":{"left":16,"right":1}}}),
        json!({"method":"load-memory","offset":65536,"data":[1]}),
        json!({"method":"write-memory","offset":4096,"value":1}),
        json!({"method":"batch","channel":0,"voice":1,"action":"on","audition":true,"commands":[{"method":"set-channel","channel":1,"options":{"volume":99}}]}),
    ];
    for bad in bad {
        assert!(s
            .enqueue(
                1,
                &[command(1, 0, patch(0, 0, [15, 15])), command(1, 0, bad)]
            )
            .is_err());
    }
    render(&mut r, &s, 1);
    assert_eq!(state(&s, 1).channels[0].volume, 0);
}
#[test]
fn extra_rf_batch_raw_actual_selected_cannot_edit_other_manual_voice() {
    let (s, mut r) = new(None);
    send(&s, 1, vec![wave(0, 0xc0), patch(7, 0, [15, 15])]);
    audition(&s, 7, 1, "on", vec![json!({"method":"key-on","channel":7})]);
    render(&mut r, &s, 1);
    audition(
        &s,
        0,
        2,
        "edit",
        vec![json!({"method":"write","register":0,"value":0})],
    );
    render(&mut r, &s, 1);
    assert_eq!(state(&s, 1).channels[7].volume, 100);
}
#[test]
fn extra_rf_cancellation_removes_only_worker_owned_source_and_future_writes() {
    let (s, mut r) = new(Some(100000));
    s.performance("start", 7, None).unwrap();
    send(
        &s,
        1,
        vec![wave(0, 0xc0), patch(0, 0, [15, 0]), patch(1, 0, [0, 15])],
    );
    let mut a = command(1, 0, json!({"method":"key-on","channel":0}));
    a.performance = Some(7);
    a.owner = Some(1);
    s.enqueue(1, &[a]).unwrap();
    let mut future = command(1, 2000, json!({"method":"key-on","channel":0}));
    future.performance = Some(7);
    future.owner = Some(1);
    s.enqueue(1, &[future]).unwrap();
    audition(&s, 1, 2, "on", vec![json!({"method":"key-on","channel":1})]);
    render(&mut r, &s, 1000);
    s.performance("cancel-owner", 7, Some(1)).unwrap();
    let p = render(&mut r, &s, 1500);
    assert!(!state(&s, 1).channels[0].active);
    assert!(state(&s, 1).channels[1].active);
    assert!(p.last().unwrap()[0].abs() < 1e-6);
    assert!(p.last().unwrap()[1] > 0.);
}
#[test]
fn extra_rf_dispose_stop_and_monotonic_replacement_drain_safely() {
    let (s, mut r) = new(None);
    send(&s, 1, vec![wave(0, 0xc0)]);
    send(&s, 1, note(0, 0, [15, 15]));
    s.dispose_extra_rf5c164(1).unwrap();
    render(&mut r, &s, 1);
    s.extra_psg.collect_retired().unwrap();
    assert_eq!(s.extra_psg.counts(), (0, 0));
    assert!(s.create_extra_rf5c164(1, None).is_err());
    s.create_extra_rf5c164(2, None).unwrap();
    send(&s, 2, vec![wave(0, 0xc0)]);
    stop(&s, &mut r);
    render(&mut r, &s, 1000);
    assert!(s.extra_rf5c164_status(2, None).is_err());
    s.create_extra_rf5c164(3, None).unwrap();
    render(&mut r, &s, 1);
    assert_eq!(state(&s, 3).mask, 255);
}
#[test]
fn extra_rf_upload_batch_limit_and_stalled_budget_reclaim() {
    let (s, mut r) = new(None);
    let baseline = s.extra_psg.memory_used();
    let a = command(
        1,
        100000,
        json!({"method":"load-memory","offset":0,"data":vec![1u8;65536]}),
    );
    let b = command(
        1,
        100000,
        json!({"method":"load-memory","offset":0,"data":[1]}),
    );
    assert!(s
        .enqueue(
            1,
            &[
                command(
                    1,
                    100000,
                    json!({"method":"load-memory","offset":0,"data":vec![1u8;65536]})
                ),
                b
            ]
        )
        .unwrap_err()
        .contains("64KiB"));
    assert_eq!(s.extra_psg.memory_used(), baseline);
    s.enqueue(1, &[a]).unwrap();
    assert!(s.extra_psg.memory_used() > baseline + 65536);
    stop(&s, &mut r);
    render(&mut r, &s, 1000);
    s.extra_psg.collect_retired().unwrap();
    assert_eq!(s.extra_psg.memory_used(), 0);
}

#[test]
fn extra_rf_main_cancel_invalidates_already_queued_future_group() {
    let (s, mut r) = new(None);
    send(&s, 1, vec![wave(0, 0xc0), patch(0, 0, [15, 15])]);
    s.enqueue(
        1,
        &[command(
            1,
            1000,
            group(json!({"method":"key-on","channel":0}), 1),
        )],
    )
    .unwrap();
    send(
        &s,
        1,
        vec![json!({"method":"cancel-main-owner","targetGroup":1})],
    );
    render(&mut r, &s, 1500);
    assert!(!state(&s, 1).channels[0].active);
}

#[test]
fn extra_rf_after_id_waits_its_own_future_completion_despite_newer_immediate_id() {
    let s = Arc::new(Shared::new(48000, 1));
    let mut r = Renderer::new(48000).unwrap();
    s.create_extra_rf5c164(1, None).unwrap();
    s.enqueue(
        1,
        &[command(
            1,
            1000,
            json!({"method":"set-channel","channel":0,"options":{"volume":33},"id":1}),
        )],
    )
    .unwrap();
    send(
        &s,
        1,
        vec![json!({"method":"load-memory","offset":0,"data":[1],"id":2})],
    );
    render(&mut r, &s, 1);
    s.extra_rf5c164_status(1, Some(2)).unwrap();
    let waiting = s.clone();
    let (tx, rx) = mpsc::channel();
    let task =
        std::thread::spawn(move || tx.send(waiting.extra_rf5c164_status(1, Some(1))).unwrap());
    assert!(rx.recv_timeout(Duration::from_millis(20)).is_err());
    render(&mut r, &s, 1200);
    assert_eq!(
        rx.recv_timeout(Duration::from_millis(200))
            .unwrap()
            .unwrap()
            .rf5c164
            .channels[0]
            .volume,
        33
    );
    task.join().unwrap();
}
#[test]
fn extra_rf_cancelled_queued_upload_reports_non_application() {
    let (s, mut r) = new(None);
    s.performance("start", 7, None).unwrap();
    let mut c = command(
        1,
        1000,
        json!({"method":"load-memory","offset":0,"data":[255],"id":1}),
    );
    c.performance = Some(7);
    c.owner = Some(1);
    s.enqueue(1, &[c]).unwrap();
    s.performance("cancel-owner", 7, Some(1)).unwrap();
    render(&mut r, &s, 1200);
    assert!(s
        .extra_rf5c164_status(1, Some(1))
        .unwrap_err()
        .contains("cancelled"));
    assert_eq!(state(&s, 1).memory_revision, 0);
}

#[test]
fn extra_rf_cancel_retires_already_admitted_heap_lookahead_before_due() {
    let (s, mut r) = new(None);
    s.enqueue(
        1,
        &[command(
            1,
            100000,
            group(
                json!({"method":"load-memory","offset":0,"data":[1],"id":1}),
                1,
            ),
        )],
    )
    .unwrap();
    render(&mut r, &s, 1);
    assert!(state(&s, 1).pending_commands.contains(&1));
    send(
        &s,
        1,
        vec![json!({"method":"cancel-main-owner","targetGroup":1})],
    );
    render(&mut r, &s, 1);
    assert!(state(&s, 1).pending_commands.is_empty());
    assert!(s
        .extra_rf5c164_status(1, Some(1))
        .unwrap_err()
        .contains("cancelled"));
}
#[test]
fn extra_rf_rejected_cross_instance_batch_does_not_cancel_older_future() {
    let (s, mut r) = new(None);
    s.create_extra_rf5c164(2, None).unwrap();
    send(&s, 2, vec![wave(0, 0xc0), patch(0, 0, [15, 15])]);
    send(
        &s,
        2,
        vec![
            json!({"method":"batch","channel":0,"voice":1,"action":"on","audition":true,"commands":[{"method":"key-on","channel":0}]}),
        ],
    );
    s.enqueue(
        1,
        &[command(
            1,
            1000,
            group(
                json!({"method":"set-channel","channel":0,"options":{"volume":42},"id":1}),
                1,
            ),
        )],
    )
    .unwrap();
    render(&mut r, &s, 1);
    assert!(s
        .enqueue(
            1,
            &[
                command(1, 0, json!({"method":"cancel-main-owner","targetGroup":1})),
                command(2, 0, json!({"method":"clock","hz":10_000_000}))
            ]
        )
        .is_err());
    render(&mut r, &s, 1200);
    assert_eq!(
        s.extra_rf5c164_status(1, Some(1)).unwrap().rf5c164.channels[0].volume,
        42
    );
}
