//! Real callback-owned NES/FDS cores. No physical audio device is opened.
use super::*;
use serde_json::{json, Value};
fn cmd(id: u64, frame: u64, m: Value) -> Command {
    serde_json::from_value(json!({"type":"extra-nes","instance":id,"frame":frame,"message":m}))
        .unwrap()
}
fn send(s: &Shared, id: u64, ms: Vec<Value>) {
    s.enqueue(
        1,
        &ms.into_iter().map(|m| cmd(id, 0, m)).collect::<Vec<_>>(),
    )
    .unwrap()
}
fn write(a: u16, v: u8) -> Value {
    json!({"method":"write","address":a,"value":v})
}
fn enable(ch: u8, on: bool) -> Value {
    json!({"method":"enable","channel":ch,"enabled":on})
}
fn group(mut m: Value, g: u64) -> Value {
    m["group"] = json!(g);
    m
}
fn memory(offset: u32, data: Vec<u8>, id: u64) -> Value {
    json!({"method":"memory","offset":offset,"data":data,"id":id})
}
fn batch(ch: u8, v: u64, a: &str, ops: Vec<Value>, manual: bool) -> Value {
    json!({"method":"batch","channel":ch,"voice":v,"action":a,"commands":ops,"audition":manual})
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
fn state(s: &Shared, id: u64) -> extra_nes::ChipStatus {
    s.extra_nes_status(id, None).unwrap().nes
}
fn new() -> (Shared, Renderer) {
    let s = Shared::new(48000, 1);
    let r = Renderer::new(48000).unwrap();
    s.create_extra_nes(1, None, true).unwrap();
    (s, r)
}
fn pulse(ch: u8) -> Vec<Value> {
    let b = 0x4000 + ch as u16 * 4;
    vec![
        enable(ch, true),
        write(b, 0xbf),
        write(b + 1, 0),
        write(b + 2, 0x40),
        write(b + 3, 8),
    ]
}
fn fds() -> Vec<Value> {
    let mut v = vec![write(0x4023, 2), write(0x4089, 128)];
    for i in 0..64 {
        v.push(write(0x4040 + i, (i & 63) as u8));
    }
    v.extend([
        write(0x4089, 0),
        write(0x4080, 0xa0),
        write(0x4082, 80),
        write(0x4083, 1),
    ]);
    v
}
fn dmc() -> Vec<Value> {
    vec![
        write(0x4010, 0x4f),
        write(0x4012, 0),
        write(0x4013, 0),
        enable(4, true),
    ]
}
fn energy(p: &[[f32; 2]]) -> f32 {
    p.iter().map(|p| p[0].abs() + p[1].abs()).sum()
}
#[test]
fn extra_nes_sixteen_kinds_share_slots_and_fractional_pal_constructor() {
    let (s, mut r) = new();
    s.create_extra_gameboy(2, None).unwrap();
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
        .create_extra_nes(17, None, true)
        .unwrap_err()
        .contains("limit"));
    send(&s, 1, pulse(0));
    assert!(energy(&render(&mut r, &s, 4096)) > 1.);
    let n = Shared::new(48000, 1);
    n.create_extra_nes(1, Some(1662607.125), false).unwrap();
    let v = state(&n, 1);
    assert_eq!(v.clock, 1662607.125);
    assert_eq!(v.base_clock, 1662607.125);
    assert!(!v.fds_enabled);
    assert!(r.nes.is_none());
    println!(
        "NES layout Instance={} Shared={} Renderer={} Core={} Session={}",
        std::mem::size_of::<extra_psg::Instance>(),
        std::mem::size_of::<extra_nes::Shared>(),
        std::mem::size_of::<extra_nes::Renderer>(),
        unsafe { extra_nes::native_nes_core_size() },
        std::mem::size_of::<Renderer>()
    );
    assert!(
        std::mem::size_of::<extra_psg::Instance>()
            + 16384 // The two control-allocated fixed coverage bitmaps.
            + (unsafe { extra_nes::native_nes_core_size() } as usize)
            < 65536 + extra_nes::RAM
    );
}
#[test]
fn extra_nes_two_instances_keep_ram_wave_modulation_clock_and_fds_option() {
    let (s, mut r) = new();
    s.create_extra_nes(2, Some(1662607.125), false).unwrap();
    send(&s, 1, vec![memory(0xc000, vec![0x55; 17], 1)]);
    send(&s, 2, vec![memory(0xc000, vec![0xaa; 17], 1)]);
    send(&s, 1, fds());
    send(&s, 2, fds());
    send(&s, 2, pulse(1));
    render(&mut r, &s, 2000);
    let a = state(&s, 1);
    let b = state(&s, 2);
    assert!(a.fds_active);
    assert!(!b.fds_active);
    assert_eq!(a.registers[0x41], 1);
    assert_eq!(b.registers[0x41], 0);
    assert!(b.gates[1]);
    assert!(a
        .memory_ranges
        .iter()
        .any(|v| v.address == 0xc000 && v.end == 0xc011));
    assert_eq!(a.clock, 1789773.);
    assert_eq!(b.clock, 1662607.125);
}
#[test]
fn extra_nes_overlapping_future_uploads_follow_applied_time_and_reset_order() {
    let state = extra_nes::Shared::default();
    let mut r = extra_nes::Renderer::new(48000, 1789773., true).unwrap();
    let apply = |r: &mut extra_nes::Renderer, m: Value| {
        let m: extra_nes::Message = serde_json::from_value(m).unwrap();
        let p = m.prepare().unwrap();
        r.apply(&p, None, &state)
    };
    apply(&mut r, memory(0xc000, vec![1, 2, 3], 1));
    apply(&mut r, memory(0xc001, vec![9], 2));
    assert_eq!(r.memory_byte(0xc000), 1);
    assert_eq!(r.memory_byte(0xc001), 9);
    assert_eq!(r.memory_byte(0xc002), 3);
    apply(&mut r, json!({"method":"reset"}));
    assert_eq!(r.memory_byte(0xc000), 0);
    apply(&mut r, memory(0xc002, vec![8], 3));
    assert_eq!(r.memory_byte(0xc000), 0);
    assert_eq!(r.memory_byte(0xc002), 8);
    let (s, mut r) = new();
    s.enqueue(
        1,
        &[
            cmd(1, 1000, memory(0xc000, vec![1, 2], 1)),
            cmd(1, 500, memory(0xc001, vec![9], 2)),
        ],
    )
    .unwrap();
    render(&mut r, &s, 600);
    assert_eq!(state_of(&s, 1).memory_ranges[0].address, 0xc001);
    render(&mut r, &s, 500);
    assert_eq!(state_of(&s, 1).memory_ranges[0].address, 0xc000);
}
fn state_of(s: &Shared, id: u64) -> extra_nes::ChipStatus {
    state(s, id)
}
#[test]
fn extra_nes_reset_clears_memory_wave_and_voices_retains_clock_mute_region() {
    let (s, mut r) = new();
    send(
        &s,
        1,
        vec![
            memory(65535, vec![7], 1),
            json!({"method":"clock","hz":1000000}),
            json!({"method":"source-mute","mask":2}),
        ],
    );
    send(&s, 1, fds());
    send(&s, 1, pulse(0));
    render(&mut r, &s, 100);
    send(&s, 1, vec![json!({"method":"reset"})]);
    render(&mut r, &s, 1);
    let v = state(&s, 1);
    assert_eq!(v.clock, 1000000.);
    assert_eq!(v.base_clock, 1789773.);
    assert_eq!(v.source_mute_mask, 2);
    assert!(v.memory_ranges.is_empty());
    assert_eq!(v.registers[0x41], 0);
    assert_eq!(v.gates, [false; 6]);
}
#[test]
fn extra_nes_stale_manual_off_global_enable_and_dac_do_not_steal_current_voice() {
    let (s, mut r) = new();
    send(&s, 1, vec![batch(0, 1, "on", pulse(0), true)]);
    render(&mut r, &s, 1);
    send(&s, 1, vec![batch(0, 2, "on", pulse(0), true)]);
    render(&mut r, &s, 1);
    send(
        &s,
        1,
        vec![
            batch(0, 1, "off", vec![enable(0, false)], true),
            write(0x4015, 0),
            write(0x4017, 0),
            write(0x4000, 0),
            json!({"method":"reset"}),
            json!({"method":"stop-voices"}),
        ],
    );
    render(&mut r, &s, 100);
    let v = state(&s, 1);
    assert!(v.gates[0]);
    assert_eq!(v.held, [0]);
    send(
        &s,
        1,
        vec![batch(0, 2, "off", vec![enable(0, false)], true)],
    );
    render(&mut r, &s, 1);
    assert!(!state(&s, 1).gates[0]);
}
#[test]
fn extra_nes_manual_fds_wave_modulation_io_and_gain_are_protected() {
    let (s, mut r) = new();
    send(&s, 1, vec![batch(5, 8, "on", fds(), true)]);
    render(&mut r, &s, 100);
    send(
        &s,
        1,
        vec![
            group(write(0x4023, 0), 1),
            group(write(0x4041, 63), 1),
            group(write(0x4080, 128), 1),
            group(write(0x4083, 128), 1),
            group(write(0x4088, 7), 1),
            group(write(0x4089, 128), 1),
        ],
    );
    render(&mut r, &s, 100);
    let v = state(&s, 1);
    assert!(v.fds_active);
    assert_eq!(v.held, [5]);
    assert_eq!(v.registers[0x41], 1);
    assert_eq!(v.fds_gain, 32);
    assert_eq!(v.modulation, [0; 32]);
}
#[test]
fn extra_nes_main_owner_stop_and_raw_retrigger_preserve_later_owner() {
    let (s, mut r) = new();
    send(&s, 1, pulse(0).into_iter().map(|m| group(m, 1)).collect());
    render(&mut r, &s, 1);
    send(&s, 1, vec![group(write(0x4003, 8), 2)]);
    render(&mut r, &s, 1);
    send(
        &s,
        1,
        vec![
            group(enable(0, false), 1),
            json!({"method":"cancel-main-owner","targetGroup":1}),
        ],
    );
    render(&mut r, &s, 1);
    assert!(state(&s, 1).gates[0]);
    send(
        &s,
        1,
        vec![json!({"method":"cancel-main-owner","targetGroup":2})],
    );
    render(&mut r, &s, 1);
    assert!(!state(&s, 1).gates[0]);
}
#[test]
fn extra_nes_manual_dmc_holds_memory_and_clock_at_admission_and_application() {
    let (s, mut r) = new();
    send(&s, 1, vec![memory(0xc000, vec![0x55; 17], 1)]);
    s.enqueue(
        1,
        &[
            cmd(1, 1000, memory(0xc000, vec![0xaa; 17], 2)),
            cmd(1, 1000, json!({"method":"clock","hz":1000000,"id":3})),
        ],
    )
    .unwrap();
    send(&s, 1, vec![batch(4, 10, "on", dmc(), true)]);
    render(&mut r, &s, 1200);
    assert_eq!(state(&s, 1).held, [4]);
    assert!(s
        .extra_nes_status(1, Some(2))
        .unwrap_err()
        .contains("manually held"));
    assert!(s
        .extra_nes_status(1, Some(3))
        .unwrap_err()
        .contains("manually held"));
    assert_eq!(state(&s, 1).clock, 1789773.);
    assert!(s
        .enqueue(1, &[cmd(1, 0, memory(0xc000, vec![0], 4))])
        .unwrap_err()
        .contains("manually held"));
    send(&s, 1, vec![write(0x4011, 0), write(0x4015, 0)]);
    render(&mut r, &s, 1);
    assert!(state(&s, 1).dmc_active);
}
#[test]
fn extra_nes_finite_length_releases_manual_clock_hold() {
    let (s, mut r) = new();
    let mut p = pulse(0);
    p[1] = write(0x4000, 0x9f);
    p[4] = write(0x4003, 0x18);
    send(&s, 1, vec![batch(0, 1, "on", p, true)]);
    render(&mut r, &s, 1);
    assert_eq!(state(&s, 1).held, [0]);
    render(&mut r, &s, 3000);
    assert!(state(&s, 1).held.is_empty());
    send(&s, 1, vec![json!({"method":"clock","hz":1000000})]);
    render(&mut r, &s, 1);
    assert_eq!(state(&s, 1).clock, 1000000.);
}
#[test]
fn extra_nes_source_mute_retains_gates_and_registers() {
    let (s, mut r) = new();
    send(&s, 1, pulse(0));
    let audible = energy(&render(&mut r, &s, 2000));
    assert!(audible > 1.);
    let before = state(&s, 1).registers;
    send(&s, 1, vec![json!({"method":"source-mute","mask":1})]);
    render(&mut r, &s, 3000);
    let p = render(&mut r, &s, 1000);
    let v = state(&s, 1);
    assert_eq!(v.registers, before);
    assert!(v.gates[0]);
    assert!(p.iter().flatten().all(|v| v.is_finite()));
    assert!(energy(&p) < audible * 0.01);
    send(&s, 1, vec![json!({"method":"source-mute","mask":0})]);
    assert!(energy(&render(&mut r, &s, 2000)) > audible * 0.5);
}
#[test]
fn extra_nes_worker_and_main_cancel_invalidate_future_uploads_without_affecting_manual() {
    let (s, mut r) = new();
    s.performance("start", 7, None).unwrap();
    send(&s, 1, vec![batch(0, 1, "on", pulse(0), true)]);
    let mut c = cmd(1, 100000, memory(0xc000, vec![7; 1024], 10));
    c.performance = Some(7);
    c.owner = Some(1);
    s.enqueue(1, &[c]).unwrap();
    s.enqueue(
        1,
        &[cmd(1, 100000, group(memory(0xc800, vec![9; 1024], 11), 42))],
    )
    .unwrap();
    render(&mut r, &s, 100);
    s.performance("cancel-owner", 7, Some(1)).unwrap();
    send(
        &s,
        1,
        vec![json!({"method":"cancel-main-owner","targetGroup":42})],
    );
    render(&mut r, &s, 1);
    for id in [10, 11] {
        assert!(s
            .extra_nes_status(1, Some(id))
            .unwrap_err()
            .contains("cancelled"));
    }
    assert!(state(&s, 1).gates[0]);
    assert!(state(&s, 1).memory_ranges.is_empty());
    s.extra_psg.reserve_payloads(std::iter::empty()).unwrap();
    assert_eq!(
        s.extra_psg.memory_used(),
        extra_budget::INSTANCE_BYTES + extra_nes::RAM
    );
}
#[test]
fn extra_nes_other_family_failed_admission_rolls_back_cancel() {
    let (s, mut r) = new();
    s.create_extra_gameboy(2, None).unwrap();
    s.enqueue(1, &[cmd(1, 1000, group(memory(0xc000, vec![7], 1), 42))])
        .unwrap();
    let gb = |m: Value| {
        serde_json::from_value::<Command>(
            json!({"type":"extra-gameboy","instance":2,"frame":0,"message":m}),
        )
        .unwrap()
    };
    s.enqueue(1, &[gb(json!({"method":"clock","hz":2000000,"id":1}))])
        .unwrap();
    assert!(s
        .enqueue(
            1,
            &[
                cmd(1, 0, json!({"method":"cancel-main-owner","targetGroup":42})),
                gb(json!({"method":"clock","hz":1000000,"id":1}))
            ]
        )
        .unwrap_err()
        .contains("Duplicate"));
    render(&mut r, &s, 1200);
    assert!(s.extra_nes_status(1, Some(1)).is_ok());
    assert_eq!(state(&s, 1).memory_ranges[0].address, 0xc000);
}
#[test]
fn extra_nes_upload_batch_and_shared_pending_limits_bound_callback_bursts() {
    let (s, mut r) = new();
    assert!(s
        .enqueue(
            1,
            &[
                cmd(1, 0, memory(0, vec![1; 65536], 1)),
                cmd(1, 0, memory(0, vec![2], 2))
            ]
        )
        .unwrap_err()
        .contains("64 KiB"));
    for i in 0..16 {
        s.enqueue(1, &[cmd(1, 100000, memory(0, vec![i as u8; 65536], i + 1))])
            .unwrap();
    }
    assert!(s
        .enqueue(1, &[cmd(1, 0, memory(0, vec![9], 17))])
        .unwrap_err()
        .contains("1 MiB"));
    send(&s, 1, vec![json!({"method":"stop-voices"})]);
    render(&mut r, &s, 1);
    s.enqueue(1, &[cmd(1, 0, memory(65536, vec![], 18))])
        .unwrap();
    render(&mut r, &s, 1);
    assert!(s.extra_nes_status(1, Some(18)).is_ok());
}
#[test]
fn extra_nes_after_wait_does_not_hold_control_fifo() {
    use std::{sync::mpsc, time::Duration};
    let (s, mut r) = new();
    let s = Arc::new(s);
    s.enqueue(1, &[cmd(1, 1000, memory(0xc000, vec![5], 1))])
        .unwrap();
    let (tx, rx) = mpsc::channel();
    let shared = s.clone();
    let t = std::thread::spawn(move || tx.send(shared.extra_nes_status(1, Some(1))).unwrap());
    assert!(rx.recv_timeout(Duration::from_millis(20)).is_err());
    send(&s, 1, pulse(0));
    render(&mut r, &s, 1200);
    assert!(rx.recv_timeout(Duration::from_millis(200)).unwrap().is_ok());
    t.join().unwrap();
    assert!(state(&s, 1).gates[0]);
}
#[test]
fn extra_nes_invalid_commands_constructor_and_batch_are_atomic() {
    let (s, mut r) = new();
    render(&mut r, &s, 1);
    let before = state(&s, 1).registers;
    for m in [
        write(0x4090, 0),
        json!({"method":"clock","hz":99999}),
        json!({"method":"source-mute","mask":64}),
        memory(65536, vec![1], 1),
        json!({"method":"cancel-main-owner","targetGroup":0}),
        batch(0, 1, "on", pulse(1), false),
        batch(0, 1, "on", vec![write(0x4015, 0)], false),
        batch(6, 1, "on", pulse(0), false),
        batch(0, 1, "on", vec![write(0x4000, 1); 513], false),
    ] {
        assert!(s.enqueue(1, &[cmd(1, 0, m)]).is_err());
    }
    for clock in [1599999., 1900001., f64::NAN] {
        assert!(s.create_extra_nes(2, Some(clock), false).is_err());
    }
    assert_eq!(state(&s, 1).registers, before);
    assert!(state(&s, 1).pending_commands.is_empty());
}
#[test]
fn extra_nes_dispose_expired_ids_and_aggregate_budget() {
    let (s, mut r) = new();
    send(&s, 1, pulse(0));
    render(&mut r, &s, 100);
    s.dispose_extra_nes(1).unwrap();
    render(&mut r, &s, 1);
    s.extra_psg.collect_retired().unwrap();
    assert_eq!(s.extra_psg.memory_used(), 0);
    assert!(s.create_extra_nes(1, None, true).is_err());
    s.create_extra_nes(2, None, false).unwrap();
    assert!(s.enqueue(1, &[cmd(1, 0, write(0x4000, 0))]).is_err());
    let mut limited = Shared::new(48000, 1);
    limited.extra_psg =
        extra_psg::Shared::with_memory_limit(extra_budget::INSTANCE_BYTES + extra_nes::RAM + 100);
    limited.create_extra_nes(1, None, true).unwrap();
    assert!(limited
        .enqueue(1, &[cmd(1, 0, memory(0, vec![0; 65536], 1))])
        .unwrap_err()
        .contains("budget"));
    assert_eq!(state(&limited, 1).clock, 1789773.);
}
#[test]
fn extra_nes_bounded_completion_history_expires_completed_not_future() {
    let (s, mut r) = new();
    s.enqueue(
        1,
        &[cmd(1, 1000, json!({"method":"clock","hz":1000000,"id":1}))],
    )
    .unwrap();
    s.enqueue(
        1,
        &(2..=601)
            .map(|id| {
                cmd(
                    1,
                    0,
                    json!({"method":"write","address":0x4000,"value":31,"id":id}),
                )
            })
            .collect::<Vec<_>>(),
    )
    .unwrap();
    render(&mut r, &s, 1);
    state(&s, 1);
    assert!(s
        .extra_nes_status(1, Some(2))
        .unwrap_err()
        .contains("expired"));
    assert!(s.extra_nes_status(1, Some(601)).is_ok());
    render(&mut r, &s, 1200);
    assert_eq!(s.extra_nes_status(1, Some(1)).unwrap().nes.clock, 1000000.);
}

