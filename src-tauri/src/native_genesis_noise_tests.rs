use super::*;
fn command(frame: u64, namespace: u64, slot: u8, token: u64, action: serde_json::Value) -> Command {
    let mut value =
        serde_json::json!({"op":"noise","namespace":namespace,"slot":slot,"token":token});
    value
        .as_object_mut()
        .unwrap()
        .extend(action.as_object().unwrap().clone());
    Command {
        frame,
        performance: None,
        owner: None,
        action: CommandAction::Noise {
            message: serde_json::from_value(value).unwrap(),
        },
    }
}
fn create(frame: u64, namespace: u64, slot: u8, token: u64, seed: u32) -> Command {
    command(
        frame,
        namespace,
        slot,
        token,
        serde_json::json!({"action":"create","type":0,"seed":seed}),
    )
}
fn op(frame: u64, namespace: u64, slot: u8, token: u64, action: &str) -> Command {
    command(
        frame,
        namespace,
        slot,
        token,
        serde_json::json!({"action":action}),
    )
}
fn param(frame: u64, namespace: u64, slot: u8, token: u64, param: u8, value: f64) -> Command {
    command(
        frame,
        namespace,
        slot,
        token,
        serde_json::json!({"action":"parameter","param":param,"value":value,"seconds":0}),
    )
}
fn tagged(mut command: Command, owner: u32) -> Command {
    command.performance = Some(1);
    command.owner = Some(owner);
    command
}
fn render(renderer: &mut Renderer, shared: &Shared, frames: usize) -> Vec<[f32; 2]> {
    renderer.prepare(shared);
    (0..frames).map(|_| renderer.sample(shared)).collect()
}
fn energy(pcm: &[[f32; 2]]) -> f64 {
    pcm.iter().flatten().map(|v| (*v as f64).powi(2)).sum()
}
#[test]
fn inactive_noise_leaves_chip_audio_bit_identical_and_seed_zero_uses_logical_slot() {
    let a = Shared::new(48000, 1);
    let b = Shared::new(48000, 2);
    let mut ra = Renderer::new(48000).unwrap();
    let mut rb = Renderer::new(48000).unwrap();
    for shared in [&a, &b] {
        shared
            .enqueue(
                shared.session,
                &[Command {
                    frame: 0,
                    owner: None,
                    performance: None,
                    action: CommandAction::PsgWrite { value: 0x90 },
                }],
            )
            .unwrap();
    }
    b.enqueue(2, &[create(0, 2, 0, 1, 123), param(0, 2, 0, 1, 0, 0.5)])
        .unwrap();
    assert_eq!(render(&mut ra, &a, 4096), render(&mut rb, &b, 4096));
    let left = Shared::new(48000, 3);
    let right = Shared::new(48000, 4);
    let mut l = Renderer::new(48000).unwrap();
    let mut r = Renderer::new(48000).unwrap();
    left.enqueue(3, &[create(0, 0, 7, 1, 0), op(0, 0, 7, 1, "start")])
        .unwrap();
    right
        .enqueue(
            4,
            &[
                create(0, 99, 0, 1, 100),
                create(0, 0, 7, 1, 0),
                op(0, 0, 7, 1, "start"),
            ],
        )
        .unwrap();
    assert_eq!(render(&mut l, &left, 4096), render(&mut r, &right, 4096));
}
#[test]
fn future_stop_and_restart_preserve_prepared_noise_voice() {
    let shared = Shared::new(48000, 1);
    let mut renderer = Renderer::new(48000).unwrap();
    shared
        .enqueue(
            1,
            &[
                create(0, 0, 0, 1, 123),
                op(0, 0, 0, 1, "start"),
                op(1000, 0, 0, 1, "stop"),
                op(2000, 0, 0, 1, "start"),
            ],
        )
        .unwrap();
    let pcm = render(&mut renderer, &shared, 3000);
    assert!(energy(&pcm[512..1000]) > 0.01);
    assert!(pcm[1000..2000].iter().all(|v| *v == [0.0; 2]));
    assert!(energy(&pcm[2000..]) > 0.01);
    assert_eq!(shared.noise.lock().unwrap().reserved(), 1);
    shared.enqueue(1, &[op(0, 0, 0, 1, "stop")]).unwrap();
    render(&mut renderer, &shared, 1024);
    assert_eq!(shared.noise.lock().unwrap().reserved(), 1);
    shared.enqueue(1, &[op(0, 0, 0, 1, "start")]).unwrap();
    assert!(energy(&render(&mut renderer, &shared, 1024)) > 0.01);
}
#[test]
fn prepared_noise_survives_owner_cancel_and_new_owner_ignores_old_finally() {
    let shared = Shared::new(48000, 1);
    let mut renderer = Renderer::new(48000).unwrap();
    shared.performance("start", 1, None).unwrap();
    shared
        .enqueue(
            1,
            &[
                tagged(create(0, 1, 0, 1, 123), 1),
                tagged(param(0, 1, 0, 1, 0, 0.4), 1),
                tagged(op(0, 1, 0, 1, "start"), 2),
                tagged(op(2000, 1, 0, 1, "start"), 3),
                tagged(op(2200, 1, 0, 1, "stop"), 2),
                tagged(op(2300, 1, 0, 1, "dispose"), 2),
            ],
        )
        .unwrap();
    assert!(energy(&render(&mut renderer, &shared, 512)) > 0.01);
    shared.performance("cancel-owner", 1, Some(2)).unwrap();
    let pcm = render(&mut renderer, &shared, 3000);
    assert!(pcm[..1400].iter().all(|v| *v == [0.0; 2]));
    assert!(energy(&pcm[2000..]) > 0.01);
    assert_eq!(shared.noise.lock().unwrap().reserved(), 1);
    // Owner 2 is still live here, but its delayed cleanup must not stop owner 3.
    let other = Shared::new(48000, 2);
    let mut voice = Renderer::new(48000).unwrap();
    other.performance("start", 1, None).unwrap();
    other
        .enqueue(
            2,
            &[
                tagged(create(0, 2, 0, 1, 456), 1),
                tagged(op(0, 2, 0, 1, "start"), 2),
                tagged(op(512, 2, 0, 1, "start"), 3),
                tagged(op(1024, 2, 0, 1, "stop"), 2),
                tagged(op(1152, 2, 0, 1, "retire"), 2),
                tagged(op(1280, 2, 0, 1, "dispose"), 2),
            ],
        )
        .unwrap();
    let pcm = render(&mut voice, &other, 3000);
    assert!(energy(&pcm[1500..]) > 0.01);
}
#[test]
fn namespace_tokens_are_isolated_and_a_bad_batch_never_reserves_a_voice() {
    let shared = Shared::new(48000, 1);
    let mut renderer = Renderer::new(48000).unwrap();
    let invalid = vec![create(0, 0, 0, 1, 123), param(0, 0, 0, 1, 0, 9.0)];
    assert!(shared.enqueue(1, &invalid).is_err());
    assert_eq!(shared.noise.lock().unwrap().reserved(), 0);
    assert_eq!(shared.pending.load(Ordering::Acquire), 0);
    shared
        .enqueue(
            1,
            &[
                create(0, 0, 0, 1, 123),
                param(0, 0, 0, 1, 1, -1.0),
                op(0, 0, 0, 1, "start"),
                create(0, 1, 0, 1, 456),
                param(0, 1, 0, 1, 1, 1.0),
                op(0, 1, 0, 1, "start"),
            ],
        )
        .unwrap();
    let pcm = render(&mut renderer, &shared, 1024);
    assert!(pcm[512..].iter().any(|v| v[0] != 0.0 && v[1] != 0.0));
    assert_eq!(shared.noise.lock().unwrap().reserved(), 2);
    shared.enqueue(1, &[op(0, 0, 0, 1, "dispose")]).unwrap();
    let remaining = render(&mut renderer, &shared, 512);
    assert!(remaining.iter().all(|v| v[0].abs() < 1e-7));
    assert!(energy(&remaining) > 0.01);
}
#[test]
fn future_disposal_keeps_capacity_reserved_until_the_callback_acknowledges() {
    let shared = Shared::new(48000, 1);
    let mut renderer = Renderer::new(48000).unwrap();
    let creates: Vec<_> = (0..32)
        .map(|slot| create(0, 0, slot, 1, slot as u32 + 1))
        .collect();
    shared.enqueue(1, &creates).unwrap();
    shared.enqueue(1, &[op(1000, 0, 0, 1, "dispose")]).unwrap();
    assert!(shared.enqueue(1, &[create(0, 1, 0, 1, 1)]).is_err());
    assert_eq!(shared.noise.lock().unwrap().reserved(), 32);
    render(&mut renderer, &shared, 1001);
    shared
        .enqueue(1, &[create(0, 1, 0, 1, 1), op(0, 1, 0, 1, "start")])
        .unwrap();
    assert_eq!(shared.noise.lock().unwrap().reserved(), 32);
    assert!(energy(&render(&mut renderer, &shared, 1024)) > 0.01);
}
#[test]
fn retire_keeps_the_release_tail_then_reclaims_and_old_commands_cannot_hit_reuse() {
    let shared = Shared::new(48000, 1);
    let mut renderer = Renderer::new(48000).unwrap();
    shared
        .enqueue(
            1,
            &[
                create(0, 0, 0, 1, 123),
                param(0, 0, 0, 1, 5, 0.01),
                op(0, 0, 0, 1, "start"),
                op(1000, 0, 0, 1, "retire"),
                create(3000, 0, 0, 1, 123),
                op(3100, 0, 0, 1, "start"),
                op(3200, 0, 0, 1, "dispose"),
            ],
        )
        .unwrap();
    let pcm = render(&mut renderer, &shared, 1100);
    assert!(energy(&pcm[1000..]) > 0.0);
    shared.enqueue(1, &[]).unwrap();
    assert_eq!(shared.noise.lock().unwrap().reserved(), 1);
    let tail = render(&mut renderer, &shared, 500);
    assert!(tail[380..].iter().all(|v| *v == [0.0; 2]));
    shared.enqueue(1, &[]).unwrap();
    assert_eq!(shared.noise.lock().unwrap().reserved(), 0);
    shared
        .enqueue(1, &[create(0, 0, 0, 2, 789), op(0, 0, 0, 2, "start")])
        .unwrap();
    let pcm = render(&mut renderer, &shared, 2000);
    assert!(energy(&pcm[1800..]) > 0.01);
    assert_eq!(shared.noise.lock().unwrap().reserved(), 1);
}
#[test]
fn cancelled_future_create_can_be_retired_or_started_by_a_new_owner() {
    let shared = Shared::new(48000, 1);
    let mut renderer = Renderer::new(48000).unwrap();
    shared.performance("start", 1, None).unwrap();
    shared
        .enqueue(
            1,
            &[
                tagged(create(10000, 1, 0, 1, 123), 1),
                tagged(op(10000, 1, 0, 1, "start"), 1),
            ],
        )
        .unwrap();
    shared.performance("cancel-owner", 1, Some(1)).unwrap();
    shared.enqueue(1, &[op(0, 1, 0, 1, "retire")]).unwrap();
    render(&mut renderer, &shared, 128);
    shared.enqueue(1, &[]).unwrap();
    assert_eq!(shared.noise.lock().unwrap().reserved(), 0);
    shared
        .enqueue(1, &[create(10000, 2, 0, 1, 456), op(0, 2, 0, 1, "retire")])
        .unwrap();
    render(&mut renderer, &shared, 128);
    shared
        .enqueue(1, &[create(0, 3, 0, 1, 789), op(0, 3, 0, 1, "start")])
        .unwrap();
    assert!(energy(&render(&mut renderer, &shared, 11000)) > 0.01);
    let adopted = Shared::new(48000, 2);
    let mut voice = Renderer::new(48000).unwrap();
    adopted.performance("start", 1, None).unwrap();
    adopted
        .enqueue(
            2,
            &[
                tagged(create(1000, 1, 0, 1, 123), 1),
                tagged(op(2000, 1, 0, 1, "start"), 2),
            ],
        )
        .unwrap();
    adopted.performance("cancel-owner", 1, Some(1)).unwrap();
    let pcm = render(&mut voice, &adopted, 3000);
    assert!(pcm[..2000].iter().all(|v| *v == [0.0; 2]));
    assert!(energy(&pcm[2200..]) > 0.01);
}
#[test]
fn global_stop_resets_noise_dsp_and_control_leases_and_restarts_cleanly() {
    let shared = Arc::new(Shared::new(48000, 1));
    shared
        .enqueue(1, &[create(0, 0, 0, 1, 123), op(0, 0, 0, 1, "start")])
        .unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let exit = stop.clone();
    let callback = shared.clone();
    let thread = std::thread::spawn(move || {
        let mut renderer = Renderer::new(48000).unwrap();
        while !exit.load(Ordering::Acquire) {
            renderer.prepare(&callback);
            for _ in 0..128 {
                renderer.sample(&callback);
            }
            callback.frame.store(renderer.frame, Ordering::Release);
            std::thread::sleep(Duration::from_millis(1));
        }
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    while shared.frame.load(Ordering::Acquire) < 512 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    let stopped = shared.stop();
    let remaining = shared.noise.lock().unwrap().reserved();
    let restarted = shared.enqueue(1, &[create(0, 0, 0, 2, 456), op(0, 0, 0, 2, "start")]);
    stop.store(true, Ordering::Release);
    thread.join().unwrap();
    assert!(stopped.is_ok());
    assert_eq!(remaining, 0);
    assert!(restarted.is_ok());
}

#[test]
fn a_future_create_cannot_reset_an_earlier_valid_start_or_its_new_owner() {
    let a = Shared::new(48000, 1);
    let b = Shared::new(48000, 2);
    let mut ra = Renderer::new(48000).unwrap();
    let mut rb = Renderer::new(48000).unwrap();
    a.performance("start", 1, None).unwrap();
    b.performance("start", 1, None).unwrap();
    a.enqueue(
        1,
        &[
            tagged(create(1000, 1, 0, 1, 123), 1),
            tagged(op(0, 1, 0, 1, "start"), 2),
            tagged(op(1500, 1, 0, 1, "stop"), 1),
        ],
    )
    .unwrap();
    b.enqueue(
        2,
        &[
            tagged(create(0, 1, 0, 1, 123), 1),
            tagged(op(0, 1, 0, 1, "start"), 2),
            tagged(op(1500, 1, 0, 1, "stop"), 1),
        ],
    )
    .unwrap();
    let expected = render(&mut rb, &b, 4096);
    let actual = render(&mut ra, &a, 4096);
    assert!(energy(&actual[1500..]) > 0.01);
    assert_eq!(actual, expected);
}

#[test]
fn lazy_adoption_preserves_initial_gain_pan_envelope_and_filter_configuration() {
    for gain in [0.0, 0.5] {
        let a = Shared::new(48000, 1);
        let b = Shared::new(48000, 2);
        let mut ra = Renderer::new(48000).unwrap();
        let mut rb = Renderer::new(48000).unwrap();
        a.performance("start", 1, None).unwrap();
        b.performance("start", 1, None).unwrap();
        let configure = |frame| {
            vec![
                tagged(create(frame, 1, 0, 1, 123), 1),
                tagged(param(frame, 1, 0, 1, 0, gain), 1),
                tagged(param(frame, 1, 0, 1, 1, 1.0), 1),
                tagged(param(frame, 1, 0, 1, 4, 0.005), 1),
                tagged(param(frame, 1, 0, 1, 5, 0.01), 1),
                tagged(param(frame, 1, 0, 1, 2, 800.0), 1),
                tagged(
                    command(
                        frame,
                        1,
                        0,
                        1,
                        serde_json::json!({"action":"filter","mode":1}),
                    ),
                    1,
                ),
            ]
        };
        let mut staggered = configure(1000);
        for (index, command) in staggered.iter_mut().enumerate() {
            command.frame += index as u64;
        }
        a.enqueue(1, &staggered).unwrap();
        a.enqueue(
            1,
            &[
                tagged(op(0, 1, 0, 1, "start"), 2),
                tagged(op(2000, 1, 0, 1, "stop"), 2),
            ],
        )
        .unwrap();
        a.performance("cancel-owner", 1, Some(1)).unwrap();
        b.enqueue(2, &configure(0)).unwrap();
        b.enqueue(
            2,
            &[
                tagged(op(0, 1, 0, 1, "start"), 2),
                tagged(op(2000, 1, 0, 1, "stop"), 2),
            ],
        )
        .unwrap();
        let actual = render(&mut ra, &a, 3000);
        let expected = render(&mut rb, &b, 3000);
        assert_eq!(actual, expected);
        if gain == 0.0 {
            assert!(actual.iter().all(|v| *v == [0.0; 2]));
        } else {
            assert!(energy(&actual[512..2000]) > 0.01);
            assert!(actual.iter().all(|v| v[0].abs() < 1e-7));
            assert!(energy(&actual[2000..2400]) > 0.0);
            assert!(actual[2480..].iter().all(|v| *v == [0.0; 2]));
        }
    }
}
#[test]
fn lazy_adoption_does_not_apply_an_unrelated_future_parameter_early() {
    let shared = Shared::new(48000, 1);
    let mut renderer = Renderer::new(48000).unwrap();
    shared
        .enqueue(
            1,
            &[
                create(1000, 0, 0, 1, 123),
                op(0, 0, 0, 1, "start"),
                param(3000, 0, 0, 1, 0, 0.0),
            ],
        )
        .unwrap();
    let pcm = render(&mut renderer, &shared, 4000);
    assert!(energy(&pcm[512..3000]) > 0.01);
    assert!(pcm[3000..].iter().all(|v| *v == [0.0; 2]));
}
