//! Real pinned POKEY cores; no OS output device is opened by these tests.
use super::*;
use serde_json::{json, Value};
fn command(id: u64, frame: u64, message: Value) -> Command {
    serde_json::from_value(
        json!({"type":"extra-pokey","instance":id,"frame":frame,"message":message}),
    )
    .unwrap()
}
fn send(s: &Shared, id: u64, messages: Vec<Value>) {
    s.enqueue(
        1,
        &messages
            .into_iter()
            .map(|m| command(id, 0, m))
            .collect::<Vec<_>>(),
    )
    .unwrap();
}
fn write(r: u8, v: u8) -> Value {
    json!({"method":"write","register":r,"value":v})
}
fn tone() -> Vec<Value> {
    vec![
        write(15, 3),
        write(8, 0),
        write(0, 72),
        write(1, 0xa8),
        write(9, 0),
    ]
}
fn render(r: &mut Renderer, s: &Shared, n: usize) -> Vec<[f32; 2]> {
    let pcm = (0..n)
        .map(|i| {
            if i % 128 == 0 {
                r.prepare(s);
            }
            r.sample(s)
        })
        .collect();
    r.prepare(s);
    pcm
}
fn state(s: &Shared, id: u64) -> extra_pokey::ChipStatus {
    s.extra_pokey_status(id, None).unwrap().pokey
}
fn setup() -> (Shared, Renderer) {
    let s = Shared::new_modes(48000, 1, false, false, false, Some("pokey-dual"));
    let r = Renderer::new_modes(48000, false, false, false, Some("pokey-dual")).unwrap();
    (s, r)
}
fn energy(pcm: &[[f32; 2]]) -> f32 {
    pcm.iter().map(|p| p[0].abs() + p[1].abs()).sum()
}
#[test]
fn extra_pokey_modes_are_empty_buses_with_correct_names() {
    for mode in ["pokey", "pokey-dual"] {
        assert_eq!(
            NativeGenesis::mode_config(Some(mode)).unwrap(),
            (false, false, false, Some(mode))
        );
        let s = Shared::new_modes(48000, 1, false, false, false, Some(mode));
        let mut r = Renderer::new_modes(48000, false, false, false, Some(mode)).unwrap();
        assert!(r.core.0.is_null());
        assert_eq!(s.status().mode, mode);
        assert_eq!(energy(&render(&mut r, &s, 256)), 0.);
    }
}
#[test]
fn extra_pokey_two_cores_have_independent_registers_clock_phase_and_disposal() {
    let (s, mut r) = setup();
    s.create_extra_pokey(1, Some(1789773)).unwrap();
    s.create_extra_pokey(2, Some(1789790)).unwrap();
    send(&s, 1, tone());
    assert!(energy(&render(&mut r, &s, 1024)) > 1.);
    assert_eq!(state(&s, 2).registers, vec![0; 16]);
    assert_eq!(state(&s, 1).clock, 1789773);
    assert_eq!(state(&s, 2).clock, 1789790);
    send(
        &s,
        2,
        vec![write(15, 3), write(4, 90), write(5, 0xa6), write(9, 0)],
    );
    render(&mut r, &s, 256);
    let before = state(&s, 2).registers;
    s.dispose_extra_pokey(1).unwrap();
    assert!(energy(&render(&mut r, &s, 1024)) > 1.);
    assert_eq!(state(&s, 2).registers, before);
    assert!(s.extra_pokey_status(1, None).is_err());
}
#[test]
fn extra_pokey_mute_reset_and_stop_are_exact_zero_without_cross_core_edits() {
    let (s, mut r) = setup();
    s.create_extra_pokey(1, None).unwrap();
    s.create_extra_pokey(2, None).unwrap();
    send(&s, 1, tone());
    render(&mut r, &s, 256);
    send(
        &s,
        1,
        vec![json!({"method":"channel-mute","channel":0,"muted":true})],
    );
    assert_eq!(energy(&render(&mut r, &s, 256)), 0.);
    assert_eq!(state(&s, 1).source_mute_mask, 1);
    send(
        &s,
        1,
        vec![json!({"method":"channel-mute","channel":0,"muted":false})],
    );
    assert!(energy(&render(&mut r, &s, 256)) > 1.);
    send(&s, 1, vec![json!({"method":"reset"})]);
    assert_eq!(energy(&render(&mut r, &s, 256)), 0.);
    assert_eq!(state(&s, 1).registers, vec![0; 16]);
    assert_eq!(state(&s, 1).source_mute_mask, 0);
    assert_eq!(state(&s, 2).registers, vec![0; 16]);
    send(&s, 1, tone());
    render(&mut r, &s, 256);
    send(&s, 1, vec![json!({"method":"stop-voices"})]);
    assert_eq!(energy(&render(&mut r, &s, 256)), 0.);
}
#[test]
fn extra_pokey_read_bank_is_hardware_peek_not_write_shadow() {
    let (s, mut r) = setup();
    s.create_extra_pokey(1, None).unwrap();
    send(
        &s,
        1,
        vec![
            write(0, 72),
            write(10, 0x23),
            json!({"method":"barrier","id":1}),
        ],
    );
    render(&mut r, &s, 256);
    let p = state(&s, 1);
    assert_eq!(p.registers[0], 72);
    assert_ne!(p.read_registers[0], 72);
    assert_eq!(p.read_registers[10], 255);
    s.extra_pokey_status(1, Some(1)).unwrap();
}
#[test]
fn extra_pokey_monitor_mutes_and_retained_levels_are_independent() {
    let (s, mut r) = setup();
    s.create_extra_pokey(1, None).unwrap();
    send(
        &s,
        1,
        vec![
            json!({"method":"set-retained-level","channel":2,"level":13}),
            json!({"method":"channel-mute","channel":0,"muted":true}),
            json!({"method":"channel-mute","channel":2,"muted":true}),
        ],
    );
    render(&mut r, &s, 128);
    let p = state(&s, 1);
    assert_eq!(p.levels, vec![8, 8, 13, 8]);
    assert_eq!(p.source_mute_mask, 5);
    assert_eq!(p.registers[5], 0);
}
#[test]
fn extra_pokey_clock_changes_reject_without_replacing_running_core() {
    let (s, mut r) = setup();
    s.create_extra_pokey(1, None).unwrap();
    send(&s, 1, tone());
    render(&mut r, &s, 256);
    send(&s, 1, vec![json!({"method":"clock","hz":2000000,"id":1})]);
    render(&mut r, &s, 256);
    assert_eq!(state(&s, 1).clock, 1789790);
    assert!(s
        .extra_pokey_status(1, Some(1))
        .unwrap_err()
        .contains("Create a new POKEY"));
    send(&s, 1, vec![json!({"method":"clock","hz":1789790,"id":2})]);
    render(&mut r, &s, 128);
    s.extra_pokey_status(1, Some(2)).unwrap();
    assert!(energy(&render(&mut r, &s, 256)) > 1.);
}
#[test]
fn extra_pokey_cancelled_owner_does_not_stop_manual_keyboard_lease() {
    let (s, mut r) = setup();
    s.create_extra_pokey(1, None).unwrap();
    send(&s, 1, vec![write(15, 3), write(8, 0)]);
    send(
        &s,
        1,
        vec![
            json!({"method":"batch","channel":0,"audition":true,"voice":42,"action":"on","commands":[write(0,72),write(1,0xa8)]}),
        ],
    );
    render(&mut r, &s, 256);
    assert_eq!(state(&s, 1).held, vec![0]);
    send(&s, 1, vec![json!({"method":"stop-voices","group":90})]);
    assert!(energy(&render(&mut r, &s, 256)) > 1.);
    assert_eq!(state(&s, 1).held, vec![0]);
    send(
        &s,
        1,
        vec![
            json!({"method":"batch","channel":0,"audition":true,"voice":41,"action":"off","commands":[{"method":"key-off","channel":0}]}),
        ],
    );
    render(&mut r, &s, 128);
    assert_eq!(state(&s, 1).held, vec![0]);
    send(
        &s,
        1,
        vec![
            json!({"method":"batch","channel":0,"audition":true,"voice":42,"action":"off","commands":[{"method":"key-off","channel":0}]}),
        ],
    );
    assert_eq!(energy(&render(&mut r, &s, 128)), 0.);
    assert!(state(&s, 1).held.is_empty());
}
#[test]
fn extra_pokey_global_stop_retires_both_instances_and_fresh_ids_can_restart() {
    let (s, mut r) = setup();
    s.create_extra_pokey(1, None).unwrap();
    s.create_extra_pokey(2, None).unwrap();
    send(&s, 1, tone());
    send(&s, 2, tone());
    render(&mut r, &s, 256);
    std::thread::scope(|scope| {
        let stop = scope.spawn(|| s.stop());
        while s.epoch.load(Ordering::Acquire) == 0 {
            std::thread::yield_now();
        }
        render(&mut r, &s, 1024);
        stop.join().unwrap().unwrap();
    });
    assert_eq!(energy(&render(&mut r, &s, 128)), 0.);
    assert!(s.extra_pokey_status(1, None).is_err());
    s.create_extra_pokey(3, None).unwrap();
    send(&s, 3, tone());
    assert!(energy(&render(&mut r, &s, 1024)) > 1.);
}
#[test]
fn extra_pokey_invalid_batch_is_atomic_and_slot_budget_covers_tables() {
    let (s, mut r) = setup();
    for id in 1..=16 {
        s.create_extra_pokey(id, None).unwrap();
    }
    assert!(s
        .create_extra_pokey(17, None)
        .unwrap_err()
        .contains("limit"));
    let invalid = command(
        1,
        0,
        json!({"method":"batch","channel":0,"action":"edit","commands":[write(0,72),write(2,90)]}),
    );
    assert!(s.enqueue(1, &[invalid]).is_err());
    render(&mut r, &s, 1);
    assert_eq!(state(&s, 1).registers, vec![0; 16]);
}