#[test]
fn extra_nes_stop_forces_fds_halt_with_register_io_disabled_retains_patch() {
    let (s, mut r) = new();
    send(&s, 1, fds().into_iter().map(|m| group(m, 1)).collect());
    render(&mut r, &s, 100);
    send(&s, 1, vec![group(write(0x4023, 0), 1)]);
    render(&mut r, &s, 1);
    assert!(state(&s, 1).fds_active);
    send(&s, 1, vec![group(json!({"method":"stop-voices"}), 1)]);
    render(&mut r, &s, 1);
    let v = state(&s, 1);
    assert!(!v.fds_active);
    assert_eq!(v.registers[0x41], 1);
    assert_eq!(v.fds_gain, 32);
    assert_eq!(v.registers[0x23] & 2, 0);
}
#[test]
fn extra_nes_fds_configured_metadata_edits_do_not_start_or_rewrite_core_on_read() {
    let (s, mut r) = new();
    let m = json!({"method":"batch","channel":5,"action":"edit","commands":[],"fdsGain":21,"fdsMaster":2,"id":1});
    send(&s, 1, vec![m]);
    render(&mut r, &s, 1);
    let before = state(&s, 1);
    assert_eq!(before.fds_gain, 21);
    assert_eq!(before.fds_master, 2);
    assert!(!before.fds_active);
    let frame = s.frame.load(Ordering::Acquire);
    for _ in 0..100 {
        let v = state(&s, 1);
        assert_eq!(v.registers, before.registers);
        assert_eq!(v.fds_gain, 21);
        assert_eq!(v.fds_master, 2);
        assert!(!v.fds_active);
    }
    assert_eq!(s.frame.load(Ordering::Acquire), frame);
}
#[test]
fn extra_nes_full_upload_callback_measurement_includes_coverage_and_publication() {
    let mut report = Vec::new();
    for count in [1u64, 16] {
        let mut samples = Vec::new();
        for _ in 0..30 {
            let s = Shared::new(48000, 1);
            let mut r = Renderer::new(48000).unwrap();
            for id in 1..=count {
                s.create_extra_nes(id, None, false).unwrap();
            }
            render(&mut r, &s, 1);
            for id in 1..=count {
                s.enqueue(1, &[cmd(id, 0, memory(0, vec![0x55; 65536], 1))])
                    .unwrap();
            }
            let start = std::time::Instant::now();
            r.prepare(&s);
            r.sample(&s);
            samples.push(start.elapsed().as_secs_f64() * 1e6);
            assert!(s.extra_nes_status(1, Some(1)).is_ok());
        }
        samples.sort_by(|a, b| a.total_cmp(b));
        report.push((count, samples[15], samples[28], samples[29]));
    }
    println!(
        "NES Rust test-profile upload callback microseconds (cores,median,p97,max): {:?}",
        report
    );
}

