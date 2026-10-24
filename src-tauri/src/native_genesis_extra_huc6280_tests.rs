//! Real additional PC Engine cores rendered without opening an audio device.
use super::*;
use serde_json::{json, Value};
fn cmd(id: u64, frame: u64, m: Value) -> Command {
    serde_json::from_value(json!({"type":"extra-huc6280","instance":id,"frame":frame,"message":m}))
        .unwrap()
}
fn send(s: &Shared, id: u64, ms: Vec<Value>) {
    s.enqueue(
        1,
        &ms.into_iter().map(|m| cmd(id, 0, m)).collect::<Vec<_>>(),
    )
    .unwrap()
}
fn write(ch: u8, reg: u8, v: u8) -> Value {
    json!({"method":"write","register":reg,"value":v,"channel":ch})
}
fn group(mut m: Value, g: u64) -> Value {
    m["group"] = json!(g);
    m
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
fn state(s: &Shared, id: u64) -> extra_huc6280::ChipStatus {
    s.extra_huc6280_status(id, None).unwrap().huc6280
}
fn new() -> (Shared, Renderer) {
    let s = Shared::new(48000, 1);
    let r = Renderer::new(48000).unwrap();
    s.create_extra_huc6280(1, None).unwrap();
    send(&s, 1, vec![write(0, 1, 255)]);
    (s, r)
}
fn wave(ch: u8, reverse: bool) -> Value {
    json!({"method":"waveform","channel":ch,"samples":(0..32).map(|i|if reverse{31-i}else{i}).collect::<Vec<u8>>()})
}
fn tone(ch: u8) -> Vec<Value> {
    vec![
        wave(ch, false),
        write(ch, 2, 253),
        write(ch, 3, 0),
        write(ch, 5, 255),
        write(ch, 4, 159),
    ]
}
fn batch(ch: u8, v: u64, action: &str, ops: Vec<Value>, manual: bool) -> Value {
    json!({"method":"batch","channel":ch,"voice":v,"action":action,"commands":ops,"audition":manual})
}
fn off(ch: u8, v: u64, manual: bool) -> Value {
    batch(
        ch,
        v,
        "off",
        vec![json!({"method":"key-off","channel":ch})],
        manual,
    )
}
fn gate(s: &extra_huc6280::ChipStatus, ch: usize) -> bool {
    s.registers[ch][4] & 128 != 0
}
fn energy(p: &[[f32; 2]]) -> f32 {
    p.iter().map(|v| v[0].abs() + v[1].abs()).sum()
}
#[test]
fn extra_huc6280_seventeen_kinds_share_sixteen_slots_and_fixed_budget() {
    let (s, mut r) = new();
    s.create_extra_nes(2, None, true).unwrap();
    s.create_extra_gameboy(3, None).unwrap();
    s.create_extra_psg(4, None).unwrap();
    s.create_extra_ym2612(5, None).unwrap();
    s.create_extra_ym2203(6, None).unwrap();
    s.create_extra_ym2608(7, None).unwrap();
    s.create_extra_ym2610(8, None).unwrap();
    s.create_extra_ym2151(9, None).unwrap();
    s.create_extra_ym2413(10, None).unwrap();
    s.create_extra_ay8910(11, None).unwrap();
    s.create_extra_ym3526(12, None).unwrap();
    s.create_extra_ym3812(13, None).unwrap();
    s.create_extra_y8950(14, None).unwrap();
    s.create_extra_ymf262(15, None).unwrap();
    s.create_extra_rf5c164(16, None).unwrap();
    assert!(s
        .create_extra_okim6258(17, None)
        .unwrap_err()
        .contains("limit"));
    send(&s, 1, tone(0));
    assert!(energy(&render(&mut r, &s, 4096)) > 1.);
    s.dispose_extra_huc6280(1).unwrap();
    render(&mut r, &s, 1);
    s.extra_psg.collect_retired().unwrap();
    s.create_extra_okim6258(17, None).unwrap();
    assert!(r.huc6280.is_none());
    println!(
        "HuC6280 Instance={} Shared={} Renderer={} Session={}",
        std::mem::size_of::<extra_psg::Instance>(),
        std::mem::size_of::<extra_huc6280::Shared>(),
        std::mem::size_of::<extra_huc6280::Renderer>(),
        std::mem::size_of::<Renderer>()
    );
    assert!(std::mem::size_of::<extra_psg::Instance>() + 704 < 65536);
}
#[test]
fn extra_huc6280_wave_dda_noise_lfo_clock_latch_do_not_leak() {
    let (s, mut r) = new();
    s.create_extra_huc6280(2, None).unwrap();
    send(&s, 2, vec![write(0, 1, 255)]);
    send(&s, 1, tone(0));
    send(
        &s,
        1,
        vec![
            write(0, 0, 7),
            wave(1, true),
            write(0, 8, 3),
            write(0, 9, 2),
            json!({"method":"clock","hz":4000000}),
        ],
    );
    send(
        &s,
        2,
        vec![
            write(4, 4, 0xdf),
            write(4, 5, 255),
            write(4, 6, 29),
            write(5, 7, 0x9f),
        ],
    );
    assert!(energy(&render(&mut r, &s, 2000)) > 1.);
    let a = state(&s, 1);
    let b = state(&s, 2);
    assert_eq!(a.selected_channel, 7);
    assert_eq!(a.waveforms[1], (0..32).rev().collect::<Vec<u8>>());
    assert_eq!(b.waveforms[1], vec![0; 32]);
    assert_eq!(b.dda_values[4], 29);
    assert_eq!(a.dda_values[4], 0);
    assert_eq!(b.registers[5][7], 159);
    assert_eq!(a.globals[9], 2);
    assert_eq!(b.globals[9], 0);
    assert_eq!(a.clock, 4000000);
    assert_eq!(b.clock, 3579545);
}
#[test]
fn extra_huc6280_raw_unused_latches_and_targeted_waveforms_remain_independent() {
    let (s, mut r) = new();
    send(
        &s,
        1,
        vec![
            write(0, 0, 7),
            json!({"method":"write","register":4,"value":95}),
            json!({"method":"write","register":6,"value":29}),
            wave(2, true),
            write(2, 2, 111),
        ],
    );
    render(&mut r, &s, 1);
    let a = state(&s, 1);
    assert_eq!(a.selected_channel, 7);
    assert_eq!(a.dda_values[7], 29);
    assert_eq!(a.registers[2][2], 111);
    assert_eq!(a.waveforms[2][0], 31);
    assert_eq!(a.indices[7], 0);
}
#[test]
fn extra_huc6280_manual_voice_protects_global_balance_lfo_and_modulator() {
    let (s, mut r) = new();
    send(
        &s,
        1,
        vec![
            write(0, 8, 3),
            write(0, 9, 2),
            wave(1, true),
            batch(0, 1, "on", tone(0), true),
        ],
    );
    render(&mut r, &s, 100);
    send(
        &s,
        1,
        vec![
            write(0, 1, 0),
            write(0, 8, 0),
            write(0, 9, 0),
            wave(1, false),
            write(1, 2, 9),
            write(0, 4, 0),
            json!({"method":"reset"}),
            json!({"method":"stop-voices"}),
            off(0, 2, true),
        ],
    );
    render(&mut r, &s, 100);
    let a = state(&s, 1);
    assert!(gate(&a, 0));
    assert_eq!(a.held, [0]);
    assert_eq!(a.globals[1], 255);
    assert_eq!(a.globals[8], 3);
    assert_eq!(a.globals[9], 2);
    assert_eq!(a.waveforms[1][0], 31);
    assert_eq!(a.registers[1][2], 0);
    send(&s, 1, vec![off(0, 1, true)]);
    render(&mut r, &s, 1);
    assert!(state(&s, 1).held.is_empty());
    assert!(!gate(&state(&s, 1), 0));
}
#[test]
fn extra_huc6280_main_groups_stale_release_and_cancel_preserve_other_channels() {
    let (s, mut r) = new();
    send(
        &s,
        1,
        vec![
            group(batch(0, 1, "on", tone(0), false), 1),
            group(batch(3, 2, "on", tone(3), false), 2),
        ],
    );
    render(&mut r, &s, 100);
    send(
        &s,
        1,
        vec![
            group(write(0, 4, 0), 2),
            group(write(0, 1, 0), 2),
            json!({"method":"cancel-main-owner","targetGroup":2}),
        ],
    );
    render(&mut r, &s, 1);
    let a = state(&s, 1);
    assert!(gate(&a, 0));
    assert!(!gate(&a, 3));
    assert_eq!(a.globals[1], 255);
    assert_eq!(a.waveforms[3][10], 10);
    send(
        &s,
        1,
        vec![json!({"method":"cancel-main-owner","targetGroup":1})],
    );
    render(&mut r, &s, 1);
    assert!(!gate(&state(&s, 1), 0));
}
#[test]
fn extra_huc6280_clock_held_admission_and_apply_recheck() {
    let (s, mut r) = new();
    s.enqueue(
        1,
        &[cmd(1, 1000, json!({"method":"clock","hz":4000000,"id":1}))],
    )
    .unwrap();
    send(&s, 1, vec![batch(0, 1, "on", tone(0), true)]);
    render(&mut r, &s, 1200);
    assert!(s
        .extra_huc6280_status(1, Some(1))
        .unwrap_err()
        .contains("manually held"));
    assert_eq!(state(&s, 1).clock, 3579545);
    assert!(s
        .enqueue(1, &[cmd(1, 0, json!({"method":"clock","hz":4000000}))])
        .unwrap_err()
        .contains("manually held"));
    send(
        &s,
        1,
        vec![
            off(0, 1, true),
            json!({"method":"clock","hz":4000000,"audition":true,"id":2}),
        ],
    );
    render(&mut r, &s, 1);
    assert_eq!(
        s.extra_huc6280_status(1, Some(2)).unwrap().huc6280.clock,
        4000000
    );
}
#[test]
fn extra_huc6280_mute_keeps_phase_wave_lfo_and_reset_preserves_clock_mute() {
    let (s, mut r) = new();
    send(&s, 1, tone(0));
    assert!(energy(&render(&mut r, &s, 1000)) > 1.);
    send(
        &s,
        1,
        vec![
            json!({"method":"source-mute","mask":1}),
            json!({"method":"clock","hz":4000000}),
        ],
    );
    let p = render(&mut r, &s, 1000);
    assert!(energy(&p) < 1e-6);
    let a = state(&s, 1);
    assert!(gate(&a, 0));
    assert_eq!(a.waveforms[0][10], 10);
    send(&s, 1, vec![json!({"method":"source-mute","mask":0})]);
    assert!(energy(&render(&mut r, &s, 1000)) > 1.);
    send(
        &s,
        1,
        vec![
            json!({"method":"source-mute","mask":1}),
            json!({"method":"reset"}),
        ],
    );
    render(&mut r, &s, 1);
    let a = state(&s, 1);
    assert_eq!(a.clock, 4000000);
    assert_eq!(a.source_mute_mask, 1);
    assert_eq!(a.waveforms[0], vec![0; 32]);
    assert!(!gate(&a, 0));
}
#[test]
fn extra_huc6280_worker_cancel_preserves_manual_and_reclaims_payloads() {
    let (s, mut r) = new();
    s.performance("start", 7, None).unwrap();
    send(&s, 1, vec![batch(0, 1, "on", tone(0), true)]);
    let mut c = cmd(
        1,
        100000,
        json!({"method":"batch","channel":2,"voice":2,"action":"on","commands":tone(2),"id":10}),
    );
    c.performance = Some(7);
    c.owner = Some(1);
    s.enqueue(1, &[c]).unwrap();
    render(&mut r, &s, 100);
    s.performance("cancel-owner", 7, Some(1)).unwrap();
    render(&mut r, &s, 1);
    assert!(s
        .extra_huc6280_status(1, Some(10))
        .unwrap_err()
        .contains("cancelled"));
    assert!(gate(&state(&s, 1), 0));
    assert!(!gate(&state(&s, 1), 2));
    s.extra_psg.reserve_payloads(std::iter::empty()).unwrap();
    assert_eq!(s.extra_psg.memory_used(), extra_budget::INSTANCE_BYTES);
}
#[test]
fn extra_huc6280_main_cancel_invalidates_only_target_future_group() {
    let (s, mut r) = new();
    s.enqueue(
        1,
        &[
            cmd(
                1,
                100000,
                group(json!({"method":"clock","hz":4000000,"id":1}), 1),
            ),
            cmd(
                1,
                100000,
                group(json!({"method":"clock","hz":3000000,"id":2}), 2),
            ),
        ],
    )
    .unwrap();
    send(
        &s,
        1,
        vec![json!({"method":"cancel-main-owner","targetGroup":1})],
    );
    render(&mut r, &s, 1);
    assert!(s
        .extra_huc6280_status(1, Some(1))
        .unwrap_err()
        .contains("cancelled"));
    assert_eq!(state(&s, 1).pending_commands, [2]);
    render(&mut r, &s, 100100);
    assert_eq!(
        s.extra_huc6280_status(1, Some(2)).unwrap().huc6280.clock,
        3000000
    );
}
#[test]
fn extra_huc6280_failed_other_family_retain_does_not_commit_cancel() {
    let (s, mut r) = new();
    s.create_extra_okim6258(2, None).unwrap();
    s.enqueue(
        1,
        &[cmd(
            1,
            1000,
            group(json!({"method":"clock","hz":4000000,"id":1}), 42),
        )],
    )
    .unwrap();
    let oki = |m: Value| {
        serde_json::from_value::<Command>(
            json!({"type":"extra-okim6258","instance":2,"frame":0,"message":m}),
        )
        .unwrap()
    };
    s.enqueue(1,&[oki(json!({"method":"batch","channel":0,"voice":1,"audition":true,"action":"on","commands":[{"method":"write","register":0,"value":2}]}))]).unwrap();
    render(&mut r, &s, 1);
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
    render(&mut r, &s, 1200);
    assert_eq!(
        s.extra_huc6280_status(1, Some(1)).unwrap().huc6280.clock,
        4000000
    );
}
#[test]
fn extra_huc6280_after_wait_allows_control_and_completion_history_is_bounded() {
    use std::{sync::mpsc, time::Duration};
    let (s, mut r) = new();
    let s = Arc::new(s);
    s.enqueue(
        1,
        &[cmd(1, 1000, json!({"method":"clock","hz":4000000,"id":1}))],
    )
    .unwrap();
    s.enqueue(
        1,
        &(2..=601)
            .map(|id| {
                cmd(
                    1,
                    0,
                    json!({"method":"write","register":2,"channel":2,"value":18,"id":id}),
                )
            })
            .collect::<Vec<_>>(),
    )
    .unwrap();
    render(&mut r, &s, 1);
    state(&s, 1);
    assert!(s
        .extra_huc6280_status(1, Some(2))
        .unwrap_err()
        .contains("expired"));
    assert!(s.extra_huc6280_status(1, Some(601)).is_ok());
    let (tx, rx) = mpsc::channel();
    let waiting = s.clone();
    let task =
        std::thread::spawn(move || tx.send(waiting.extra_huc6280_status(1, Some(1))).unwrap());
    assert!(rx.recv_timeout(Duration::from_millis(20)).is_err());
    send(&s, 1, vec![wave(2, true)]);
    render(&mut r, &s, 1200);
    assert_eq!(
        rx.recv_timeout(Duration::from_millis(200))
            .unwrap()
            .unwrap()
            .huc6280
            .clock,
        4000000
    );
    task.join().unwrap();
    assert_eq!(state(&s, 1).waveforms[2][0], 31);
}
#[test]
fn extra_huc6280_invalid_transaction_and_duplicate_ids_do_not_mutate() {
    let (s, mut r) = new();
    render(&mut r, &s, 1);
    let initial = state(&s, 1).registers;
    for m in [
        write(0, 10, 0),
        write(8, 4, 0),
        json!({"method":"clock","hz":99999}),
        json!({"method":"source-mute","mask":64}),
        json!({"method":"cancel-main-owner","targetGroup":0}),
        batch(0, 1, "on", tone(1), false),
        batch(6, 1, "on", tone(0), false),
        batch(0, 1, "on", vec![], false),
        batch(0, 1, "on", vec![write(0, 2, 0); 513], false),
        json!({"method":"waveform","channel":0,"samples":vec![32;32]}),
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
    let c = || cmd(1, 10000, json!({"method":"clock","hz":4000000,"id":1}));
    assert!(s.enqueue(1, &[c(), c()]).unwrap_err().contains("Duplicate"));
    render(&mut r, &s, 1);
    assert_eq!(state(&s, 1).registers, initial);
    assert!(state(&s, 1).pending_commands.is_empty());
    let mut limited = Shared::new(48000, 1);
    limited.extra_psg = extra_psg::Shared::with_memory_limit(extra_budget::INSTANCE_BYTES + 300);
    limited.create_extra_huc6280(1, None).unwrap();
    assert!(limited
        .enqueue(
            1,
            &[cmd(
                1,
                10000,
                batch(0, 1, "on", vec![write(0, 2, 0); 512], false)
            )]
        )
        .unwrap_err()
        .contains("budget"));
    assert_eq!(
        limited.extra_psg.memory_used(),
        extra_budget::INSTANCE_BYTES
    );
}
#[test]
fn extra_huc6280_dispose_frees_on_control_rejects_stale_id_and_new_instance_is_clean() {
    let (s, mut r) = new();
    send(&s, 1, tone(0));
    render(&mut r, &s, 100);
    s.dispose_extra_huc6280(1).unwrap();
    render(&mut r, &s, 1);
    s.extra_psg.collect_retired().unwrap();
    assert_eq!(s.extra_psg.counts(), (0, 0));
    assert_eq!(s.extra_psg.memory_used(), 0);
    assert!(s.create_extra_huc6280(1, None).is_err());
    s.create_extra_huc6280(2, None).unwrap();
    assert_eq!(state(&s, 2).waveforms[0], vec![0; 32]);
    assert!(s.enqueue(1, &[cmd(1, 0, write(0, 4, 0))]).is_err());
}
#[test]
fn extra_huc6280_stop_invalidates_future_preserves_wave_lfo_clock_and_dda_becomes_silent() {
    let (s, mut r) = new();
    send(
        &s,
        1,
        vec![
            wave(0, true),
            write(0, 4, 223),
            write(0, 5, 255),
            write(0, 6, 29),
            write(0, 8, 3),
            write(0, 9, 2),
        ],
    );
    s.enqueue(
        1,
        &[cmd(1, 1000, json!({"method":"clock","hz":4000000,"id":1}))],
    )
    .unwrap();
    assert!(energy(&render(&mut r, &s, 100)) > 1.);
    send(&s, 1, vec![json!({"method":"stop-voices"})]);
    let p = render(&mut r, &s, 1200);
    assert!(energy(&p[20..]) < 1e-6);
    assert!(s
        .extra_huc6280_status(1, Some(1))
        .unwrap_err()
        .contains("cancelled"));
    let a = state(&s, 1);
    assert_eq!(a.clock, 3579545);
    assert_eq!(a.waveforms[0][0], 31);
    assert_eq!(a.globals[8], 3);
    assert_eq!(a.globals[9], 2);
    assert_eq!(a.dda_values[0], 29);
    assert!(!gate(&a, 0));
}
#[test]
fn extra_huc6280_authorized_manual_edit_updates_shared_controls_with_multiple_held_channels() {
    let (s, mut r) = new();
    send(
        &s,
        1,
        vec![
            batch(0, 1, "on", tone(0), true),
            batch(3, 2, "on", tone(3), true),
        ],
    );
    render(&mut r, &s, 100);
    send(
        &s,
        1,
        vec![batch(
            0,
            1,
            "edit",
            vec![write(0, 1, 0xef), write(0, 8, 3), write(0, 9, 2)],
            true,
        )],
    );
    render(&mut r, &s, 1);
    let a = state(&s, 1);
    assert_eq!(a.globals[1], 0xef);
    assert_eq!(a.globals[8], 3);
    assert_eq!(a.globals[9], 2);
    assert_eq!(a.held, [0, 3]);
}
#[test]
fn extra_huc6280_lfo_modulator_owner_can_release_audible_channel_without_touching_wave_or_dda() {
    let (s, mut r) = new();
    send(
        &s,
        1,
        vec![
            group(batch(1, 1, "on", tone(1), false), 1),
            write(0, 8, 3),
            write(0, 9, 2),
            batch(0, 2, "on", tone(0), true),
        ],
    );
    render(&mut r, &s, 100);
    send(&s, 1, vec![group(off(1, 1, false), 1)]);
    render(&mut r, &s, 1);
    let a = state(&s, 1);
    assert!(!gate(&a, 1));
    assert!(gate(&a, 0));
    assert_eq!(a.held, [0]);
    assert_eq!(a.waveforms[1][10], 10);
    assert_eq!(a.globals[9], 2);
}
#[test]
fn extra_huc6280_lfo_modulator_pan_and_volume_edits_preserve_foreign_carrier_pcm() {
    let (s, mut r) = new();
    let (reference, mut reference_renderer) = new();
    for source in [&s, &reference] {
        send(
            source,
            1,
            vec![
                // Mute only CH2 output; its LFO waveform and tick keep running.
                json!({"method":"source-mute","mask":2}),
                group(batch(1, 1, "on", tone(1), false), 1),
                write(0, 8, 3),
                write(0, 9, 2),
                batch(0, 2, "on", tone(0), true),
            ],
        );
    }
    assert_eq!(
        render(&mut r, &s, 100),
        render(&mut reference_renderer, &reference, 100)
    );
    send(
        &s,
        1,
        vec![group(
            batch(
                1,
                1,
                "edit",
                vec![
                    write(1, 5, 0x8f),
                    write(1, 4, 0x91),
                    // These still change the LFO resource and must remain protected.
                    write(1, 2, 9),
                    wave(1, true),
                    write(1, 4, 0xdf),
                ],
                false,
            ),
            1,
        )],
    );
    let actual = render(&mut r, &s, 4096);
    let expected = render(&mut reference_renderer, &reference, 4096);
    let difference = actual
        .iter()
        .zip(&expected)
        .flat_map(|(a, b)| (0..2).map(move |ch| (a[ch] - b[ch]).abs()))
        .fold(0.0f32, f32::max);
    assert_eq!(difference, 0.0, "foreign LFO carrier PCM changed");
    let a = state(&s, 1);
    assert_eq!(a.registers[1][5], 0x8f);
    assert_eq!(a.registers[1][4], 0x91);
    assert_eq!(a.registers[1][2], 253);
    assert_eq!(a.waveforms[1][10], 10);
    assert_eq!(a.held, [0]);
    assert!(gate(&a, 0) && gate(&a, 1));
}

#[test]
fn native_sessions_construct_render_and_drop_on_one_megabyte_stack() {
    const CHILD: &str = "TETORICA_NATIVE_SMALL_STACK_CHILD";
    const TEST: &str = "native_genesis::extra_huc6280_tests::native_sessions_construct_render_and_drop_on_one_megabyte_stack";
    // A stack overflow aborts instead of unwinding. Run the constrained thread
    // in a child process so this regression reports a normal test failure.
    if std::env::var_os(CHILD).is_some() {
        std::thread::Builder::new()
            .name("native-one-megabyte-stack".into())
            .stack_size(1024 * 1024)
            .spawn(|| {
                // This exact two-session PCM comparison overflowed the default
                // Windows test stack when inactive primary snapshots were inline.
                extra_huc6280_lfo_modulator_pan_and_volume_edits_preserve_foreign_carrier_pcm();
                exercise_primary_modes_on_small_stack();
            })
            .unwrap()
            .join()
            .unwrap();
        return;
    }
    println!(
        "Native session stack values: Shared={} Renderer={}",
        std::mem::size_of::<Shared>(),
        std::mem::size_of::<Renderer>()
    );
    assert!(std::mem::size_of::<Shared>() < 32 * 1024);
    assert!(std::mem::size_of::<Renderer>() < 64 * 1024);
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", TEST, "--nocapture"])
        .env(CHILD, "1")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        result.status.success() && stdout.contains("1 passed"),
        "Small-stack session subprocess failed ({:?}):\n{stdout}\n{stderr}",
        result.status
    );
}

#[inline(never)]
fn exercise_primary_modes_on_small_stack() {
    // Exercise the production constructor branches too, including the large
    // SNES RAM snapshot and the FM owner tables. Each allocation is made and
    // released on this control thread; no audio device is opened.
    for mode in [
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
    ] {
        let (mega_cd, dx7, nes, console) = NativeGenesis::mode_config(Some(mode)).unwrap();
        println!("Small-stack primary mode: {mode}");
        let shared = Shared::new_modes(48000, 1, mega_cd, dx7, nes, console);
        let mut renderer = Renderer::new_modes(48000, mega_cd, dx7, nes, console).unwrap();
        assert_eq!(shared.status().sample_rate, 48000);
        assert!(render(&mut renderer, &shared, 16)
            .iter()
            .flatten()
            .all(|sample| sample.is_finite()));
    }
}
