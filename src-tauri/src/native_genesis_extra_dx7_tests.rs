//! Additional DX7 integration rendered without a device or JavaScript runtime.
use super::*;
use serde_json::{json, Value};
fn cmd(id: u64, frame: u64, message: Value) -> Command {
    serde_json::from_value(
        json!({"type":"extra-dx7","instance":id,"frame":frame,"message":message}),
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
    out
}
fn new() -> (Shared, Renderer) {
    let s = Shared::new(48000, 1);
    let r = Renderer::new(48000).unwrap();
    s.create_extra_dx7(1).unwrap();
    (s, r)
}
fn state(s: &Shared, id: u64) -> extra_dx7::ChipStatus {
    s.extra_dx7_status(id, None).unwrap().dx7
}
fn voice() -> Vec<u8> {
    dx7::init_voice().to_vec()
}
fn on(note: u8, lease: u64, manual: bool) -> Value {
    json!({"method":"note-on","note":note,"velocity":100,"voice":lease,"audition":manual})
}
fn off(note: u8, lease: u64, manual: bool) -> Value {
    json!({"method":"note-off","note":note,"voice":lease,"audition":manual})
}
fn group(mut m: Value, id: u64) -> Value {
    m["group"] = json!(id);
    m
}
fn preset(index: usize, value: u8) -> Value {
    let mut v = voice();
    v[index] = value;
    json!({"method":"set-preset","voice":v})
}
fn energy(p: &[[f32; 2]]) -> f32 {
    p.iter().map(|v| v[0].abs() + v[1].abs()).sum()
}
fn fixed() -> usize {
    extra_budget::INSTANCE_BYTES + extra_dx7::storage_bytes()
}
#[test]
fn extra_dx7_eighteen_kinds_share_sixteen_slots_and_bounded_layout() {
    let (s, mut r) = new();
    s.create_extra_huc6280(2, None).unwrap();
    s.create_extra_nes(3, None, true).unwrap();
    s.create_extra_gameboy(4, None).unwrap();
    s.create_extra_psg(5, None).unwrap();
    s.create_extra_ym2612(6, None).unwrap();
    s.create_extra_ym2203(7, None).unwrap();
    s.create_extra_ym2608(8, None).unwrap();
    s.create_extra_ym2610(9, None).unwrap();
    s.create_extra_ym2151(10, None).unwrap();
    s.create_extra_ym2413(11, None).unwrap();
    s.create_extra_ay8910(12, None).unwrap();
    s.create_extra_ym3526(13, None).unwrap();
    s.create_extra_ym3812(14, None).unwrap();
    s.create_extra_y8950(15, None).unwrap();
    s.create_extra_ymf262(16, None).unwrap();
    assert!(s
        .create_extra_rf5c164(17, None)
        .unwrap_err()
        .contains("limit"));
    send(&s, 1, vec![on(60, 1, false)]);
    assert!(energy(&render(&s, &mut r, 4096)) > 1.);
    s.dispose_extra_dx7(1).unwrap();
    render(&s, &mut r, 1);
    s.extra_psg.collect_retired().unwrap();
    s.create_extra_rf5c164(17, None).unwrap();
    assert!(r.dx7.is_none());
    println!(
        "DX7 Instance={} Shared={} Renderer={} Session={} Storage={} Fixed={}",
        std::mem::size_of::<extra_psg::Instance>(),
        std::mem::size_of::<extra_dx7::Shared>(),
        std::mem::size_of::<extra_dx7::Renderer>(),
        std::mem::size_of::<Renderer>(),
        extra_dx7::storage_bytes(),
        fixed()
    );
    assert!(std::mem::size_of::<extra_psg::Instance>() + 27040 < 65536);
}
#[test]
fn extra_dx7_bank_patch_allocator_and_audio_are_independent() {
    let (s, mut r) = new();
    s.create_extra_dx7(2).unwrap();
    let mut v = voice();
    v[134] = 0;
    let mut w = voice();
    w[135] = 7;
    send(
        &s,
        1,
        vec![
            json!({"method":"load-bank","voices":[v,w]}),
            json!({"method":"select-voice","index":1}),
            on(60, 1, false),
        ],
    );
    render(&s, &mut r, 1000);
    let a = state(&s, 1);
    let b = state(&s, 2);
    assert_eq!(a.voices.len(), 2);
    assert_eq!(a.selected_voice, 1);
    assert_eq!(a.voices[1][135], 7);
    assert_eq!(b.voices.len(), 1);
    assert_eq!(b.voices[0], voice());
    assert!(b.active_notes.is_empty());
    send(&s, 1, vec![json!({"method":"panic"})]);
    render(&s, &mut r, 128);
    assert!(energy(&render(&s, &mut r, 2048)) < 1e-6);
    send(&s, 2, vec![on(72, 1, false)]);
    assert!(energy(&render(&s, &mut r, 2048)) > 1.);
}
#[test]
fn extra_dx7_pcm_matches_existing_primary_at_three_rates() {
    for rate in [44100, 48000, 96000] {
        let s = Shared::new(rate, 1);
        let mut r = Renderer::new(rate).unwrap();
        s.create_extra_dx7(1).unwrap();
        let p = Shared::new_primary(rate, 1, false, true);
        let mut pr = Renderer::new_primary(rate, false, true).unwrap();
        let mut patch = voice();
        patch[134] = 4;
        patch[135] = 3;
        let messages = [
            json!({"method":"set-preset","voice":patch}),
            on(57, 1, false),
            json!({"method":"edit-voice","voice":voice(),"indices":[135]}),
        ];
        let frames = [0, 100, 1000];
        s.enqueue(
            1,
            &messages
                .iter()
                .zip(frames)
                .map(|(m, f)| cmd(1, f, m.clone()))
                .collect::<Vec<_>>(),
        )
        .unwrap();
        p.enqueue(
            1,
            &messages
                .iter()
                .zip(frames)
                .map(|(m, f)| {
                    serde_json::from_value(json!({"type":"dx7","frame":f,"message":m})).unwrap()
                })
                .collect::<Vec<_>>(),
        )
        .unwrap();
        let a = render(&s, &mut r, 8000);
        let b = render(&p, &mut pr, 8000);
        assert!(a.iter().flatten().all(|v| v.is_finite()));
        assert_eq!(a, b, "rate {rate}");
    }
}
#[test]
fn extra_dx7_future_edits_merge_applied_voice_at_due_frame() {
    let (s, mut r) = new();
    let mut a = voice();
    a[121] = 40;
    let mut b = voice();
    b[135] = 7;
    s.enqueue(
        1,
        &[
            cmd(
                1,
                100,
                json!({"method":"edit-voice","voice":a,"indices":[121],"id":1}),
            ),
            cmd(
                1,
                200,
                json!({"method":"edit-voice","voice":b,"indices":[135],"id":2}),
            ),
        ],
    )
    .unwrap();
    assert_eq!(state(&s, 1).voices[0][121], 85);
    render(&s, &mut r, 201);
    let a = s.extra_dx7_status(1, Some(2)).unwrap().dx7;
    assert_eq!(a.voices[0][121], 40);
    assert_eq!(a.voices[0][135], 7);
}
#[test]
fn extra_dx7_manual_pitch_and_stale_leases_are_protected() {
    let (s, mut r) = new();
    send(&s, 1, vec![group(on(60, 1, false), 5), on(60, 2, true)]);
    render(&s, &mut r, 1);
    send(
        &s,
        1,
        vec![
            group(off(60, 1, false), 5),
            group(on(60, 3, false), 5),
            off(60, 1, true),
            json!({"method":"panic"}),
            json!({"method":"all-notes-off"}),
        ],
    );
    render(&s, &mut r, 100);
    let a = state(&s, 1);
    assert_eq!(a.held, [60]);
    assert_eq!(a.active_notes.len(), 1);
    send(&s, 1, vec![off(60, 2, true)]);
    render(&s, &mut r, 1);
    assert!(state(&s, 1).held.is_empty());
}
#[test]
fn extra_dx7_sixteen_manual_slots_cannot_be_stolen_by_script() {
    let (s, mut r) = new();
    send(&s, 1, (40..56).map(|n| on(n, n as u64, true)).collect());
    render(&s, &mut r, 1);
    send(&s, 1, vec![on(90, 100, false), on(40, 101, false)]);
    render(&s, &mut r, 1);
    let a = state(&s, 1);
    assert_eq!(a.held.len(), 16);
    assert_eq!(a.active_notes.len(), 16);
    assert!(!a.active_notes.iter().any(|n| n.midi == 90));
    send(&s, 1, vec![off(45, 45, true), on(90, 101, false)]);
    render(&s, &mut r, 1);
    let a = state(&s, 1);
    assert_eq!(a.held.len(), 15);
    assert!(a.active_notes.iter().any(|n| n.midi == 90));
}
#[test]
fn extra_dx7_manual_can_take_one_script_slot_when_all_sixteen_are_busy() {
    let (s, mut r) = new();
    send(&s, 1, (40..56).map(|n| on(n, n as u64, false)).collect());
    render(&s, &mut r, 1);
    send(&s, 1, vec![on(90, 100, true)]);
    render(&s, &mut r, 1);
    let a = state(&s, 1);
    assert_eq!(a.active_notes.len(), 16);
    assert_eq!(a.held, [90]);
}
#[test]
fn extra_dx7_patch_held_admission_and_apply_recheck() {
    let (s, mut r) = new();
    let mut patch = preset(134, 0);
    patch["id"] = json!(1);
    s.enqueue(1, &[cmd(1, 1000, patch)]).unwrap();
    send(&s, 1, vec![on(60, 2, true)]);
    render(&s, &mut r, 1200);
    assert!(s
        .extra_dx7_status(1, Some(1))
        .unwrap_err()
        .contains("manually held"));
    assert_eq!(state(&s, 1).voices[0][134], 31);
    assert!(s
        .enqueue(1, &[cmd(1, 0, preset(134, 0))])
        .unwrap_err()
        .contains("manually held"));
    let mut patch = preset(134, 0);
    patch["audition"] = json!(true);
    patch["id"] = json!(2);
    send(&s, 1, vec![patch]);
    render(&s, &mut r, 1);
    assert_eq!(
        s.extra_dx7_status(1, Some(2)).unwrap().dx7.voices[0][134],
        0
    );
}
#[test]
fn extra_dx7_worker_and_main_same_pitch_leases_are_independent() {
    let (s, mut r) = new();
    s.performance("start", 7, None).unwrap();
    let mut w = cmd(1, 0, on(60, 1, false));
    w.performance = Some(7);
    w.owner = Some(1);
    s.enqueue(
        1,
        &[
            w,
            cmd(1, 0, group(on(60, 2, false), 9)),
            cmd(1, 0, group(on(60, 3, false), 10)),
        ],
    )
    .unwrap();
    render(&s, &mut r, 1);
    send(&s, 1, vec![group(off(60, 2, false), 9)]);
    render(&s, &mut r, 1);
    assert_eq!(state(&s, 1).active_notes.len(), 2);
    s.performance("cancel-owner", 7, Some(1)).unwrap();
    render(&s, &mut r, 1);
    assert_eq!(state(&s, 1).active_notes.len(), 1);
    send(
        &s,
        1,
        vec![json!({"method":"cancel-main-owner","targetGroup":10})],
    );
    render(&s, &mut r, 1);
    assert!(state(&s, 1).active_notes.is_empty());
}
#[test]
fn extra_dx7_worker_cancel_future_preserves_manual_and_reclaims_payloads() {
    let (s, mut r) = new();
    s.performance("start", 7, None).unwrap();
    send(&s, 1, vec![on(60, 1, true)]);
    let mut c = cmd(
        1,
        100000,
        json!({"method":"load-bank","voices":vec![voice();32],"id":10}),
    );
    c.performance = Some(7);
    c.owner = Some(1);
    s.enqueue(1, &[c]).unwrap();
    render(&s, &mut r, 100);
    s.performance("cancel-owner", 7, Some(1)).unwrap();
    render(&s, &mut r, 1);
    assert!(s
        .extra_dx7_status(1, Some(10))
        .unwrap_err()
        .contains("cancelled"));
    assert_eq!(state(&s, 1).held, [60]);
    s.extra_psg.reserve_payloads(std::iter::empty()).unwrap();
    assert_eq!(s.extra_psg.memory_used(), fixed());
}
#[test]
fn extra_dx7_main_cancel_invalidates_only_target_future_group() {
    let (s, mut r) = new();
    let mut a = preset(134, 0);
    a["id"] = json!(1);
    let mut b = preset(134, 4);
    b["id"] = json!(2);
    s.enqueue(1, &[cmd(1, 1000, group(a, 1)), cmd(1, 1000, group(b, 2))])
        .unwrap();
    send(
        &s,
        1,
        vec![json!({"method":"cancel-main-owner","targetGroup":1})],
    );
    render(&s, &mut r, 1);
    assert!(s
        .extra_dx7_status(1, Some(1))
        .unwrap_err()
        .contains("cancelled"));
    assert_eq!(state(&s, 1).pending_commands, [2]);
    render(&s, &mut r, 1100);
    assert_eq!(
        s.extra_dx7_status(1, Some(2)).unwrap().dx7.voices[0][134],
        4
    );
}
#[test]
fn extra_dx7_failed_other_family_retain_does_not_commit_cancel() {
    let (s, mut r) = new();
    s.create_extra_okim6258(2, None).unwrap();
    let mut a = preset(134, 0);
    a["id"] = json!(1);
    s.enqueue(1, &[cmd(1, 1000, group(a, 42))]).unwrap();
    let oki = |m: Value| {
        serde_json::from_value::<Command>(
            json!({"type":"extra-okim6258","instance":2,"frame":0,"message":m}),
        )
        .unwrap()
    };
    s.enqueue(1,&[oki(json!({"method":"batch","channel":0,"voice":1,"audition":true,"action":"on","commands":[{"method":"write","register":0,"value":2}]}))]).unwrap();
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
    render(&s, &mut r, 1100);
    assert_eq!(
        s.extra_dx7_status(1, Some(1)).unwrap().dx7.voices[0][134],
        0
    );
}
#[test]
fn extra_dx7_after_wait_does_not_block_control_and_history_is_bounded() {
    use std::{sync::mpsc, time::Duration};
    let (s, mut r) = new();
    let s = Arc::new(s);
    let mut p = preset(134, 0);
    p["id"] = json!(1);
    s.enqueue(1, &[cmd(1, 1000, p)]).unwrap();
    s.enqueue(
        1,
        &(2..=601)
            .map(|id| cmd(1, 0, json!({"method":"select-voice","index":0,"id":id})))
            .collect::<Vec<_>>(),
    )
    .unwrap();
    render(&s, &mut r, 1);
    state(&s, 1);
    assert!(s
        .extra_dx7_status(1, Some(2))
        .unwrap_err()
        .contains("expired"));
    assert!(s.extra_dx7_status(1, Some(601)).is_ok());
    let (tx, rx) = mpsc::channel();
    let waiting = s.clone();
    let task = std::thread::spawn(move || tx.send(waiting.extra_dx7_status(1, Some(1))).unwrap());
    assert!(rx.recv_timeout(Duration::from_millis(20)).is_err());
    send(&s, 1, vec![on(60, 1, false)]);
    render(&s, &mut r, 1200);
    assert_eq!(
        rx.recv_timeout(Duration::from_millis(200))
            .unwrap()
            .unwrap()
            .dx7
            .voices[0][134],
        0
    );
    task.join().unwrap();
}
#[test]
fn extra_dx7_invalid_messages_duplicate_ids_and_budget_roll_back() {
    let (s, mut r) = new();
    render(&s, &mut r, 1);
    for m in [
        json!({"method":"set-preset","voice":[1]}),
        json!({"method":"load-bank","voices":[]}),
        json!({"method":"load-bank","voices":vec![voice();33]}),
        json!({"method":"select-voice","index":32}),
        json!({"method":"note-on","note":128}),
        json!({"method":"note-on","note":60,"velocity":128}),
        json!({"method":"note-on","note":60,"voice":0}),
        json!({"method":"mix","volume":5,"pan":0}),
        json!({"method":"cancel-main-owner","targetGroup":0}),
        json!({"method":"edit-voice","voice":voice(),"indices":[121,121]}),
        json!({"method":"edit-voice","voice":voice(),"indices":[155]}),
    ] {
        assert!(s.enqueue(1, &[cmd(1, 0, m)]).is_err());
    }
    assert!(s
        .enqueue(
            1,
            &[cmd(
                1,
                1,
                json!({"method":"cancel-main-owner","targetGroup":1})
            )]
        )
        .is_err());
    let c = || cmd(1, 1000, json!({"method":"select-voice","index":0,"id":1}));
    assert!(s.enqueue(1, &[c(), c()]).unwrap_err().contains("Duplicate"));
    assert_eq!(state(&s, 1).voices[0], voice());
    assert!(state(&s, 1).pending_commands.is_empty());
    s.extra_psg.reserve_payloads(std::iter::empty()).unwrap();
    assert_eq!(s.extra_psg.memory_used(), fixed());
    let mut limited = Shared::new(48000, 1);
    limited.extra_psg = extra_psg::Shared::with_memory_limit(fixed() + 300);
    limited.create_extra_dx7(1).unwrap();
    assert!(limited
        .enqueue(
            1,
            &[cmd(
                1,
                1000,
                json!({"method":"load-bank","voices":vec![voice();32]})
            )]
        )
        .unwrap_err()
        .contains("budget"));
    assert_eq!(limited.extra_psg.memory_used(), fixed());
}
#[test]
fn extra_dx7_dispose_stale_ids_and_stop_preserve_bank() {
    let (s, mut r) = new();
    send(&s, 1, vec![preset(134, 4), on(60, 1, false)]);
    render(&s, &mut r, 128);
    send(&s, 1, vec![json!({"method":"panic"})]);
    render(&s, &mut r, 6000);
    assert!(state(&s, 1).active_notes.is_empty());
    assert_eq!(state(&s, 1).voices[0][134], 4);
    s.dispose_extra_dx7(1).unwrap();
    render(&s, &mut r, 1);
    s.extra_psg.collect_retired().unwrap();
    assert_eq!(s.extra_psg.counts(), (0, 0));
    assert_eq!(s.extra_psg.memory_used(), 0);
    assert!(s.create_extra_dx7(1).is_err());
    s.create_extra_dx7(2).unwrap();
    assert_eq!(state(&s, 2).voices[0], voice());
    assert!(s.enqueue(1, &[cmd(1, 0, on(60, 1, false))]).is_err());
}
#[test]
fn extra_dx7_mixer_is_linear_stereo_balance_and_finite() {
    let (s, mut r) = new();
    send(
        &s,
        1,
        vec![
            on(60, 1, false),
            json!({"method":"mix","volume":0.5,"pan":1}),
        ],
    );
    let p = render(&s, &mut r, 4000);
    assert!(p.iter().flatten().all(|v| v.is_finite()));
    assert!(p[500..].iter().all(|v| v[0].abs() < 1e-7));
    assert!(p[500..].iter().any(|v| v[1].abs() > 0.001));
}

#[test]
fn extra_dx7_scoped_cancel_kills_released_tail_but_note_off_keeps_normal_release() {
    let (s, mut r) = new();
    let mut patch = voice();
    patch[108] = 0;
    send(
        &s,
        1,
        vec![
            json!({"method":"set-preset","voice":patch}),
            group(on(60, 1, false), 42),
        ],
    );
    render(&s, &mut r, 2000);
    send(&s, 1, vec![group(off(60, 1, false), 42)]);
    assert!(energy(&render(&s, &mut r, 2000)) > 1.);
    assert!(state(&s, 1).active_notes.is_empty());
    send(
        &s,
        1,
        vec![json!({"method":"cancel-main-owner","targetGroup":42})],
    );
    let p = render(&s, &mut r, 1000);
    assert!(energy(&p[300..]) < 1e-6);
    s.performance("start", 7, None).unwrap();
    let mut on = cmd(1, 0, on(64, 2, false));
    on.performance = Some(7);
    on.owner = Some(1);
    s.enqueue(1, &[on]).unwrap();
    render(&s, &mut r, 1000);
    let mut off = cmd(1, 0, off(64, 2, false));
    off.performance = Some(7);
    off.owner = Some(1);
    s.enqueue(1, &[off]).unwrap();
    assert!(energy(&render(&s, &mut r, 1000)) > 1.);
    s.performance("cancel-owner", 7, Some(1)).unwrap();
    let p = render(&s, &mut r, 1000);
    assert!(energy(&p[300..]) < 1e-6);
}

#[test]
fn extra_dx7_masked_preset_starts_new_cohort_and_preserves_applied_future_edits() {
    let (s, mut r) = new();
    let primary = Shared::new_primary(48000, 1, false, true);
    let mut primary_renderer = Renderer::new_primary(48000, false, true).unwrap();
    let mut transposed = voice();
    transposed[144] = 36;
    let mut algorithm = voice();
    algorithm[134] = 0;
    let mut primary_algorithm = algorithm.clone();
    primary_algorithm[144] = 36;
    let mut silent = voice();
    silent[121] = 0;
    let extra_messages = [
        (0, on(60, 1, false)),
        (
            50,
            json!({"method":"edit-voice","voice":transposed,"indices":[144]}),
        ),
        (
            100,
            json!({"method":"set-preset","voice":algorithm,"indices":[134],"audition":true}),
        ),
        (
            200,
            json!({"method":"edit-voice","voice":silent,"indices":[121],"audition":true}),
        ),
        (400, on(72, 2, false)),
    ];
    let mut primary_messages = extra_messages.clone();
    primary_messages[2].1 =
        json!({"method":"set-preset","voice":primary_algorithm,"audition":true});
    s.enqueue(
        1,
        &extra_messages
            .iter()
            .map(|(frame, m)| cmd(1, *frame, m.clone()))
            .collect::<Vec<_>>(),
    )
    .unwrap();
    primary
        .enqueue(
            1,
            &primary_messages
                .iter()
                .map(|(frame, m)| {
                    serde_json::from_value(json!({"type":"dx7","frame":frame,"message":m})).unwrap()
                })
                .collect::<Vec<_>>(),
        )
        .unwrap();
    let actual = render(&s, &mut r, 4000);
    let expected = render(&primary, &mut primary_renderer, 4000);
    assert_eq!(actual, expected);
    // The old note keeps its original cohort; the subsequent live edit changes
    // the newly selected preset and new notes, leaving the held note audible.
    assert!(energy(&actual[1000..3000]) > 1.);
    let state = state(&s, 1);
    assert_eq!(state.voices[0][134], 0);
    assert_eq!(state.voices[0][144], 36);
    assert_eq!(state.voices[0][121], 0);
    assert_eq!(state.active_notes.len(), 2);
}
