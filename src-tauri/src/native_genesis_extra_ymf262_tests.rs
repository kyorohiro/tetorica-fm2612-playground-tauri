//! Offline OPL3 expansion: actual decoded writes, four-op leases, and stereo.
use super::*;
use serde_json::{json, Value};
fn command(id: u64, frame: u64, message: Value) -> Command {
    serde_json::from_value(
        json!({"type":"extra-ymf262","instance":id,"frame":frame,"message":message}),
    )
    .unwrap()
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
fn state(s: &Shared, id: u64) -> extra_opl::ChipStatus {
    s.extra_ymf262_status(id, None, None)
        .unwrap()
        .ymf262
        .unwrap()
}
fn effective(v: &extra_opl::ChipStatus, reg: usize) -> u8 {
    v.effective_registers.as_ref().unwrap()[reg]
}
fn gate(v: &extra_opl::ChipStatus, ch: usize) -> bool {
    effective(v, (if ch >= 9 { 256 } else { 0 }) + 0xb0 + ch % 9) & 32 != 0
}
fn note(ch: u8, pan: u8) -> Vec<Value> {
    let bank = if ch >= 9 { 256 } else { 0 };
    let slot = bank + [0u16, 1, 2, 8, 9, 10, 16, 17, 18][(ch % 9) as usize];
    let mut v = Vec::new();
    for (op, tl) in [(slot, 63), (slot + 3, 0)] {
        for (reg, val) in [
            (0x20 + op, 0x21),
            (0x40 + op, tl),
            (0x60 + op, 0xf0),
            (0x80 + op, 0x0f),
        ] {
            v.push(write(reg, val));
        }
    }
    v.extend([
        write(bank + 0xa0 + (ch % 9) as u16, 0x98),
        write(bank + 0xc0 + (ch % 9) as u16, pan | 1),
        write(bank + 0xb0 + (ch % 9) as u16, 0x31),
    ]);
    v
}
fn four_note(ch: u8, pan: u8) -> Vec<Value> {
    let mut v = note(ch, pan);
    v.pop();
    v.extend(note(ch + 3, pan));
    v.pop();
    v.push(write(
        (if ch >= 9 { 256 } else { 0 }) + 0xb0 + (ch % 9) as u16,
        0x31,
    ));
    v
}
fn energy(v: &[[f32; 2]], ch: usize) -> f32 {
    v.iter().map(|p| p[ch].abs()).sum()
}
fn new(enabled: bool, hz: Option<u32>) -> (Shared, Renderer) {
    let s = Shared::new(48000, 1);
    let mut r = Renderer::new(48000).unwrap();
    s.create_extra_ymf262(1, hz).unwrap();
    if enabled {
        play(&s, 1, vec![write(0x105, 1)]);
        render(&mut r, &s, 1);
    }
    (s, r)
}
#[test]
fn extra_ymf262_two_each_four_profiles_and_eight_other_kinds_fill_sixteen_slots() {
    let s = Shared::new(48000, 1);
    let mut r = Renderer::new(48000).unwrap();
    s.create_extra_ymf262(1, None).unwrap();
    s.create_extra_ymf262(2, Some(16000000)).unwrap();
    play(&s, 1, vec![write(0x105, 1)]);
    play(&s, 2, vec![write(0x105, 1)]);
    play(&s, 1, note(0, 16));
    play(&s, 2, note(17, 32));
    s.create_extra_ym3526(3, None).unwrap();
    s.create_extra_ym3526(4, None).unwrap();
    s.create_extra_ym3812(5, None).unwrap();
    s.create_extra_ym3812(6, None).unwrap();
    s.create_extra_y8950(7, None).unwrap();
    s.create_extra_y8950(8, None).unwrap();
    s.create_extra_psg(9, None).unwrap();
    s.create_extra_ym2612(10, None).unwrap();
    s.create_extra_ym2203(11, None).unwrap();
    s.create_extra_ym2608(12, None).unwrap();
    s.create_extra_ym2610(13, None).unwrap();
    s.create_extra_ym2151(14, None).unwrap();
    s.create_extra_ym2413(15, None).unwrap();
    s.create_extra_ay8910(16, None).unwrap();
    let pcm = render(&mut r, &s, 1000);
    assert!(energy(&pcm, 0) > 1. && energy(&pcm, 1) > 1.);
    assert_eq!(state(&s, 1).clock, 14318180);
    assert_eq!(state(&s, 2).clock, 16000000);
    assert_eq!(state(&s, 1).registers.len(), 512);
    assert_eq!(state(&s, 1).channels, 18);
    assert!(gate(&state(&s, 1), 0));
    assert!(gate(&state(&s, 2), 17));
    assert_eq!(s.extra_psg.counts(), (16, 16));
    assert!(s
        .create_extra_ymf262(17, None)
        .unwrap_err()
        .contains("limit"));
    assert_eq!(r.fm_owners, [None; 6]);
    play(&s, 1, vec![json!({"method":"reset"})]);
    render(&mut r, &s, 1);
    assert!(!gate(&state(&s, 1), 0));
    assert!(gate(&state(&s, 2), 17));
}
#[test]
fn extra_ymf262_applied_mask_and_new_are_distinct_from_authored_registers() {
    let (s, mut r) = new(false, None);
    play(&s, 1, vec![write(0x104, 1)]);
    render(&mut r, &s, 1);
    let v = state(&s, 1);
    assert_eq!(v.registers[0x104], 1);
    assert_eq!(v.effective_four_op_mask, Some(0));
    assert_eq!(v.new_mode, Some(false));
    assert_eq!(v.layout_revision, Some(0));
    play(&s, 1, vec![write(0x105, 1)]);
    render(&mut r, &s, 1);
    let v = state(&s, 1);
    assert_eq!(v.effective_four_op_mask, Some(0));
    assert_eq!(v.new_mode, Some(true));
    assert_eq!(v.layout_revision, Some(1));
    play(
        &s,
        1,
        vec![write(0x104, 1), write(0x105, 0), write(0x104, 0)],
    );
    render(&mut r, &s, 1);
    let v = state(&s, 1);
    assert_eq!(v.registers[0x104], 0);
    assert_eq!(v.effective_four_op_mask, Some(1));
    assert_eq!(v.new_mode, Some(false));
    assert_eq!(v.layout_revision, Some(3));
}
#[test]
fn extra_ymf262_stereo_pan_and_banked_sources_are_independent() {
    let (s, mut r) = new(true, None);
    play(&s, 1, note(0, 16));
    let pcm = render(&mut r, &s, 1000);
    assert!(energy(&pcm, 0) > 1.);
    assert_eq!(energy(&pcm, 1), 0.);
    play(&s, 1, vec![write(0xc0, 32 | 1)]);
    let pcm = render(&mut r, &s, 1000);
    assert!(energy(&pcm[100..], 1) > 1.);
    assert_eq!(energy(&pcm[100..], 0), 0.);
    play(&s, 1, note(9, 16));
    render(&mut r, &s, 1000);
    play(&s, 1, vec![json!({"method":"key-off","channel":0})]);
    let pcm = render(&mut r, &s, 1000);
    assert!(energy(&pcm, 0) > 1.);
    assert_eq!(energy(&pcm[100..], 1), 0.);
}
#[test]
fn extra_ymf262_all_six_pairs_allow_primary_four_operator_preview_and_edit() {
    for (bit, ch) in [0u8, 1, 2, 9, 10, 11].into_iter().enumerate() {
        let (s, mut r) = new(true, None);
        play(&s, 1, vec![write(0x104, 1 << bit)]);
        audition(&s, ch, 1, "on", four_note(ch, 48));
        render(&mut r, &s, 1000);
        let v = state(&s, 1);
        assert_eq!(v.held, [ch]);
        assert!(gate(&v, ch as usize));
        let bank = if ch >= 9 { 256 } else { 0 };
        let slot = bank + [0u16, 1, 2, 8, 9, 10, 16, 17, 18][(ch % 9 + 3) as usize] + 3;
        audition(&s, ch, 1, "edit", vec![write(0x40 + slot, 12)]);
        render(&mut r, &s, 1);
        assert_eq!(effective(&state(&s, 1), (0x40 + slot) as usize) & 63, 12);
        audition(
            &s,
            ch,
            1,
            "off",
            vec![json!({"method":"key-off","channel":ch})],
        );
        render(&mut r, &s, 300);
        assert!(!gate(&state(&s, 1), ch as usize));
        assert!(state(&s, 1).held.is_empty());
    }
}
#[test]
fn extra_ymf262_pair_enable_batch_validates_transition_before_cross_channel_edits() {
    let (s, mut r) = new(true, None);
    let mut commands = vec![write(0xb0, 0), write(0xb3, 0), write(0x104, 1)];
    commands.extend(four_note(0, 48));
    audition(&s, 0, 1, "on", commands);
    render(&mut r, &s, 1000);
    let v = state(&s, 1);
    assert_eq!(v.effective_four_op_mask, Some(1));
    assert_eq!(v.held, [0]);
    assert!(gate(&v, 0));
    let before = effective(&v, 0x4b);
    audition(&s, 3, 2, "edit", vec![write(0x4b, 55)]);
    render(&mut r, &s, 1);
    assert_eq!(effective(&state(&s, 1), 0x4b), before);
}
#[test]
fn extra_ymf262_manual_primary_protects_all_four_operators_and_secondary_frequency() {
    let (s, mut r) = new(true, None);
    play(&s, 1, vec![write(0x104, 1)]);
    audition(&s, 0, 1, "on", four_note(0, 48));
    render(&mut r, &s, 1000);
    let before = state(&s, 1);
    play(
        &s,
        1,
        vec![
            write(0x43, 1),
            write(0x4b, 1),
            write(0xa3, 1),
            write(0xc3, 1),
            write(0x104, 0),
            write(0x105, 0),
        ],
    );
    render(&mut r, &s, 1);
    let v = state(&s, 1);
    for reg in [0x43, 0x4b, 0xa3, 0xc3] {
        assert_eq!(effective(&v, reg), effective(&before, reg));
    }
    assert_eq!(v.effective_four_op_mask, Some(1));
    assert_eq!(v.new_mode, Some(true));
    assert_eq!(v.held, [0]);
}
#[test]
fn extra_ymf262_foreign_pair_layout_bit_does_not_stop_active_owner() {
    for (bit, ch) in [0u8, 1, 2, 9, 10, 11].into_iter().enumerate() {
        let (s, mut r) = new(true, None);
        play(
            &s,
            1,
            note(ch + 3, 48).into_iter().map(|v| group(v, 2)).collect(),
        );
        play(&s, 1, vec![group(write(0x104, 1 << bit), 1)]);
        render(&mut r, &s, 1000);
        let v = state(&s, 1);
        assert_eq!(v.effective_four_op_mask, Some(0));
        assert!(gate(&v, (ch + 3) as usize));
        play(
            &s,
            1,
            vec![json!({"method":"cancel-main-owner","targetGroup":1})],
        );
        render(&mut r, &s, 1);
        assert!(gate(&state(&s, 1), (ch + 3) as usize));
    }
}
#[test]
fn extra_ymf262_secondary_latent_off_and_primary_cancel_preserve_each_other() {
    let (s, mut r) = new(true, None);
    play(&s, 1, vec![write(0x104, 1)]);
    play(
        &s,
        1,
        four_note(0, 48).into_iter().map(|v| group(v, 1)).collect(),
    );
    play(&s, 1, vec![group(write(0xb3, 0x31), 2)]);
    render(&mut r, &s, 1000);
    play(
        &s,
        1,
        vec![
            group(write(0xb3, 0x11), 3),
            json!({"method":"cancel-main-owner","targetGroup":3}),
        ],
    );
    render(&mut r, &s, 1);
    let v = state(&s, 1);
    assert!(gate(&v, 0));
    assert!(gate(&v, 3));
    play(
        &s,
        1,
        vec![json!({"method":"cancel-main-owner","targetGroup":1})],
    );
    render(&mut r, &s, 1);
    let v = state(&s, 1);
    assert!(!gate(&v, 0));
    assert!(gate(&v, 3));
    play(
        &s,
        1,
        vec![json!({"method":"cancel-main-owner","targetGroup":2})],
    );
    render(&mut r, &s, 1);
    assert!(!gate(&state(&s, 1), 3));
}
#[test]
fn extra_ymf262_stale_owner_after_pair_disconnect_cannot_stop_new_secondary() {
    let (s, mut r) = new(true, None);
    play(&s, 1, vec![group(write(0x104, 1), 1)]);
    play(
        &s,
        1,
        four_note(0, 48).into_iter().map(|v| group(v, 1)).collect(),
    );
    render(&mut r, &s, 1000);
    play(&s, 1, vec![group(write(0x104, 0), 1)]);
    play(
        &s,
        1,
        note(3, 48).into_iter().map(|v| group(v, 2)).collect(),
    );
    render(&mut r, &s, 1000);
    play(
        &s,
        1,
        vec![
            group(json!({"method":"key-off","channel":0}), 1),
            json!({"method":"cancel-main-owner","targetGroup":1}),
        ],
    );
    render(&mut r, &s, 1);
    assert!(gate(&state(&s, 1), 3));
}
#[test]
fn extra_ymf262_new_zero_banked_write_owner_uses_actual_lower_source() {
    let (s, mut r) = new(false, None);
    play(
        &s,
        1,
        note(0, 48).into_iter().map(|v| group(v, 1)).collect(),
    );
    play(&s, 1, vec![group(write(0x1b0, 0x31), 2)]);
    render(&mut r, &s, 1000);
    assert!(gate(&state(&s, 1), 0));
    play(
        &s,
        1,
        vec![
            group(write(0xb0, 0x11), 1),
            json!({"method":"cancel-main-owner","targetGroup":1}),
        ],
    );
    render(&mut r, &s, 1);
    assert!(gate(&state(&s, 1), 0));
    play(&s, 1, vec![write(0x105, 1)]);
    play(
        &s,
        1,
        note(9, 32).into_iter().map(|v| group(v, 3)).collect(),
    );
    render(&mut r, &s, 1000);
    play(
        &s,
        1,
        vec![json!({"method":"cancel-main-owner","targetGroup":2})],
    );
    render(&mut r, &s, 1);
    let v = state(&s, 1);
    assert!(!gate(&v, 0));
    assert!(gate(&v, 9));
}
#[test]
fn extra_ymf262_new_zero_upper_owner_cancel_does_not_fold_into_lower_new_voice() {
    let (s, mut r) = new(true, None);
    play(
        &s,
        1,
        note(9, 32).into_iter().map(|v| group(v, 1)).collect(),
    );
    play(&s, 1, vec![write(0x105, 0)]);
    play(
        &s,
        1,
        note(0, 48).into_iter().map(|v| group(v, 2)).collect(),
    );
    render(&mut r, &s, 1000);
    play(
        &s,
        1,
        vec![json!({"method":"cancel-main-owner","targetGroup":1})],
    );
    render(&mut r, &s, 1);
    let v = state(&s, 1);
    assert!(!gate(&v, 9));
    assert!(gate(&v, 0));
}
#[test]
fn extra_ymf262_new_zero_manual_lower_guard_and_upper_preview_rejection() {
    let (s, mut r) = new(false, None);
    audition(&s, 0, 1, "on", note(0, 48));
    render(&mut r, &s, 1000);
    let before = state(&s, 1);
    play(
        &s,
        1,
        vec![write(0x1a0, 1), write(0x1b0, 0), write(0x140, 1)],
    );
    render(&mut r, &s, 1);
    let v = state(&s, 1);
    for reg in [0xa0, 0xb0, 0x40] {
        assert_eq!(effective(&v, reg), effective(&before, reg));
    }
    audition(&s, 9, 2, "on", note(9, 48));
    render(&mut r, &s, 1);
    assert_eq!(state(&s, 1).held, [0]);
    assert!(!gate(&state(&s, 1), 9));
}
#[test]
fn extra_ymf262_upper_bank_operators_do_not_alias_lower_drums() {
    let (s, mut r) = new(true, None);
    play(&s, 1, vec![write(0xbd, 32)]);
    audition(
        &s,
        19,
        1,
        "on",
        vec![json!({"method":"audition-key-on","channel":19})],
    );
    audition(&s, 16, 2, "on", note(16, 48));
    render(&mut r, &s, 1000);
    play(&s, 1, vec![write(0x151, 12)]);
    render(&mut r, &s, 1);
    let v = state(&s, 1);
    assert_eq!(v.held, [16, 19]);
    assert_ne!(effective(&v, 0x151), 12);
    assert_eq!(effective(&v, 0xbd) & 8, 8);
}
#[test]
fn extra_ymf262_raw_csm_bits_never_claim_a_source_or_global_manual_hold() {
    let (s, mut r) = new(true, None);
    play(&s, 1, vec![group(write(8, 128), 1), group(write(4, 1), 1)]);
    render(&mut r, &s, 1);
    assert!(state(&s, 1).held.is_empty());
    play(
        &s,
        1,
        note(9, 48).into_iter().map(|v| group(v, 2)).collect(),
    );
    play(
        &s,
        1,
        vec![json!({"method":"cancel-main-owner","targetGroup":1})],
    );
    render(&mut r, &s, 1000);
    let v = state(&s, 1);
    assert!(gate(&v, 9));
    assert_eq!(v.registers[8], 128);
    assert_eq!(v.registers[4] & 1, 1);
}
#[test]
fn extra_ymf262_pending_preview_pair_cancel_and_clock_held_race() {
    let (s, mut r) = new(true, Some(100000));
    play(&s, 1, vec![write(0x104, 1)]);
    s.enqueue(
        1,
        &[command(
            1,
            1000,
            json!({"method":"clock","hz":16000000,"id":1}),
        )],
    )
    .unwrap();
    let mut patch = four_note(0, 48);
    patch.pop();
    patch.push(json!({"method":"key-off","channel":0}));
    patch.push(json!({"method":"audition-key-on","channel":0}));
    audition(&s, 0, 1, "on", patch.clone());
    r.prepare(&s);
    assert!(!gate(&state(&s, 1), 0));
    audition(
        &s,
        0,
        1,
        "off",
        vec![json!({"method":"key-off","channel":0})],
    );
    render(&mut r, &s, 300);
    assert!(!gate(&state(&s, 1), 0));
    audition(&s, 0, 2, "on", patch);
    render(&mut r, &s, 1500);
    assert!(s
        .extra_ymf262_status(1, Some(1), None)
        .unwrap_err()
        .contains("manually held additional YMF262"));
    audition(
        &s,
        0,
        1,
        "off",
        vec![json!({"method":"key-off","channel":0})],
    );
    render(&mut r, &s, 1);
    assert!(gate(&state(&s, 1), 0));
}
#[test]
fn extra_ymf262_layout_observer_tracks_overtaking_new_pair_and_folded_rhythm() {
    let s = Arc::new(Shared::new(48000, 1));
    let mut r = Renderer::new(48000).unwrap();
    s.create_extra_ymf262(1, None).unwrap();
    s.performance("start", 7, None).unwrap();
    s.enqueue(
        1,
        &[own(
            command(
                1,
                1000,
                json!({"method":"write","register":0x105,"value":0,"id":1}),
            ),
            1,
        )],
    )
    .unwrap();
    let pending = s.clone();
    let (tx, rx) = mpsc::channel();
    let waiting = std::thread::spawn(move || {
        tx.send(pending.extra_ymf262_status(1, None, Some(0)))
            .unwrap()
    });
    assert!(rx.recv_timeout(Duration::from_millis(20)).is_err());
    play(&s, 1, vec![write(0x105, 1)]);
    render(&mut r, &s, 1);
    let v = rx
        .recv_timeout(Duration::from_millis(200))
        .unwrap()
        .unwrap()
        .ymf262
        .unwrap();
    waiting.join().unwrap();
    assert_eq!(v.new_mode, Some(true));
    assert_eq!(v.layout_revision, Some(1));
    assert_eq!(v.layout_pending, Some(true));
    render(&mut r, &s, 1100);
    let v = state(&s, 1);
    assert_eq!(v.new_mode, Some(false));
    assert_eq!(v.layout_revision, Some(2));
    play(&s, 1, vec![write(0x1bd, 32)]);
    render(&mut r, &s, 1);
    let v = state(&s, 1);
    assert_eq!(v.registers[0xbd], 0);
    assert_eq!(v.registers[0x1bd], 32);
    assert_eq!(effective(&v, 0xbd) & 32, 32);
    assert_eq!(v.layout_revision, Some(3));
}
#[test]
fn extra_ymf262_atomic_budget_identity_stop_and_dispose_lifetime() {
    let (s, mut r) = new(true, None);
    assert!(s.dispose_extra_ym3812(1).unwrap_err().contains("match"));
    for bad in [
        write(512, 1),
        json!({"method":"key-off","channel":23}),
        json!({"method":"batch","channel":0,"action":"edit","commands":[write(0x1b0,1)]}),
    ] {
        assert!(s
            .enqueue(1, &[command(1, 0, write(0, 33)), command(1, 0, bad)])
            .is_err());
    }
    assert_eq!(state(&s, 1).registers[0], 0);
    s.enqueue(
        1,
        &note(9, 48)
            .into_iter()
            .map(|v| command(1, 100000, v))
            .collect::<Vec<_>>(),
    )
    .unwrap();
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
    assert!(s
        .create_extra_ymf262(1, None)
        .unwrap_err()
        .contains("increasing"));
    s.create_extra_ymf262(2, None).unwrap();
    let mut s = Shared::new(48000, 1);
    s.extra_psg = extra_psg::Shared::with_memory_limit(extra_budget::INSTANCE_BYTES + 128);
    s.create_extra_ymf262(1, None).unwrap();
    assert!(s
        .enqueue(1, &[command(1, 0, write(0x105, 1))])
        .unwrap_err()
        .contains("memory budget"));
    assert_eq!(s.extra_psg.memory_used(), extra_budget::INSTANCE_BYTES);
}
#[test]
fn extra_ymf262_worker_future_cancel_and_source_replacement() {
    let (s, mut r) = new(true, None);
    s.performance("start", 7, None).unwrap();
    for owner in [1, 2] {
        s.enqueue(
            1,
            &note(9, 48)
                .into_iter()
                .map(|v| own(command(1, 0, v), owner))
                .collect::<Vec<_>>(),
        )
        .unwrap();
    }
    s.enqueue(
        1,
        &note(0, 48)
            .into_iter()
            .map(|v| own(command(1, 10000, v), 3))
            .collect::<Vec<_>>(),
    )
    .unwrap();
    s.performance("cancel-owner", 7, Some(3)).unwrap();
    render(&mut r, &s, 1000);
    s.performance("cancel-owner", 7, Some(1)).unwrap();
    render(&mut r, &s, 1);
    assert!(gate(&state(&s, 1), 9));
    s.performance("cancel-owner", 7, Some(2)).unwrap();
    render(&mut r, &s, 11000);
    assert!(!gate(&state(&s, 1), 9));
    assert!(!gate(&state(&s, 1), 0));
    assert_eq!(s.pending.load(Ordering::Acquire), 0);
}
#[test]
fn extra_ymf262_all_native_modes_share_stereo_mixer_without_primary_changes() {
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
        s.create_extra_ymf262(1, None).unwrap();
        play(&s, 1, vec![write(0x105, 1)]);
        play(&s, 1, note(9, 48));
        assert!(energy(&render(&mut r, &s, 1000), 0) > 1., "{mode}");
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
        assert_eq!(state(&s, 1).clock, 14318180);
    }
}
#[test]
fn extra_ymf262_fixed_layout_budget_headroom() {
    eprintln!(
        "extra-ymf262-layout instance={} shared={} renderer={} session={} fixed={}",
        std::mem::size_of::<extra_psg::Instance>(),
        std::mem::size_of::<extra_opl::Shared>(),
        std::mem::size_of::<extra_opl::Renderer>(),
        std::mem::size_of::<Shared>(),
        extra_budget::INSTANCE_BYTES
    );
    assert!(std::mem::size_of::<extra_psg::Instance>() < 32768);
}
#[test]
fn extra_ymf262_manual_pair_preview_preserves_foreign_latent_secondary_gate() {
    let (s, mut r) = new(true, None);
    play(&s, 1, vec![write(0x104, 1)]);
    play(
        &s,
        1,
        four_note(0, 48).into_iter().map(|v| group(v, 1)).collect(),
    );
    play(&s, 1, vec![group(write(0xb3, 0x31), 2)]);
    render(&mut r, &s, 1000);
    let mut patch = four_note(0, 48);
    patch.pop();
    patch.push(json!({"method":"key-off","channel":0}));
    patch.push(json!({"method":"audition-key-on","channel":0}));
    audition(&s, 0, 1, "on", patch);
    render(&mut r, &s, 1000);
    let v = state(&s, 1);
    assert!(gate(&v, 0));
    assert!(gate(&v, 3));
    assert_eq!(v.held, [0]);
    audition(
        &s,
        0,
        1,
        "off",
        vec![json!({"method":"key-off","channel":0})],
    );
    render(&mut r, &s, 1);
    let v = state(&s, 1);
    assert!(!gate(&v, 0));
    assert!(gate(&v, 3));
}
#[test]
fn extra_ymf262_folded_rhythm_source_preview_preserves_actual_sibling_bits() {
    let (s, mut r) = new(false, None);
    play(&s, 1, vec![group(write(0x1bd, 41), 2)]);
    render(&mut r, &s, 1000);
    let before = state(&s, 1);
    assert_eq!(before.registers[0xbd], 0);
    assert_eq!(effective(&before, 0xbd) & 41, 41);
    audition(
        &s,
        19,
        1,
        "on",
        vec![
            json!({"method":"key-off","channel":19}),
            json!({"method":"audition-key-on","channel":19}),
        ],
    );
    render(&mut r, &s, 1000);
    let v = state(&s, 1);
    assert_eq!(effective(&v, 0xbd) & 41, 41);
    assert_eq!(v.held, [19]);
    audition(
        &s,
        19,
        1,
        "off",
        vec![json!({"method":"key-off","channel":19})],
    );
    render(&mut r, &s, 1);
    assert_eq!(effective(&state(&s, 1), 0xbd) & 41, 33);
}
#[test]
fn extra_ymf262_instance_and_command_ids_do_not_collide_with_legacy_opl() {
    let (s, mut r) = new(true, None);
    s.create_extra_ym3812(2, None).unwrap();
    assert!(s
        .create_extra_ym3812(1, None)
        .unwrap_err()
        .contains("increasing"));
    s.enqueue(1,&[command(1,0,json!({"method":"clock","hz":16000000,"id":7})),serde_json::from_value(json!({"type":"extra-ym3812","instance":2,"frame":0,"message":{"method":"clock","hz":4000000,"id":7}})).unwrap()]).unwrap();
    render(&mut r, &s, 1);
    assert_eq!(
        s.extra_ymf262_status(1, Some(7), None)
            .unwrap()
            .ymf262
            .unwrap()
            .clock,
        16000000
    );
    assert_eq!(
        s.extra_ym3812_status(2, Some(7), None)
            .unwrap()
            .ym3812
            .unwrap()
            .clock,
        4000000
    );
    s.enqueue(
        1,
        &[command(
            1,
            10000,
            json!({"method":"write","register":0x104,"value":1,"id":8}),
        )],
    )
    .unwrap();
    assert!(s
        .enqueue(
            1,
            &[command(
                1,
                0,
                json!({"method":"write","register":0x104,"value":2,"id":8})
            )]
        )
        .unwrap_err()
        .contains("Duplicate pending"));
    render(&mut r, &s, 1);
    assert_eq!(state(&s, 1).effective_four_op_mask, Some(0));
}
#[test]
fn extra_ymf262_pair_disconnect_does_not_expose_cancelled_secondary_release_tail() {
    for latent in [false, true] {
        let (s, mut r) = new(true, None);
        play(&s, 1, vec![group(write(0x104, 1), 1)]);
        play(
            &s,
            1,
            four_note(0, 48).into_iter().map(|v| group(v, 1)).collect(),
        );
        if latent {
            play(&s, 1, vec![group(write(0xb3, 0x31), 1)]);
        }
        assert!(energy(&render(&mut r, &s, 1000), 0) > 0.01);
        play(&s, 1, vec![group(write(0x104, 0), 1)]);
        let pcm = render(&mut r, &s, 1000);
        assert_eq!(energy(&pcm, 0), 0.);
        assert_eq!(energy(&pcm, 1), 0.);
        let v = state(&s, 1);
        assert!(!gate(&v, 0));
        assert!(!gate(&v, 3));
    }
}
#[test]
fn extra_ymf262_typed_public_alloff_new_zero_retains_upper_cleanup_owner() {
    let (s, mut r) = new(true, None);
    play(
        &s,
        1,
        note(9, 48).into_iter().map(|v| group(v, 2)).collect(),
    );
    play(
        &s,
        1,
        vec![write(0x105, 0), json!({"method":"all-notes-off"})],
    );
    render(&mut r, &s, 1000);
    assert!(gate(&state(&s, 1), 9));
    play(
        &s,
        1,
        vec![json!({"method":"cancel-main-owner","targetGroup":2})],
    );
    render(&mut r, &s, 1);
    assert!(!gate(&state(&s, 1), 9));
}
#[test]
fn extra_ymf262_old_upper_manual_off_survives_new_zero_layout_change() {
    let (s, mut r) = new(true, None);
    audition(&s, 9, 1, "on", note(9, 48));
    render(&mut r, &s, 1000);
    let mut mode = write(0x105, 0);
    mode["audition"] = json!(true);
    play(&s, 1, vec![mode]);
    play(
        &s,
        1,
        note(0, 48).into_iter().map(|v| group(v, 2)).collect(),
    );
    render(&mut r, &s, 1000);
    assert_eq!(state(&s, 1).held, [9]);
    audition(
        &s,
        9,
        1,
        "off",
        vec![json!({"method":"key-off","channel":9})],
    );
    render(&mut r, &s, 1);
    let v = state(&s, 1);
    assert!(!gate(&v, 9));
    assert!(gate(&v, 0));
    assert!(v.held.is_empty());
}
