//! Real callback/rack Y8950 ADPCM tests; never opens an output device.
use super::*;
use serde_json::{json, Value};
fn setup(primary: bool) -> (Shared, Renderer) {
    if primary {
        let shared = Shared::new_modes(48000, 1, false, false, false, Some("msx-audio"));
        let renderer = Renderer::new_modes(48000, false, false, false, Some("msx-audio")).unwrap();
        (shared, renderer)
    } else {
        let shared = Shared::new(48000, 1);
        shared.create_extra_y8950(1, None).unwrap();
        (shared, Renderer::new(48000).unwrap())
    }
}
fn command(primary: bool, frame: u64, mut message: Value) -> Command {
    let value = if primary {
        message["chip"] = json!("y8950");
        json!({"type":"opl","frame":frame,"message":message})
    } else {
        json!({"type":"extra-y8950","instance":1,"frame":frame,"message":message})
    };
    serde_json::from_value(value).unwrap()
}
fn render(s: &Shared, r: &mut Renderer, n: usize) -> Vec<[f32; 2]> {
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
fn play(s: &Shared, primary: bool, messages: Vec<Value>) {
    s.enqueue(
        1,
        &messages
            .into_iter()
            .map(|v| command(primary, 0, v))
            .collect::<Vec<_>>(),
    )
    .unwrap();
}
fn write(register: u16, value: u8) -> Value {
    json!({"method":"write","register":register,"value":value})
}
fn memory(id: u64, address: u32, bytes: Vec<u8>) -> Value {
    json!({"method":"adpcm-memory","id":id,"address":address,"bytes":bytes})
}
fn start() -> Vec<Value> {
    [
        (8, 2),
        (9, 0),
        (10, 0),
        (11, 7),
        (12, 0),
        (16, 0x32),
        (17, 0x29),
        (18, 220),
        (7, 0xb0),
    ]
    .into_iter()
    .map(|(r, v)| write(r, v))
    .collect()
}
fn snapshot(s: &Shared, primary: bool) -> Value {
    let v = if primary {
        serde_json::to_value(s.opl.as_ref().unwrap().status(0).unwrap()).unwrap()
    } else {
        serde_json::to_value(s.extra_y8950_status(1, None, None).unwrap()).unwrap()
    };
    v["y8950"].clone()
}
fn wait(s: &Shared, primary: bool, id: u64) -> Result<(), String> {
    if primary {
        s.opl.as_ref().unwrap().wait_applied(id)
    } else {
        s.extra_y8950_status(1, Some(id), None).map(|_| ())
    }
}
fn wave() -> Vec<u8> {
    [0x77, 0x77, 0xff, 0xff]
        .into_iter()
        .cycle()
        .take(256)
        .collect()
}
fn varies(v: &[[f32; 2]]) -> bool {
    let (min, max) = v
        .iter()
        .fold((1f32, -1f32), |(lo, hi), s| (lo.min(s[0]), hi.max(s[0])));
    max - min > 0.005
}
#[test]
fn y8950_primary_and_extra_upload_adpcm_only_start_reset_and_atomic_clear() {
    for primary in [false, true] {
        let (s, mut r) = setup(primary);
        play(&s, primary, vec![memory(1, 0, wave())]);
        render(&s, &mut r, 1);
        wait(&s, primary, 1).unwrap();
        assert_eq!(snapshot(&s, primary)["adpcmMemorySize"], 256);
        assert_eq!(snapshot(&s, primary)["registers"][7], 1);
        play(&s, primary, start());
        assert!(
            varies(&render(&s, &mut r, 4096)),
            "ADPCM-only wake {primary}"
        );
        play(&s, primary, vec![json!({"method":"reset"})]);
        render(&s, &mut r, 1);
        assert_eq!(snapshot(&s, primary)["adpcmMemorySize"], 256);
        play(&s, primary, start());
        assert!(varies(&render(&s, &mut r, 4096)));
        let before = snapshot(&s, primary);
        play(&s, primary, vec![json!({"method":"adpcm-clear","id":2})]);
        let tail = render(&s, &mut r, 1024);
        wait(&s, primary, 2).unwrap();
        let after = snapshot(&s, primary);
        assert_eq!(after["adpcmMemorySize"], 0);
        assert_eq!(after["registers"][7], 1);
        assert_eq!(after["clock"], before["clock"]);
        assert!(tail.iter().all(|p| p[0].abs() + p[1].abs() < 1e-7));
    }
}
#[test]
fn y8950_memory_preserves_fm_registers_clock_and_exact_applied_fence() {
    for primary in [false, true] {
        let (s, mut r) = setup(primary);
        play(
            &s,
            primary,
            vec![
                write(0x20, 0x21),
                write(0x23, 0x21),
                write(0x40, 63),
                write(0x43, 18),
                write(0x60, 0xf0),
                write(0x63, 0xf0),
                write(0x80, 0x0f),
                write(0x83, 0x0f),
                write(0xa0, 0x44),
                write(0xb0, 0x32),
            ],
        );
        render(&s, &mut r, 1024);
        play(&s, primary, vec![json!({"method":"clock","hz":100000})]);
        render(&s, &mut r, 128);
        let before = snapshot(&s, primary);
        s.enqueue(1, &[command(primary, r.frame + 512, memory(1, 0, wave()))])
            .unwrap();
        render(&s, &mut r, 128);
        assert_eq!(snapshot(&s, primary)["adpcmMemorySize"], 0);
        let audio = render(&s, &mut r, 1024);
        wait(&s, primary, 1).unwrap();
        let after = snapshot(&s, primary);
        assert!(
            audio.iter().any(|p| p[0].abs() > 0.00001),
            "FM must survive upload {primary}: {:?}",
            audio
                .iter()
                .fold((1f32, -1f32), |(a, b), p| (a.min(p[0]), b.max(p[0])))
        );
        assert_eq!(after["clock"], before["clock"]);
        for register in [0x20, 0x23, 0x40, 0x43, 0x60, 0x63, 0x80, 0x83, 0xa0, 0xb0] {
            assert_eq!(after["registers"][register], before["registers"][register]);
        }
        // A small replacement may never shrink the previously applied extent.
        play(&s, primary, vec![memory(2, 2, vec![1, 2])]);
        render(&s, &mut r, 1);
        wait(&s, primary, 2).unwrap();
        assert_eq!(snapshot(&s, primary)["adpcmMemorySize"], 256);
    }
}
#[test]
fn y8950_rejected_transactions_leave_ram_registers_and_queue_unchanged() {
    for primary in [false, true] {
        let (s, mut r) = setup(primary);
        play(&s, primary, vec![memory(1, 0, wave())]);
        render(&s, &mut r, 1);
        wait(&s, primary, 1).unwrap();
        let before = snapshot(&s, primary);
        let pending = s.pending.load(Ordering::Acquire);
        assert!(s
            .enqueue(
                1,
                &[
                    command(primary, 0, memory(2, 2097152, vec![1])),
                    command(primary, 0, write(0x20, 15))
                ]
            )
            .is_err());
        assert_eq!(s.pending.load(Ordering::Acquire), pending);
        render(&s, &mut r, 1);
        assert_eq!(snapshot(&s, primary)["registers"], before["registers"]);
        assert_eq!(snapshot(&s, primary)["adpcmMemorySize"], 256);
        for bytes in [json!([-1]), json!([256]), json!([0.5])] {
            let mut m = memory(3, 0, vec![]);
            m["bytes"] = bytes;
            let v = if primary {
                m["chip"] = json!("y8950");
                json!({"type":"opl","frame":0,"message":m})
            } else {
                json!({"type":"extra-y8950","instance":1,"frame":0,"message":m})
            };
            assert!(serde_json::from_value::<Command>(v).is_err());
        }
    }
    for chip in [opl::Chip::Ym3526, opl::Chip::Ym3812, opl::Chip::Ymf262] {
        let mut m = memory(1, 0, vec![1]);
        m["chip"] = json!(chip.name());
        assert!(serde_json::from_value::<opl::Message>(m)
            .unwrap()
            .prepare()
            .is_err());
    }
}
#[test]
fn y8950_cancelled_future_memory_never_applies_and_reports_non_application() {
    for primary in [false, true] {
        let (s, mut r) = setup(primary);
        s.performance("start", 7, None).unwrap();
        let mut future = command(primary, 5000, memory(1, 4096, wave()));
        future.performance = Some(7);
        future.owner = Some(1);
        s.enqueue(1, &[future]).unwrap();
        s.performance("cancel-owner", 7, Some(1)).unwrap();
        render(&s, &mut r, 6000);
        assert_eq!(snapshot(&s, primary)["adpcmMemorySize"], 0);
        assert!(wait(&s, primary, 1).unwrap_err().contains("cancelled"));
    }
}
#[test]
fn y8950_owner_cancel_stops_adpcm_and_keeps_other_fm_source() {
    for primary in [false, true] {
        let (s, mut r) = setup(primary);
        play(&s, primary, vec![memory(1, 0, wave())]);
        render(&s, &mut r, 1);
        s.performance("start", 7, None).unwrap();
        let commands = start()
            .into_iter()
            .map(|v| {
                let mut c = command(primary, 0, v);
                c.performance = Some(7);
                c.owner = Some(1);
                c
            })
            .collect::<Vec<_>>();
        s.enqueue(1, &commands).unwrap();
        assert!(varies(&render(&s, &mut r, 2048)));
        s.performance("cancel-owner", 7, Some(1)).unwrap();
        let out = render(&s, &mut r, 1024);
        assert_eq!(snapshot(&s, primary)["registers"][7], 1);
        assert!(out.iter().all(|v| v[0].abs() + v[1].abs() < 1e-7));
        assert_eq!(snapshot(&s, primary)["adpcmMemorySize"], 256);
    }
}
#[test]
fn y8950_full_ram_candidate_replacements_share_aggregate_budget_and_reclaim_on_control() {
    let (s, mut r) = setup(false);
    let baseline = s.extra_psg.memory_used();
    play(&s, false, vec![memory(1, 2097151, vec![13])]);
    assert!(s.extra_psg.memory_used() > baseline + 2097152);
    render(&s, &mut r, 1);
    wait(&s, false, 1).unwrap();
    let applied = s.extra_psg.memory_used();
    assert!(applied >= baseline + 2097152);
    play(&s, false, vec![memory(2, 2097151, vec![14])]);
    assert!(s.extra_psg.memory_used() > applied + 2097152);
    render(&s, &mut r, 1);
    wait(&s, false, 2).unwrap();
    s.extra_psg.collect_retired().unwrap();
    assert_eq!(s.extra_psg.memory_used(), applied);
    play(&s, false, vec![json!({"method":"adpcm-clear","id":3})]);
    render(&s, &mut r, 1);
    wait(&s, false, 3).unwrap();
    s.extra_psg.collect_retired().unwrap();
    assert_eq!(s.extra_psg.memory_used(), baseline);
    s.dispose_extra_y8950(1).unwrap();
    render(&s, &mut r, 1);
    s.extra_psg.collect_retired().unwrap();
    assert_eq!(s.extra_psg.memory_used(), 0);
}

#[test]
fn y8950_callback_upload_replace_clear_retire_never_allocate_or_free_rust_memory() {
    for primary in [false, true] {
        let (s, mut r) = setup(primary);
        r.prepare(&s);
        let _ = r.sample(&s);
        for (id, address, size) in [(1, 0, 2097152), (2, 4095, 3), (3, 2097151, 1)] {
            play(&s, primary, vec![memory(id, address, vec![23; size])]);
            let counts = y8950_memory::allocation_probe::measure(|| {
                r.prepare(&s);
                for _ in 0..128 {
                    let _ = r.sample(&s);
                }
                r.prepare(&s);
            });
            assert_eq!(counts, (0, 0), "upload callback {primary} id {id}");
            wait(&s, primary, id).unwrap();
        }
        play(&s, primary, vec![json!({"method":"adpcm-clear","id":4})]);
        assert_eq!(
            y8950_memory::allocation_probe::measure(|| {
                r.prepare(&s);
                let _ = r.sample(&s);
                r.prepare(&s);
            }),
            (0, 0)
        );
        wait(&s, primary, 4).unwrap();
    }
}

#[test]
fn y8950_manual_adpcm_guards_memory_both_admission_and_applied_race() {
    for primary in [false, true] {
        let (s, mut r) = setup(primary);
        play(&s, primary, vec![memory(1, 0, wave())]);
        render(&s, &mut r, 1);
        wait(&s, primary, 1).unwrap();
        s.enqueue(
            1,
            &[command(primary, r.frame + 512, memory(2, 0, vec![1; 32]))],
        )
        .unwrap();
        let mut manual = start();
        manual.last_mut().unwrap()["audition"] = json!(true);
        play(&s, primary, manual);
        render(&s, &mut r, 128);
        let before = snapshot(&s, primary);
        assert!(s
            .enqueue(1, &[command(primary, 0, memory(3, 0, vec![1; 32]))])
            .unwrap_err()
            .contains("manually held"));
        render(&s, &mut r, 1024);
        assert!(wait(&s, primary, 2).unwrap_err().contains("manually held"));
        assert_eq!(
            snapshot(&s, primary)["registers"][7],
            before["registers"][7]
        );
        assert_eq!(snapshot(&s, primary)["adpcmMemorySize"], 256);
        let mut release = write(7, 1);
        release["audition"] = json!(true);
        play(&s, primary, vec![release]);
        render(&s, &mut r, 1);
        play(&s, primary, vec![memory(4, 0, vec![1; 32])]);
        render(&s, &mut r, 1);
        wait(&s, primary, 4).unwrap();
    }
}
#[test]
fn y8950_reg8_adpcm_mode_edit_preserves_manual_fm_and_existing_csm_owner() {
    let (s, mut r) = setup(false);
    s.performance("start", 7, None).unwrap();
    let mut commands = vec![write(2, 255), write(8, 128), write(4, 1)]
        .into_iter()
        .map(|m| {
            let mut c = command(false, 0, m);
            c.performance = Some(7);
            c.owner = Some(1);
            c
        })
        .collect::<Vec<_>>();
    s.enqueue(1, &commands).unwrap();
    render(&s, &mut r, 128);
    let mut safe = command(false, 0, write(8, 130));
    safe.performance = Some(7);
    safe.owner = Some(2);
    s.enqueue(1, &[safe]).unwrap();
    render(&s, &mut r, 128);
    s.performance("cancel-owner", 7, Some(2)).unwrap();
    render(&s, &mut r, 128);
    assert_eq!(snapshot(&s, false)["registers"][8], 130);
    s.performance("cancel-owner", 7, Some(1)).unwrap();
    render(&s, &mut r, 128);
    assert_eq!(
        snapshot(&s, false)["registers"][8].as_u64().unwrap() & 128,
        0
    );
    commands.clear();
}

#[test]
fn y8950_adpcm_configuration_during_manual_fm_keeps_independent_keyboard_source() {
    for primary in [false, true] {
        let (s, mut r) = setup(primary);
        let mut fm = vec![
            write(0x20, 0x21),
            write(0x23, 0x21),
            write(0x40, 63),
            write(0x43, 18),
            write(0x60, 0xf0),
            write(0x63, 0xf0),
            write(0x80, 0x0f),
            write(0x83, 0x0f),
            write(0xa0, 0x44),
            write(0xb0, 0x32),
        ];
        fm.last_mut().unwrap()["audition"] = json!(true);
        play(&s, primary, fm);
        assert!(varies(&render(&s, &mut r, 2048)));
        play(&s, primary, vec![memory(1, 0, wave())]);
        render(&s, &mut r, 1);
        wait(&s, primary, 1).unwrap();
        play(&s, primary, start());
        render(&s, &mut r, 256);
        let state = snapshot(&s, primary);
        assert_eq!(state["registers"][8], 2);
        assert_eq!(state["registers"][11], 7);
        assert_eq!(state["registers"][16], 0x32);
        assert_eq!(state["registers"][7], 0xb0);
        assert_eq!(state["registers"][0xb0], 0x32);
        play(&s, primary, vec![json!({"method":"stop-voices"})]);
        assert!(varies(&render(&s, &mut r, 2048)));
        let state = snapshot(&s, primary);
        assert_eq!(state["registers"][7], 1);
        assert_eq!(state["registers"][0xb0], 0x32);
    }
}

#[test]
fn y8950_retired_pages_are_reclaimed_before_next_large_payload_admission() {
    let mut s = Shared::new(48000, 1);
    s.extra_psg = extra_psg::Shared::with_memory_limit(4 * 1024 * 1024);
    s.create_extra_y8950(1, None).unwrap();
    let mut r = Renderer::new(48000).unwrap();
    // Both commands retire in one callback. Their old global payload guard
    // still owns the upload pages during the first per-chip collection pass.
    play(
        &s,
        false,
        vec![
            memory(1, 2097151, vec![1]),
            json!({"method":"adpcm-clear","id":2}),
        ],
    );
    render(&s, &mut r, 128);
    assert!(s.extra_psg.memory_used() > 3 * 1024 * 1024);
    // Its one-MiB payload could not be reserved unless the second control
    // collection pass releases pages after dropping those old payload guards.
    play(&s, false, vec![memory(3, 0, vec![2; 1024 * 1024])]);
    render(&s, &mut r, 1);
    wait(&s, false, 3).unwrap();
    assert_eq!(snapshot(&s, false)["adpcmMemorySize"], 1024 * 1024);
}

fn voice_batch(channel: u8, voice: u64, action: &str, commands: Vec<Value>) -> Value {
    json!({"method":"batch","channel":channel,"voice":voice,"action":action,"audition":true,"commands":commands})
}
fn manual_fm_voice() -> Vec<Value> {
    vec![
        write(0x20, 0x21),
        write(0x23, 0x21),
        write(0x40, 63),
        write(0x43, 18),
        write(0x60, 0xf0),
        write(0x63, 0xf0),
        write(0x80, 0x0f),
        write(0x83, 0x0f),
        write(0xa0, 0x44),
        write(0xb0, 0x12),
        json!({"method":"audition-key-on","channel":0}),
    ]
}
fn manual_adpcm_voice(repeat: bool) -> Vec<Value> {
    vec![
        write(7, if repeat { 16 } else { 0 }),
        json!({"method":"audition-key-on","channel":23}),
    ]
}
#[test]
fn y8950_adpcm_keyboard_channel23_has_independent_held_and_stale_release_tokens() {
    for primary in [false, true] {
        let (s, mut r) = setup(primary);
        play(&s, primary, vec![memory(1, 0, wave())]);
        render(&s, &mut r, 1);
        let mut config = start();
        config.pop();
        play(&s, primary, config);
        play(
            &s,
            primary,
            vec![voice_batch(23, 101, "on", manual_adpcm_voice(true))],
        );
        assert!(varies(&render(&s, &mut r, 1024)));
        let a = snapshot(&s, primary);
        assert_eq!(a["held"], json!([23]));
        assert_eq!(a["registers"][7], 0xb0);
        play(
            &s,
            primary,
            vec![voice_batch(0, 201, "on", manual_fm_voice())],
        );
        render(&s, &mut r, 1024);
        assert_eq!(snapshot(&s, primary)["held"], json!([0, 23]));
        // Script Stop must retain both independent keyboard sources.
        play(
            &s,
            primary,
            vec![
                json!({"method":"stop-voices"}),
                json!({"method":"key-off","channel":23}),
            ],
        );
        render(&s, &mut r, 128);
        assert_eq!(snapshot(&s, primary)["held"], json!([0, 23]));
        assert_eq!(snapshot(&s, primary)["registers"][7], 0xb0);
        play(
            &s,
            primary,
            vec![
                voice_batch(23, 102, "on", manual_adpcm_voice(false)),
                voice_batch(
                    23,
                    101,
                    "off",
                    vec![json!({"method":"key-off","channel":23})],
                ),
                voice_batch(23, 101, "edit", vec![write(18, 1)]),
            ],
        );
        render(&s, &mut r, 128);
        let state = snapshot(&s, primary);
        assert_eq!(state["held"], json!([0, 23]));
        assert_eq!(state["registers"][7], 0xa0);
        assert_eq!(state["registers"][18], 220);
        play(
            &s,
            primary,
            vec![voice_batch(
                23,
                102,
                "edit",
                vec![write(18, 64), write(8, 0xc2)],
            )],
        );
        render(&s, &mut r, 128);
        let state = snapshot(&s, primary);
        assert_eq!(state["registers"][8], 2);
        assert_eq!(state["registers"][18], 64);
        assert_eq!(state["registers"][0xb0], 0x32);
        play(
            &s,
            primary,
            vec![voice_batch(
                0,
                201,
                "off",
                vec![json!({"method":"key-off","channel":0})],
            )],
        );
        render(&s, &mut r, 128);
        assert_eq!(snapshot(&s, primary)["held"], json!([23]));
        assert_eq!(snapshot(&s, primary)["registers"][7], 0xa0);
        play(
            &s,
            primary,
            vec![voice_batch(
                23,
                102,
                "off",
                vec![json!({"method":"key-off","channel":23})],
            )],
        );
        render(&s, &mut r, 128);
        let state = snapshot(&s, primary);
        assert_eq!(state["held"], json!([]));
        assert_eq!(state["registers"][7], 1);
    }
}
#[test]
fn y8950_adpcm_keyboard_does_not_change_csm_manual_fm_scope() {
    let (s, mut r) = setup(false);
    play(&s, false, vec![memory(1, 0, wave())]);
    render(&s, &mut r, 1);
    let mut config = start();
    config.pop();
    play(&s, false, config);
    let mut csm = vec![write(2, 255), write(8, 0x82), write(4, 1)];
    for m in &mut csm {
        m["audition"] = json!(true);
    }
    play(&s, false, csm);
    render(&s, &mut r, 128);
    assert_eq!(
        snapshot(&s, false)["held"],
        json!([0, 1, 2, 3, 4, 5, 6, 7, 8])
    );
    play(
        &s,
        false,
        vec![
            voice_batch(23, 1, "on", manual_adpcm_voice(true)),
            voice_batch(23, 1, "edit", vec![write(8, 2)]),
        ],
    );
    render(&s, &mut r, 128);
    let state = snapshot(&s, false);
    assert_eq!(state["held"], json!([0, 1, 2, 3, 4, 5, 6, 7, 8, 23]));
    assert_eq!(state["registers"][8], 0x82);
    play(
        &s,
        false,
        vec![voice_batch(
            23,
            1,
            "off",
            vec![json!({"method":"key-off","channel":23})],
        )],
    );
    render(&s, &mut r, 128);
    assert_eq!(
        snapshot(&s, false)["held"],
        json!([0, 1, 2, 3, 4, 5, 6, 7, 8])
    );
    assert_eq!(snapshot(&s, false)["registers"][8], 0x82);
    assert_eq!(snapshot(&s, false)["registers"][7], 0x11);
}
#[test]
fn y8950_adpcm_keyboard_release_and_stop_preserve_independent_fm_without_callback_allocations() {
    for primary in [false, true] {
        let (s, mut r) = setup(primary);
        play(&s, primary, vec![memory(1, 0, wave())]);
        render(&s, &mut r, 1);
        let mut config = start();
        config.pop();
        play(&s, primary, config);
        for message in [
            voice_batch(0, 201, "on", manual_fm_voice()),
            voice_batch(23, 101, "on", manual_adpcm_voice(true)),
            voice_batch(23, 101, "edit", vec![write(18, 64)]),
            voice_batch(
                23,
                101,
                "off",
                vec![json!({"method":"key-off","channel":23})],
            ),
        ] {
            play(&s, primary, vec![message]);
            assert_eq!(
                y8950_memory::allocation_probe::measure(|| {
                    r.prepare(&s);
                    for _ in 0..128 {
                        let _ = r.sample(&s);
                    }
                    r.prepare(&s);
                }),
                (0, 0),
                "manual callback primary={primary}"
            );
        }
        let state = snapshot(&s, primary);
        assert_eq!(state["held"], json!([0]));
        assert_eq!(state["registers"][7], 0x11);
        assert_eq!(state["registers"][0xb0], 0x32);
        play(
            &s,
            primary,
            vec![voice_batch(
                23,
                102,
                "on",
                vec![json!({"method":"audition-key-on","channel":23})],
            )],
        );
        render(&s, &mut r, 128);
        assert_eq!(snapshot(&s, primary)["held"], json!([0, 23]));
        assert_eq!(snapshot(&s, primary)["registers"][7], 0xb0);
        // An explicit manual Stop has stronger scope than script Stop and
        // must clear both leases while keeping the sample RAM reusable.
        play(
            &s,
            primary,
            vec![json!({"method":"stop-voices","audition":true})],
        );
        render(&s, &mut r, 128);
        let state = snapshot(&s, primary);
        assert_eq!(state["held"], json!([]));
        assert_eq!(state["registers"][7], 1);
        assert_eq!(state["registers"][0xb0].as_u64().unwrap() & 32, 0);
        assert_eq!(state["adpcmMemorySize"], 256);
    }
}
#[test]
fn y8950_adpcm_keyboard_reg8_protects_fm_and_adpcm_bits_independently() {
    for primary in [false, true] {
        let (s, mut r) = setup(primary);
        play(&s, primary, vec![memory(1, 0, wave())]);
        render(&s, &mut r, 1);
        let mut config = start();
        config.pop();
        play(&s, primary, config);
        play(
            &s,
            primary,
            vec![voice_batch(23, 101, "on", manual_adpcm_voice(true))],
        );
        render(&s, &mut r, 128);
        // A script may edit FM note-select while ADPCM owns memory mode.
        play(&s, primary, vec![write(8, 0x40)]);
        render(&s, &mut r, 128);
        assert_eq!(snapshot(&s, primary)["registers"][8], 0x42);
        play(
            &s,
            primary,
            vec![voice_batch(0, 201, "on", manual_fm_voice())],
        );
        render(&s, &mut r, 128);
        play(&s, primary, vec![write(8, 5)]);
        render(&s, &mut r, 128);
        assert_eq!(snapshot(&s, primary)["registers"][8], 0x42);
        play(
            &s,
            primary,
            vec![voice_batch(
                23,
                101,
                "off",
                vec![json!({"method":"key-off","channel":23})],
            )],
        );
        render(&s, &mut r, 128);
        play(&s, primary, vec![write(8, 5)]);
        render(&s, &mut r, 128);
        let state = snapshot(&s, primary);
        assert_eq!(state["registers"][8], 0x45);
        assert_eq!(state["held"], json!([0]));
        assert_eq!(state["registers"][0xb0], 0x32);
    }
}
#[test]
fn y8950_adpcm_keyboard_rejects_foreign_chip_and_cross_source_batches() {
    for chip in [opl::Chip::Ym3526, opl::Chip::Ym3812, opl::Chip::Ymf262] {
        let mut v = voice_batch(23, 1, "on", manual_adpcm_voice(false));
        v["chip"] = json!(chip.name());
        assert!(serde_json::from_value::<opl::Message>(v)
            .unwrap()
            .prepare()
            .is_err());
    }
    for primary in [false, true] {
        let (s, mut r) = setup(primary);
        for commands in [
            vec![write(0xb0, 0x32)],
            vec![json!({"method":"key-off","channel":0})],
            vec![json!({"method":"clock","hz":8000000})],
        ] {
            assert!(s
                .enqueue(
                    1,
                    &[command(primary, 0, voice_batch(23, 1, "on", commands))]
                )
                .is_err());
        }
        render(&s, &mut r, 128);
        assert_eq!(snapshot(&s, primary)["held"], json!([]));
    }
}
#[test]
fn y8950_memory_revisions_distinguish_resource_commit_cpu_write_and_reset() {
    for primary in [false, true] {
        let (s, mut r) = setup(primary);
        let initial = snapshot(&s, primary);
        assert_eq!(initial["adpcmMemoryRevision"], 0);
        assert_eq!(initial["adpcmMemoryUploadRevision"], 0);
        play(&s, primary, vec![memory(1, 0, vec![0; 256])]);
        render(&s, &mut r, 1);
        let loaded = snapshot(&s, primary);
        assert_eq!(loaded["adpcmMemoryRevision"], 1);
        assert_eq!(loaded["adpcmMemoryUploadRevision"], 1);
        play(
            &s,
            primary,
            vec![
                write(8, 2),
                write(9, 0),
                write(10, 0),
                write(11, 7),
                write(12, 0),
                write(7, 0x60),
            ],
        );
        let data = (0..32).map(|i| write(15, i * 7)).collect();
        play(&s, primary, data);
        render(&s, &mut r, 128);
        let raw = snapshot(&s, primary);
        assert!(raw["adpcmMemoryRevision"].as_u64().unwrap() > 1);
        assert_eq!(raw["adpcmMemoryUploadRevision"], 1);
        play(&s, primary, vec![json!({"method":"reset"})]);
        render(&s, &mut r, 128);
        let reset = snapshot(&s, primary);
        assert_eq!(reset["adpcmMemoryRevision"], raw["adpcmMemoryRevision"]);
        assert_eq!(reset["adpcmMemoryUploadRevision"], 1);
        play(&s, primary, vec![json!({"method":"adpcm-clear","id":2})]);
        render(&s, &mut r, 128);
        let cleared = snapshot(&s, primary);
        assert_eq!(
            cleared["adpcmMemoryRevision"],
            cleared["adpcmMemoryUploadRevision"]
        );
        assert!(
            cleared["adpcmMemoryRevision"].as_u64().unwrap()
                > raw["adpcmMemoryRevision"].as_u64().unwrap()
        );
    }
}
