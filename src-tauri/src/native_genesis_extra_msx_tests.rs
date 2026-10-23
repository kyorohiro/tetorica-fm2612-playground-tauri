//! Independent additional OPLL/AY tests. All renderers are offline.
use super::*;
use extra_msx::Chip;
use serde_json::{json, Value};
fn create(s: &Shared, chip: Chip, id: u64, hz: Option<u32>) {
    match chip {
        Chip::Ym2413 => {
            s.create_extra_ym2413(id, hz).unwrap();
        }
        Chip::Ay8910 => {
            s.create_extra_ay8910(id, hz).unwrap();
        }
    };
}
fn command(chip: Chip, id: u64, frame: u64, message: Value) -> Command {
    serde_json::from_value(json!({"type":if chip==Chip::Ym2413{"extra-ym2413"}else{"extra-ay8910"},"instance":id,"frame":frame,"message":message})).unwrap()
}
fn write(register: u8, value: u8) -> Value {
    json!({"method":"write","register":register,"value":value})
}
fn group(mut v: Value, id: u64) -> Value {
    v["group"] = json!(id);
    v
}
fn owned(mut c: Command, owner: u32) -> Command {
    c.performance = Some(7);
    c.owner = Some(owner);
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
fn state(s: &Shared, chip: Chip, id: u64) -> extra_msx::ChipStatus {
    match chip {
        Chip::Ym2413 => s.extra_ym2413_status(id, None).unwrap().ym2413,
        Chip::Ay8910 => s.extra_ay8910_status(id, None).unwrap().ay8910,
    }
}
fn tone(chip: Chip, ch: u8) -> Vec<Value> {
    match chip {
        Chip::Ym2413 => vec![
            write(0x30 + ch, 0x10),
            write(0x10 + ch, 0x80),
            write(0x20 + ch, 0x19),
        ],
        Chip::Ay8910 => vec![
            write(ch * 2, 128),
            write(ch * 2 + 1, 1),
            write(7, 0x38),
            write(ch + 8, 9),
        ],
    }
}
fn energy(v: &[[f32; 2]]) -> f32 {
    v.iter().map(|v| v[0].abs() + v[1].abs()).sum()
}
fn gate(v: &extra_msx::ChipStatus, chip: Chip, ch: usize) -> bool {
    if chip == Chip::Ay8910 {
        v.gates[ch]
    } else if ch < 9 {
        v.registers[0x20 + ch] & 16 != 0
    } else {
        v.registers[14] & (16 >> (ch - 9)) != 0
    }
}
fn new(chip: Chip, hz: Option<u32>) -> (Shared, Renderer) {
    let s = Shared::new(48000, 1);
    let r = Renderer::new(48000).unwrap();
    create(&s, chip, 1, hz);
    (s, r)
}
#[test]
fn extra_msx_eight_kinds_two_opll_two_ay_and_other_cores_are_independent() {
    let (s, mut r) = new(Chip::Ym2413, None);
    create(&s, Chip::Ym2413, 2, Some(4000000));
    create(&s, Chip::Ay8910, 3, None);
    create(&s, Chip::Ay8910, 4, Some(2000000));
    s.create_extra_psg(5, None).unwrap();
    s.create_extra_ym2612(6, None).unwrap();
    s.create_extra_ym2203(7, None).unwrap();
    s.create_extra_ym2608(8, None).unwrap();
    s.create_extra_ym2610(9, None).unwrap();
    s.create_extra_ym2151(10, None).unwrap();
    for (chip, id, ch) in [
        (Chip::Ym2413, 1, 0),
        (Chip::Ym2413, 2, 8),
        (Chip::Ay8910, 3, 0),
        (Chip::Ay8910, 4, 2),
    ] {
        play(&s, chip, id, tone(chip, ch));
    }
    assert!(energy(&render(&mut r, &s, 1000)) > 1.);
    for (chip, id, hz, n, ch) in [
        (Chip::Ym2413, 1, 3579545, 64, 0),
        (Chip::Ym2413, 2, 4000000, 64, 8),
        (Chip::Ay8910, 3, 1789773, 16, 0),
        (Chip::Ay8910, 4, 2000000, 16, 2),
    ] {
        let v = state(&s, chip, id);
        assert_eq!(v.clock, hz);
        assert_eq!(v.registers.len(), n);
        assert!(gate(&v, chip, ch));
    }
    play(&s, Chip::Ym2413, 1, vec![json!({"method":"reset"})]);
    play(&s, Chip::Ay8910, 3, vec![json!({"method":"reset"})]);
    render(&mut r, &s, 1);
    assert!(!gate(&state(&s, Chip::Ym2413, 1), Chip::Ym2413, 0));
    assert!(gate(&state(&s, Chip::Ym2413, 2), Chip::Ym2413, 8));
    assert!(!gate(&state(&s, Chip::Ay8910, 3), Chip::Ay8910, 0));
    assert!(gate(&state(&s, Chip::Ay8910, 4), Chip::Ay8910, 2));
    assert_eq!(r.fm_owners, [None; 6]);
    assert_eq!(s.extra_psg.counts(), (10, 10));
}
#[test]
fn extra_msx_shared_limit_pending_retirement_and_typed_identity() {
    let s = Shared::new(48000, 1);
    let mut r = Renderer::new(48000).unwrap();
    for id in 1..=16 {
        create(
            &s,
            if id % 2 == 0 {
                Chip::Ay8910
            } else {
                Chip::Ym2413
            },
            id,
            None,
        );
    }
    assert_eq!(s.extra_psg.memory_used(), 16 * extra_budget::INSTANCE_BYTES);
    assert!(s
        .create_extra_ym2413(17, None)
        .unwrap_err()
        .contains("limit"));
    assert!(s.dispose_extra_ay8910(1).unwrap_err().contains("match"));
    s.dispose_extra_ym2413(1).unwrap();
    assert!(s
        .create_extra_ay8910(17, None)
        .unwrap_err()
        .contains("limit"));
    render(&mut r, &s, 1);
    s.create_extra_ay8910(17, None).unwrap();
    assert!(s
        .create_extra_psg(1, None)
        .unwrap_err()
        .contains("increasing"));
    assert!(s
        .enqueue(1, &[command(Chip::Ym2413, 2, 0, write(0, 1))])
        .unwrap_err()
        .contains("not a YM2413"));
    assert!(s
        .enqueue(1, &[command(Chip::Ay8910, 3, 0, write(0, 1))])
        .unwrap_err()
        .contains("not an AY8910"));
}
#[test]
fn extra_msx_global_reset_is_independent_but_preserves_manually_held_sources() {
    for chip in [Chip::Ym2413, Chip::Ay8910] {
        let (s, mut r) = new(chip, None);
        play(&s, chip, 1, tone(chip, 0));
        play(
            &s,
            chip,
            1,
            tone(chip, 1).into_iter().map(|v| group(v, 2)).collect(),
        );
        render(&mut r, &s, 500);
        play(&s, chip, 1, vec![group(json!({"method":"reset"}), 1)]);
        render(&mut r, &s, 1);
        assert!(!gate(&state(&s, chip, 1), chip, 1));
        audition(&s, chip, 0, 1, "on", tone(chip, 0));
        render(&mut r, &s, 500);
        play(
            &s,
            chip,
            1,
            vec![
                json!({"method":"reset"}),
                json!({"method":"all-notes-off"}),
                json!({"method":"stop-voices"}),
            ],
        );
        render(&mut r, &s, 1);
        let v = state(&s, chip, 1);
        assert!(gate(&v, chip, 0));
        assert_eq!(v.held, [0]);
    }
}
#[test]
fn extra_msx_main_group_replacement_and_stale_off_do_not_steal_new_owner() {
    for chip in [Chip::Ym2413, Chip::Ay8910] {
        let (s, mut r) = new(chip, None);
        for id in [1, 2] {
            play(
                &s,
                chip,
                1,
                tone(chip, 0).into_iter().map(|v| group(v, id)).collect(),
            );
        }
        render(&mut r, &s, 500);
        let off = if chip == Chip::Ym2413 {
            write(0x20, 9)
        } else {
            write(8, 0)
        };
        play(
            &s,
            chip,
            1,
            vec![
                group(off, 1),
                group(json!({"method":"all-notes-off"}), 1),
                group(json!({"method":"stop-voices"}), 1),
                json!({"method":"cancel-main-owner","targetGroup":1}),
            ],
        );
        render(&mut r, &s, 1);
        assert!(gate(&state(&s, chip, 1), chip, 0));
        play(
            &s,
            chip,
            1,
            vec![json!({"method":"cancel-main-owner","targetGroup":2})],
        );
        render(&mut r, &s, 1);
        assert!(!gate(&state(&s, chip, 1), chip, 0));
    }
}
#[test]
fn extra_msx_worker_replacement_cancel_and_manual_sources_are_independent() {
    for chip in [Chip::Ym2413, Chip::Ay8910] {
        let (s, mut r) = new(chip, None);
        s.performance("start", 7, None).unwrap();
        for owner in [1, 2] {
            s.enqueue(
                1,
                &tone(chip, 0)
                    .into_iter()
                    .map(|v| owned(command(chip, 1, 0, v), owner))
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        }
        audition(&s, chip, 1, 9, "on", tone(chip, 1));
        render(&mut r, &s, 500);
        s.performance("cancel-owner", 7, Some(1)).unwrap();
        render(&mut r, &s, 1);
        assert!(gate(&state(&s, chip, 1), chip, 0));
        s.performance("cancel-owner", 7, Some(2)).unwrap();
        render(&mut r, &s, 1);
        let v = state(&s, chip, 1);
        assert!(!gate(&v, chip, 0));
        assert!(gate(&v, chip, 1));
        assert_eq!(v.held, [1]);
    }
}
#[test]
fn extra_msx_pending_preview_cancellation_and_stale_manual_off_never_resurrect() {
    for chip in [Chip::Ym2413, Chip::Ay8910] {
        let (s, mut r) = new(chip, Some(100000));
        let mut patch = tone(chip, 0);
        patch.pop();
        patch.push(json!({"method":"key-off","channel":0}));
        patch.push(json!({"method":"audition-key-on","channel":0}));
        audition(&s, chip, 0, 1, "on", patch.clone());
        r.prepare(&s);
        if chip == Chip::Ym2413 {
            assert!(!gate(&state(&s, chip, 1), chip, 0));
        }
        audition(
            &s,
            chip,
            0,
            1,
            "off",
            vec![json!({"method":"key-off","channel":0})],
        );
        render(&mut r, &s, 300);
        assert!(!gate(&state(&s, chip, 1), chip, 0));
        audition(&s, chip, 0, 2, "on", patch);
        render(&mut r, &s, 1500);
        assert!(gate(&state(&s, chip, 1), chip, 0));
        audition(
            &s,
            chip,
            0,
            1,
            "off",
            vec![json!({"method":"key-off","channel":0})],
        );
        render(&mut r, &s, 1);
        assert!(gate(&state(&s, chip, 1), chip, 0));
        audition(
            &s,
            chip,
            0,
            2,
            "off",
            vec![json!({"method":"key-off","channel":0})],
        );
        render(&mut r, &s, 300);
        let v = state(&s, chip, 1);
        assert!(!gate(&v, chip, 0));
        assert!(v.held.is_empty());
    }
}
#[test]
fn extra_msx_opll_shared_patch_edit_guards_only_manual_user_voices() {
    let (s, mut r) = new(Chip::Ym2413, None);
    let mut patch = tone(Chip::Ym2413, 0);
    patch[0] = write(0x30, 0);
    audition(&s, Chip::Ym2413, 0, 1, "on", patch);
    render(&mut r, &s, 1);
    play(&s, Chip::Ym2413, 1, vec![write(0, 0x71), write(7, 0x22)]);
    render(&mut r, &s, 1);
    assert_eq!(state(&s, Chip::Ym2413, 1).registers[0], 0);
    audition(
        &s,
        Chip::Ym2413,
        0,
        1,
        "edit",
        vec![write(0, 0x71), write(7, 0x22)],
    );
    render(&mut r, &s, 1);
    assert_eq!(state(&s, Chip::Ym2413, 1).registers[0], 0x71);
    audition(&s, Chip::Ym2413, 0, 1, "edit", vec![write(0x30, 0x10)]);
    play(&s, Chip::Ym2413, 1, vec![write(0, 0x44)]);
    render(&mut r, &s, 1);
    assert_eq!(state(&s, Chip::Ym2413, 1).registers[0], 0x44);
}
#[test]
fn extra_msx_opll_rhythm_same_bit_intent_replaces_only_requested_owner() {
    let (s, mut r) = new(Chip::Ym2413, None);
    play(
        &s,
        Chip::Ym2413,
        1,
        vec![group(write(14, 0x30), 1), group(write(14, 0x38), 2)],
    );
    render(&mut r, &s, 1);
    let mut replace = group(write(14, 0x38), 3);
    replace["intentMask"] = json!(16);
    play(&s, Chip::Ym2413, 1, vec![replace]);
    render(&mut r, &s, 1);
    play(
        &s,
        Chip::Ym2413,
        1,
        vec![json!({"method":"cancel-main-owner","targetGroup":1})],
    );
    render(&mut r, &s, 1);
    assert_eq!(state(&s, Chip::Ym2413, 1).registers[14] & 31, 24);
    play(
        &s,
        Chip::Ym2413,
        1,
        vec![json!({"method":"cancel-main-owner","targetGroup":3})],
    );
    render(&mut r, &s, 1);
    assert_eq!(state(&s, Chip::Ym2413, 1).registers[14] & 31, 8);
    play(
        &s,
        Chip::Ym2413,
        1,
        vec![json!({"method":"cancel-main-owner","targetGroup":2})],
    );
    render(&mut r, &s, 1);
    assert_eq!(state(&s, Chip::Ym2413, 1).registers[14] & 31, 0);
}
#[test]
fn extra_msx_opll_raw_preserved_rhythm_bit_does_not_transfer_lease() {
    let (s, mut r) = new(Chip::Ym2413, None);
    play(
        &s,
        Chip::Ym2413,
        1,
        vec![group(write(14, 0x30), 1), group(write(14, 0x30), 2)],
    );
    render(&mut r, &s, 1);
    play(
        &s,
        Chip::Ym2413,
        1,
        vec![json!({"method":"cancel-main-owner","targetGroup":2})],
    );
    render(&mut r, &s, 1);
    assert_eq!(state(&s, Chip::Ym2413, 1).registers[14] & 16, 16);
    play(
        &s,
        Chip::Ym2413,
        1,
        vec![json!({"method":"cancel-main-owner","targetGroup":1})],
    );
    render(&mut r, &s, 1);
    assert_eq!(state(&s, Chip::Ym2413, 1).registers[14] & 16, 0);
}
#[test]
fn extra_msx_opll_manual_drum_aliases_keep_independent_nibbles_and_pending_gates() {
    let (s, mut r) = new(Chip::Ym2413, None);
    play(&s, Chip::Ym2413, 1, vec![write(14, 32), write(0x37, 0x25)]);
    render(&mut r, &s, 1);
    audition(
        &s,
        Chip::Ym2413,
        10,
        1,
        "on",
        vec![json!({"method":"audition-key-on","channel":10})],
    );
    audition(
        &s,
        Chip::Ym2413,
        13,
        2,
        "on",
        vec![json!({"method":"audition-key-on","channel":13})],
    );
    render(&mut r, &s, 1000);
    assert_eq!(state(&s, Chip::Ym2413, 1).held, [10, 13]);
    play(
        &s,
        Chip::Ym2413,
        1,
        vec![
            write(14, 0),
            write(0x37, 0xff),
            write(0x17, 1),
            write(0x27, 0),
        ],
    );
    render(&mut r, &s, 1);
    let v = state(&s, Chip::Ym2413, 1);
    assert_eq!(v.registers[14] & 41, 41);
    assert_eq!(v.registers[0x37], 0x25);
    assert_eq!(v.registers[0x17], 0);
    audition(
        &s,
        Chip::Ym2413,
        10,
        1,
        "off",
        vec![json!({"method":"key-off","channel":10})],
    );
    render(&mut r, &s, 1000);
    let v = state(&s, Chip::Ym2413, 1);
    assert_eq!(v.held, [13]);
    assert_eq!(v.registers[14] & 9, 1);
    assert_eq!(v.registers[0x37] & 0xf0, 0x20);
    audition(&s, Chip::Ym2413, 13, 2, "edit", vec![write(0x37, 0x65)]);
    render(&mut r, &s, 1);
    assert_eq!(state(&s, Chip::Ym2413, 1).registers[0x37], 0x65);
}
#[test]
fn extra_msx_opll_fm_drum_alias_mode_and_inverse_cancel_protect_manual_fm() {
    let (s, mut r) = new(Chip::Ym2413, None);
    audition(&s, Chip::Ym2413, 6, 1, "on", tone(Chip::Ym2413, 6));
    render(&mut r, &s, 1000);
    play(
        &s,
        Chip::Ym2413,
        1,
        vec![group(write(14, 0x30), 2), write(0x16, 1)],
    );
    render(&mut r, &s, 1);
    let v = state(&s, Chip::Ym2413, 1);
    assert_eq!(v.registers[14] & 32, 0);
    assert_eq!(v.registers[0x16], 0x80);
    audition(
        &s,
        Chip::Ym2413,
        9,
        2,
        "on",
        vec![json!({"method":"audition-key-on","channel":9})],
    );
    render(&mut r, &s, 1);
    assert_eq!(state(&s, Chip::Ym2413, 1).held, [6]);
    play(
        &s,
        Chip::Ym2413,
        1,
        vec![json!({"method":"cancel-main-owner","targetGroup":2})],
    );
    let pcm = render(&mut r, &s, 1000);
    assert!(energy(&pcm) > 0.01);
    let v = state(&s, Chip::Ym2413, 1);
    assert_eq!(v.held, [6]);
    assert!(gate(&v, Chip::Ym2413, 6));
}
#[test]
fn extra_msx_ay_retained_zero_and_stale_explicit_zero_are_distinct_from_keyoff() {
    let (s, mut r) = new(Chip::Ay8910, None);
    play(&s, Chip::Ay8910, 1, tone(Chip::Ay8910, 0));
    render(&mut r, &s, 1);
    assert_eq!(state(&s, Chip::Ay8910, 1).levels[0], 9);
    play(
        &s,
        Chip::Ay8910,
        1,
        vec![json!({"method":"key-off","channel":0})],
    );
    render(&mut r, &s, 1);
    assert_eq!(state(&s, Chip::Ay8910, 1).levels[0], 9);
    play(
        &s,
        Chip::Ay8910,
        1,
        vec![
            write(8, 0),
            json!({"method":"set-retained-level","channel":0,"level":0}),
        ],
    );
    render(&mut r, &s, 1);
    audition(
        &s,
        Chip::Ay8910,
        0,
        1,
        "on",
        vec![json!({"method":"audition-key-on","channel":0})],
    );
    render(&mut r, &s, 1);
    let v = state(&s, Chip::Ay8910, 1);
    assert_eq!(v.levels[0], 0);
    assert!(!v.gates[0]);
    audition(
        &s,
        Chip::Ay8910,
        0,
        1,
        "off",
        vec![json!({"method":"key-off","channel":0})],
    );
    play(
        &s,
        Chip::Ay8910,
        1,
        tone(Chip::Ay8910, 0)
            .into_iter()
            .map(|v| group(v, 2))
            .collect(),
    );
    play(
        &s,
        Chip::Ay8910,
        1,
        vec![
            group(write(8, 0), 1),
            group(
                json!({"method":"set-retained-level","channel":0,"level":0}),
                1,
            ),
        ],
    );
    render(&mut r, &s, 1);
    let v = state(&s, Chip::Ay8910, 1);
    assert!(v.gates[0]);
    assert_eq!(v.levels[0], 9);
}
#[test]
fn extra_msx_ay_shared_noise_envelope_and_mixer_preserve_manual_channels() {
    let (s, mut r) = new(Chip::Ay8910, None);
    play(
        &s,
        Chip::Ay8910,
        1,
        vec![write(6, 17), write(11, 21), write(12, 3), write(13, 10)],
    );
    audition(
        &s,
        Chip::Ay8910,
        0,
        1,
        "on",
        vec![write(7, 0x30), write(8, 16)],
    );
    audition(&s, Chip::Ay8910, 1, 2, "on", tone(Chip::Ay8910, 1));
    render(&mut r, &s, 1);
    let before = state(&s, Chip::Ay8910, 1);
    play(
        &s,
        Chip::Ay8910,
        1,
        vec![
            write(6, 0),
            write(11, 0),
            write(12, 0),
            write(13, 0),
            write(7, 63),
            write(8, 0),
            write(9, 0),
        ],
    );
    render(&mut r, &s, 1);
    let v = state(&s, Chip::Ay8910, 1);
    assert_eq!(v.registers[6], 17);
    assert_eq!(&v.registers[11..14], &[21, 3, 10]);
    assert_eq!(v.registers[7] & 27, before.registers[7] & 27);
    assert_eq!(v.held, [0, 1]);
    assert!(v.gates[0] && v.gates[1]);
    audition(
        &s,
        Chip::Ay8910,
        0,
        1,
        "edit",
        vec![write(6, 23), write(11, 42)],
    );
    render(&mut r, &s, 1);
    let v = state(&s, Chip::Ay8910, 1);
    assert_eq!(v.registers[6], 23);
    assert_eq!(v.registers[11], 42);
}
#[test]
fn extra_msx_future_cancellation_and_all_notes_off_queue_semantics() {
    for chip in [Chip::Ym2413, Chip::Ay8910] {
        for stop in [false, true] {
            let (s, mut r) = new(chip, None);
            s.performance("start", 7, None).unwrap();
            s.enqueue(
                1,
                &tone(chip, 0)
                    .into_iter()
                    .map(|v| owned(command(chip, 1, 1000, v), 1))
                    .collect::<Vec<_>>(),
            )
            .unwrap();
            s.enqueue(
                1,
                &[owned(
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
            assert_eq!(gate(&state(&s, chip, 1), chip, 0), !stop);
            s.enqueue(
                1,
                &tone(chip, 1)
                    .into_iter()
                    .map(|v| owned(command(chip, 1, 10000, v), 2))
                    .collect::<Vec<_>>(),
            )
            .unwrap();
            s.performance("cancel-owner", 7, Some(2)).unwrap();
            render(&mut r, &s, 12000);
            assert!(!gate(&state(&s, chip, 1), chip, 1));
            assert_eq!(s.pending.load(Ordering::Acquire), 0);
        }
    }
}
#[test]
fn extra_msx_clock_held_admission_and_callback_race_recover_per_core() {
    for chip in [Chip::Ym2413, Chip::Ay8910] {
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
        audition(&s, chip, 0, 1, "on", tone(chip, 0));
        render(&mut r, &s, 2000);
        let err = match chip {
            Chip::Ym2413 => s.extra_ym2413_status(1, Some(1)).unwrap_err(),
            Chip::Ay8910 => s.extra_ay8910_status(1, Some(1)).unwrap_err(),
        };
        assert!(err.contains("manually held additional"));
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
        assert_eq!(state(&s, chip, 1).clock, 4000000);
        assert_eq!(
            state(&s, chip, 2).clock,
            if chip == Chip::Ym2413 {
                3579545
            } else {
                1789773
            }
        );
    }
}
#[test]
fn extra_msx_cancelled_mode_and_reset_ids_complete_without_stale_layout() {
    let (s, mut r) = new(Chip::Ym2413, None);
    s.performance("start", 7, None).unwrap();
    s.enqueue(
        1,
        &[
            owned(
                command(
                    Chip::Ym2413,
                    1,
                    10000,
                    json!({"method":"write","register":14,"value":32,"id":1}),
                ),
                1,
            ),
            owned(
                command(Chip::Ym2413, 1, 10000, json!({"method":"reset","id":2})),
                1,
            ),
        ],
    )
    .unwrap();
    s.performance("cancel-owner", 7, Some(1)).unwrap();
    render(&mut r, &s, 1);
    let v = s.extra_ym2413_status(1, Some(2)).unwrap().ym2413;
    assert_eq!(v.registers[14] & 32, 0);
    assert!(v.pending_commands.is_empty());
    s.extra_ym2413_status(1, Some(1)).unwrap();
}
#[test]
fn extra_msx_atomic_validation_and_budget_preflight_leave_instances_unchanged() {
    for chip in [Chip::Ym2413, Chip::Ay8910] {
        let mut s = Shared::new(48000, 1);
        s.extra_psg = extra_psg::Shared::with_memory_limit(extra_budget::INSTANCE_BYTES + 128);
        create(&s, chip, 1, None);
        assert!(s
            .enqueue(1, &[command(chip, 1, 0, write(0, 1))])
            .unwrap_err()
            .contains("memory budget"));
        assert_eq!(s.extra_psg.memory_used(), extra_budget::INSTANCE_BYTES);
        let (s, mut r) = new(chip, None);
        let max = if chip == Chip::Ym2413 { 64 } else { 16 };
        for bad in [
            json!({"method":"write","register":max,"value":1}),
            json!({"method":"key-off","channel":14}),
            json!({"method":"batch","channel":0,"action":"edit","commands":[write(if chip==Chip::Ym2413{0x21}else{2},1)]}),
            json!({"method":"write","register":14,"value":1,"intentMask":32}),
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
fn extra_msx_pending_status_wait_releases_running_mutex_and_dispose_wakes() {
    for chip in [Chip::Ym2413, Chip::Ay8910] {
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
            &[command(chip, 1, 100000, json!({"method":"reset","id":1}))],
        )
        .unwrap();
        let pending = rack.session_shared(1).unwrap();
        let (tx, rx) = mpsc::channel();
        let waiting = std::thread::spawn(move || {
            let result = match chip {
                Chip::Ym2413 => pending.extra_ym2413_status(1, Some(1)).map(|_| ()),
                Chip::Ay8910 => pending.extra_ay8910_status(1, Some(1)).map(|_| ()),
            };
            tx.send(result).unwrap()
        });
        rack.with_session(1, |shared| match chip {
            Chip::Ym2413 => shared.dispose_extra_ym2413(1),
            Chip::Ay8910 => shared.dispose_extra_ay8910(1),
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
fn extra_msx_stop_disposes_future_cores_and_reclaims_only_after_control_collect() {
    for chip in [Chip::Ym2413, Chip::Ay8910] {
        for installed in [false, true] {
            let (s, mut r) = new(chip, None);
            s.enqueue(
                1,
                &tone(chip, 0)
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
fn extra_msx_all_twenty_two_modes_share_mixer_and_preserve_primary_clocks() {
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
        for (chip, id) in [(Chip::Ym2413, 1), (Chip::Ay8910, 2)] {
            create(&s, chip, id, None);
            play(&s, chip, id, tone(chip, 0));
        }
        assert!(energy(&render(&mut r, &s, 1000)) > 1., "{mode}");
        for (chip, id) in [(Chip::Ym2413, 1), (Chip::Ay8910, 2)] {
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
        assert_eq!(state(&s, Chip::Ym2413, 1).clock, 3579545);
        assert_eq!(state(&s, Chip::Ay8910, 2).clock, 1789773);
    }
}
#[test]
fn extra_msx_fixed_layout_and_metadata_fit_independent_core_headroom() {
    let instance = std::mem::size_of::<extra_psg::Instance>();
    let shared = std::mem::size_of::<extra_msx::Shared>();
    let renderer = std::mem::size_of::<extra_msx::Renderer>();
    eprintln!("extra-msx-layout instance={instance} shared={shared} renderer={renderer} session={} fixed={} retirementqueue={} cpp-opll=11184 cpp-ay=504",std::mem::size_of::<Shared>(),extra_budget::INSTANCE_BYTES,CAPACITY*16);
    assert!(instance + 11184 < 65536);
    assert!(shared + renderer < 65536);
    assert_eq!(extra_budget::INSTANCE_BYTES, 1024 * 1024 + 65536);
}
#[test]
fn extra_msx_inspector_edits_keep_ay_other_mixer_pairs_and_opll_sibling_nibble() {
    let (s, mut r) = new(Chip::Ay8910, None);
    audition(&s, Chip::Ay8910, 0, 1, "on", tone(Chip::Ay8910, 0));
    audition(&s, Chip::Ay8910, 1, 2, "on", tone(Chip::Ay8910, 1));
    render(&mut r, &s, 1);
    audition(&s, Chip::Ay8910, 0, 1, "edit", vec![write(7, 63)]);
    render(&mut r, &s, 1);
    let v = state(&s, Chip::Ay8910, 1);
    assert_eq!(v.registers[7] & 9, 9);
    assert_eq!(v.registers[7] & 18, 16);
    assert_eq!(v.held, [0, 1]);
    let (s, mut r) = new(Chip::Ym2413, None);
    play(&s, Chip::Ym2413, 1, vec![write(14, 32), write(0x37, 0x25)]);
    audition(
        &s,
        Chip::Ym2413,
        10,
        1,
        "on",
        vec![json!({"method":"audition-key-on","channel":10})],
    );
    audition(
        &s,
        Chip::Ym2413,
        13,
        2,
        "on",
        vec![json!({"method":"audition-key-on","channel":13})],
    );
    render(&mut r, &s, 1000);
    audition(&s, Chip::Ym2413, 10, 1, "edit", vec![write(0x37, 0xa7)]);
    render(&mut r, &s, 1);
    assert_eq!(state(&s, Chip::Ym2413, 1).registers[0x37], 0x27);
}
#[test]
fn extra_msx_worker_same_rhythm_bit_intent_replacement_preserves_new_source() {
    let (s, mut r) = new(Chip::Ym2413, None);
    s.performance("start", 7, None).unwrap();
    s.enqueue(1, &[owned(command(Chip::Ym2413, 1, 0, write(14, 0x30)), 1)])
        .unwrap();
    let mut replacement = write(14, 0x30);
    replacement["intentMask"] = json!(16);
    s.enqueue(1, &[owned(command(Chip::Ym2413, 1, 0, replacement), 2)])
        .unwrap();
    render(&mut r, &s, 1);
    s.performance("cancel-owner", 7, Some(1)).unwrap();
    render(&mut r, &s, 1);
    assert_eq!(state(&s, Chip::Ym2413, 1).registers[14] & 16, 16);
    s.performance("cancel-owner", 7, Some(2)).unwrap();
    render(&mut r, &s, 1);
    assert_eq!(state(&s, Chip::Ym2413, 1).registers[14] & 16, 0);
}
#[test]
fn extra_msx_failed_mixed_retain_rolls_back_prepared_guards_and_payload_budget() {
    let (s, mut r) = new(Chip::Ym2413, None);
    create(&s, Chip::Ay8910, 2, None);
    play(
        &s,
        Chip::Ay8910,
        2,
        vec![
            json!({"method":"batch","channel":0,"voice":1,"action":"on","audition":true,"commands":tone(Chip::Ay8910,0)}),
        ],
    );
    render(&mut r, &s, 1);
    s.extra_psg.collect_retired().unwrap();
    let baseline = s.extra_psg.memory_used();
    let error = s
        .enqueue(
            1,
            &[
                command(
                    Chip::Ym2413,
                    1,
                    0,
                    json!({"method":"write","register":0,"value":33,"id":1}),
                ),
                command(
                    Chip::Ay8910,
                    2,
                    0,
                    json!({"method":"clock","hz":4000000,"id":2}),
                ),
            ],
        )
        .unwrap_err();
    assert!(error.contains("manually held additional AY8910"));
    s.extra_psg.collect_retired().unwrap();
    assert_eq!(s.extra_psg.memory_used(), baseline);
    let v = state(&s, Chip::Ym2413, 1);
    assert_eq!(v.registers[0], 0);
    assert!(v.pending_commands.is_empty());
    assert_eq!(state(&s, Chip::Ay8910, 2).clock, 1789773);
}
#[test]
fn extra_msx_opll_applied_layout_barrier_waits_past_default_status_deadline() {
    let s = Arc::new(Shared::new(48000, 1));
    let mut r = Renderer::new(48000).unwrap();
    create(&s, Chip::Ym2413, 1, None);
    s.enqueue(
        1,
        &[command(
            Chip::Ym2413,
            1,
            12800,
            json!({"method":"write","register":14,"value":32,"id":1}),
        )],
    )
    .unwrap();
    let pending = s.clone();
    let (tx, rx) = mpsc::channel();
    let waiting =
        std::thread::spawn(move || tx.send(pending.extra_ym2413_status(1, Some(1))).unwrap());
    assert!(rx.recv_timeout(Duration::from_millis(150)).is_err());
    assert_eq!(state(&s, Chip::Ym2413, 1).registers[14] & 32, 0);
    render(&mut r, &s, 13000);
    assert_eq!(
        rx.recv_timeout(Duration::from_millis(200))
            .unwrap()
            .unwrap()
            .ym2413
            .registers[14]
            & 32,
        32
    );
    waiting.join().unwrap();
}
#[test]
fn extra_msx_layout_revision_observes_overtaking_keyboard_before_future_off() {
    let s = Arc::new(Shared::new(48000, 1));
    let mut r = Renderer::new(48000).unwrap();
    create(&s, Chip::Ym2413, 1, None);
    s.performance("start", 7, None).unwrap();
    s.enqueue(
        1,
        &[owned(
            command(
                Chip::Ym2413,
                1,
                1000,
                json!({"method":"write","register":14,"value":0,"id":1}),
            ),
            1,
        )],
    )
    .unwrap();
    let initial = state(&s, Chip::Ym2413, 1);
    assert_eq!(initial.layout_revision, Some(0));
    assert_eq!(initial.layout_pending, Some(true));
    let pending = s.clone();
    let (tx, rx) = mpsc::channel();
    let waiting = std::thread::spawn(move || {
        tx.send(pending.extra_ym2413_status_layout(1, None, Some(0)))
            .unwrap()
    });
    assert!(rx.recv_timeout(Duration::from_millis(20)).is_err());
    s.enqueue(
        1,
        &[owned(
            command(
                Chip::Ym2413,
                1,
                0,
                json!({"method":"write","register":14,"value":32,"id":2}),
            ),
            2,
        )],
    )
    .unwrap();
    render(&mut r, &s, 1);
    let first = rx
        .recv_timeout(Duration::from_millis(200))
        .unwrap()
        .unwrap()
        .ym2413;
    waiting.join().unwrap();
    assert_eq!(first.registers[14] & 32, 32);
    assert_eq!(first.layout_revision, Some(1));
    assert_eq!(first.layout_pending, Some(true));
    assert_eq!(first.pending_commands, [1]);
    let pending = s.clone();
    let (tx, rx) = mpsc::channel();
    let waiting = std::thread::spawn(move || {
        tx.send(pending.extra_ym2413_status_layout(1, None, Some(1)))
            .unwrap()
    });
    assert!(rx.recv_timeout(Duration::from_millis(20)).is_err());
    render(&mut r, &s, 1100);
    let second = rx
        .recv_timeout(Duration::from_millis(200))
        .unwrap()
        .unwrap()
        .ym2413;
    waiting.join().unwrap();
    assert_eq!(second.registers[14] & 32, 0);
    assert_eq!(second.layout_revision, Some(2));
    assert_eq!(second.layout_pending, Some(false));
}
#[test]
fn extra_msx_layout_observer_wakes_on_no_edge_progress_cancel_and_dispose() {
    for cancel in [false, true] {
        let s = Arc::new(Shared::new(48000, 1));
        let mut r = Renderer::new(48000).unwrap();
        create(&s, Chip::Ym2413, 1, None);
        s.performance("start", 7, None).unwrap();
        s.enqueue(
            1,
            &[owned(
                command(
                    Chip::Ym2413,
                    1,
                    1000,
                    json!({"method":"write","register":14,"value":0,"id":1}),
                ),
                1,
            )],
        )
        .unwrap();
        let pending = s.clone();
        let (tx, rx) = mpsc::channel();
        let waiting = std::thread::spawn(move || {
            tx.send(pending.extra_ym2413_status_layout(1, None, Some(0)))
                .unwrap()
        });
        assert!(rx.recv_timeout(Duration::from_millis(20)).is_err());
        if cancel {
            s.performance("cancel-owner", 7, Some(1)).unwrap();
            render(&mut r, &s, 1);
        } else {
            render(&mut r, &s, 1100);
        }
        let v = rx
            .recv_timeout(Duration::from_millis(200))
            .unwrap()
            .unwrap()
            .ym2413;
        waiting.join().unwrap();
        assert_eq!(v.layout_revision, Some(0));
        assert_eq!(v.layout_pending, Some(false));
        assert_eq!(v.registers[14] & 32, 0);
    }
    let rack = NativeGenesis::default();
    let s = Arc::new(Shared::new(48000, 1));
    let mut r = Renderer::new(48000).unwrap();
    let (stop, _) = mpsc::channel();
    *rack.running.lock().unwrap() = Some(Running {
        shared: s.clone(),
        stop,
        thread: None,
    });
    create(&s, Chip::Ym2413, 1, None);
    s.enqueue(
        1,
        &[command(
            Chip::Ym2413,
            1,
            100000,
            json!({"method":"write","register":14,"value":32,"id":1}),
        )],
    )
    .unwrap();
    let pending = rack.session_shared(1).unwrap();
    let (tx, rx) = mpsc::channel();
    let waiting = std::thread::spawn(move || {
        tx.send(pending.extra_ym2413_status_layout(1, None, Some(0)))
            .unwrap()
    });
    assert!(rx.recv_timeout(Duration::from_millis(20)).is_err());
    rack.with_session(1, |shared| shared.dispose_extra_ym2413(1))
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
