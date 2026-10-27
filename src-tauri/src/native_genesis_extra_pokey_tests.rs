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

fn compact_trace() -> Value {
    json!({"version":1,"clock":1789790,"durationSamples":441,
        "data":[0,15,3,0,8,0,0,0,72,0,1,168,0,9,0,100,0,40,100,1,160,50,1,168]})
}
fn timeline_message(value: Value) -> Arc<extra_pokey::Prepared> {
    serde_json::from_value::<extra_pokey::Message>(value)
        .unwrap()
        .prepare()
        .unwrap()
}
#[test]
fn extra_pokey_timeline_native_pcm_matches_exact_register_schedule_at_both_rates() {
    let mut captures = Vec::new();
    for rate in [44100, 48000] {
        let shared = extra_pokey::Shared::default();
        let mut core = extra_pokey::Renderer::new_with_clock(rate, 1789790).unwrap();
        let mut reference = extra_pokey::Renderer::new_with_clock(rate, 1789790).unwrap();
        let reference_shared = extra_pokey::Shared::default();
        core.set_frame(100);
        core.apply(
            &timeline_message(json!({"method":"prepare-timeline","trace":compact_trace()})),
            None,
            &shared,
        );
        core.apply(
            &timeline_message(json!({"method":"play-timeline","token":1,"startFrame":356})),
            None,
            &shared,
        );
        let schedule = [
            (0, 15, 3),
            (0, 8, 0),
            (0, 0, 72),
            (0, 1, 168),
            (0, 9, 0),
            (100, 0, 40),
            (200, 1, 160),
            (250, 1, 168),
        ];
        let output = |sample: u64| (sample * u64::from(rate)).div_ceil(44100);
        let end = output(441);
        let mut pcm = Vec::new();
        for frame in 100..356 + end + 256 {
            let mut actual = [0.; 2];
            core.mix_additional(&mut actual, &shared);
            let mut expected = [0.; 2];
            if frame >= 356 {
                let position = frame - 356;
                for (sample, r, v) in schedule {
                    if output(sample) == position {
                        reference.apply(&timeline_message(write(r, v)), None, &reference_shared);
                    }
                }
                if position == end {
                    reference.apply(
                        &timeline_message(json!({"method":"stop-voices"})),
                        None,
                        &reference_shared,
                    );
                }
                reference.mix(&mut expected);
            }
            assert_eq!(actual, expected, "rate {rate} frame {frame}");
            pcm.push(actual[0]);
        }
        let status = shared.status(356 + end + 256).unwrap();
        assert!(status.pokey.timeline.ended);
        assert!(!status.pokey.timeline.playing);
        captures.push(json!({"sampleRate":rate,"initialFrame":100,"startFrame":356,"trace":compact_trace(),"pcm":pcm}));
    }
    if let Some(path) = std::env::var_os("TETORICA_POKEY_TIMELINE_CAPTURE") {
        std::fs::write(path, serde_json::to_vec(&captures).unwrap()).unwrap();
    }
}
#[test]
fn extra_pokey_timeline_dual_absolute_start_is_aligned_and_both_end_silent() {
    let (s, mut r) = setup();
    s.create_extra_pokey(1, None).unwrap();
    s.create_extra_pokey(2, None).unwrap();
    send(&s, 1, vec![json!({"method":"mix","volume":1.,"pan":-1.})]);
    send(&s, 2, vec![json!({"method":"mix","volume":1.,"pan":1.})]);
    render(&mut r, &s, 256);
    for id in [1, 2] {
        send(
            &s,
            id,
            vec![
                json!({"method":"prepare-timeline","trace":compact_trace()}),
                json!({"method":"play-timeline","token":id,"startFrame":768}),
            ],
        );
    }
    let pcm = render(&mut r, &s, 1400);
    assert!(energy(&pcm) > 1.);
    for (i, p) in pcm.iter().enumerate() {
        assert_eq!(p[0], p[1], "dual frame {i}");
    }
    assert!(pcm[..512].iter().all(|p| *p == [0.; 2]));
    assert!(pcm[1200..].iter().all(|p| *p == [0.; 2]));
    for id in [1, 2] {
        assert!(state(&s, id).timeline.ended);
    }
}
#[test]
fn extra_pokey_timeline_invalid_late_held_busy_stop_and_reset_are_atomic() {
    let (s, mut r) = setup();
    s.create_extra_pokey(1, None).unwrap();
    render(&mut r, &s, 256);
    for trace in [
        json!({"version":1,"clock":1789790,"durationSamples":441,"data":[128,0,1,1]}),
        json!({"version":1,"clock":2000000,"durationSamples":441,"data":[]}),
    ] {
        assert!(s
            .enqueue(
                1,
                &[command(
                    1,
                    0,
                    json!({"method":"prepare-timeline","trace":trace})
                )]
            )
            .is_err());
        assert!(!state(&s, 1).timeline.loaded);
    }
    send(
        &s,
        1,
        vec![json!({"method":"prepare-timeline","trace":compact_trace()})],
    );
    render(&mut r, &s, 128);
    assert!(state(&s, 1).timeline.loaded);
    send(
        &s,
        1,
        vec![json!({"method":"play-timeline","token":1,"startFrame":1,"id":1})],
    );
    render(&mut r, &s, 128);
    assert!(s
        .extra_pokey_status(1, Some(1))
        .unwrap_err()
        .contains("past"));
    assert!(!state(&s, 1).timeline.playing);
    send(
        &s,
        1,
        vec![json!({"method":"play-timeline","token":2,"startFrame":2048})],
    );
    render(&mut r, &s, 128);
    assert!(state(&s, 1).timeline.playing);
    assert!(s
        .enqueue(
            1,
            &[command(
                1,
                0,
                json!({"method":"prepare-timeline","trace":compact_trace()})
            )]
        )
        .unwrap_err()
        .contains("playing"));
    send(&s, 1, vec![json!({"method":"stop-timeline"})]);
    assert_eq!(energy(&render(&mut r, &s, 128)), 0.);
    assert!(state(&s, 1).timeline.cancelled);
    send(&s, 1, vec![json!({"method":"reset"})]);
    render(&mut r, &s, 128);
    assert!(!state(&s, 1).timeline.loaded);
    send(
        &s,
        1,
        vec![
            json!({"method":"batch","audition":true,"channel":0,"voice":1,"action":"on","commands":[write(1,168)]}),
        ],
    );
    render(&mut r, &s, 128);
    assert!(s
        .enqueue(
            1,
            &[command(
                1,
                0,
                json!({"method":"prepare-timeline","trace":compact_trace()})
            )]
        )
        .unwrap_err()
        .contains("manually held"));
}

