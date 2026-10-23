//! Independent OPM rack tests use offline renderers; no audio devices.
use super::*;
use serde_json::{json, Value};
fn command(instance: u64, frame: u64, message: Value) -> Command {
    serde_json::from_value(
        json!({"type":"extra-ym2151","instance":instance,"frame":frame,"message":message}),
    )
    .unwrap()
}
fn write(register: u8, value: u8) -> Value {
    json!({"method":"write","register":register,"value":value})
}
fn own(mut c: Command, n: u32) -> Command {
    c.performance = Some(7);
    c.owner = Some(n);
    c
}
fn grouped(mut v: Value, group: u64) -> Value {
    v["group"] = json!(group);
    v
}
fn fm(ch: u8) -> Vec<Value> {
    let mut out = Vec::new();
    for slot in [0, 8, 16, 24] {
        for (register, value) in [
            (0x40 + slot + ch, 1),
            (0x60 + slot + ch, if slot == 24 { 48 } else { 127 }),
            (0x80 + slot + ch, 31),
            (0xa0 + slot + ch, 0),
            (0xc0 + slot + ch, 0),
            (0xe0 + slot + ch, 15),
        ] {
            out.push(write(register, value))
        }
    }
    out.extend([
        write(0x20 + ch, 0xc7),
        write(0x28 + ch, 0x4a),
        write(0x30 + ch, 0),
        write(8, ch | 0x78),
    ]);
    out
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
fn audition(s: &Shared, ch: u8, voice: u64, action: &str, commands: Vec<Value>) {
    play(
        s,
        1,
        vec![
            json!({"method":"batch","channel":ch,"voice":voice,"action":action,"audition":true,"commands":commands}),
        ],
    );
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
fn snapshot(s: &Shared, id: u64) -> extra_ym2151::AdditionalChipStatus {
    s.extra_ym2151_status(id, None).unwrap().ym2151
}
fn energy(v: &[[f32; 2]]) -> f32 {
    v.iter().map(|v| v[0].abs() + v[1].abs()).sum()
}
#[test]
fn extra_ym2151_two_opm_mix_with_five_other_kinds_and_primary_independently() {
    let s = Shared::new(48000, 1);
    let mut r = Renderer::new(48000).unwrap();
    s.create_extra_ym2151(1, None).unwrap();
    s.create_extra_ym2151(2, Some(4000000)).unwrap();
    s.create_extra_ym2612(3, None).unwrap();
    s.create_extra_ym2203(4, None).unwrap();
    s.create_extra_ym2608(5, None).unwrap();
    s.create_extra_ym2610(6, None).unwrap();
    s.create_extra_psg(7, None).unwrap();
    play(&s, 1, fm(0));
    play(&s, 2, fm(7));
    assert!(energy(&render(&mut r, &s, 1000)) > 1.);
    assert_eq!(snapshot(&s, 1).clock, 3579545);
    assert_eq!(snapshot(&s, 2).clock, 4000000);
    assert_eq!(snapshot(&s, 1).key_masks, [15, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(snapshot(&s, 2).key_masks, [0, 0, 0, 0, 0, 0, 0, 15]);
    assert_eq!(snapshot(&s, 1).registers.len(), 256);
    assert_eq!(r.fm_owners, [None; 6]);
    play(&s, 1, vec![json!({"method":"reset"})]);
    render(&mut r, &s, 1);
    assert_eq!(snapshot(&s, 1).key_masks, [0; 8]);
    assert_eq!(snapshot(&s, 2).key_masks[7], 15);
    assert_eq!(s.extra_psg.counts(), (7, 7));
}
#[test]
fn extra_ym2151_six_kind_limit_counts_pending_retiring_and_typed_identity() {
    let s = Shared::new(48000, 1);
    let mut r = Renderer::new(48000).unwrap();
    for id in 1..=16 {
        s.create_extra_ym2151(id, None).unwrap();
    }
    assert_eq!(s.extra_psg.memory_used(), 16 * extra_budget::INSTANCE_BYTES);
    assert!(s
        .create_extra_ym2151(17, None)
        .unwrap_err()
        .contains("limit"));
    assert!(s.dispose_extra_ym2610(1).unwrap_err().contains("match"));
    s.dispose_extra_ym2151(1).unwrap();
    assert!(s
        .create_extra_ym2151(17, None)
        .unwrap_err()
        .contains("limit"));
    render(&mut r, &s, 1);
    s.create_extra_ym2151(17, None).unwrap();
    assert!(s
        .create_extra_psg(1, None)
        .unwrap_err()
        .contains("increasing"));
    let wrong=serde_json::from_value::<Command>(json!({"type":"extra-ym2612","instance":2,"frame":0,"message":{"method":"write","port":0,"register":8,"value":1}})).unwrap();
    assert!(s.enqueue(1, &[wrong]).unwrap_err().contains("not a YM2612"));
}
#[test]
fn extra_ym2151_shared_lfo_depths_noise_and_timers_preserve_manual_sources() {
    let s = Shared::new(48000, 1);
    let mut r = Renderer::new(48000).unwrap();
    s.create_extra_ym2151(1, None).unwrap();
    play(
        &s,
        1,
        vec![
            write(0x18, 22),
            write(0x19, 33),
            write(0x19, 0x80 | 44),
            write(0x1b, 2),
            write(15, 0x85),
            write(0x10, 20),
            write(0x11, 1),
        ],
    );
    audition(&s, 7, 1, "on", fm(7));
    render(&mut r, &s, 1000);
    play(
        &s,
        1,
        vec![
            write(0x18, 0),
            write(0x19, 1),
            write(0x19, 0x81),
            write(0x1b, 0),
            write(15, 0),
            write(0x10, 0),
            write(0x11, 0),
            write(0x14, 0x85),
            json!({"method":"reset"}),
            json!({"method":"all-notes-off"}),
            json!({"method":"stop-voices"}),
        ],
    );
    render(&mut r, &s, 1);
    let v = snapshot(&s, 1);
    assert_eq!((v.am_depth, v.pm_depth), (33, 44));
    assert_eq!(v.registers[0x18], 22);
    assert_eq!(v.registers[0x1b], 2);
    assert_eq!(v.registers[15], 0x85);
    assert_eq!(v.registers[0x10], 20);
    assert_eq!(v.key_masks[7], 15);
    assert_eq!(v.held, [7]);
    audition(
        &s,
        7,
        1,
        "edit",
        vec![write(0x19, 55), write(0x19, 0x80 | 66), write(15, 0x89)],
    );
    render(&mut r, &s, 1);
    let v = snapshot(&s, 1);
    assert_eq!((v.am_depth, v.pm_depth), (55, 66));
    assert_eq!(v.registers[15], 0x89);
}
#[test]
fn extra_ym2151_pending_audition_retrigger_cancel_and_stale_off_never_resurrect() {
    let s = Shared::new(48000, 1);
    let mut r = Renderer::new(48000).unwrap();
    s.create_extra_ym2151(1, Some(100000)).unwrap();
    let mut patch = fm(0);
    patch.pop();
    patch.push(json!({"method":"audition-key-on","channel":0,"mask":15}));
    audition(&s, 0, 1, "on", patch.clone());
    r.prepare(&s);
    assert_eq!(snapshot(&s, 1).key_masks[0], 0);
    audition(
        &s,
        0,
        1,
        "off",
        vec![json!({"method":"key-off","channel":0})],
    );
    render(&mut r, &s, 200);
    assert_eq!(snapshot(&s, 1).key_masks[0], 0);
    audition(&s, 0, 2, "on", patch);
    assert!(energy(&render(&mut r, &s, 1000)) > 0.01);
    assert_eq!(snapshot(&s, 1).key_masks[0], 15);
    audition(
        &s,
        0,
        1,
        "off",
        vec![json!({"method":"key-off","channel":0})],
    );
    render(&mut r, &s, 1);
    assert_eq!(snapshot(&s, 1).key_masks[0], 15);
    audition(
        &s,
        0,
        2,
        "off",
        vec![json!({"method":"key-off","channel":0})],
    );
    render(&mut r, &s, 1);
    let v = snapshot(&s, 1);
    assert_eq!(v.key_masks[0], 0);
    assert!(v.held.is_empty());
    assert_eq!(v.registers[0x78], 48);
}
#[test]
fn extra_ym2151_main_groups_normal_csm_and_all_notes_off_are_source_scoped() {
    for cancelled in [1, 2] {
        let s = Shared::new(48000, 1);
        let mut r = Renderer::new(48000).unwrap();
        s.create_extra_ym2151(1, None).unwrap();
        play(
            &s,
            1,
            vec![
                grouped(write(0x10, 255), 1),
                grouped(write(0x11, 3), 1),
                grouped(write(0x14, 0x85), 1),
            ],
        );
        play(&s, 1, fm(3).into_iter().map(|v| grouped(v, 2)).collect());
        play(&s, 1, fm(7).into_iter().map(|v| grouped(v, 3)).collect());
        render(&mut r, &s, 1000);
        play(
            &s,
            1,
            vec![json!({"method":"cancel-main-owner","targetGroup":cancelled})],
        );
        render(&mut r, &s, 1);
        let v = snapshot(&s, 1);
        if cancelled == 1 {
            assert_eq!(v.registers[0x14] & 0x81, 0);
            assert_eq!(v.key_masks[3], 15);
        } else {
            assert_eq!(v.registers[0x14] & 0x81, 0x81);
            assert_eq!(v.key_masks[3], 0);
        }
        assert_eq!(v.key_masks[7], 15);
        play(
            &s,
            1,
            vec![
                grouped(write(8, 7), 2),
                grouped(json!({"method":"all-notes-off"}), 2),
                grouped(json!({"method":"stop-voices"}), 2),
            ],
        );
        render(&mut r, &s, 1);
        assert_eq!(snapshot(&s, 1).key_masks[7], 15);
    }
}
#[test]
fn extra_ym2151_worker_global_csm_lease_independent_of_normal_and_manual_keys() {
    for cancelled in [1, 2] {
        let s = Shared::new(48000, 1);
        let mut r = Renderer::new(48000).unwrap();
        s.create_extra_ym2151(1, None).unwrap();
        s.performance("start", 7, None).unwrap();
        s.enqueue(1, &[own(command(1, 0, write(0x14, 0x85)), 1)])
            .unwrap();
        s.enqueue(
            1,
            &fm(0)
                .into_iter()
                .map(|v| own(command(1, 0, v), 2))
                .collect::<Vec<_>>(),
        )
        .unwrap();
        audition(&s, 7, 9, "on", fm(7));
        render(&mut r, &s, 1000);
        s.performance("cancel-owner", 7, Some(cancelled)).unwrap();
        render(&mut r, &s, 1);
        let v = snapshot(&s, 1);
        assert_eq!(v.key_masks[7], 15);
        assert_eq!(v.held, [7]);
        if cancelled == 1 {
            assert_eq!(v.registers[0x14] & 0x81, 0);
            assert_eq!(v.key_masks[0], 15);
        } else {
            assert_eq!(v.registers[0x14] & 0x81, 0x81);
            assert_eq!(v.key_masks[0], 0);
        }
    }
}
#[test]
fn extra_ym2151_csm_only_manual_holds_all_eight_voices_and_blocks_reset_clock() {
    let s = Shared::new(48000, 1);
    let mut r = Renderer::new(48000).unwrap();
    s.create_extra_ym2151(1, None).unwrap();
    play(
        &s,
        1,
        vec![{
            let mut v = write(0x14, 0x85);
            v["audition"] = json!(true);
            v
        }],
    );
    render(&mut r, &s, 1);
    assert_eq!(snapshot(&s, 1).held, (0..8).collect::<Vec<u8>>());
    play(
        &s,
        1,
        vec![
            json!({"method":"reset"}),
            json!({"method":"all-notes-off"}),
            json!({"method":"stop-voices"}),
        ],
    );
    render(&mut r, &s, 1);
    assert_eq!(snapshot(&s, 1).registers[0x14] & 0x81, 0x81);
    assert!(s
        .enqueue(1, &[command(1, 0, json!({"method":"clock","hz":4000000}))])
        .unwrap_err()
        .contains("manually held additional YM2151"));
}
#[test]
fn extra_ym2151_future_worker_cancel_and_all_notes_off_queue_semantics() {
    for stop in [false, true] {
        let s = Shared::new(48000, 1);
        let mut r = Renderer::new(48000).unwrap();
        s.create_extra_ym2151(1, None).unwrap();
        s.performance("start", 7, None).unwrap();
        s.enqueue(
            1,
            &fm(0)
                .into_iter()
                .map(|v| own(command(1, 1000, v), 1))
                .collect::<Vec<_>>(),
        )
        .unwrap();
        s.enqueue(
            1,
            &[own(
                command(
                    1,
                    0,
                    json!({"method":if stop{"stop-voices"}else{"all-notes-off"}}),
                ),
                1,
            )],
        )
        .unwrap();
        render(&mut r, &s, 2000);
        assert_eq!(snapshot(&s, 1).key_masks[0], if stop { 0 } else { 15 });
        s.enqueue(
            1,
            &fm(1)
                .into_iter()
                .map(|v| own(command(1, 10000, v), 2))
                .collect::<Vec<_>>(),
        )
        .unwrap();
        s.performance("cancel-owner", 7, Some(2)).unwrap();
        render(&mut r, &s, 12000);
        assert_eq!(snapshot(&s, 1).key_masks[1], 0);
        assert_eq!(s.pending.load(Ordering::Acquire), 0);
    }
}
#[test]
fn extra_ym2151_clock_held_race_is_recoverable_and_other_core_unaffected() {
    let s = Shared::new(48000, 1);
    let mut r = Renderer::new(48000).unwrap();
    s.create_extra_ym2151(1, None).unwrap();
    s.create_extra_ym2151(2, None).unwrap();
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
        .extra_ym2151_status(1, Some(1))
        .unwrap_err()
        .contains("manually held additional YM2151"));
    audition(
        &s,
        0,
        1,
        "off",
        vec![json!({"method":"key-off","channel":0})],
    );
    render(&mut r, &s, 1);
    play(&s, 1, vec![json!({"method":"clock","hz":4000000,"id":2})]);
    render(&mut r, &s, 1);
    assert_eq!(
        s.extra_ym2151_status(1, Some(2)).unwrap().ym2151.clock,
        4000000
    );
    assert_eq!(snapshot(&s, 2).clock, 3579545);
}
#[test]
fn extra_ym2151_atomic_validation_and_preflight_budget_reject_before_mutation() {
    let mut s = Shared::new(48000, 1);
    s.extra_psg = extra_psg::Shared::with_memory_limit(extra_budget::INSTANCE_BYTES + 128);
    s.create_extra_ym2151(1, None).unwrap();
    assert!(s
        .enqueue(1, &[command(1, 0, write(8, 0x78))])
        .unwrap_err()
        .contains("memory budget"));
    assert_eq!(s.extra_psg.memory_used(), extra_budget::INSTANCE_BYTES);
    let s = Shared::new(48000, 1);
    s.create_extra_ym2151(1, None).unwrap();
    for bad in [
        json!({"method":"key-off","channel":8}),
        json!({"method":"audition-key-on","channel":0,"mask":16}),
        json!({"method":"batch","channel":0,"action":"edit","commands":[write(0x21,0xc7)]}),
        json!({"method":"batch","channel":0,"audition":true,"action":"on","commands":[write(8,0x78)]}),
    ] {
        assert!(s
            .enqueue(1, &[command(1, 0, write(0x19, 33)), command(1, 0, bad)])
            .is_err());
        assert_eq!(snapshot(&s, 1).am_depth, 0);
    }
}
#[test]
fn extra_ym2151_pending_status_wait_does_not_hold_running_mutex() {
    let rack = NativeGenesis::default();
    let s = Arc::new(Shared::new(48000, 1));
    let mut r = Renderer::new(48000).unwrap();
    let (stop, _) = mpsc::channel();
    *rack.running.lock().unwrap() = Some(Running {
        shared: s.clone(),
        stop,
        thread: None,
    });
    s.create_extra_ym2151(1, None).unwrap();
    s.enqueue(1, &[command(1, 100000, json!({"method":"reset","id":1}))])
        .unwrap();
    let pending = rack.session_shared(1).unwrap();
    let (tx, rx) = mpsc::channel();
    let waiting =
        std::thread::spawn(move || tx.send(pending.extra_ym2151_status(1, Some(1))).unwrap());
    rack.with_session(1, |shared| shared.dispose_extra_ym2151(1))
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
fn extra_ym2151_dispose_and_stop_do_not_resurrect_future_notes_and_reclaim_on_control() {
    for installed in [false, true] {
        let s = Shared::new(48000, 1);
        let mut r = Renderer::new(48000).unwrap();
        s.create_extra_ym2151(1, None).unwrap();
        s.enqueue(
            1,
            &fm(0)
                .into_iter()
                .map(|v| command(1, 100000, v))
                .collect::<Vec<_>>(),
        )
        .unwrap();
        if installed {
            render(&mut r, &s, 1);
        }
        let target = s.extra_psg.get(1, 0).unwrap();
        let weak = Arc::downgrade(&target);
        drop(target);
        s.extra_psg.deactivate_all().unwrap();
        s.epoch.fetch_add(1, Ordering::AcqRel);
        render(&mut r, &s, 2000);
        assert!(weak.upgrade().is_some());
        s.extra_psg.collect_retired().unwrap();
        assert!(weak.upgrade().is_none());
        assert_eq!(s.extra_psg.memory_used(), 0);
        assert_eq!(s.pending.load(Ordering::Acquire), 0);
        s.create_extra_ym2151(2, None).unwrap();
        render(&mut r, &s, 1);
        assert_eq!(snapshot(&s, 2).key_masks, [0; 8]);
    }
}
#[test]
fn extra_ym2151_all_modes_mix_fx_and_x68000_primary_clock_stay_independent() {
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
        s.create_extra_ym2151(1, None).unwrap();
        play(&s, 1, fm(0));
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
        assert_eq!(snapshot(&s, 1).clock, 3579545);
        if let Some(primary) = &s.x68000 {
            assert_eq!(primary.status(0).unwrap().ym2151.clock, 4_000_000);
        }
    }
}
#[test]
fn extra_ym2151_fixed_memory_layout_has_no_oki_or_rom_buffers() {
    eprintln!("extra-ym2151-memory-layout: instance={} shared={} renderer={} session={} fixed={} retirequeue={}",std::mem::size_of::<extra_psg::Instance>(),std::mem::size_of::<extra_ym2151::Shared>(),std::mem::size_of::<extra_ym2151::Renderer>(),std::mem::size_of::<Shared>(),extra_budget::INSTANCE_BYTES,CAPACITY*16);
}
#[test]
fn extra_ym2151_public_all_notes_off_retains_timer_csm_and_resumes_on_normal_keyon() {
    let s = Shared::new(48000, 1);
    let mut r = Renderer::new(48000).unwrap();
    s.create_extra_ym2151(1, None).unwrap();
    play(&s, 1, fm(0));
    play(
        &s,
        1,
        vec![
            grouped(write(0x10, 255), 1),
            grouped(write(0x11, 3), 1),
            grouped(write(0x14, 0x85), 1),
        ],
    );
    render(&mut r, &s, 1000);
    play(&s, 1, vec![json!({"method":"all-notes-off"})]);
    assert_eq!(energy(&render(&mut r, &s, 256)), 0.);
    let v = snapshot(&s, 1);
    assert_eq!(v.registers[0x14], 0x85);
    assert_eq!(v.key_masks, [0; 8]);
    play(&s, 1, vec![write(8, 0x78)]);
    assert!(energy(&render(&mut r, &s, 1000)) > 0.);
    assert_eq!(snapshot(&s, 1).registers[0x14], 0x85);
    play(
        &s,
        1,
        vec![json!({"method":"cancel-main-owner","targetGroup":1})],
    );
    render(&mut r, &s, 1);
    let v = snapshot(&s, 1);
    assert_eq!(v.registers[0x14] & 0x81, 0);
    assert_eq!(v.key_masks[0], 15);
    play(&s, 1, vec![json!({"method":"stop-voices"})]);
    render(&mut r, &s, 1);
    assert_eq!(snapshot(&s, 1).registers[0x14] & 0x81, 0);
}