#[test]
fn extra_nes_foreign_enable_and_other_channel_stop_preserve_final_dmc_byte_exactly() {
    let sa = extra_nes::Shared::default();
    let sb = extra_nes::Shared::default();
    let mut a = extra_nes::Renderer::new(48000, 1789773., false).unwrap();
    let mut b = extra_nes::Renderer::new(48000, 1789773., false).unwrap();
    let apply = |r: &mut extra_nes::Renderer, s: &extra_nes::Shared, m: Value| {
        let m: extra_nes::Message = serde_json::from_value(m).unwrap();
        r.apply(&m.prepare().unwrap(), None, s);
    };
    for (r, s) in [(&mut a, &sa), (&mut b, &sb)] {
        apply(r, s, memory(0xc000, vec![0x55], 1));
        let ops = vec![
            write(0x4010, 0x0f),
            write(0x4012, 0),
            write(0x4013, 0),
            enable(4, true),
        ];
        apply(r, s, group(batch(4, 1, "on", ops, false), 1));
    }
    let mut tail = false;
    for _ in 0..100 {
        let mut x = [0.; 2];
        let mut y = [0.; 2];
        a.mix(&mut x);
        b.mix(&mut y);
        assert_eq!(x, y);
        if a.dmc_status() == (0, true) {
            tail = true;
            break;
        }
    }
    assert!(tail, "DMC must have prefetched its final byte");
    apply(&mut a, &sa, group(enable(0, false), 2));
    apply(&mut a, &sa, group(write(0x4015, 0), 2));
    apply(&mut a, &sa, group(write(0x4015, 16), 2));
    assert_eq!(a.dmc_status(), b.dmc_status());
    for _ in 0..100 {
        let mut x = [0.; 2];
        let mut y = [0.; 2];
        a.mix(&mut x);
        b.mix(&mut y);
        assert_eq!(x, y);
    }
    assert_eq!(a.dmc_status(), (0, false));
    assert_eq!(b.dmc_status(), (0, false));
}