#[test]
fn extra_pokey_future_timeline_admission_is_silent_and_preserves_monitor_mutes() {
    for rate in [44100, 48000] {
        let state = extra_pokey::Shared::default();
        let mut core = extra_pokey::Renderer::new_with_clock(rate, 1789790).unwrap();
        for message in tone() {
            core.apply(&timeline_message(message), None, &state);
        }
        let mut sounded = 0.;
        for _ in 0..128 {
            let mut pcm = [0.; 2];
            core.mix_additional(&mut pcm, &state);
            sounded += pcm[0].abs();
        }
        assert!(sounded > 1.);
        core.apply(
            &timeline_message(json!({"method":"channel-mute","channel":2,"muted":true})),
            None,
            &state,
        );
        core.apply(
            &timeline_message(json!({"method":"prepare-timeline","trace":compact_trace()})),
            None,
            &state,
        );
        core.apply(
            &timeline_message(json!({"method":"play-timeline","token":1,"startFrame":512})),
            None,
            &state,
        );
        for _ in 128..512 {
            let mut pcm = [0.; 2];
            core.mix_additional(&mut pcm, &state);
            assert_eq!(pcm, [0.; 2]);
        }
        assert_eq!(state.status(512).unwrap().pokey.source_mute_mask, 4);
        let mut sounded = 0.;
        for _ in 0..128 {
            let mut pcm = [0.; 2];
            core.mix_additional(&mut pcm, &state);
            sounded += pcm[0].abs();
        }
        assert!(sounded > 1.);
        assert_eq!(state.status(640).unwrap().pokey.source_mute_mask, 4);
    }
}
#[test]
fn extra_pokey_timeline_end_must_fit_the_shared_safe_frame_range_atomically() {
    for rate in [44100, 48000] {
        let state = extra_pokey::Shared::default();
        let mut core = extra_pokey::Renderer::new_with_clock(rate, 1789790).unwrap();
        for message in tone() {
            core.apply(&timeline_message(message), None, &state);
        }
        core.apply(
            &timeline_message(json!({"method":"prepare-timeline","trace":compact_trace()})),
            None,
            &state,
        );
        let frames = (441 * u64::from(rate)).div_ceil(44100);
        let end_overflow = timeline_message(
            json!({"method":"play-timeline","token":1,"startFrame":MAX_SAFE_INTEGER-frames+1,"id":1}),
        );
        state.retain(&[(end_overflow.clone(), None)]).unwrap();
        core.apply(&end_overflow, None, &state);
        assert!(state
            .wait_applied(1)
            .unwrap_err()
            .contains("invalid or late"));
        state.retire(end_overflow);
        let before = state.status(0).unwrap().pokey;
        assert!(!before.timeline.playing);
        assert_eq!(before.registers[1], 168);
        let accepted = timeline_message(
            json!({"method":"play-timeline","token":2,"startFrame":MAX_SAFE_INTEGER-frames,"id":2}),
        );
        state.retain(&[(accepted.clone(), None)]).unwrap();
        core.apply(&accepted, None, &state);
        state.wait_applied(2).unwrap();
        state.retire(accepted);
        assert!(state.status(0).unwrap().pokey.timeline.playing);
        assert_eq!(state.status(0).unwrap().pokey.registers[1] & 15, 0);
    }
}
