//! Offline standalone OPL family rack and source-ownership regressions.
use super::*;
use extra_opl::Chip;
use serde_json::{json, Value};
const CHIPS: [Chip; 3] = [Chip::Ym3526, Chip::Ym3812, Chip::Y8950];
fn create(s: &Shared, chip: Chip, id: u64, hz: Option<u32>) {
    match chip {
        Chip::Ym3526 => s.create_extra_ym3526(id, hz).unwrap(),
        Chip::Ym3812 => s.create_extra_ym3812(id, hz).unwrap(),
        Chip::Y8950 => s.create_extra_y8950(id, hz).unwrap(),
    };
}
fn command(chip: Chip, id: u64, frame: u64, message: Value) -> Command {
    serde_json::from_value(json!({"type":format!("extra-{}",chip.name()),"instance":id,"frame":frame,"message":message})).unwrap()
}
fn write(register: u16, value: u8) -> Value {
    json!({"method":"write","register":register,"value":value})
}
fn group(mut v: Value, id: u64) -> Value {
    v["group"] = json!(id);
    v
}
fn own(mut c: Command, n: u32) -> Command {
    c.performance = Some(7);
    c.owner = Some(n);
    c
}
fn play(s: &Shared, chip: Chip, id: u64, values: Vec<Value>) {
    s.enqueue(
        1,
        &values
            .into_iter()
            .map(|v| command(chip, id, 0, v))
            .collect::<Vec<_>>(),
    )
    .unwrap();
}
fn audition(s: &Shared, chip: Chip, ch: u8, voice: u64, action: &str, commands: Vec<Value>) {
    play(
        s,
        chip,
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
fn status(
    s: &Shared,
    chip: Chip,
    id: u64,
    after: Option<u64>,
    layout: Option<u64>,
) -> Result<extra_opl::Status, String> {
    match chip {
        Chip::Ym3526 => s.extra_ym3526_status(id, after, layout),
        Chip::Ym3812 => s.extra_ym3812_status(id, after, layout),
        Chip::Y8950 => s.extra_y8950_status(id, after, layout),
    }
}
fn inner(v: extra_opl::Status) -> extra_opl::ChipStatus {
    v.ym3526.or(v.ym3812).or(v.y8950).unwrap()
}
fn state(s: &Shared, chip: Chip, id: u64) -> extra_opl::ChipStatus {
    inner(status(s, chip, id, None, None).unwrap())
}
fn note(ch: u8) -> Vec<Value> {
    let slot = [0u16, 1, 2, 8, 9, 10, 16, 17, 18][ch as usize];
    let mut out = Vec::new();
    for (op, tl) in [(slot, 63), (slot + 3, 0)] {
        for (reg, value) in [
            (0x20 + op, 0x21),
            (0x40 + op, tl),
            (0x60 + op, 0xf0),
            (0x80 + op, 0x0f),
        ] {
            out.push(write(reg, value));
        }
    }
    out.extend([
        write(0xa0 + ch as u16, 0x98),
        write(0xc0 + ch as u16, 1),
        write(0xb0 + ch as u16, 0x31),
    ]);
    out
}
fn gate(v: &extra_opl::ChipStatus, ch: usize) -> bool {
    if ch < 9 {
        v.registers[0xb0 + ch] & 32 != 0
    } else {
        v.registers[0xbd] & (16 >> (ch - 18)) != 0
    }
}
fn energy(v: &[[f32; 2]]) -> f32 {
    v.iter().map(|v| v[0].abs() + v[1].abs()).sum()
}
fn new(chip: Chip, hz: Option<u32>) -> (Shared, Renderer) {
    let s = Shared::new(48000, 1);
    let r = Renderer::new(48000).unwrap();
    create(&s, chip, 1, hz);
    (s, r)
}
#[test]
fn extra_opl_two_of_each_mix_with_eight_other_kinds_and_primary_independently() {
    let s = Shared::new(48000, 1);
    let mut r = Renderer::new(48000).unwrap();
    let mut id = 1;
    for chip in CHIPS {
        for ch in [0, 8] {
            create(&s, chip, id, if ch == 8 { Some(4000000) } else { None });
            play(&s, chip, id, note(ch));
            id += 1;
        }
    }
    s.create_extra_psg(7, None).unwrap();
    s.create_extra_ym2612(8, None).unwrap();
    s.create_extra_ym2203(9, None).unwrap();
    s.create_extra_ym2608(10, None).unwrap();
    s.create_extra_ym2610(11, None).unwrap();
    s.create_extra_ym2151(12, None).unwrap();
    s.create_extra_ym2413(13, None).unwrap();
    s.create_extra_ay8910(14, None).unwrap();
    assert!(energy(&render(&mut r, &s, 1000)) > 1.);
    id = 1;
    for chip in CHIPS {
        for ch in [0, 8] {
            let v = state(&s, chip, id);
            assert_eq!(v.chip, chip.name());
            assert_eq!(v.channels, 9);
            assert_eq!(v.registers.len(), 256);
            assert_eq!(v.clock, if ch == 8 { 4000000 } else { 3579545 });
            assert!(gate(&v, ch as usize));
            id += 1;
        }
    }
    play(&s, Chip::Ym3526, 1, vec![json!({"method":"reset"})]);
    render(&mut r, &s, 1);
    assert!(!gate(&state(&s, Chip::Ym3526, 1), 0));
    assert!(gate(&state(&s, Chip::Ym3526, 2), 8));
    assert!(gate(&state(&s, Chip::Y8950, 6), 8));
    assert_eq!(r.fm_owners, [None; 6]);
    assert_eq!(s.extra_psg.counts(), (14, 14));
}
#[test]
fn extra_opl_eleven_kind_limit_pending_retirement_and_typed_identity() {
    let s = Shared::new(48000, 1);
    let mut r = Renderer::new(48000).unwrap();
    for id in 1..=16 {
        create(&s, CHIPS[(id as usize - 1) % 3], id, None);
    }
    assert_eq!(s.extra_psg.memory_used(), 16 * extra_budget::INSTANCE_BYTES);
    assert!(s
        .create_extra_y8950(17, None)
        .unwrap_err()
        .contains("limit"));
    assert!(s.dispose_extra_y8950(1).unwrap_err().contains("match"));
    s.dispose_extra_ym3526(1).unwrap();
    assert!(s
        .create_extra_ym3812(17, None)
        .unwrap_err()
        .contains("limit"));
    render(&mut r, &s, 1);
    s.create_extra_ym3812(17, None).unwrap();
    assert!(s
        .create_extra_ym3526(1, None)
        .unwrap_err()
        .contains("increasing"));
    assert!(s
        .enqueue(1, &[command(Chip::Ym3526, 2, 0, write(0, 1))])
        .unwrap_err()
        .contains("not a YM3526"));
}
#[test]
fn extra_opl_main_normal_replacement_and_stale_off_preserve_new_source() {
    for chip in CHIPS {
        let (s, mut r) = new(chip, None);
        for id in [1, 2] {
            play(
                &s,
                chip,
                1,
                note(0).into_iter().map(|v| group(v, id)).collect(),
            );
        }
        render(&mut r, &s, 1000);
        play(
            &s,
            chip,
            1,
            vec![
                group(write(0xb0, 0x11), 1),
                group(json!({"method":"all-notes-off"}), 1),
                group(json!({"method":"stop-voices"}), 1),
                json!({"method":"cancel-main-owner","targetGroup":1}),
            ],
        );
        render(&mut r, &s, 1);
        assert!(gate(&state(&s, chip, 1), 0));
        play(
            &s,
            chip,
            1,
            vec![json!({"method":"cancel-main-owner","targetGroup":2})],
        );
        render(&mut r, &s, 1);
        assert!(!gate(&state(&s, chip, 1), 0));
    }
}
#[test]
fn extra_opl_worker_normal_and_global_csm_sources_cancel_independently() {
    for chip in CHIPS {
        for cancelled in [1, 2] {
            let (s, mut r) = new(chip, None);
            s.performance("start", 7, None).unwrap();
            s.enqueue(
                1,
                &[
                    own(command(chip, 1, 0, write(2, 255)), 1),
                    own(command(chip, 1, 0, write(8, 128)), 1),
                    own(command(chip, 1, 0, write(4, 1)), 1),
                ],
            )
            .unwrap();
            s.enqueue(
                1,
                &note(0)
                    .into_iter()
                    .map(|v| own(command(chip, 1, 0, v), 2))
                    .collect::<Vec<_>>(),
            )
            .unwrap();
            audition(&s, chip, 8, 1, "on", note(8));
            render(&mut r, &s, 1000);
            s.performance("cancel-owner", 7, Some(cancelled)).unwrap();
            render(&mut r, &s, 1);
            let v = state(&s, chip, 1);
            assert!(gate(&v, 8));
            assert_eq!(v.held, [8]);
            if cancelled == 1 {
                assert_eq!(v.registers[8] & 128, 0);
                assert_eq!(v.registers[4] & 1, 0);
                assert!(gate(&v, 0));
            } else {
                assert_eq!(v.registers[8] & 128, 128);
                assert_eq!(v.registers[4] & 1, 1);
                assert!(!gate(&v, 0));
            }
        }
    }
}
#[test]
fn extra_opl_csm_irq_reset_preserves_effective_timer_ownership_and_manual_guard() {
    for chip in CHIPS {
        let (s, mut r) = new(chip, None);
        play(
            &s,
            chip,
            1,
            vec![
                group(write(2, 255), 1),
                group(write(8, 128), 1),
                group(write(4, 1), 1),
            ],
        );
        play(&s, chip, 1, vec![group(write(4, 128), 2)]);
        render(&mut r, &s, 1);
        assert_eq!(state(&s, chip, 1).registers[4], 128);
        play(
            &s,
            chip,
            1,
            vec![json!({"method":"cancel-main-owner","targetGroup":2})],
        );
        render(&mut r, &s, 1);
        assert_eq!(state(&s, chip, 1).registers[8] & 128, 128);
        play(
            &s,
            chip,
            1,
            vec![json!({"method":"cancel-main-owner","targetGroup":1})],
        );
        render(&mut r, &s, 1);
        assert_eq!(state(&s, chip, 1).registers[8] & 128, 0);
        play(
            &s,
            chip,
            1,
            vec![
                {
                    let mut v = write(4, 1);
                    v["audition"] = json!(true);
                    v
                },
                {
                    let mut v = write(8, 128);
                    v["audition"] = json!(true);
                    v
                },
            ],
        );
        render(&mut r, &s, 1);
        assert_eq!(state(&s, chip, 1).held, (0..9).collect::<Vec<u8>>());
        play(
            &s,
            chip,
            1,
            vec![
                json!({"method":"reset"}),
                json!({"method":"stop-voices"}),
                write(8, 0),
                write(4, 0),
            ],
        );
        render(&mut r, &s, 1);
        assert_eq!(state(&s, chip, 1).registers[8] & 128, 128);
        assert!(s
            .enqueue(
                1,
                &[command(chip, 1, 0, json!({"method":"clock","hz":4000000}))]
            )
            .unwrap_err()
            .contains("manually held additional"));
    }
}
#[test]
fn extra_opl_public_all_notes_off_preserves_csm_transport_and_later_owner_cleanup() {
    for chip in CHIPS {
        let (s, mut r) = new(chip, None);
        play(&s, chip, 1, note(0));
        play(
            &s,
            chip,
            1,
            vec![
                group(write(2, 255), 1),
                group(write(8, 128), 1),
                group(write(4, 1), 1),
            ],
        );
        render(&mut r, &s, 1000);
        play(&s, chip, 1, vec![json!({"method":"all-notes-off"})]);
        render(&mut r, &s, 1000);
        let v = state(&s, chip, 1);
        assert!(!gate(&v, 0));
        assert_eq!(v.registers[8] & 128, 128);
        assert_eq!(v.registers[4] & 1, 1);
        play(
            &s,
            chip,
            1,
            vec![json!({"method":"cancel-main-owner","targetGroup":1})],
        );
        render(&mut r, &s, 1);
        assert_eq!(state(&s, chip, 1).registers[8] & 128, 0);
    }
}
#[test]
fn extra_opl_global_reset_manual_lfo_waveform_and_timer_guards() {
    for chip in CHIPS {
        let (s, mut r) = new(chip, None);
        play(
            &s,
            chip,
            1,
            vec![
                write(1, 32),
                write(2, 99),
                write(3, 88),
                write(8, 64),
                write(0xbd, 0xc0),
            ],
        );
        audition(&s, chip, 0, 1, "on", note(0));
        render(&mut r, &s, 1000);
        play(
            &s,
            chip,
            1,
            vec![
                write(1, 0),
                write(2, 0),
                write(3, 0),
                write(8, 0),
                write(0xbd, 0),
                json!({"method":"reset"}),
                json!({"method":"all-notes-off"}),
                json!({"method":"stop-voices"}),
            ],
        );
        render(&mut r, &s, 1);
        let v = state(&s, chip, 1);
        assert_eq!(v.registers[1], 32);
        assert_eq!(v.registers[2], 99);
        assert_eq!(v.registers[3], 88);
        assert_eq!(v.registers[8], 64);
        assert_eq!(v.registers[0xbd], 0xc0);
        assert!(gate(&v, 0));
        assert_eq!(v.held, [0]);
        audition(
            &s,
            chip,
            0,
            1,
            "off",
            vec![json!({"method":"key-off","channel":0})],
        );
        play(
            &s,
            chip,
            1,
            note(1).into_iter().map(|v| group(v, 2)).collect(),
        );
        play(&s, chip, 1, vec![group(json!({"method":"reset"}), 1)]);
        render(&mut r, &s, 1);
        assert!(!gate(&state(&s, chip, 1), 1));
    }
}
#[test]
fn extra_opl_rhythm_intent_claims_same_on_only_explicit_bits() {
    for chip in CHIPS {
        let (s, mut r) = new(chip, None);
        play(
            &s,
            chip,
            1,
            vec![group(write(0xbd, 0x30), 1), group(write(0xbd, 0x38), 2)],
        );
        let mut replace = group(write(0xbd, 0x38), 3);
        replace["intentMask"] = json!(16);
        play(&s, chip, 1, vec![replace]);
        render(&mut r, &s, 1);
        for (owner, flags) in [(1, 24), (3, 8), (2, 0)] {
            play(
                &s,
                chip,
                1,
                vec![json!({"method":"cancel-main-owner","targetGroup":owner})],
            );
            render(&mut r, &s, 1);
            assert_eq!(state(&s, chip, 1).registers[0xbd] & 31, flags);
        }
    }
}
#[test]
fn extra_opl_rhythm_alias_manual_operators_frequency_and_mode_are_protected() {
    for chip in CHIPS {
        let (s, mut r) = new(chip, None);
        play(
            &s,
            chip,
            1,
            vec![
                write(0xbd, 32),
                write(0x51, 0x12),
                write(0x54, 0x23),
                write(0xa7, 0x98),
            ],
        );
        audition(
            &s,
            chip,
            19,
            1,
            "on",
            vec![json!({"method":"audition-key-on","channel":19})],
        );
        audition(
            &s,
            chip,
            22,
            2,
            "on",
            vec![json!({"method":"audition-key-on","channel":22})],
        );
        render(&mut r, &s, 1000);
        play(
            &s,
            chip,
            1,
            vec![
                write(0xbd, 0),
                write(0x51, 63),
                write(0x54, 63),
                write(0xa7, 1),
            ],
        );
        render(&mut r, &s, 1);
        let v = state(&s, chip, 1);
        assert_eq!(v.registers[0xbd] & 41, 41);
        assert_eq!(v.registers[0x51], 0x12);
        assert_eq!(v.registers[0x54], 0x23);
        assert_eq!(v.registers[0xa7], 0x98);
        assert_eq!(v.held, [19, 22]);
        audition(
            &s,
            chip,
            19,
            1,
            "edit",
            vec![write(0x51, 0x33), write(0x54, 0x34)],
        );
        render(&mut r, &s, 1);
        let v = state(&s, chip, 1);
        assert_eq!(v.registers[0x51], 0x12);
        assert_eq!(v.registers[0x54], 0x34);
        audition(
            &s,
            chip,
            19,
            1,
            "off",
            vec![json!({"method":"key-off","channel":19})],
        );
        render(&mut r, &s, 1);
        let v = state(&s, chip, 1);
        assert_eq!(v.registers[0xbd] & 9, 1);
        assert_eq!(v.held, [22]);
    }
}
#[test]
fn extra_opl_inactive_alias_cancel_does_not_silence_manual_fm() {
    for chip in CHIPS {
        let (s, mut r) = new(chip, None);
        audition(&s, chip, 6, 1, "on", note(6));
        render(&mut r, &s, 1000);
        play(&s, chip, 1, vec![group(write(0xbd, 0x30), 2)]);
        render(&mut r, &s, 1);
        assert_eq!(state(&s, chip, 1).registers[0xbd] & 32, 0);
        audition(
            &s,
            chip,
            18,
            2,
            "on",
            vec![json!({"method":"audition-key-on","channel":18})],
        );
        render(&mut r, &s, 1);
        assert_eq!(state(&s, chip, 1).held, [6]);
        play(
            &s,
            chip,
            1,
            vec![json!({"method":"cancel-main-owner","targetGroup":2})],
        );
        assert!(energy(&render(&mut r, &s, 1000)) > 0.01);
        assert!(gate(&state(&s, chip, 1), 6));
    }
}
#[test]
fn extra_opl_low_clock_pending_preview_off_and_stale_lease_do_not_resurrect() {
    for chip in CHIPS {
        let (s, mut r) = new(chip, Some(100000));
        let mut patch = note(0);
        patch.pop();
        patch.push(json!({"method":"key-off","channel":0}));
        patch.push(json!({"method":"audition-key-on","channel":0}));
        audition(&s, chip, 0, 1, "on", patch.clone());
        r.prepare(&s);
        assert!(!gate(&state(&s, chip, 1), 0));
        audition(
            &s,
            chip,
            0,
            1,
            "off",
            vec![json!({"method":"key-off","channel":0})],
        );
        render(&mut r, &s, 300);
        assert!(!gate(&state(&s, chip, 1), 0));
        audition(&s, chip, 0, 2, "on", patch);
        render(&mut r, &s, 1500);
        assert!(gate(&state(&s, chip, 1), 0));
        audition(
            &s,
            chip,
            0,
            1,
            "off",
            vec![json!({"method":"key-off","channel":0})],
        );
        render(&mut r, &s, 1);
        assert!(gate(&state(&s, chip, 1), 0));
        audition(
            &s,
            chip,
            0,
            2,
            "off",
            vec![json!({"method":"key-off","channel":0})],
        );
        render(&mut r, &s, 300);
        assert!(!gate(&state(&s, chip, 1), 0));
    }
}
#[test]
fn extra_opl_future_worker_cancel_and_all_notes_off_queue_semantics() {
    for chip in CHIPS {
        for stop in [false, true] {
            let (s, mut r) = new(chip, None);
            s.performance("start", 7, None).unwrap();
            s.enqueue(
                1,
                &note(0)
                    .into_iter()
                    .map(|v| own(command(chip, 1, 1000, v), 1))
                    .collect::<Vec<_>>(),
            )
            .unwrap();
            s.enqueue(
                1,
                &[own(
                    command(
                        chip,
                        1,
                        0,
                        json!({"method":if stop{"stop-voices"}else{"all-notes-off"}}),
                    ),
                    1,
                )],
            )
            .unwrap();
            render(&mut r, &s, 2000);
            assert_eq!(gate(&state(&s, chip, 1), 0), !stop);
            s.enqueue(
                1,
                &note(1)
                    .into_iter()
                    .map(|v| own(command(chip, 1, 10000, v), 2))
                    .collect::<Vec<_>>(),
            )
            .unwrap();
            s.performance("cancel-owner", 7, Some(2)).unwrap();
            render(&mut r, &s, 12000);
            assert!(!gate(&state(&s, chip, 1), 1));
            assert_eq!(s.pending.load(Ordering::Acquire), 0);
        }
    }
}
#[test]
fn extra_opl_clock_held_races_are_recoverable_and_other_instances_unchanged() {
    for chip in CHIPS {
        let (s, mut r) = new(chip, None);
        create(&s, chip, 2, None);
        s.enqueue(
            1,
            &[command(
                chip,
                1,
                1000,
                json!({"method":"clock","hz":4000000,"id":1}),
            )],
        )
        .unwrap();
        audition(&s, chip, 0, 1, "on", note(0));
        render(&mut r, &s, 2000);
        assert!(status(&s, chip, 1, Some(1), None)
            .unwrap_err()
            .contains("manually held additional"));
        assert!(s
            .enqueue(
                1,
                &[command(chip, 1, 0, json!({"method":"clock","hz":4000000}))]
            )
            .unwrap_err()
            .contains("manually held additional"));
        audition(
            &s,
            chip,
            0,
            1,
            "off",
            vec![json!({"method":"key-off","channel":0})],
        );
        render(&mut r, &s, 1);
        play(
            &s,
            chip,
            1,
            vec![json!({"method":"clock","hz":4000000,"id":2})],
        );
        render(&mut r, &s, 1);
        assert_eq!(
            inner(status(&s, chip, 1, Some(2), None).unwrap()).clock,
            4000000
        );
        assert_eq!(state(&s, chip, 2).clock, 3579545);
    }
}
#[test]
fn extra_opl_validation_and_preflight_budget_are_atomic() {
    for chip in CHIPS {
        let mut s = Shared::new(48000, 1);
        s.extra_psg = extra_psg::Shared::with_memory_limit(extra_budget::INSTANCE_BYTES + 128);
        create(&s, chip, 1, None);
        assert!(s
            .enqueue(1, &[command(chip, 1, 0, write(0, 1))])
            .unwrap_err()
            .contains("memory budget"));
        assert_eq!(s.extra_psg.memory_used(), extra_budget::INSTANCE_BYTES);
        let (s, mut r) = new(chip, None);
        for bad in [
            write(256, 1),
            json!({"method":"key-off","channel":9}),
            json!({"method":"batch","channel":0,"action":"edit","commands":[write(0xb1,1)]}),
            json!({"method":"write","register":0xbd,"value":1,"intentMask":32}),
        ] {
            assert!(s
                .enqueue(
                    1,
                    &[command(chip, 1, 0, write(0, 33)), command(chip, 1, 0, bad)]
                )
                .is_err());
            render(&mut r, &s, 1);
            assert_eq!(state(&s, chip, 1).registers[0], 0);
        }
    }
}
#[test]
fn extra_opl_failed_mixed_retain_rolls_back_all_guards() {
    let (s, mut r) = new(Chip::Ym3526, None);
    create(&s, Chip::Y8950, 2, None);
    play(
        &s,
        Chip::Y8950,
        2,
        vec![
            json!({"method":"batch","channel":0,"voice":1,"action":"on","audition":true,"commands":note(0)}),
        ],
    );
    render(&mut r, &s, 1);
    s.extra_psg.collect_retired().unwrap();
    let baseline = s.extra_psg.memory_used();
    assert!(s
        .enqueue(
            1,
            &[
                command(
                    Chip::Ym3526,
                    1,
                    0,
                    json!({"method":"write","register":0,"value":33,"id":1})
                ),
                command(
                    Chip::Y8950,
                    2,
                    0,
                    json!({"method":"clock","hz":4000000,"id":2})
                )
            ]
        )
        .unwrap_err()
        .contains("manually held additional Y8950"));
    s.extra_psg.collect_retired().unwrap();
    assert_eq!(s.extra_psg.memory_used(), baseline);
    assert_eq!(state(&s, Chip::Ym3526, 1).registers[0], 0);
    assert!(state(&s, Chip::Ym3526, 1).pending_commands.is_empty());
}
#[test]
fn extra_opl_layout_observer_handles_overtaking_keyboard_and_unchanged_cancel() {
    for chip in CHIPS {
        let s = Arc::new(Shared::new(48000, 1));
        let mut r = Renderer::new(48000).unwrap();
        create(&s, chip, 1, None);
        s.performance("start", 7, None).unwrap();
        s.enqueue(
            1,
            &[own(
                command(
                    chip,
                    1,
                    1000,
                    json!({"method":"write","register":0xbd,"value":0,"id":1}),
                ),
                1,
            )],
        )
        .unwrap();
        let pending = s.clone();
        let (tx, rx) = mpsc::channel();
        let waiting =
            std::thread::spawn(move || tx.send(status(&pending, chip, 1, None, Some(0))).unwrap());
        assert!(rx.recv_timeout(Duration::from_millis(20)).is_err());
        s.enqueue(
            1,
            &[own(
                command(
                    chip,
                    1,
                    0,
                    json!({"method":"write","register":0xbd,"value":32,"id":2}),
                ),
                2,
            )],
        )
        .unwrap();
        render(&mut r, &s, 1);
        let v = inner(
            rx.recv_timeout(Duration::from_millis(200))
                .unwrap()
                .unwrap(),
        );
        waiting.join().unwrap();
        assert_eq!(v.layout_revision, Some(1));
        assert_eq!(v.layout_pending, Some(true));
        assert_eq!(v.registers[0xbd] & 32, 32);
        render(&mut r, &s, 1100);
        let v = inner(status(&s, chip, 1, None, Some(1)).unwrap());
        assert_eq!(v.layout_revision, Some(2));
        assert_eq!(v.layout_pending, Some(false));
        s.enqueue(
            1,
            &[own(
                command(
                    chip,
                    1,
                    10000,
                    json!({"method":"write","register":0xbd,"value":0,"id":3}),
                ),
                3,
            )],
        )
        .unwrap();
        let pending = s.clone();
        let (tx, rx) = mpsc::channel();
        let waiting =
            std::thread::spawn(move || tx.send(status(&pending, chip, 1, None, Some(2))).unwrap());
        assert!(rx.recv_timeout(Duration::from_millis(20)).is_err());
        s.performance("cancel-owner", 7, Some(3)).unwrap();
        render(&mut r, &s, 1);
        let v = inner(
            rx.recv_timeout(Duration::from_millis(200))
                .unwrap()
                .unwrap(),
        );
        waiting.join().unwrap();
        assert_eq!(v.layout_revision, Some(2));
        assert_eq!(v.layout_pending, Some(false));
    }
}
#[test]
fn extra_opl_status_wait_does_not_hold_running_mutex_and_dispose_wakes() {
    for chip in CHIPS {
        let rack = NativeGenesis::default();
        let s = Arc::new(Shared::new(48000, 1));
        let mut r = Renderer::new(48000).unwrap();
        let (stop, _) = mpsc::channel();
        *rack.running.lock().unwrap() = Some(Running {
            shared: s.clone(),
            stop,
            thread: None,
        });
        create(&s, chip, 1, None);
        s.enqueue(
            1,
            &[command(
                chip,
                1,
                100000,
                json!({"method":"write","register":0xbd,"value":32,"id":1}),
            )],
        )
        .unwrap();
        let pending = rack.session_shared(1).unwrap();
        let (tx, rx) = mpsc::channel();
        let waiting =
            std::thread::spawn(move || tx.send(status(&pending, chip, 1, None, Some(0))).unwrap());
        assert!(rx.recv_timeout(Duration::from_millis(20)).is_err());
        rack.with_session(1, |shared| match chip {
            Chip::Ym3526 => shared.dispose_extra_ym3526(1),
            Chip::Ym3812 => shared.dispose_extra_ym3812(1),
            Chip::Y8950 => shared.dispose_extra_y8950(1),
        })
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
}
#[test]
fn extra_opl_stop_dispose_future_commands_reclaim_only_on_control() {
    for chip in CHIPS {
        for installed in [false, true] {
            let (s, mut r) = new(chip, None);
            s.enqueue(
                1,
                &note(0)
                    .into_iter()
                    .map(|v| command(chip, 1, 100000, v))
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
            create(&s, chip, 2, None);
            render(&mut r, &s, 1);
            assert!(state(&s, chip, 2).held.is_empty());
        }
    }
}
#[test]
fn extra_opl_all_twenty_two_modes_share_mixer_without_changing_primary() {
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
        for (chip, id) in [(Chip::Ym3526, 1), (Chip::Ym3812, 2), (Chip::Y8950, 3)] {
            create(&s, chip, id, None);
            play(&s, chip, id, note(0));
        }
        assert!(energy(&render(&mut r, &s, 1000)) > 1., "{mode}");
        for (chip, id) in [(Chip::Ym3526, 1), (Chip::Ym3812, 2), (Chip::Y8950, 3)] {
            play(
                &s,
                chip,
                id,
                vec![json!({"method":"mix","volume":0.,"pan":0.})],
            );
        }
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
fn extra_opl_y8950_adpcm_execute_off_keeps_lease_until_reset_or_owner_cancel() {
    let (s, mut r) = new(Chip::Y8950, None);
    play(
        &s,
        Chip::Y8950,
        1,
        vec![group(write(7, 128), 1), group(write(7, 0), 1)],
    );
    render(&mut r, &s, 1);
    assert_eq!(state(&s, Chip::Y8950, 1).registers[7], 0);
    play(
        &s,
        Chip::Y8950,
        1,
        vec![json!({"method":"cancel-main-owner","targetGroup":1})],
    );
    render(&mut r, &s, 1);
    assert_eq!(state(&s, Chip::Y8950, 1).registers[7], 1);
    play(
        &s,
        Chip::Y8950,
        1,
        vec![
            group(write(7, 128), 1),
            group(write(7, 128), 2),
            group(write(7, 0), 1),
            json!({"method":"cancel-main-owner","targetGroup":1}),
        ],
    );
    render(&mut r, &s, 1);
    assert_eq!(state(&s, Chip::Y8950, 1).registers[7], 128);
    play(
        &s,
        Chip::Y8950,
        1,
        vec![json!({"method":"cancel-main-owner","targetGroup":2})],
    );
    render(&mut r, &s, 1);
    assert_eq!(state(&s, Chip::Y8950, 1).registers[7], 1);
}
#[test]
fn extra_opl_fixed_layout_fits_common_core_headroom_without_sample_memory() {
    eprintln!(
        "extra-opl-layout instance={} shared={} renderer={} session={} fixed={} retirementqueue={}",
        std::mem::size_of::<extra_psg::Instance>(),
        std::mem::size_of::<extra_opl::Shared>(),
        std::mem::size_of::<extra_opl::Renderer>(),
        std::mem::size_of::<Shared>(),
        extra_budget::INSTANCE_BYTES,
        CAPACITY * 16
    );
    assert!(std::mem::size_of::<extra_psg::Instance>() < 32768);
}
#[test]
fn extra_opl_stale_csm_disable_cannot_reclaim_protected_source() {
    for chip in CHIPS {
        for register in [4, 8] {
            let (s, mut r) = new(chip, None);
            play(
                &s,
                chip,
                1,
                vec![
                    group(write(2, 255), 2),
                    group(write(8, 128), 2),
                    group(write(4, 1), 2),
                ],
            );
            render(&mut r, &s, 1);
            play(
                &s,
                chip,
                1,
                vec![
                    group(write(register, 0), 1),
                    json!({"method":"cancel-main-owner","targetGroup":1}),
                ],
            );
            render(&mut r, &s, 1);
            let v = state(&s, chip, 1);
            assert_eq!(v.registers[8] & 128, 128);
            assert_eq!(v.registers[4] & 1, 1);
            play(
                &s,
                chip,
                1,
                vec![json!({"method":"cancel-main-owner","targetGroup":2})],
            );
            render(&mut r, &s, 1);
            assert_eq!(state(&s, chip, 1).registers[8] & 128, 0);
        }
    }
}