#[test]
fn extra_nes_explicit_dmc_retrigger_takes_over_nonmanual_foreign_stream() {
    let s = extra_nes::Shared::default();
    let mut r = extra_nes::Renderer::new(48000, 1789773., false).unwrap();
    let apply = |r: &mut extra_nes::Renderer, m: Value| {
        let m: extra_nes::Message = serde_json::from_value(m).unwrap();
        r.apply(&m.prepare().unwrap(), None, &s);
    };
    apply(&mut r, memory(0xc000, vec![0x55; 512], 1));
    for m in [
        write(0x4010, 0x0f),
        write(0x4012, 0),
        write(0x4013, 15),
        enable(4, true),
    ] {
        apply(&mut r, group(m, 1));
    }
    assert!(r.dmc_status().0 > 100);
    for m in [
        enable(4, false),
        write(0x4012, 1),
        write(0x4013, 1),
        enable(4, true),
    ] {
        apply(&mut r, group(m, 2));
    }
    let remaining = r.dmc_status().0;
    assert!(remaining > 0 && remaining <= 17);
    apply(
        &mut r,
        json!({"method":"cancel-main-owner","targetGroup":1}),
    );
    assert!(r.dmc_status().1);
    apply(
        &mut r,
        json!({"method":"cancel-main-owner","targetGroup":2}),
    );
    assert!(!r.dmc_status().1);
}
