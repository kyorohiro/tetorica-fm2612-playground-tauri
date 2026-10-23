//! Offline independent YM2203 rack, ownership and timeline tests. No devices.
use super::*;
use serde_json::{json, Value};
fn command(id: u64, frame: u64, message: Value) -> Command {
    serde_json::from_value(
        json!({"type":"extra-ym2203","instance":id,"frame":frame,"message":message}),
    )
    .unwrap()
}
fn write(reg: u8, value: u8) -> Value {
    json!({"method":"write","port":0,"register":reg,"value":value})
}
fn own(mut c: Command, n: u32) -> Command {
    c.performance = Some(7);
    c.owner = Some(n);
    c
}
fn fm(ch: u8) -> Vec<Value> {
    let mut out = Vec::new();
    for slot in [0, 4, 8, 12] {
        for (reg, value) in [
            (0x30 + slot + ch, 1),
            (0x40 + slot + ch, if slot == 12 { 48 } else { 127 }),
            (0x50 + slot + ch, 31),
            (0x60 + slot + ch, 0),
            (0x70 + slot + ch, 0),
            (0x80 + slot + ch, 15),
        ] {
            out.push(write(reg, value));
        }
    }
    out.extend([
        write(0xb0 + ch, 7),
        write(0xa4 + ch, 0x22),
        write(0xa0 + ch, 0x69),
        write(0x28, 0xf0 | ch),
    ]);
    out
}
fn ssg(ch: u8, level: u8) -> Vec<Value> {
    vec![
        write(ch * 2, 128),
        write(ch * 2 + 1, 1),
        write(7, 0x3f & !(1 << ch)),
        write(8 + ch, level),
    ]
}
fn play(shared: &Shared, id: u64, commands: Vec<Value>, owner: Option<u32>) {
    shared
        .enqueue(
            1,
            &commands
                .into_iter()
                .map(|v| {
                    let c = command(id, 0, v);
                    if let Some(n) = owner {
                        own(c, n)
                    } else {
                        c
                    }
                })
                .collect::<Vec<_>>(),
        )
        .unwrap();
}
fn audition(shared: &Shared, id: u64, ch: u8, voice: u64, action: &str, commands: Vec<Value>) {
    shared.enqueue(1,&[command(id,0,json!({"method":"batch","channel":ch,"voice":voice,"action":action,"audition":true,"commands":commands}))]).unwrap();
}
fn render(r: &mut Renderer, s: &Shared, n: usize) -> Vec<[f32; 2]> {
    let out = (0..n)
        .map(|i| {
            if i % 128 == 0 {
                r.prepare(s);
            }
            r.sample(s)
        })
        .collect();
    r.prepare(s);
    out
}
fn snapshot(s: &Shared, id: u64) -> ym2203::AdditionalChipStatus {
    s.extra_ym2203_status(id, None).unwrap().ym2203
}
fn schedule(token: u64, id: u64, entries: Vec<(u64, u8, u8)>, duration: u64) -> Value {
    json!({"method":"schedule","token":token,"id":id,"entries":entries,"durationSamples":duration})
}
fn energy(v: &[[f32; 2]]) -> f32 {
    v.iter().map(|v| v[0].abs() + v[1].abs()).sum()
}
#[test]
fn extra_ym2203_two_instances_mix_independently_with_ym2612_psg_and_primary() {
    let s = Shared::new(48_000, 1);
    let mut r = Renderer::new(48_000).unwrap();
    s.create_extra_ym2203(1, None).unwrap();
    s.create_extra_ym2203(2, Some(3_579_545)).unwrap();
    s.create_extra_ym2612(3, None).unwrap();
    s.create_extra_psg(4, None).unwrap();
    play(&s, 1, fm(0), None);
    play(&s, 2, ssg(1, 7), None);
    assert!(energy(&render(&mut r, &s, 1000)) > 1.);
    assert_eq!(r.fm_owners, [None; 6]);
    assert_eq!(r.psg_owners, [None; 4]);
    let a = snapshot(&s, 1);
    let b = snapshot(&s, 2);
    assert_eq!(a.clock, 4_000_000);
    assert_eq!(b.clock, 3_579_545);
    assert_eq!(a.key_masks, [15, 0, 0]);
    assert_eq!(b.key_masks, [0; 3]);
    assert!(b.ssg_gates[1]);
    play(&s, 1, vec![json!({"method":"reset"})], None);
    render(&mut r, &s, 1);
    assert_eq!(snapshot(&s, 1).key_masks, [0; 3]);
    assert!(snapshot(&s, 2).ssg_gates[1]);
    assert_eq!(
        s.extra_ym2612_status(3, None).unwrap().ym2612.clock,
        ym2612::CLOCK
    );
    assert_eq!(s.extra_psg.counts(), (4, 4));
}
#[test]
fn extra_ym2203_shares_three_kind_limit_and_disposal_type_identity() {
    let s = Shared::new(48_000, 1);
    let mut r = Renderer::new(48_000).unwrap();
    for id in 1..=16 {
        match id % 3 {
            0 => {
                s.create_extra_psg(id, None).unwrap();
            }
            1 => {
                s.create_extra_ym2612(id, None).unwrap();
            }
            _ => {
                s.create_extra_ym2203(id, None).unwrap();
            }
        }
    }
    assert!(s
        .create_extra_ym2203(17, None)
        .unwrap_err()
        .contains("limit"));
    assert!(s.dispose_extra_ym2203(1).unwrap_err().contains("match"));
    assert!(s
        .enqueue(1, &[command(1, 0, write(0, 1))])
        .unwrap_err()
        .contains("not a YM2203"));
    for id in 1..=16 {
        match id % 3 {
            0 => s.dispose_extra_psg(id),
            1 => s.dispose_extra_ym2612(id),
            _ => s.dispose_extra_ym2203(id),
        }
        .unwrap();
    }
    assert!(s
        .create_extra_ym2203(17, None)
        .unwrap_err()
        .contains("limit"));
    render(&mut r, &s, 1);
    s.create_extra_ym2203(17, None).unwrap();
    assert!(s
        .create_extra_psg(16, None)
        .unwrap_err()
        .contains("increasing"));
    assert_eq!(s.extra_psg.counts(), (1, 1));
}
#[test]
fn extra_ym2203_manual_ssg_guards_shared_noise_envelope_mixer_and_prescaler() {
    let s = Shared::new(48_000, 1);
    let mut r = Renderer::new(48_000).unwrap();
    s.create_extra_ym2203(1, None).unwrap();
    audition(
        &s,
        1,
        3,
        1,
        "on",
        vec![
            write(0, 128),
            write(1, 1),
            write(6, 3),
            write(7, 0x36),
            write(8, 16),
            write(11, 100),
            write(12, 1),
            write(13, 10),
        ],
    );
    render(&mut r, &s, 1000);
    let before = snapshot(&s, 1);
    play(
        &s,
        1,
        vec![
            write(6, 31),
            write(11, 1),
            write(12, 0),
            write(13, 0),
            write(7, 0x3f),
            write(0x2f, 0),
        ],
        None,
    );
    render(&mut r, &s, 1);
    let after = snapshot(&s, 1);
    for reg in [6, 11, 12, 13] {
        assert_eq!(before.registers[0][reg], after.registers[0][reg]);
    }
    assert_eq!(after.registers[0][7] & 9, before.registers[0][7] & 9);
    assert_eq!(after.prescaler, 6);
    audition(&s, 1, 4, 2, "on", ssg(1, 6));
    render(&mut r, &s, 1000);
    let after = snapshot(&s, 1);
    assert_eq!(after.registers[0][7] & 9, before.registers[0][7] & 9);
    assert_eq!(after.held, [3, 4]);
    audition(
        &s,
        1,
        3,
        1,
        "off",
        vec![json!({"method":"key-off","channel":3})],
    );
    audition(
        &s,
        1,
        4,
        2,
        "off",
        vec![json!({"method":"key-off","channel":4})],
    );
    render(&mut r, &s, 1);
    play(&s, 1, vec![write(0x2f, 0)], None);
    render(&mut r, &s, 1);
    assert_eq!(snapshot(&s, 1).prescaler, 2);
}
#[test]
fn extra_ym2203_csm_normal_fm_and_ssg_keep_separate_worker_owners() {
    for cancelled in [1, 2] {
        let s = Shared::new(48_000, 1);
        let mut r = Renderer::new(48_000).unwrap();
        s.create_extra_ym2203(1, None).unwrap();
        s.performance("start", 7, None).unwrap();
        play(&s, 1, fm(2), Some(1));
        play(&s, 1, vec![write(0x27, 0x85)], Some(1));
        play(&s, 1, fm(2), Some(2));
        play(&s, 1, ssg(0, 5), Some(3));
        render(&mut r, &s, 1000);
        s.performance("cancel-owner", 7, Some(cancelled)).unwrap();
        render(&mut r, &s, 1000);
        let a = snapshot(&s, 1);
        assert!(a.ssg_gates[0]);
        if cancelled == 1 {
            assert_eq!(a.registers[0][0x27] & 0x81, 0);
            assert_eq!(a.key_masks[2], 15);
        } else {
            assert_eq!(a.registers[0][0x27] & 0x81, 0x81);
            assert_eq!(a.key_masks[2], 0);
        }
        s.performance("cancel-owner", 7, Some(3)).unwrap();
        render(&mut r, &s, 1000);
        assert!(!snapshot(&s, 1).ssg_gates[0]);
    }
}
#[test]
fn extra_ym2203_schedule_matches_explicit_frame_events_and_finishes_at_terminal_write() {
    for rate in [44_100, 48_000, 96_000] {
        let scheduled = Shared::new(rate, 1);
        let reference = Shared::new(rate, 1);
        let mut a = Renderer::new(rate).unwrap();
        let mut b = Renderer::new(rate).unwrap();
        scheduled.create_extra_ym2203(1, None).unwrap();
        reference.create_extra_ym2203(1, None).unwrap();
        let mut entries = vec![
            (0, 0, 128),
            (0, 1, 1),
            (0, 7, 0x3e),
            (0, 8, 6),
            (10, 0x2e, 0),
            (20, 0x24, 255),
            (20, 0x25, 3),
            (20, 0x27, 5),
            (33, 0, 64),
            (80, 6, 5),
            (81, 7, 0x36),
            (100, 8, 0),
        ];
        entries.sort_by_key(|v| v.0);
        scheduled
            .enqueue(1, &[command(1, 0, schedule(1, 1, entries.clone(), 100))])
            .unwrap();
        reference
            .enqueue(
                1,
                &entries
                    .iter()
                    .map(|&(sample, reg, val)| {
                        command(1, (sample * rate as u64 + 44099) / 44100, write(reg, val))
                    })
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        let n = ((100 * rate as u64 + 44099) / 44100 + 20) as usize;
        assert_eq!(
            render(&mut a, &scheduled, n),
            render(&mut b, &reference, n),
            "{rate}"
        );
        let end = snapshot(&scheduled, 1);
        assert!(end.schedule.ended);
        assert!(!end.schedule.pending && !end.schedule.playing && !end.schedule.cancelled);
        assert_eq!(end.registers[0][8], 0);
        scheduled.extra_ym2203_status(1, Some(1)).unwrap();
    }
}
#[test]
fn extra_ym2203_schedule_reserves_future_start_and_reset_cancels_it_under_manual_hold() {
    let s = Shared::new(48_000, 1);
    let mut r = Renderer::new(48_000).unwrap();
    s.create_extra_ym2203(1, None).unwrap();
    s.enqueue(
        1,
        &[command(1, 10_000, schedule(1, 1, vec![(0, 8, 15)], 100))],
    )
    .unwrap();
    assert!(snapshot(&s, 1).schedule.pending);
    assert!(s
        .enqueue(1, &[command(1, 0, schedule(2, 2, vec![], 100))])
        .unwrap_err()
        .contains("already playing"));
    audition(&s, 1, 0, 1, "on", fm(0));
    render(&mut r, &s, 1000);
    play(&s, 1, vec![json!({"method":"reset","id":3})], None);
    render(&mut r, &s, 1);
    let v = snapshot(&s, 1);
    assert!(v.schedule.cancelled);
    assert_eq!(v.held, [0]);
    assert_eq!(v.key_masks[0], 15);
    assert_eq!(s.pending.load(Ordering::Acquire), 0);
    s.extra_ym2203_status(1, Some(3)).unwrap();
    s.enqueue(1, &[command(1, 0, schedule(2, 4, vec![], 1))])
        .unwrap();
    render(&mut r, &s, 3);
    assert!(snapshot(&s, 1).schedule.ended);
}
#[test]
fn extra_ym2203_schedule_scoped_cancel_and_owner_expiry_preserve_other_sources() {
    let s = Shared::new(48_000, 1);
    let mut r = Renderer::new(48_000).unwrap();
    s.create_extra_ym2203(1, None).unwrap();
    s.performance("start", 7, None).unwrap();
    let entries = vec![
        (0, 0, 128),
        (0, 1, 1),
        (0, 7, 0x3e),
        (0, 8, 8),
        (1000, 8, 15),
    ];
    s.enqueue(
        1,
        &[own(command(1, 0, schedule(1, 1, entries.clone(), 2000)), 1)],
    )
    .unwrap();
    play(&s, 1, fm(1), Some(2));
    render(&mut r, &s, 100);
    s.enqueue(
        1,
        &[own(
            command(1, 0, json!({"method":"cancel-schedule","token":1})),
            2,
        )],
    )
    .unwrap();
    render(&mut r, &s, 100);
    assert!(snapshot(&s, 1).schedule.playing);
    s.performance("cancel-owner", 7, Some(1)).unwrap();
    render(&mut r, &s, 2000);
    let v = snapshot(&s, 1);
    assert!(v.schedule.cancelled);
    assert_eq!(v.key_masks[1], 15);
    assert!(!v.ssg_gates[0]);
    s.enqueue(1, &[command(1, 0, schedule(2, 2, vec![(0, 8, 4)], 1000))])
        .unwrap();
    render(&mut r, &s, 1);
    play(
        &s,
        1,
        vec![json!({"method":"cancel-schedule","token":1})],
        None,
    );
    render(&mut r, &s, 1);
    assert!(snapshot(&s, 1).schedule.playing);
    play(
        &s,
        1,
        vec![json!({"method":"cancel-schedule","token":2})],
        None,
    );
    render(&mut r, &s, 1);
    assert!(snapshot(&s, 1).schedule.cancelled);
}
#[test]
fn extra_ym2203_schedule_validation_and_mixed_admission_are_atomic() {
    eprintln!("extra-ym2203-memory-layout: instance={} ym2203_shared={} renderer={} session_shared={} retirement_queue=1048576 bytes",std::mem::size_of::<extra_psg::Instance>(),std::mem::size_of::<ym2203::Shared>(),std::mem::size_of::<ym2203::Renderer>(),std::mem::size_of::<Shared>());
    let s = Shared::new(48_000, 1);
    s.create_extra_ym2203(1, None).unwrap();
    s.create_extra_ym2612(2, None).unwrap();
    for value in [
        schedule(0, 1, vec![], 1),
        schedule(1, 1, vec![], 0),
        schedule(1, 1, vec![(2, 8, 1), (1, 8, 1)], 3),
        schedule(1, 1, vec![(4, 8, 1)], 3),
        schedule(1, 1, vec![(0, 8, 1); 131073], 3),
    ] {
        assert!(s.enqueue(1, &[command(1, 0, value)]).is_err());
    }
    let bad=serde_json::from_value::<Command>(json!({"type":"extra-ym2612","instance":2,"frame":0,"message":{"method":"key-off","channel":6}})).unwrap();
    assert!(s
        .enqueue(1, &[command(1, 0, schedule(1, 1, vec![], 1)), bad])
        .is_err());
    assert_eq!(s.pending.load(Ordering::Acquire), 2);
    assert_eq!(snapshot(&s, 1).schedule.token, 0);
    s.enqueue(1, &[command(1, 0, schedule(1, 1, vec![], 1))])
        .unwrap();
}
#[test]
fn extra_ym2203_disposal_and_stop_retire_installed_and_pending_schedule_cores_off_callback() {
    for installed in [false, true] {
        let s = Shared::new(48_000, 1);
        let mut r = Renderer::new(48_000).unwrap();
        s.create_extra_ym2203(1, None).unwrap();
        s.enqueue(
            1,
            &[command(1, 0, schedule(1, 1, vec![(0, 8, 15)], 100000))],
        )
        .unwrap();
        let target = s.extra_psg.get(1, 0).unwrap();
        let weak = Arc::downgrade(&target);
        drop(target);
        if installed {
            render(&mut r, &s, 128);
        }
        s.dispose_extra_ym2203(1).unwrap();
        render(&mut r, &s, 128);
        assert!(weak.upgrade().is_some());
        s.extra_psg.collect_retired().unwrap();
        assert!(weak.upgrade().is_none());
        assert!(s.extra_ym2203_status(1, None).is_err());
        s.create_extra_ym2203(2, None).unwrap();
        s.extra_psg.deactivate_all().unwrap();
        s.epoch.fetch_add(1, Ordering::AcqRel);
        render(&mut r, &s, 2000);
        s.extra_psg.collect_retired().unwrap();
        assert_eq!(s.extra_psg.counts(), (0, 0));
        assert_eq!(s.pending.load(Ordering::Acquire), 0);
    }
}
#[test]
fn extra_ym2203_available_in_every_native_mode_uses_common_mixer_and_fx() {
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
        "pc98-26",
        "pc98",
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
        let (mega, dx, nes, console) = NativeGenesis::mode_config(Some(mode)).unwrap();
        let s = Shared::new_modes(48_000, 1, mega, dx, nes, console);
        let mut r = Renderer::new_modes(48_000, mega, dx, nes, console).unwrap();
        s.create_extra_ym2203(1, None).unwrap();
        play(&s, 1, ssg(0, 6), None);
        assert!(energy(&render(&mut r, &s, 1000)) > 1., "{mode}");
        play(
            &s,
            1,
            vec![json!({"method":"mix","volume":0.,"pan":0.})],
            None,
        );
        let muted = render(&mut r, &s, 1000);
        let mut baseline = Renderer::new_modes(48_000, mega, dx, nes, console).unwrap();
        baseline.idle = false;
        let expected = render(&mut baseline, &s, 2000);
        assert!(
            muted[300..]
                .iter()
                .zip(&expected[1300..])
                .all(|(a, b)| (a[0] - b[0]).abs() < 1e-7 && (a[1] - b[1]).abs() < 1e-7),
            "{mode}"
        );
    }
}
#[test]
fn extra_ym2203_same_batch_schedule_then_reset_stop_or_token_cancel_resolves_ordered_target() {
    for cancel in [
        json!({"method":"reset"}),
        json!({"method":"stop-voices"}),
        json!({"method":"cancel-schedule","token":1}),
    ] {
        let s = Shared::new(48_000, 1);
        let mut r = Renderer::new(48_000).unwrap();
        s.create_extra_ym2203(1, None).unwrap();
        s.enqueue(
            1,
            &[
                command(1, 0, schedule(1, 1, vec![(1000, 8, 15)], 2000)),
                command(1, 0, cancel),
            ],
        )
        .unwrap();
        render(&mut r, &s, 3000);
        let v = snapshot(&s, 1);
        assert!(v.schedule.cancelled);
        assert!(!v.ssg_gates[0]);
        assert_eq!(s.pending.load(Ordering::Acquire), 0);
    }
}
#[test]
fn extra_ym2203_schedule_density_and_cross_instance_retain_failures_leave_no_reservation() {
    let s = Shared::new(48_000, 1);
    s.create_extra_ym2203(1, None).unwrap();
    s.create_extra_ym2612(2, None).unwrap();
    assert!(s
        .enqueue(
            1,
            &[command(1, 0, schedule(1, 1, vec![(0, 8, 1); 4097], 1))]
        )
        .unwrap_err()
        .contains("256 writes"));
    let duplicate = |value| {
        serde_json::from_value::<Command>(json!({"type":"extra-ym2612","instance":2,"frame":0,"message":{"id":1,"method":"write","port":0,"register":0x22,"value":value}})).unwrap()
    };
    assert!(s
        .enqueue(
            1,
            &[
                command(1, 0, schedule(1, 1, vec![], 1)),
                duplicate(1),
                duplicate(2)
            ]
        )
        .unwrap_err()
        .contains("Duplicate"));
    let state = snapshot(&s, 1);
    assert_eq!(state.schedule.token, 0);
    assert!(
        !state.schedule.playing
            && !state.schedule.pending
            && !state.schedule.cancelled
            && !state.schedule.ended
    );
    assert!(state.pending_commands.is_empty());
    s.enqueue(1, &[command(1, 0, schedule(2, 2, vec![], 1))])
        .unwrap();
}

