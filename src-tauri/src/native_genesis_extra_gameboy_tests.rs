//! The real DMG C++ core rendered through the additional rack; no audio device.
use super::*;
use serde_json::{json, Value};
fn cmd(id: u64, frame: u64, m: Value) -> Command {
    serde_json::from_value(json!({"type":"extra-gameboy","instance":id,"frame":frame,"message":m}))
        .unwrap()
}
fn send(s: &Shared, id: u64, ms: Vec<Value>) {
    s.enqueue(
        1,
        &ms.into_iter().map(|m| cmd(id, 0, m)).collect::<Vec<_>>(),
    )
    .unwrap()
}
fn write(a: u8, v: u8) -> Value {
    json!({"method":"write","address":a,"value":v})
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
fn state(s: &Shared, id: u64) -> extra_gameboy::ChipStatus {
    s.extra_gameboy_status(id, None).unwrap().gameboy
}
fn new() -> (Shared, Renderer) {
    let s = Shared::new(48000, 1);
    let r = Renderer::new(48000).unwrap();
    s.create_extra_gameboy(1, None).unwrap();
    init(&s, 1);
    (s, r)
}
fn init(s: &Shared, id: u64) {
    send(
        s,
        id,
        vec![write(22, 128), write(20, 0x77), write(21, 0xff)],
    )
}
fn pulse(ch: u8) -> Vec<Value> {
    let b = ch * 5;
    vec![
        write(b + 1, 128),
        write(b + 2, 0xa0),
        write(b + 3, 34),
        write(b + 4, 135),
    ]
}
fn batch(ch: u8, v: u64, action: &str, commands: Vec<Value>, audition: bool) -> Value {
    json!({"method":"batch","channel":ch,"voice":v,"action":action,"commands":commands,"audition":audition})
}
fn off(ch: u8, v: u64, a: bool) -> Value {
    batch(ch, v, "off", vec![write(ch * 5 + 2, 0)], a)
}
fn energy(p: &[[f32; 2]], ch: usize) -> f32 {
    p.iter().map(|p| p[ch].abs()).sum()
}
#[test]
fn extra_gameboy_fifteen_kinds_share_sixteen_slots_and_independent_clocks() {
    let (s, mut r) = new();
    s.create_extra_gameboy(2, Some(2097152)).unwrap();
    init(&s, 2);
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
        .create_extra_gameboy(17, None)
        .unwrap_err()
        .contains("limit"));
    send(&s, 1, vec![write(21, 0x10)]);
    send(&s, 1, pulse(0));
    send(&s, 2, vec![write(21, 0x01)]);
    send(&s, 2, pulse(0));
    let p = render(&mut r, &s, 4096);
    assert!(energy(&p, 0) > 10. && energy(&p, 1) > 10.);
    assert_eq!(state(&s, 1).clock, 4194304);
    assert_eq!(state(&s, 2).clock, 2097152);
    assert!(r.gameboy.is_none());
    println!(
        "GameBoy layout Instance={} Shared={} Renderer={} Session={}",
        std::mem::size_of::<extra_psg::Instance>(),
        std::mem::size_of::<extra_gameboy::Shared>(),
        std::mem::size_of::<extra_gameboy::Renderer>(),
        std::mem::size_of::<Renderer>()
    );
    assert!(
        std::mem::size_of::<extra_psg::Instance>()
            + std::mem::size_of::<extra_gameboy::Renderer>()
            + 16384
            < 65536
    );
}
#[test]
fn extra_gameboy_reset_retains_clock_and_mute_restores_power_wave_ram() {
    let (s, mut r) = new();
    send(
        &s,
        1,
        vec![
            write(32, 18),
            json!({"method":"clock","hz":2097152}),
            json!({"method":"source-mute","mask":3}),
        ],
    );
    send(&s, 1, pulse(0));
    render(&mut r, &s, 100);
    send(&s, 1, vec![json!({"method":"reset"})]);
    let p = render(&mut r, &s, 100);
    let v = state(&s, 1);
    assert_eq!(v.clock, 2097152);
    assert_eq!(v.source_mute_mask, 3);
    assert_eq!(v.registers[32], 0xac);
    assert_eq!(v.gates, [false; 4]);
    assert!(energy(&p, 0) < 1e-6);
    assert_eq!(v.voice[2], 0);
}
#[test]
fn extra_gameboy_wave_ram_and_noise_do_not_leak_between_instances() {
    let (s, mut r) = new();
    s.create_extra_gameboy(2, None).unwrap();
    init(&s, 2);
    send(
        &s,
        1,
        vec![
            write(10, 0),
            write(32, 18),
            write(12, 32),
            write(10, 128),
            write(13, 34),
            write(14, 135),
        ],
    );
    send(
        &s,
        2,
        vec![write(15, 0), write(17, 0xa0), write(18, 5), write(19, 128)],
    );
    assert!(energy(&render(&mut r, &s, 2000), 0) > 1.);
    assert_eq!(state(&s, 1).registers[32], 18);
    assert_eq!(state(&s, 2).registers[32], 0xac);
    assert!(state(&s, 1).gates[2]);
    assert!(state(&s, 2).gates[3]);
    assert!(!state(&s, 1).gates[3]);
    send(&s, 1, vec![json!({"method":"stop-voices"})]);
    render(&mut r, &s, 1);
    assert!(!state(&s, 1).gates[2]);
    assert!(state(&s, 2).gates[3]);
}
#[test]
fn extra_gameboy_manual_new_voice_replaces_stale_release_protects_all_global_controls() {
    let (s, mut r) = new();
    send(&s, 1, vec![batch(0, 1, "on", pulse(0), true)]);
    render(&mut r, &s, 100);
    send(&s, 1, vec![batch(0, 2, "on", pulse(0), true)]);
    render(&mut r, &s, 100);
    send(
        &s,
        1,
        vec![
            off(0, 1, true),
            write(2, 0),
            write(20, 0),
            write(21, 0),
            write(22, 0),
            json!({"method":"reset"}),
            json!({"method":"stop-voices"}),
        ],
    );
    render(&mut r, &s, 100);
    let v = state(&s, 1);
    assert!(v.gates[0]);
    assert_eq!(v.held, [0]);
    assert_eq!(v.registers[21] & 0x11, 0x11);
    assert_eq!(v.registers[20], 0x77);
    assert_eq!(v.registers[22] & 128, 128);
    send(&s, 1, vec![off(0, 2, true)]);
    render(&mut r, &s, 100);
    let v = state(&s, 1);
    assert!(!v.gates[0]);
    assert!(v.held.is_empty());
    assert_eq!(v.voice[2], 0xa0);
}
#[test]
fn extra_gameboy_main_groups_foreign_off_pan_power_and_cancel_are_scoped() {
    let (s, mut r) = new();
    send(
        &s,
        1,
        vec![
            group(batch(0, 1, "on", pulse(0), false), 1),
            group(batch(1, 2, "on", pulse(1), false), 2),
        ],
    );
    render(&mut r, &s, 100);
    send(
        &s,
        1,
        vec![
            group(write(2, 0), 2),
            group(write(20, 0), 2),
            group(write(21, 0), 2),
            group(write(22, 0), 2),
        ],
    );
    render(&mut r, &s, 100);
    let v = state(&s, 1);
    assert!(v.gates[0]);
    assert!(!v.gates[1]);
    assert_eq!(v.registers[21] & 0x11, 0x11);
    assert_eq!(v.registers[20], 0x77);
    assert_eq!(v.registers[22] & 128, 128);
    send(
        &s,
        1,
        vec![json!({"method":"cancel-main-owner","targetGroup":2})],
    );
    render(&mut r, &s, 100);
    assert!(state(&s, 1).gates[0]);
    send(
        &s,
        1,
        vec![json!({"method":"cancel-main-owner","targetGroup":1})],
    );
    render(&mut r, &s, 100);
    assert!(!state(&s, 1).gates[0]);
}
#[test]
fn extra_gameboy_manual_wave_ram_keeps_dac_and_memory_when_other_owner_edits() {
    let (s, mut r) = new();
    send(
        &s,
        1,
        vec![
            write(32, 18),
            batch(
                2,
                7,
                "on",
                vec![write(10, 128), write(12, 32), write(13, 34), write(14, 135)],
                true,
            ),
        ],
    );
    render(&mut r, &s, 100);
    send(
        &s,
        1,
        vec![
            group(write(10, 0), 1),
            group(write(32, 255), 1),
            group(write(21, 0), 1),
            group(write(22, 0), 1),
        ],
    );
    render(&mut r, &s, 100);
    let v = state(&s, 1);
    assert!(v.gates[2]);
    assert_eq!(v.held, [2]);
    assert_eq!(v.registers[32], 18);
    assert_eq!(v.registers[21] & 0x44, 0x44);
}
#[test]
fn extra_gameboy_finite_length_manual_voice_releases_clock_hold() {
    let (s, mut r) = new();
    let mut p = pulse(0);
    // Length 62 leaves one count after DMG's immediate enable tick.
    p[0] = write(1, 0xbe);
    p[3] = write(4, 0xc7);
    send(&s, 1, vec![batch(0, 1, "on", p, true)]);
    render(&mut r, &s, 1);
    assert_eq!(state(&s, 1).held, [0]);
    render(&mut r, &s, 2000);
    let v = state(&s, 1);
    assert!(!v.gates[0]);
    assert!(v.held.is_empty());
    send(&s, 1, vec![json!({"method":"clock","hz":2097152})]);
    render(&mut r, &s, 1);
    assert_eq!(state(&s, 1).clock, 2097152);
}
#[test]
fn extra_gameboy_clock_held_admission_and_application_recheck_preserve_frequency() {
    let (s, mut r) = new();
    s.enqueue(
        1,
        &[cmd(1, 1000, json!({"method":"clock","hz":2097152,"id":1}))],
    )
    .unwrap();
    send(&s, 1, vec![batch(0, 1, "on", pulse(0), true)]);
    render(&mut r, &s, 1200);
    assert!(s
        .extra_gameboy_status(1, Some(1))
        .unwrap_err()
        .contains("manually held"));
    assert_eq!(state(&s, 1).clock, 4194304);
    assert!(s
        .enqueue(1, &[cmd(1, 0, json!({"method":"clock","hz":2097152}))])
        .unwrap_err()
        .contains("manually held"));
    send(
        &s,
        1,
        vec![json!({"method":"clock","hz":2097152,"audition":true,"id":2})],
    );
    render(&mut r, &s, 1);
    assert_eq!(
        s.extra_gameboy_status(1, Some(2)).unwrap().gameboy.clock,
        2097152
    );
    assert_eq!(state(&s, 1).held, [0]);
}
#[test]
fn extra_gameboy_source_mute_retains_registers_gates_and_audio_phase() {
    let (s, mut r) = new();
    send(&s, 1, pulse(0));
    assert!(energy(&render(&mut r, &s, 2000), 0) > 1.);
    let before = state(&s, 1).registers;
    send(&s, 1, vec![json!({"method":"source-mute","mask":1})]);
    let p = render(&mut r, &s, 1000);
    let v = state(&s, 1);
    assert_eq!(v.registers, before);
    assert!(v.gates[0]);
    assert_eq!(v.source_mute_mask, 1);
    assert!(energy(&p, 0) < 1e-6);
    send(&s, 1, vec![json!({"method":"source-mute","mask":0})]);
    assert!(energy(&render(&mut r, &s, 1000), 0) > 1.);
}
#[test]
fn extra_gameboy_worker_cancel_future_preserves_manual_and_reclaims_payloads() {
    let (s, mut r) = new();
    s.performance("start", 7, None).unwrap();
    send(&s, 1, vec![batch(0, 1, "on", pulse(0), true)]);
    let mut c = cmd(
        1,
        100000,
        json!({"method":"batch","channel":1,"voice":2,"action":"on","commands":pulse(1),"id":10}),
    );
    c.performance = Some(7);
    c.owner = Some(1);
    s.enqueue(1, &[c]).unwrap();
    render(&mut r, &s, 100);
    s.performance("cancel-owner", 7, Some(1)).unwrap();
    render(&mut r, &s, 1);
    assert!(s
        .extra_gameboy_status(1, Some(10))
        .unwrap_err()
        .contains("cancelled"));
    assert!(state(&s, 1).gates[0]);
    assert!(!state(&s, 1).gates[1]);
    s.extra_psg.reserve_payloads(std::iter::empty()).unwrap();
    assert_eq!(s.extra_psg.memory_used(), extra_budget::INSTANCE_BYTES);
}
#[test]
fn extra_gameboy_main_cancel_invalidates_admitted_future_but_not_another_group() {
    let (s, mut r) = new();
    s.enqueue(
        1,
        &[
            cmd(
                1,
                100000,
                group(json!({"method":"clock","hz":2097152,"id":1}), 1),
            ),
            cmd(
                1,
                100000,
                group(json!({"method":"clock","hz":1048576,"id":2}), 2),
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
        .extra_gameboy_status(1, Some(1))
        .unwrap_err()
        .contains("cancelled"));
    assert_eq!(state(&s, 1).pending_commands, [2]);
    render(&mut r, &s, 100100);
    assert_eq!(
        s.extra_gameboy_status(1, Some(2)).unwrap().gameboy.clock,
        1048576
    );
}
#[test]
fn extra_gameboy_failed_other_family_retention_does_not_commit_main_cancel() {
    let (s, mut r) = new();
    s.create_extra_okim6258(2, None).unwrap();
    s.enqueue(
        1,
        &[cmd(
            1,
            1000,
            group(json!({"method":"clock","hz":2097152,"id":1}), 42),
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
        s.extra_gameboy_status(1, Some(1)).unwrap().gameboy.clock,
        2097152
    );
}
#[test]
fn extra_gameboy_after_wait_does_not_block_other_control_writes() {
    use std::{sync::mpsc, time::Duration};
    let (s, mut r) = new();
    let s = Arc::new(s);
    s.enqueue(
        1,
        &[cmd(1, 1000, json!({"method":"clock","hz":2097152,"id":1}))],
    )
    .unwrap();
    let (tx, rx) = mpsc::channel();
    let waiting = s.clone();
    let task =
        std::thread::spawn(move || tx.send(waiting.extra_gameboy_status(1, Some(1))).unwrap());
    assert!(rx.recv_timeout(Duration::from_millis(20)).is_err());
    send(&s, 1, vec![write(32, 18)]);
    render(&mut r, &s, 1200);
    assert_eq!(
        rx.recv_timeout(Duration::from_millis(200))
            .unwrap()
            .unwrap()
            .gameboy
            .clock,
        2097152
    );
    task.join().unwrap();
    assert_eq!(state(&s, 1).registers[32], 18);
}
#[test]
fn extra_gameboy_invalid_commands_do_not_admit_or_mutate() {
    let (s, mut r) = new();
    render(&mut r, &s, 1);
    let initial = state(&s, 1).registers;
    for m in [
        write(48, 0),
        json!({"method":"clock","hz":99999}),
        json!({"method":"source-mute","mask":16}),
        json!({"method":"cancel-main-owner","targetGroup":0}),
        batch(0, 1, "on", pulse(1), false),
        batch(0, 1, "on", vec![write(22, 128)], false),
        batch(4, 1, "on", pulse(0), false),
        batch(0, 1, "on", vec![], false),
        batch(0, 1, "on", vec![write(1, 0); 513], false),
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
    render(&mut r, &s, 1);
    assert_eq!(state(&s, 1).registers, initial);
    assert!(state(&s, 1).pending_commands.is_empty());
}
#[test]
fn extra_gameboy_duplicate_pending_ids_and_aggregate_budget_roll_back() {
    let (s, mut r) = new();
    render(&mut r, &s, 1);
    let c = || cmd(1, 10000, json!({"method":"clock","hz":2097152,"id":1}));
    assert!(s.enqueue(1, &[c(), c()]).unwrap_err().contains("Duplicate"));
    assert!(state(&s, 1).pending_commands.is_empty());
    s.extra_psg.reserve_payloads(std::iter::empty()).unwrap();
    assert_eq!(s.extra_psg.memory_used(), extra_budget::INSTANCE_BYTES);
    let mut limited = Shared::new(48000, 1);
    limited.extra_psg = extra_psg::Shared::with_memory_limit(extra_budget::INSTANCE_BYTES + 300);
    limited.create_extra_gameboy(1, None).unwrap();
    assert!(limited
        .enqueue(
            1,
            &[cmd(
                1,
                10000,
                batch(0, 1, "on", vec![write(1, 0); 512], false)
            )]
        )
        .unwrap_err()
        .contains("budget"));
    assert_eq!(state(&limited, 1).clock, 4194304);
    assert_eq!(
        limited.extra_psg.memory_used(),
        extra_budget::INSTANCE_BYTES
    );
}
#[test]
fn extra_gameboy_dispose_stale_instance_rejected_and_budget_freed_on_control() {
    let (s, mut r) = new();
    send(&s, 1, pulse(0));
    render(&mut r, &s, 100);
    s.dispose_extra_gameboy(1).unwrap();
    render(&mut r, &s, 1);
    s.extra_psg.collect_retired().unwrap();
    assert_eq!(s.extra_psg.counts(), (0, 0));
    assert_eq!(s.extra_psg.memory_used(), 0);
    assert!(s.create_extra_gameboy(1, None).is_err());
    s.create_extra_gameboy(2, None).unwrap();
    init(&s, 2);
    send(&s, 2, pulse(0));
    assert!(s.enqueue(1, &[cmd(1, 0, write(2, 0))]).is_err());
    assert!(energy(&render(&mut r, &s, 1000), 0) > 1.);
    assert!(state(&s, 2).gates[0]);
}
#[test]
fn extra_gameboy_stop_invalidates_older_future_events_without_destroying_voice_patch() {
    let (s, mut r) = new();
    send(&s, 1, pulse(0));
    s.enqueue(
        1,
        &[cmd(1, 1000, json!({"method":"clock","hz":2097152,"id":1}))],
    )
    .unwrap();
    render(&mut r, &s, 100);
    send(&s, 1, vec![json!({"method":"stop-voices"})]);
    let p = render(&mut r, &s, 1200);
    assert!(energy(&p[20..], 0) < 1e-6);
    assert!(s
        .extra_gameboy_status(1, Some(1))
        .unwrap_err()
        .contains("cancelled"));
    let v = state(&s, 1);
    assert_eq!(v.clock, 4194304);
    assert_eq!(v.voice[2], 0xa0);
    assert!(!v.gates[0]);
}

#[test]
fn extra_gameboy_bounded_completion_outcomes_do_not_expire_a_future_waiter() {
    use std::{sync::mpsc, time::Duration};
    let (s, mut r) = new();
    let s = Arc::new(s);
    s.enqueue(
        1,
        &[cmd(1, 1000, json!({"method":"clock","hz":2097152,"id":1}))],
    )
    .unwrap();
    s.enqueue(
        1,
        &(2..=601)
            .map(|id| {
                cmd(
                    1,
                    0,
                    json!({"method":"write","address":32,"value":18,"id":id}),
                )
            })
            .collect::<Vec<_>>(),
    )
    .unwrap();
    render(&mut r, &s, 1);
    state(&s, 1);
    s.extra_psg.reserve_payloads(std::iter::empty()).unwrap();
    assert!(s
        .extra_gameboy_status(1, Some(2))
        .unwrap_err()
        .contains("expired"));
    assert!(s.extra_gameboy_status(1, Some(601)).is_ok());
    let (tx, rx) = mpsc::channel();
    let waiting = s.clone();
    let task =
        std::thread::spawn(move || tx.send(waiting.extra_gameboy_status(1, Some(1))).unwrap());
    assert!(rx.recv_timeout(Duration::from_millis(20)).is_err());
    render(&mut r, &s, 1200);
    assert_eq!(
        rx.recv_timeout(Duration::from_millis(200))
            .unwrap()
            .unwrap()
            .gameboy
            .clock,
        2097152
    );
    task.join().unwrap();
}
