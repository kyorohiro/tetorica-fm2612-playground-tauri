//! Additional OPNA tests use offline renderers and never open an audio device.
use super::*;
use serde_json::{json, Value};
fn command(instance: u64, frame: u64, message: Value) -> Command {
    serde_json::from_value(
        json!({"type":"extra-ym2608","instance":instance,"frame":frame,"message":message}),
    )
    .unwrap()
}
fn write(port: u8, register: u8, value: u8) -> Value {
    json!({"method":"write","port":port,"register":register,"value":value})
}
fn own(mut c: Command, owner: u32) -> Command {
    c.performance = Some(7);
    c.owner = Some(owner);
    c
}
fn grouped(mut v: Value, group: u64) -> Value {
    v["group"] = json!(group);
    v
}
fn fm(ch: u8) -> Vec<Value> {
    let port = ch / 3;
    let local = ch % 3;
    let mut out = vec![write(0, 0x29, 0x80)];
    for slot in [0, 4, 8, 12] {
        for (register, value) in [
            (0x30 + slot + local, 1),
            (0x40 + slot + local, if slot == 12 { 48 } else { 127 }),
            (0x50 + slot + local, 31),
            (0x60 + slot + local, 0),
            (0x70 + slot + local, 0),
            (0x80 + slot + local, 15),
        ] {
            out.push(write(port, register, value));
        }
    }
    out.extend([
        write(port, 0xb0 + local, 7),
        write(port, 0xb4 + local, 0xc0),
        write(port, 0xa4 + local, 0x22),
        write(port, 0xa0 + local, 0x69),
        write(0, 0x28, 0xf0 | local | if port == 1 { 4 } else { 0 }),
    ]);
    out
}
fn ssg(ch: u8) -> Vec<Value> {
    vec![
        write(0, ch * 2, 128),
        write(0, ch * 2 + 1, 1),
        write(0, 7, 0x3f & !(1 << ch)),
        write(0, 8 + ch, 7),
    ]
}
fn play(s: &Shared, id: u64, values: Vec<Value>) {
    s.enqueue(
        1,
        &values
            .into_iter()
            .map(|v| command(id, 0, v))
            .collect::<Vec<_>>(),
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
fn snapshot(s: &Shared, id: u64) -> extra_ym2608::AdditionalChipStatus {
    s.extra_ym2608_status(id, None).unwrap().ym2608
}
fn energy(p: &[[f32; 2]]) -> f32 {
    p.iter().map(|v| v[0].abs() + v[1].abs()).sum()
}
fn prepare(events: Vec<(u64, u8, u32, u32)>, blocks: Vec<Vec<u8>>, duration: u64) -> Value {
    json!({"method":"prepare-timeline","events":events,"blocks":blocks,"durationSamples":duration})
}
fn events(values: Vec<Value>) -> Vec<(u64, u8, u32, u32)> {
    values
        .iter()
        .map(|v| {
            (
                0,
                v["port"].as_u64().unwrap() as u8,
                v["register"].as_u64().unwrap() as u32,
                v["value"].as_u64().unwrap() as u32,
            )
        })
        .collect()
}
fn audition(s: &Shared, ch: u8, voice: u64, action: &str, commands: Vec<Value>) {
    play(
        s,
        1,
        vec![
            json!({"method":"batch","audition":true,"channel":ch,"voice":voice,"action":action,"commands":commands}),
        ],
    );
}
#[test]
fn extra_ym2608_two_cores_mixed_rack_and_primary_are_independent() {
    let s = Shared::new(48000, 1);
    let mut r = Renderer::new(48000).unwrap();
    s.create_extra_ym2608(1, None).unwrap();
    s.create_extra_ym2608(2, Some(7987200)).unwrap();
    s.create_extra_ym2203(3, None).unwrap();
    s.create_extra_ym2612(4, None).unwrap();
    s.create_extra_psg(5, None).unwrap();
    play(&s, 1, fm(5));
    play(&s, 2, ssg(1));
    assert!(energy(&render(&mut r, &s, 1000)) > 1.);
    assert_eq!(r.fm_owners, [None; 6]);
    assert_eq!(snapshot(&s, 1).key_masks, [0, 0, 0, 0, 0, 15]);
    assert!(snapshot(&s, 2).ssg_gates[1]);
    assert_eq!(snapshot(&s, 2).clock, 7987200);
    play(&s, 1, vec![json!({"method":"reset"})]);
    render(&mut r, &s, 1);
    assert_eq!(snapshot(&s, 1).key_masks, [0; 6]);
    assert!(snapshot(&s, 2).ssg_gates[1]);
    assert!(snapshot(&s, 1).rom_loaded);
    assert_eq!(s.extra_psg.counts(), (5, 5));
}
#[test]
fn extra_ym2608_four_kind_limit_includes_pending_and_retiring() {
    let s = Shared::new(48000, 1);
    let mut r = Renderer::new(48000).unwrap();
    for id in 1..=16 {
        match id % 4 {
            0 => {
                s.create_extra_ym2608(id, None).unwrap();
            }
            1 => {
                s.create_extra_psg(id, None).unwrap();
            }
            2 => {
                s.create_extra_ym2612(id, None).unwrap();
            }
            _ => {
                s.create_extra_ym2203(id, None).unwrap();
            }
        }
    }
    assert!(s
        .create_extra_ym2608(17, None)
        .unwrap_err()
        .contains("limit"));
    assert!(s.dispose_extra_ym2608(1).unwrap_err().contains("match"));
    s.dispose_extra_ym2608(4).unwrap();
    assert!(s
        .create_extra_ym2608(17, None)
        .unwrap_err()
        .contains("limit"));
    render(&mut r, &s, 1);
    s.create_extra_ym2608(17, None).unwrap();
    assert!(s
        .create_extra_psg(4, None)
        .unwrap_err()
        .contains("increasing"));
    assert!(s
        .enqueue(1, &[command(1, 0, write(0, 0, 1))])
        .unwrap_err()
        .contains("not a YM2608"));
}
#[test]
fn extra_ym2608_atomic_upload_ranges_and_batch_budget() {
    let s = Shared::new(48000, 1);
    let mut r = Renderer::new(48000).unwrap();
    s.create_extra_ym2608(1, None).unwrap();
    s.create_extra_ym2608(2, None).unwrap();
    let data = vec![0x55; 2 * 1024 * 1024];
    play(
        &s,
        1,
        vec![json!({"method":"memory","offset":0,"data":data})],
    );
    render(&mut r, &s, 1);
    assert_eq!(snapshot(&s, 1).adpcm_memory_bytes, 2 * 1024 * 1024);
    let before = s.pending.load(Ordering::Acquire);
    for bad in [
        json!({"method":"rom","data":[1]}),
        json!({"method":"memory","offset":2097151,"data":[1,2]}),
    ] {
        assert!(s
            .enqueue(1, &[command(1, 0, bad), command(2, 0, write(0, 8, 15))])
            .is_err());
        assert_eq!(s.pending.load(Ordering::Acquire), before);
        assert_eq!(snapshot(&s, 2).registers[0][8], 0);
    }
    let large = vec![1u8; 2 * 1024 * 1024];
    assert!(s
        .enqueue(
            1,
            &[
                command(1, 0, json!({"method":"memory","offset":0,"data":large})),
                command(2, 0, json!({"method":"memory","offset":0,"data":large})),
                command(1, 0, json!({"method":"memory","offset":0,"data":[1]}))
            ]
        )
        .unwrap_err()
        .contains("4 MiB"));
}
#[test]
fn extra_ym2608_timeline_applies_exact_boundaries_and_replays_after_reset() {
    for rate in [44100, 48000, 96000] {
        let s = Shared::new(rate, 1);
        let mut r = Renderer::new(rate).unwrap();
        s.create_extra_ym2608(1, None).unwrap();
        play(
            &s,
            1,
            vec![
                prepare(
                    vec![(0, 0, 8, 7), (441, 0, 8, 0), (882, 0, 8, 9)],
                    vec![],
                    882,
                ),
                json!({"method":"play-timeline","token":1,"id":1}),
            ],
        );
        render(&mut r, &s, 1);
        assert_eq!(snapshot(&s, 1).registers[0][8], 7);
        let boundary = rate as usize / 100;
        render(&mut r, &s, boundary - 1);
        assert_eq!(snapshot(&s, 1).registers[0][8], 7);
        render(&mut r, &s, 1);
        assert_eq!(snapshot(&s, 1).registers[0][8], 0);
        render(&mut r, &s, boundary);
        let last = snapshot(&s, 1);
        assert_eq!(last.registers[0][8], 9);
        assert!(last.timeline.ended && last.timeline.prepared);
        play(&s, 1, vec![json!({"method":"reset"})]);
        render(&mut r, &s, 1);
        assert!(snapshot(&s, 1).timeline.prepared);
        play(&s, 1, vec![json!({"method":"play-timeline","token":2})]);
        render(&mut r, &s, 1);
        assert_eq!(snapshot(&s, 1).registers[0][8], 7);
    }
}
#[test]
fn extra_ym2608_timeline_work_bounds_are_atomic_and_allow_large_initial_copy() {
    let s = Shared::new(48000, 1);
    let mut r = Renderer::new(48000).unwrap();
    s.create_extra_ym2608(1, None).unwrap();
    let huge = vec![(0, 0, 8, 1); 131072];
    assert!(s
        .enqueue(1, &[command(1, 0, prepare(huge, vec![], 44100))])
        .unwrap_err()
        .contains("256 writes"));
    assert!(!snapshot(&s, 1).timeline.prepared);
    let block = vec![1; 2 * 1024 * 1024];
    play(
        &s,
        1,
        vec![
            prepare(vec![(0, 2, 0, 0)], vec![block.clone()], 44100),
            json!({"method":"play-timeline","token":1}),
        ],
    );
    render(&mut r, &s, 1);
    assert_eq!(snapshot(&s, 1).adpcm_memory_bytes, 2097152);
    play(&s, 1, vec![json!({"method":"reset"})]);
    render(&mut r, &s, 1);
    assert!(s
        .enqueue(
            1,
            &[command(
                1,
                0,
                prepare(
                    vec![(0, 2, 0, 0), (0, 2, 0, 0), (116, 2, 0, 0)],
                    vec![block],
                    44100
                )
            )]
        )
        .unwrap_err()
        .contains("4 MiB copies"));
    let legal = (0..512)
        .map(|i| (if i < 256 { 0 } else { 118 }, 0, 8, 1))
        .collect();
    play(&s, 1, vec![prepare(legal, vec![], 44100)]);
    render(&mut r, &s, 1);
}
#[test]
fn extra_ym2608_reset_same_batch_cancels_new_play_and_preserves_prepared() {
    for stop in [
        json!({"method":"reset"}),
        json!({"method":"stop-voices"}),
        json!({"method":"cancel-main-owner","targetGroup":7}),
    ] {
        let s = Shared::new(48000, 1);
        let mut r = Renderer::new(48000).unwrap();
        s.create_extra_ym2608(1, None).unwrap();
        play(
            &s,
            1,
            vec![
                grouped(
                    prepare(vec![(0, 0, 8, 10), (44100, 0, 8, 15)], vec![], 88200),
                    7,
                ),
                grouped(json!({"method":"play-timeline","token":1}), 7),
                stop,
            ],
        );
        render(&mut r, &s, 2000);
        let state = snapshot(&s, 1);
        assert!(!state.timeline.playing);
        assert!(state.timeline.prepared);
        assert_eq!(state.registers[0][8], 0);
    }
}
#[test]
fn extra_ym2608_main_groups_release_completed_notes_preserve_replacements_manual() {
    for complete in [false, true] {
        let s = Shared::new(48000, 1);
        let mut r = Renderer::new(48000).unwrap();
        s.create_extra_ym2608(1, None).unwrap();
        play(
            &s,
            1,
            vec![
                prepare(
                    events(fm(0).into_iter().chain(ssg(0)).collect()),
                    vec![],
                    if complete { 1 } else { 100000 },
                ),
                grouped(json!({"method":"play-timeline","token":1}), 1),
            ],
        );
        render(&mut r, &s, 1000);
        play(&s, 1, fm(0).into_iter().map(|v| grouped(v, 2)).collect());
        audition(&s, 7, 99, "on", ssg(1));
        render(&mut r, &s, 1000);
        play(
            &s,
            1,
            vec![
                json!({"method":"cancel-main-owner","targetGroup":1}),
                grouped(write(0, 0x28, 0), 1),
            ],
        );
        render(&mut r, &s, 1);
        let v = snapshot(&s, 1);
        assert_eq!(v.key_masks[0], 15);
        assert!(!v.ssg_gates[0]);
        assert!(v.ssg_gates[1]);
        assert_eq!(v.held, [7]);
    }
}
#[test]
fn extra_ym2608_reset_releases_active_notes_while_manual_voice_survives() {
    let s = Shared::new(48000, 1);
    let mut r = Renderer::new(48000).unwrap();
    s.create_extra_ym2608(1, None).unwrap();
    audition(&s, 1, 99, "on", fm(1));
    render(&mut r, &s, 200);
    play(
        &s,
        1,
        vec![
            prepare(
                events(fm(0).into_iter().chain(ssg(0)).collect()),
                vec![],
                100000,
            ),
            json!({"method":"play-timeline","token":1}),
        ],
    );
    render(&mut r, &s, 1000);
    assert_eq!(snapshot(&s, 1).key_masks[0], 15);
    play(&s, 1, vec![json!({"method":"reset"})]);
    render(&mut r, &s, 1);
    let v = snapshot(&s, 1);
    assert_eq!(v.key_masks, [0, 15, 0, 0, 0, 0]);
    assert!(!v.ssg_gates[0]);
    assert_eq!(v.held, [1]);
    assert!(v.timeline.cancelled);
}
#[test]
fn extra_ym2608_manual_ssg_rhythm_adpcm_and_csm_shared_guards() {
    let s = Shared::new(48000, 1);
    let mut r = Renderer::new(48000).unwrap();
    s.create_extra_ym2608(1, None).unwrap();
    audition(
        &s,
        6,
        10,
        "on",
        vec![
            write(0, 0, 128),
            write(0, 1, 1),
            write(0, 6, 3),
            write(0, 7, 0x36),
            write(0, 8, 16),
            write(0, 11, 100),
            write(0, 12, 1),
            write(0, 13, 10),
        ],
    );
    audition(
        &s,
        9,
        11,
        "on",
        vec![write(0, 0x18, 0xdf), write(0, 0x11, 0), write(0, 0x10, 1)],
    );
    audition(&s, 15, 12, "on", vec![write(1, 1, 0xc0), write(1, 0, 0x80)]);
    render(&mut r, &s, 100);
    play(
        &s,
        1,
        vec![
            write(0, 6, 31),
            write(0, 13, 0),
            write(0, 0x11, 63),
            write(0, 0x10, 0x81),
            write(1, 0, 1),
            write(0, 0x2f, 0),
            json!({"method":"reset"}),
        ],
    );
    render(&mut r, &s, 1);
    let v = snapshot(&s, 1);
    assert_eq!(v.registers[0][6], 3);
    assert_eq!(v.registers[0][13], 10);
    assert_eq!(v.registers[0][0x11], 0);
    assert_eq!(v.registers[1][0], 0x80);
    assert_eq!(v.prescaler, 6);
    assert_eq!(v.held, [6, 9, 15]);
}
#[test]
fn extra_ym2608_worker_csm_and_fm_have_separate_source_owners() {
    let s = Shared::new(48000, 1);
    let mut r = Renderer::new(48000).unwrap();
    s.create_extra_ym2608(1, None).unwrap();
    s.performance("start", 7, None).unwrap();
    s.enqueue(
        1,
        &[
            own(command(1, 0, write(0, 0x24, 255)), 1),
            own(command(1, 0, write(0, 0x25, 3)), 1),
            own(command(1, 0, write(0, 0x27, 0x85)), 1),
        ],
    )
    .unwrap();
    s.enqueue(
        1,
        &fm(2)
            .into_iter()
            .map(|v| own(command(1, 0, v), 2))
            .collect::<Vec<_>>(),
    )
    .unwrap();
    render(&mut r, &s, 1000);
    s.performance("cancel-owner", 7, Some(1)).unwrap();
    render(&mut r, &s, 1);
    let v = snapshot(&s, 1);
    assert_eq!(v.key_masks[2], 15);
    assert_eq!(v.registers[0][0x27] & 0x81, 0);
}
#[test]
fn extra_ym2608_preflight_budget_old_new_overlap_and_stalled_retirement() {
    let mut s = Shared::new(48000, 1);
    s.extra_psg = extra_psg::Shared::with_memory_limit(12 * 1024 * 1024);
    let mut r = Renderer::new(48000).unwrap();
    s.create_extra_ym2608(1, None).unwrap();
    s.create_extra_ym2608(2, None).unwrap();
    let fixed = 2 * (extra_budget::INSTANCE_BYTES + extra_budget::YM2608_MEMORY_BYTES);
    assert_eq!(s.extra_psg.memory_used(), fixed);
    let trace = prepare(
        vec![(0, 2, 0, 0), (44100, 2, 1, 0)],
        vec![vec![1; 2097152], vec![2; 2097152]],
        88200,
    );
    play(&s, 1, vec![trace.clone()]);
    render(&mut r, &s, 1);
    let retained = s.extra_psg.memory_used();
    assert!(retained > fixed + 4 * 1024 * 1024);
    let pending = s.pending.load(Ordering::Acquire);
    assert!(s
        .enqueue(1, &[command(1, 0, trace)])
        .unwrap_err()
        .contains("memory budget"));
    assert_eq!(s.pending.load(Ordering::Acquire), pending);
    assert_eq!(s.extra_psg.memory_used(), retained);
    assert!(snapshot(&s, 1).timeline.prepared);
    // A disposed object remains charged while the callback's rack reference exists.
    s.dispose_extra_ym2608(1).unwrap();
    assert_eq!(s.extra_psg.memory_used(), retained);
    render(&mut r, &s, 1);
    s.extra_psg.collect_retired().unwrap();
    s.extra_psg.collect_retired().unwrap();
    assert_eq!(s.extra_psg.memory_used(), fixed / 2);
}
#[test]
fn extra_ym2608_aggregate_failure_rolls_back_cross_instance_prepare_and_main_cancel_pending() {
    let s = Shared::new(48000, 1);
    let mut r = Renderer::new(48000).unwrap();
    s.create_extra_ym2608(1, None).unwrap();
    s.create_extra_ym2608(2, None).unwrap();
    let duplicate = |id| {
        command(
            2,
            0,
            json!({"method":"write","id":id,"port":0,"register":8,"value":1}),
        )
    };
    assert!(s
        .enqueue(
            1,
            &[
                command(1, 0, prepare(vec![(0, 0, 8, 9)], vec![], 1000)),
                duplicate(1),
                duplicate(1)
            ]
        )
        .unwrap_err()
        .contains("Duplicate"));
    assert!(!snapshot(&s, 1).timeline.prepared);
    play(&s, 1, vec![prepare(vec![(0, 0, 8, 7)], vec![], 1000)]);
    render(&mut r, &s, 1);
    s.enqueue(
        1,
        &[command(
            1,
            100000,
            grouped(prepare(vec![(0, 0, 8, 15)], vec![], 2000), 7),
        )],
    )
    .unwrap();
    play(
        &s,
        1,
        vec![json!({"method":"cancel-main-owner","targetGroup":7,"id":4})],
    );
    render(&mut r, &s, 1);
    s.extra_ym2608_status(1, Some(4)).unwrap();
    assert_eq!(snapshot(&s, 1).timeline.duration_samples, 1000);
    play(&s, 1, vec![json!({"method":"play-timeline","token":1})]);
    render(&mut r, &s, 1);
    assert_eq!(snapshot(&s, 1).registers[0][8], 7);
    render(&mut r, &s, 100010);
    assert_ne!(snapshot(&s, 1).registers[0][8], 15);
}
#[test]
fn extra_ym2608_future_play_cancelled_reset_owner_and_dispose_never_resurrects() {
    for kind in 0..3 {
        let s = Shared::new(48000, 1);
        let mut r = Renderer::new(48000).unwrap();
        s.create_extra_ym2608(1, None).unwrap();
        play(&s, 1, vec![prepare(vec![(0, 0, 8, 15)], vec![], 1000)]);
        render(&mut r, &s, 1);
        s.enqueue(
            1,
            &[command(
                1,
                10000,
                grouped(json!({"method":"play-timeline","token":1}), 7),
            )],
        )
        .unwrap();
        assert!(snapshot(&s, 1).timeline.pending);
        match kind {
            0 => play(&s, 1, vec![json!({"method":"reset"})]),
            1 => play(
                &s,
                1,
                vec![json!({"method":"cancel-main-owner","targetGroup":7})],
            ),
            _ => s.dispose_extra_ym2608(1).unwrap(),
        };
        render(&mut r, &s, 12000);
        if kind < 2 {
            let v = snapshot(&s, 1);
            assert!(v.timeline.cancelled);
            assert_eq!(v.registers[0][8], 0);
        } else {
            s.extra_psg.collect_retired().unwrap();
            assert!(s.extra_ym2608_status(1, None).is_err());
        }
        assert_eq!(s.pending.load(Ordering::Acquire), 0);
    }
}
#[test]
fn extra_ym2608_manual_csm_only_guards_reset_stop_clock_and_timeline_reset() {
    let s = Shared::new(48000, 1);
    let mut r = Renderer::new(48000).unwrap();
    s.create_extra_ym2608(1, None).unwrap();
    play(&s, 1, vec![json!({"method":"clock","hz":4000000,"id":1})]);
    render(&mut r, &s, 1);
    play(
        &s,
        1,
        vec![{
            let mut v = write(0, 0x27, 0x85);
            v["audition"] = json!(true);
            v
        }],
    );
    render(&mut r, &s, 1);
    assert_eq!(snapshot(&s, 1).held, [2]);
    play(
        &s,
        1,
        vec![
            json!({"method":"reset"}),
            json!({"method":"stop-voices"}),
            prepare(vec![], vec![], 100),
            json!({"method":"play-timeline","token":1}),
        ],
    );
    render(&mut r, &s, 1);
    let v = snapshot(&s, 1);
    assert_eq!(v.registers[0][0x27] & 0x81, 0x81);
    assert!(s
        .enqueue(
            1,
            &[command(1, 0, json!({"method":"clock","hz":8000000,"id":2}))]
        )
        .unwrap_err()
        .contains("manually held additional YM2608"));
    assert_eq!(v.clock, 4000000);
}
#[test]
fn extra_ym2608_clock_rejection_after_admission_and_wait_does_not_hold_running_mutex() {
    let rack = NativeGenesis::default();
    let s = Arc::new(Shared::new(48000, 1));
    let mut r = Renderer::new(48000).unwrap();
    let (stop, _) = mpsc::channel();
    *rack.running.lock().unwrap() = Some(Running {
        shared: s.clone(),
        stop,
        thread: None,
    });
    s.create_extra_ym2608(1, None).unwrap();
    s.enqueue(
        1,
        &[command(
            1,
            1000,
            json!({"method":"clock","hz":4000000,"id":1}),
        )],
    )
    .unwrap();
    audition(&s, 0, 1, "on", fm(0));
    render(&mut r, &s, 2000);
    assert!(s
        .extra_ym2608_status(1, Some(1))
        .unwrap_err()
        .contains("manually held additional YM2608"));
    let pending = rack.session_shared(1).unwrap();
    s.enqueue(1, &[command(1, 100000, json!({"method":"reset","id":2}))])
        .unwrap();
    let (tx, rx) = mpsc::channel();
    let waiting =
        std::thread::spawn(move || tx.send(pending.extra_ym2608_status(1, Some(2))).unwrap());
    rack.with_session(1, |shared| shared.dispose_extra_ym2608(1))
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
fn extra_ym2608_all_modes_share_mixer_fx_and_independent_core() {
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
        let s = Shared::new_modes(48000, 1, mega, dx, nes, console);
        let mut r = Renderer::new_modes(48000, mega, dx, nes, console).unwrap();
        s.create_extra_ym2608(1, None).unwrap();
        play(&s, 1, ssg(0));
        assert!(energy(&render(&mut r, &s, 1000)) > 1., "{mode}");
        play(&s, 1, vec![json!({"method":"mix","volume":0.,"pan":0.})]);
        let muted = render(&mut r, &s, 1000);
        let mut baseline = Renderer::new_modes(48000, mega, dx, nes, console).unwrap();
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
fn extra_ym2608_layout_and_fixed_sixteen_instance_budget() {
    eprintln!("extra-ym2608-memory-layout: instance={} ym2608_shared={} renderer={} session_shared={} retirement_queue={} fixed_instance={} fixed_RAM_ROM={} payload_overhead={}",std::mem::size_of::<extra_psg::Instance>(),std::mem::size_of::<extra_ym2608::Shared>(),std::mem::size_of::<extra_ym2608::Renderer>(),std::mem::size_of::<Shared>(),CAPACITY*std::mem::size_of::<Arc<extra_ym2608::Prepared>>()*2,extra_budget::INSTANCE_BYTES,extra_budget::YM2608_MEMORY_BYTES,extra_budget::PAYLOAD_OVERHEAD);
    let s = Shared::new(48000, 1);
    for id in 1..=16 {
        s.create_extra_ym2608(id, None).unwrap();
    }
    assert_eq!(
        s.extra_psg.memory_used(),
        16 * (extra_budget::INSTANCE_BYTES + extra_budget::YM2608_MEMORY_BYTES)
    );
}
#[test]
fn extra_ym2608_cancel_then_prepare_then_play_is_atomic_in_same_wire_batch() {
    for matching in [false, true] {
        let s = Shared::new(48000, 1);
        let mut r = Renderer::new(48000).unwrap();
        s.create_extra_ym2608(1, None).unwrap();
        play(
            &s,
            1,
            vec![
                prepare(vec![(0, 0, 8, 5)], vec![], 100000),
                grouped(json!({"method":"play-timeline","token":1}), 7),
            ],
        );
        render(&mut r, &s, 1);
        let values = vec![
            command(
                1,
                0,
                json!({"method":"cancel-main-owner","targetGroup":if matching{7}else{8}}),
            ),
            command(1, 0, prepare(vec![(0, 0, 8, 9)], vec![], 1000)),
            command(
                1,
                0,
                grouped(json!({"method":"play-timeline","token":2}), 8),
            ),
        ];
        let result = s.enqueue(1, &values);
        assert_eq!(result.is_ok(), matching);
        render(&mut r, &s, 1);
        let v = snapshot(&s, 1);
        assert!(v.timeline.playing);
        assert_eq!(v.timeline.token, if matching { 2 } else { 1 });
        assert_eq!(v.registers[0][8], if matching { 9 } else { 5 });
    }
}
#[test]
fn extra_ym2608_raw_adpcm_execute_and_record_offs_preserve_foreign_group() {
    let s = Shared::new(48000, 1);
    let mut r = Renderer::new(48000).unwrap();
    s.create_extra_ym2608(1, None).unwrap();
    for off in [0, 0x40, 0xc0, 1] {
        play(&s, 1, vec![grouped(write(1, 0, 0x80), 2)]);
        render(&mut r, &s, 1);
        play(&s, 1, vec![grouped(write(1, 0, off), 1)]);
        render(&mut r, &s, 1);
        assert_eq!(snapshot(&s, 1).registers[1][0], 0x80);
        play(&s, 1, vec![grouped(write(1, 0, off), 2)]);
        render(&mut r, &s, 1);
        assert_eq!(snapshot(&s, 1).registers[1][0], off);
    }
}
#[test]
fn extra_ym2608_timeline_end_frame_overflow_is_rejected_before_reservation() {
    let s = Shared::new(48000, 1);
    let mut r = Renderer::new(48000).unwrap();
    s.create_extra_ym2608(1, None).unwrap();
    play(&s, 1, vec![prepare(vec![], vec![], 1000)]);
    render(&mut r, &s, 1);
    let pending = s.pending.load(Ordering::Acquire);
    assert!(s
        .enqueue(
            1,
            &[command(
                1,
                MAX_SAFE_INTEGER - 1,
                json!({"method":"play-timeline","token":1})
            )]
        )
        .unwrap_err()
        .contains("end exceeds safe"));
    assert_eq!(s.pending.load(Ordering::Acquire), pending);
    assert_eq!(snapshot(&s, 1).timeline.token, 0);
    play(&s, 1, vec![json!({"method":"play-timeline","token":1})]);
    render(&mut r, &s, 1);
    assert!(snapshot(&s, 1).timeline.playing);
}