#[test]
fn extra_ym2203_schedule_sliding_block_budget_accepts_boundary_and_rejects_dense_bursts() {
    for (last_sample, accepted) in [(0, false), (116, false), (117, true)] {
        let s = Shared::new(48_000, 1);
        s.create_extra_ym2203(1, None).unwrap();
        let mut entries = vec![(0, 8, 1); 256];
        entries.push((last_sample, 8, 1));
        // At 48 kHz, ceil(117*48000/44100)=128, so use exact
        // output-frame boundaries independently of the VGM sample grid.
        let frames = (last_sample * 48000 + 44099) / 44100;
        assert_eq!(frames >= 128, accepted);
        let result = s.enqueue(1, &[command(1, 0, schedule(1, 1, entries, 1000))]);
        assert_eq!(result.is_ok(), accepted);
        if !accepted {
            assert_eq!(snapshot(&s, 1).schedule.token, 0);
        }
    }
}
#[test]
fn extra_ym2203_held_clock_rejection_is_recoverable_and_other_instance_unchanged() {
    let s = Shared::new(48_000, 1);
    let mut r = Renderer::new(48_000).unwrap();
    s.create_extra_ym2203(1, None).unwrap();
    s.create_extra_ym2203(2, None).unwrap();
    s.enqueue(
        1,
        &[command(
            1,
            1000,
            json!({"method":"clock","hz":8_000_000,"id":1}),
        )],
    )
    .unwrap();
    audition(&s, 1, 0, 1, "on", fm(0));
    render(&mut r, &s, 2000);
    assert_eq!(
        s.extra_ym2203_status(1, Some(1)).unwrap_err(),
        "Release the manually held additional YM2203 voices before changing the clock"
    );
    assert!(s
        .enqueue(
            1,
            &[command(
                1,
                0,
                json!({"method":"clock","hz":8_000_000,"id":2})
            )]
        )
        .unwrap_err()
        .contains("manually held additional YM2203"));
    audition(
        &s,
        1,
        0,
        1,
        "off",
        vec![json!({"method":"key-off","channel":0})],
    );
    render(&mut r, &s, 1);
    play(
        &s,
        1,
        vec![json!({"method":"clock","hz":8_000_000,"id":3})],
        None,
    );
    render(&mut r, &s, 1);
    assert_eq!(
        s.extra_ym2203_status(1, Some(3)).unwrap().ym2203.clock,
        8_000_000
    );
    assert_eq!(snapshot(&s, 2).clock, 4_000_000);
}
#[test]
fn extra_ym2203_pending_schedule_barrier_disposal_does_not_hold_native_running_mutex() {
    let rack = NativeGenesis::default();
    let s = Arc::new(Shared::new(48_000, 1));
    let mut r = Renderer::new(48_000).unwrap();
    let (stop, _) = mpsc::channel();
    *rack.running.lock().unwrap() = Some(Running {
        shared: s.clone(),
        stop,
        thread: None,
    });
    s.create_extra_ym2203(1, None).unwrap();
    s.enqueue(1, &[command(1, 480_000, schedule(1, 1, vec![], 1000))])
        .unwrap();
    let pending = rack.session_shared(1).unwrap();
    let (tx, rx) = mpsc::channel();
    let waiting =
        std::thread::spawn(move || tx.send(pending.extra_ym2203_status(1, Some(1))).unwrap());
    rack.with_session(1, |shared| shared.dispose_extra_ym2203(1))
        .unwrap();
    render(&mut r, &s, 1);
    assert!(rx
        .recv_timeout(Duration::from_millis(200))
        .unwrap()
        .unwrap_err()
        .contains("expired"));
    waiting.join().unwrap();
    let running = rack.running.lock().unwrap().take().unwrap();
    s.failed.store(true, Ordering::Release);
    drop(running);
}
#[test]
fn extra_ym2203_cancel_schedule_releases_only_its_triggered_voices_even_after_completion() {
    for complete in [false, true] {
        let s = Shared::new(48_000, 1);
        let mut r = Renderer::new(48_000).unwrap();
        s.create_extra_ym2203(1, None).unwrap();
        let mut entries: Vec<_> = fm(0)
            .into_iter()
            .map(|v| {
                (
                    0,
                    v["register"].as_u64().unwrap() as u8,
                    v["value"].as_u64().unwrap() as u8,
                )
            })
            .collect();
        entries.extend([(0, 2, 128), (0, 3, 1), (0, 7, 0x3d), (0, 9, 7)]);
        s.enqueue(
            1,
            &[command(
                1,
                0,
                schedule(1, 1, entries, if complete { 1 } else { 100000 }),
            )],
        )
        .unwrap();
        render(&mut r, &s, 1000);
        assert_eq!(snapshot(&s, 1).key_masks[0], 15);
        assert!(snapshot(&s, 1).ssg_gates[1]);
        // Another Main loop replaces FM0 normally; a manual key also owns SSG1.
        play(&s, 1, fm(0), None);
        audition(&s, 1, 4, 99, "on", ssg(1, 6));
        render(&mut r, &s, 1000);
        play(
            &s,
            1,
            vec![json!({"method":"cancel-schedule","token":1})],
            None,
        );
        render(&mut r, &s, 1000);
        let state = snapshot(&s, 1);
        assert_eq!(state.key_masks[0], 15);
        assert!(state.ssg_gates[1]);
        assert_eq!(state.held, [4]);
    }
    // With no replacement, terminal keyed notes survive normal completion and
    // are released by the later owner cancellation rather than staying stuck.
    let s = Shared::new(48_000, 1);
    let mut r = Renderer::new(48_000).unwrap();
    s.create_extra_ym2203(1, None).unwrap();
    let entries = vec![(0, 0, 128), (0, 1, 1), (0, 7, 0x3e), (0, 8, 7)];
    s.enqueue(1, &[command(1, 0, schedule(1, 1, entries, 1))])
        .unwrap();
    render(&mut r, &s, 1000);
    assert!(snapshot(&s, 1).schedule.ended && snapshot(&s, 1).ssg_gates[0]);
    play(
        &s,
        1,
        vec![json!({"method":"cancel-schedule","token":1})],
        None,
    );
    render(&mut r, &s, 1);
    assert!(!snapshot(&s, 1).ssg_gates[0]);
}
fn grouped(mut message: Value, group: u64) -> Value {
    message["group"] = json!(group);
    message
}
#[test]
fn extra_ym2203_main_groups_preserve_scheduled_replacements_and_manual_voices() {
    let s = Shared::new(48_000, 1);
    let mut r = Renderer::new(48_000).unwrap();
    s.create_extra_ym2203(1, None).unwrap();
    play(
        &s,
        1,
        fm(0)
            .into_iter()
            .chain(ssg(0, 7))
            .map(|v| grouped(v, 1))
            .collect(),
        None,
    );
    let mut entries: Vec<_> = fm(0)
        .into_iter()
        .map(|v| {
            (
                0,
                v["register"].as_u64().unwrap() as u8,
                v["value"].as_u64().unwrap() as u8,
            )
        })
        .collect();
    entries.extend([(0, 0, 128), (0, 1, 1), (0, 7, 0x3e), (0, 8, 6)]);
    s.enqueue(
        1,
        &[command(1, 0, grouped(schedule(1, 1, entries, 100000), 2))],
    )
    .unwrap();
    render(&mut r, &s, 1000);
    play(
        &s,
        1,
        vec![
            json!({"method":"cancel-main-owner","targetGroup":1}),
            grouped(write(0x28, 0), 1),
            grouped(write(8, 0), 1),
            grouped(json!({"method":"stop-voices"}), 1),
        ],
        None,
    );
    render(&mut r, &s, 1);
    let protected = snapshot(&s, 1);
    assert!(protected.schedule.playing && protected.ssg_gates[0]);
    assert_eq!(protected.key_masks[0], 15);
    audition(&s, 1, 0, 99, "on", fm(0));
    render(&mut r, &s, 1000);
    play(
        &s,
        1,
        vec![json!({"method":"cancel-main-owner","targetGroup":2})],
        None,
    );
    render(&mut r, &s, 1);
    let stopped = snapshot(&s, 1);
    assert!(stopped.schedule.cancelled);
    assert!(!stopped.ssg_gates[0]);
    assert_eq!(stopped.key_masks[0], 15);
    assert_eq!(stopped.held, [0]);
}
#[test]
fn extra_ym2203_main_group_cancel_cleans_completed_and_pending_schedules_without_token_history() {
    let s = Shared::new(48_000, 1);
    let mut r = Renderer::new(48_000).unwrap();
    s.create_extra_ym2203(1, None).unwrap();
    for token in 1..=3 {
        s.enqueue(
            1,
            &[command(
                1,
                0,
                grouped(
                    schedule(
                        token,
                        token,
                        vec![(0, 0, 128), (0, 1, 1), (0, 7, 0x3e), (0, 8, 7)],
                        1,
                    ),
                    7,
                ),
            )],
        )
        .unwrap();
        render(&mut r, &s, 1000);
        assert!(snapshot(&s, 1).schedule.ended);
    }
    assert!(snapshot(&s, 1).ssg_gates[0]);
    play(
        &s,
        1,
        vec![json!({"method":"cancel-main-owner","targetGroup":7})],
        None,
    );
    render(&mut r, &s, 1);
    assert!(!snapshot(&s, 1).ssg_gates[0]);
    s.enqueue(
        1,
        &[command(
            1,
            100000,
            grouped(schedule(4, 4, vec![(0, 8, 15)], 1000), 8),
        )],
    )
    .unwrap();
    play(
        &s,
        1,
        vec![json!({"method":"cancel-main-owner","targetGroup":8})],
        None,
    );
    render(&mut r, &s, 1);
    assert!(snapshot(&s, 1).schedule.cancelled);
    assert_eq!(s.pending.load(Ordering::Acquire), 0);
    s.enqueue(
        1,
        &[command(
            1,
            r.frame + 1000,
            grouped(schedule(5, 5, vec![(0, 8, 8)], 10000), 9),
        )],
    )
    .unwrap();
    render(&mut r, &s, 1);
    play(
        &s,
        1,
        vec![
            json!({"method":"cancel-main-owner","targetGroup":8}),
            grouped(json!({"method":"stop-voices"}), 8),
        ],
        None,
    );
    render(&mut r, &s, 2000);
    assert!(snapshot(&s, 1).schedule.playing && snapshot(&s, 1).ssg_gates[0]);
}
#[test]
fn extra_ym2203_main_group_csm_and_normal_ch3_cancellation_are_independent() {
    for target in [1, 2] {
        let s = Shared::new(48_000, 1);
        let mut r = Renderer::new(48_000).unwrap();
        s.create_extra_ym2203(1, None).unwrap();
        play(
            &s,
            1,
            fm(2)
                .into_iter()
                .chain([write(0x27, 0x85)])
                .map(|v| grouped(v, 1))
                .collect(),
            None,
        );
        play(
            &s,
            1,
            fm(2).into_iter().map(|v| grouped(v, 2)).collect(),
            None,
        );
        render(&mut r, &s, 1000);
        play(
            &s,
            1,
            vec![json!({"method":"cancel-main-owner","targetGroup":target})],
            None,
        );
        render(&mut r, &s, 1);
        let v = snapshot(&s, 1);
        if target == 1 {
            assert_eq!(v.registers[0][0x27] & 0x81, 0);
            assert_eq!(v.key_masks[2], 15);
        } else {
            assert_eq!(v.registers[0][0x27] & 0x81, 0x81);
            assert_eq!(v.key_masks[2], 0);
        }
    }
}
#[test]
fn extra_ym2203_main_groups_track_raw_ssg_mixer_gates_and_own_schedule_offs() {
    let s = Shared::new(48_000, 1);
    let mut r = Renderer::new(48_000).unwrap();
    s.create_extra_ym2203(1, None).unwrap();
    play(
        &s,
        1,
        vec![grouped(write(7, 0x3f), 1), grouped(write(8, 7), 1)],
        None,
    );
    s.enqueue(
        1,
        &[command(
            1,
            0,
            grouped(schedule(1, 1, vec![(0, 7, 0x3e)], 100000), 2),
        )],
    )
    .unwrap();
    render(&mut r, &s, 1000);
    play(
        &s,
        1,
        vec![
            json!({"method":"cancel-main-owner","targetGroup":1}),
            grouped(write(7, 0x3f), 1),
        ],
        None,
    );
    render(&mut r, &s, 1);
    assert!(snapshot(&s, 1).ssg_gates[0]);
    assert_eq!(snapshot(&s, 1).registers[0][7] & 9, 8);
    play(
        &s,
        1,
        vec![json!({"method":"cancel-main-owner","targetGroup":2})],
        None,
    );
    render(&mut r, &s, 1);
    assert!(!snapshot(&s, 1).ssg_gates[0]);
    // A schedule can release ordinary notes already started by its own Main
    // group, despite the timeline having a distinct source token for cleanup.
    play(
        &s,
        1,
        fm(0)
            .into_iter()
            .chain(ssg(0, 7))
            .map(|v| grouped(v, 3))
            .collect(),
        None,
    );
    s.enqueue(
        1,
        &[command(
            1,
            0,
            grouped(schedule(2, 2, vec![(0, 0x28, 0), (0, 8, 0)], 1), 3),
        )],
    )
    .unwrap();
    render(&mut r, &s, 1000);
    let v = snapshot(&s, 1);
    assert_eq!(v.key_masks[0], 0);
    assert!(!v.ssg_gates[0]);
}
