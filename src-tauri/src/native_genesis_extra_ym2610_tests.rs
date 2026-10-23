//! Offline independent YM2610 rack/ownership tests. No audio devices.
use super::*;
use serde_json::{json, Value};
fn command(instance: u64, frame: u64, message: Value) -> Command {
    serde_json::from_value(
        json!({"type":"extra-ym2610","instance":instance,"frame":frame,"message":message}),
    )
    .unwrap()
}
fn memory(instance: u64, bank: u8, offset: u32, data: Vec<u8>) -> Command {
    Command {
        frame: 0,
        performance: None,
        owner: None,
        action: CommandAction::ExtraYm2610 {
            instance,
            message: extra_ym2610::Message {
                id: None,
                audition: false,
                group: None,
                method: extra_ym2610::Method::Memory {
                    rom_type: bank,
                    offset,
                    data,
                },
            },
        },
    }
}
fn write(port: u8, register: u8, value: u8) -> Value {
    json!({"method":"write","port":port,"register":register,"value":value})
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
    let port = ch / 3;
    let local = ch % 3;
    let mut out = Vec::new();
    for slot in [0, 4, 8, 12] {
        for (register, value) in [
            (0x30 + slot + local, 1),
            (0x40 + slot + local, if slot == 12 { 48 } else { 127 }),
            (0x50 + slot + local, 31),
            (0x60 + slot + local, 0),
            (0x70 + slot + local, 0),
            (0x80 + slot + local, 15),
        ] {
            out.push(write(port, register, value))
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
fn play(s: &Shared, id: u64, v: Vec<Value>) {
    s.enqueue(
        1,
        &v.into_iter().map(|v| command(id, 0, v)).collect::<Vec<_>>(),
    )
    .unwrap()
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
fn snapshot(s: &Shared, id: u64) -> extra_ym2610::AdditionalChipStatus {
    s.extra_ym2610_status(id, None).unwrap().ym2610
}
fn energy(v: &[[f32; 2]]) -> f32 {
    v.iter().map(|v| v[0].abs() + v[1].abs()).sum()
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
#[test]
fn extra_ym2610_two_instances_mix_with_other_four_kinds_independently() {
    let s = Shared::new(48000, 1);
    let mut r = Renderer::new(48000).unwrap();
    s.create_extra_ym2610(1, None).unwrap();
    s.create_extra_ym2610(2, Some(7987200)).unwrap();
    s.create_extra_ym2608(3, None).unwrap();
    s.create_extra_ym2203(4, None).unwrap();
    s.create_extra_ym2612(5, None).unwrap();
    s.create_extra_psg(6, None).unwrap();
    play(&s, 1, fm(1));
    play(&s, 2, ssg(1));
    assert!(energy(&render(&mut r, &s, 1000)) > 1.);
    assert_eq!(snapshot(&s, 1).key_masks, [0, 15, 0, 0, 0, 0]);
    assert_eq!(snapshot(&s, 2).clock, 7987200);
    assert_eq!(snapshot(&s, 1).variant, "ym2610");
    assert_eq!(snapshot(&s, 1).rom_bytes, [0, 0]);
    assert_eq!(snapshot(&s, 1).rom_allocated_bytes, [0, 0]);
    assert_eq!(r.fm_owners, [None; 6]);
    play(&s, 1, vec![json!({"method":"reset"})]);
    render(&mut r, &s, 1);
    assert_eq!(snapshot(&s, 1).key_masks, [0; 6]);
    assert!(snapshot(&s, 2).ssg_gates[1]);
    assert_eq!(s.extra_psg.counts(), (6, 6));
}
#[test]
fn extra_ym2610_sparse_sixteen_instances_and_type_identity() {
    let s = Shared::new(48000, 1);
    let mut r = Renderer::new(48000).unwrap();
    for id in 1..=16 {
        s.create_extra_ym2610(id, None).unwrap();
    }
    assert_eq!(s.extra_psg.memory_used(), 16 * extra_budget::INSTANCE_BYTES);
    assert!(s
        .create_extra_ym2610(17, None)
        .unwrap_err()
        .contains("limit"));
    assert!(s.dispose_extra_ym2608(1).unwrap_err().contains("match"));
    s.dispose_extra_ym2610(1).unwrap();
    assert!(s
        .create_extra_ym2610(17, None)
        .unwrap_err()
        .contains("limit"));
    render(&mut r, &s, 1);
    s.create_extra_ym2610(17, None).unwrap();
    assert!(s
        .create_extra_psg(1, None)
        .unwrap_err()
        .contains("increasing"));
}
#[test]
fn extra_ym2610_inactive_raw_fm_legal_but_manual_auditions_unavailable() {
    let s = Shared::new(48000, 1);
    let mut r = Renderer::new(48000).unwrap();
    s.create_extra_ym2610(1, None).unwrap();
    for ch in [0, 3] {
        play(&s, 1, fm(ch));
        assert!(s.enqueue(1,&[command(1,0,json!({"method":"batch","channel":ch,"audition":true,"voice":1,"action":"on","commands":fm(ch)}))]).is_err());
        assert!(s
            .enqueue(
                1,
                &[command(1, 0, json!({"method":"key-off","channel":ch}))]
            )
            .is_err());
    }
    render(&mut r, &s, 1000);
    assert!(snapshot(&s, 1).held.is_empty());
    for ch in [1, 2, 4, 5] {
        play(&s, 1, fm(ch));
        render(&mut r, &s, 100);
        assert_eq!(snapshot(&s, 1).key_masks[ch as usize], 15);
    }
}
#[test]
fn extra_ym2610_full_banks_replacement_fits_budget_without_double_data_copy() {
    let s = Shared::new(48000, 1);
    let mut r = Renderer::new(48000).unwrap();
    s.create_extra_ym2610(1, None).unwrap();
    s.enqueue(
        1,
        &[
            memory(1, 0, 0, vec![1; 16 * 1024 * 1024]),
            memory(1, 1, 0, vec![2; 16 * 1024 * 1024]),
        ],
    )
    .unwrap();
    render(&mut r, &s, 1);
    assert_eq!(snapshot(&s, 1).rom_bytes, [16777216, 16777216]);
    s.extra_psg.collect_retired().unwrap();
    s.enqueue(1, &[memory(1, 0, 0, vec![3; 16 * 1024 * 1024])])
        .unwrap();
    assert!(s.extra_psg.memory_used() < 64 * 1024 * 1024);
    assert_eq!(snapshot(&s, 1).rom_allocated_bytes[0], 32 * 1024 * 1024);
    render(&mut r, &s, 1);
    s.extra_psg.collect_retired().unwrap();
    assert_eq!(snapshot(&s, 1).rom_allocated_bytes, [16777216, 16777216]);
}
#[test]
fn extra_ym2610_partial_high_address_uploads_and_invalid_ranges_are_atomic() {
    let s = Shared::new(48000, 1);
    let mut r = Renderer::new(48000).unwrap();
    s.create_extra_ym2610(1, None).unwrap();
    s.create_extra_ym2610(2, None).unwrap();
    s.enqueue(
        1,
        &[
            memory(1, 0, 65535, vec![1, 2]),
            memory(1, 1, 16777215, vec![3]),
        ],
    )
    .unwrap();
    render(&mut r, &s, 1);
    let v = snapshot(&s, 1);
    assert_eq!(v.rom_bytes, [65537, 16777216]);
    assert_eq!(v.rom_allocated_bytes, [131072, 65536]);
    let bytes = s.extra_psg.memory_used();
    for bad in [
        memory(1, 0, 16777215, vec![1, 2]),
        memory(1, 2, 0, vec![1]),
        memory(1, 0, 0, vec![]),
    ] {
        assert!(s
            .enqueue(1, &[bad, command(2, 0, write(0, 8, 15))])
            .is_err());
        assert_eq!(snapshot(&s, 2).registers[0][8], 0);
        assert_eq!(s.extra_psg.memory_used(), bytes);
    }
}
#[test]
fn extra_ym2610_failed_mixed_admission_does_not_publish_or_leak_new_pages() {
    let s = Shared::new(48000, 1);
    let mut r = Renderer::new(48000).unwrap();
    s.create_extra_ym2610(1, None).unwrap();
    s.create_extra_ym2610(2, None).unwrap();
    s.enqueue(1, &[memory(1, 0, 0, vec![1])]).unwrap();
    render(&mut r, &s, 1);
    s.extra_psg.collect_retired().unwrap();
    let baseline = s.extra_psg.memory_used();
    for _ in 0..5 {
        let duplicate = || {
            command(
                2,
                0,
                json!({"method":"write","id":1,"port":0,"register":8,"value":1}),
            )
        };
        assert!(s
            .enqueue(
                1,
                &[memory(1, 1, 65535, vec![2, 3]), duplicate(), duplicate()]
            )
            .unwrap_err()
            .contains("Duplicate"));
        s.extra_psg.collect_retired().unwrap();
        assert_eq!(snapshot(&s, 1).rom_bytes, [1, 0]);
        assert_eq!(snapshot(&s, 1).rom_allocated_bytes, [65536, 0]);
        assert_eq!(s.extra_psg.memory_used(), baseline);
    }
}
#[test]
fn extra_ym2610_page_budget_failure_rolls_back_and_reclaims_after_stalled_callback() {
    let mut s = Shared::new(48000, 1);
    s.extra_psg = extra_psg::Shared::with_memory_limit(
        extra_budget::INSTANCE_BYTES + 3 * (65536 + 64) + 4096,
    );
    let mut r = Renderer::new(48000).unwrap();
    s.create_extra_ym2610(1, None).unwrap();
    s.enqueue(1, &[memory(1, 0, 0, vec![1])]).unwrap();
    render(&mut r, &s, 1);
    s.extra_psg.collect_retired().unwrap();
    let baseline = s.extra_psg.memory_used();
    assert!(s
        .enqueue(1, &[memory(1, 1, 0, vec![2; 3 * 65536])])
        .unwrap_err()
        .contains("memory budget"));
    s.extra_psg.collect_retired().unwrap();
    assert_eq!(s.extra_psg.memory_used(), baseline);
    assert_eq!(snapshot(&s, 1).rom_allocated_bytes, [65536, 0]);
    s.enqueue(1, &[memory(1, 0, 0, vec![3])]).unwrap();
    assert!(s.extra_psg.memory_used() > baseline);
    s.dispose_extra_ym2610(1).unwrap();
    assert!(s.extra_psg.memory_used() > 0);
    render(&mut r, &s, 1);
    s.extra_psg.collect_retired().unwrap();
    assert_eq!(s.extra_psg.memory_used(), 0);
}
#[test]
fn extra_ym2610_accepted_memory_is_fifo_resource_and_survives_note_owner_cancel() {
    let s = Shared::new(48000, 1);
    let mut r = Renderer::new(48000).unwrap();
    s.create_extra_ym2610(1, None).unwrap();
    s.performance("start", 7, None).unwrap();
    let mut a = memory(1, 0, 0, vec![1]);
    a.frame = 100000;
    let a = own(a, 1);
    s.enqueue(1, &[a]).unwrap();
    s.performance("cancel-owner", 7, Some(1)).unwrap();
    let mut b = memory(1, 0, 1, vec![2]);
    b.frame = 200000;
    s.enqueue(1, &[b]).unwrap();
    render(&mut r, &s, 1);
    assert_eq!(snapshot(&s, 1).rom_bytes, [2, 0]);
    assert_eq!(snapshot(&s, 1).rom_allocated_bytes, [65536, 0]);
    assert_eq!(s.pending.load(Ordering::Acquire), 0);
    let failed = own(memory(1, 1, 0, vec![3]), 1);
    assert!(s.enqueue(1, &[failed]).unwrap_err().contains("expired"));
}
#[test]
fn extra_ym2610_manual_ssg_a_b_guards_shared_controls_and_foreign_stop_masks() {
    let s = Shared::new(48000, 1);
    let mut r = Renderer::new(48000).unwrap();
    s.create_extra_ym2610(1, None).unwrap();
    audition(
        &s,
        6,
        1,
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
        2,
        "on",
        vec![write(1, 8, 0xdf), write(1, 1, 0), write(1, 0, 1)],
    );
    audition(
        &s,
        15,
        3,
        "on",
        vec![write(0, 0x11, 0xc0), write(0, 0x10, 0x80)],
    );
    render(&mut r, &s, 100);
    play(
        &s,
        1,
        vec![
            write(0, 6, 31),
            write(0, 13, 0),
            write(1, 1, 63),
            write(1, 0, 0xbf),
            write(0, 0x10, 0),
            json!({"method":"reset"}),
        ],
    );
    render(&mut r, &s, 1);
    let v = snapshot(&s, 1);
    assert_eq!(v.held, [6, 9, 15]);
    assert_eq!(v.registers[0][6], 3);
    assert_eq!(v.registers[0][13], 10);
    assert_eq!(v.registers[1][1], 0);
    assert_eq!(v.registers[1][0] & 1, 0);
    assert_eq!(v.registers[0][0x10], 0x80);
}
#[test]
fn extra_ym2610_main_groups_and_raw_b_offs_preserve_replacement_sources() {
    let s = Shared::new(48000, 1);
    let mut r = Renderer::new(48000).unwrap();
    s.create_extra_ym2610(1, None).unwrap();
    play(
        &s,
        1,
        fm(1)
            .into_iter()
            .chain(ssg(0))
            .map(|v| grouped(v, 1))
            .collect(),
    );
    play(&s, 1, fm(1).into_iter().map(|v| grouped(v, 2)).collect());
    play(
        &s,
        1,
        vec![
            grouped(write(1, 0, 1), 1),
            grouped(write(1, 0, 2), 2),
            grouped(write(0, 0x10, 0x80), 2),
        ],
    );
    render(&mut r, &s, 100);
    play(
        &s,
        1,
        vec![
            json!({"method":"cancel-main-owner","targetGroup":1}),
            grouped(write(0, 0x28, 1), 1),
            grouped(write(1, 0, 0xbf), 1),
            grouped(write(0, 0x10, 0), 1),
            grouped(write(0, 0x10, 0xc0), 1),
        ],
    );
    render(&mut r, &s, 1);
    let v = snapshot(&s, 1);
    assert_eq!(v.key_masks[1], 15);
    assert!(!v.ssg_gates[0]);
    assert_eq!(v.registers[1][0] & 2, 0);
    assert_eq!(v.registers[0][0x10], 0x80);
}
#[test]
fn extra_ym2610_worker_fm_csm_a_b_owners_cancel_independently() {
    for cancelled in [1, 2] {
        let s = Shared::new(48000, 1);
        let mut r = Renderer::new(48000).unwrap();
        s.create_extra_ym2610(1, None).unwrap();
        s.performance("start", 7, None).unwrap();
        s.enqueue(1, &[own(command(1, 0, write(0, 0x27, 0x85)), 1)])
            .unwrap();
        s.enqueue(
            1,
            &fm(2)
                .into_iter()
                .map(|v| own(command(1, 0, v), 2))
                .collect::<Vec<_>>(),
        )
        .unwrap();
        s.enqueue(
            1,
            &[
                own(command(1, 0, write(1, 0, 1)), 3),
                own(command(1, 0, write(1, 0, 2)), 4),
                own(command(1, 0, write(0, 0x10, 0x80)), 5),
            ],
        )
        .unwrap();
        render(&mut r, &s, 1000);
        s.performance("cancel-owner", 7, Some(cancelled)).unwrap();
        render(&mut r, &s, 1);
        let v = snapshot(&s, 1);
        if cancelled == 1 {
            assert_eq!(v.registers[0][0x27] & 0x81, 0);
            assert_eq!(v.key_masks[2], 15);
        } else {
            assert_eq!(v.registers[0][0x27] & 0x81, 0x81);
            assert_eq!(v.key_masks[2], 0);
        }
        assert_eq!(v.registers[0][0x10], 0x80);
        s.performance("cancel-owner", 7, Some(3)).unwrap();
        render(&mut r, &s, 1);
        assert_eq!(snapshot(&s, 1).registers[1][0] & 0x3f, 1);
    }
}
#[test]
fn extra_ym2610_clock_held_race_and_csm_only_manual_guard_are_recoverable() {
    let s = Shared::new(48000, 1);
    let mut r = Renderer::new(48000).unwrap();
    s.create_extra_ym2610(1, None).unwrap();
    s.enqueue(
        1,
        &[command(
            1,
            1000,
            json!({"method":"clock","hz":4000000,"id":1}),
        )],
    )
    .unwrap();
    audition(&s, 1, 9, "on", fm(1));
    render(&mut r, &s, 2000);
    assert!(s
        .extra_ym2610_status(1, Some(1))
        .unwrap_err()
        .contains("manually held additional YM2610"));
    audition(
        &s,
        1,
        9,
        "off",
        vec![json!({"method":"key-off","channel":1})],
    );
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
        vec![json!({"method":"reset"}), json!({"method":"stop-voices"})],
    );
    render(&mut r, &s, 1);
    assert_eq!(snapshot(&s, 1).registers[0][0x27] & 0x81, 0x81);
    assert!(s
        .enqueue(1, &[command(1, 0, json!({"method":"clock","hz":4000000}))])
        .unwrap_err()
        .contains("manually held"));
}
#[test]
fn extra_ym2610_pending_status_wait_releases_running_mutex_on_disposal() {
    let rack = NativeGenesis::default();
    let s = Arc::new(Shared::new(48000, 1));
    let mut r = Renderer::new(48000).unwrap();
    let (stop, _) = mpsc::channel();
    *rack.running.lock().unwrap() = Some(Running {
        shared: s.clone(),
        stop,
        thread: None,
    });
    s.create_extra_ym2610(1, None).unwrap();
    s.enqueue(1, &[command(1, 100000, json!({"method":"reset","id":1}))])
        .unwrap();
    let pending = rack.session_shared(1).unwrap();
    let (tx, rx) = mpsc::channel();
    let waiting =
        std::thread::spawn(move || tx.send(pending.extra_ym2610_status(1, Some(1))).unwrap());
    rack.with_session(1, |shared| shared.dispose_extra_ym2610(1))
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
fn extra_ym2610_stop_pending_resource_pages_never_resurrect_and_free_on_control() {
    for installed in [false, true] {
        let s = Shared::new(48000, 1);
        let mut r = Renderer::new(48000).unwrap();
        s.create_extra_ym2610(1, None).unwrap();
        s.enqueue(
            1,
            &[
                memory(1, 0, 0, vec![1; 65537]),
                command(1, 100000, write(0, 8, 15)),
            ],
        )
        .unwrap();
        if installed {
            render(&mut r, &s, 128);
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
        s.create_extra_ym2610(2, None).unwrap();
        render(&mut r, &s, 1);
        assert_eq!(snapshot(&s, 2).rom_bytes, [0, 0]);
    }
}
#[test]
fn extra_ym2610_thirty_two_mib_batch_limit_rejects_before_pages_are_allocated() {
    let s = Shared::new(48000, 1);
    s.create_extra_ym2610(1, None).unwrap();
    let baseline = s.extra_psg.memory_used();
    assert!(s
        .enqueue(
            1,
            &[
                memory(1, 0, 0, vec![0; 16777216]),
                memory(1, 1, 0, vec![0; 16777216]),
                memory(1, 0, 0, vec![1])
            ]
        )
        .unwrap_err()
        .contains("32 MiB"));
    assert_eq!(s.extra_psg.memory_used(), baseline);
    assert_eq!(snapshot(&s, 1).rom_allocated_bytes, [0, 0]);
}
#[test]
fn extra_ym2610_all_native_modes_use_common_mix_fx_and_isolated_core() {
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
        s.create_extra_ym2610(1, None).unwrap();
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
fn extra_ym2610_memory_layout_uses_fixed_sparse_tables() {
    eprintln!("extra-ym2610-memory-layout: instance={} shared={} renderer={} session={} fixed_instance={} retire_queue={} page=65536+64 metadata",std::mem::size_of::<extra_psg::Instance>(),std::mem::size_of::<extra_ym2610::Shared>(),std::mem::size_of::<extra_ym2610::Renderer>(),std::mem::size_of::<Shared>(),extra_budget::INSTANCE_BYTES,CAPACITY*16);
}
