//! Offline callback tests. These never open an audio device.
use super::*;
use serde_json::{json, Value};

fn command(instance: u64, frame: u64, message: Value) -> Command {
    let mut message = message;
    message["chip"] = json!("segapsg");
    serde_json::from_value(
        json!({"frame": frame,"type":"extra-psg","instance":instance,"message":message}),
    )
    .unwrap()
}
fn tone(instance: u64, frame: u64, period: u16, mask: u8) -> Vec<Command> {
    vec![
        command(instance, frame, json!({"method":"stereo","mask":mask})),
        command(
            instance,
            frame,
            json!({"method":"write","value":0x80 | (period & 15)}),
        ),
        command(
            instance,
            frame,
            json!({"method":"write","value":period >> 4,"psgRegister":0}),
        ),
        command(instance, frame, json!({"method":"write","value":0x92})),
    ]
}
fn render(renderer: &mut Renderer, shared: &Shared, count: usize) -> Vec<[f32; 2]> {
    renderer.prepare(shared);
    (0..count).map(|_| renderer.sample(shared)).collect()
}
fn energy(samples: &[[f32; 2]], ch: usize) -> f32 {
    samples.iter().map(|sample| sample[ch].abs()).sum()
}

#[test]
fn extra_psg_rack_is_independent_stereo_and_does_not_change_primary() {
    let shared = Shared::new(48_000, 1);
    let mut renderer = Renderer::new(48_000).unwrap();
    shared.create_extra_psg(1, None).unwrap();
    shared.create_extra_psg(2, Some(4_000_000)).unwrap();
    shared.enqueue(1, &tone(1, 0, 400, 0x10)).unwrap();
    let samples = render(&mut renderer, &shared, 1000);
    assert!(energy(&samples, 0) > 1.0);
    assert_eq!(energy(&samples, 1), 0.0);
    assert_eq!(renderer.psg_owners, [None; 4]);
    shared.enqueue(1, &tone(2, 0, 800, 0x01)).unwrap();
    let samples = render(&mut renderer, &shared, 1000);
    assert!(energy(&samples, 0) > 1.0 && energy(&samples, 1) > 1.0);
    let first = shared
        .extra_psg
        .get(1, 0)
        .unwrap()
        .psg()
        .unwrap()
        .game_gear_status(renderer.frame)
        .unwrap();
    let second = shared
        .extra_psg
        .get(2, 0)
        .unwrap()
        .psg()
        .unwrap()
        .game_gear_status(renderer.frame)
        .unwrap();
    let first = serde_json::to_value(first).unwrap();
    let second = serde_json::to_value(second).unwrap();
    assert_eq!(first["segapsg"]["periods"][0], 400);
    assert_eq!(second["segapsg"]["periods"][0], 800);
    assert_eq!(second["segapsg"]["clock"], 4_000_000);
    shared.dispose_extra_psg(1).unwrap();
    let samples = render(&mut renderer, &shared, 1000);
    assert_eq!(energy(&samples, 0), 0.0);
    assert!(energy(&samples, 1) > 1.0);
}

#[test]
fn extra_psg_expired_commands_and_disposal_cannot_touch_replacement() {
    let shared = Shared::new(48_000, 1);
    let mut renderer = Renderer::new(48_000).unwrap();
    shared.create_extra_psg(1, None).unwrap();
    shared.enqueue(1, &tone(1, 10_000, 400, 0x10)).unwrap();
    shared.dispose_extra_psg(1).unwrap();
    shared.create_extra_psg(2, None).unwrap();
    shared.enqueue(1, &tone(2, 0, 800, 0x01)).unwrap();
    assert!(shared
        .enqueue(1, &tone(1, 0, 1, 0xff))
        .unwrap_err()
        .contains("expired"));
    assert!(shared
        .enqueue(2, &tone(2, 0, 1, 0xff))
        .unwrap_err()
        .contains("expired"));
    assert!(shared
        .create_extra_psg(1, None)
        .unwrap_err()
        .contains("increasing"));
    shared.dispose_extra_psg(1).unwrap();
    let samples = render(&mut renderer, &shared, 11_000);
    assert_eq!(energy(&samples, 0), 0.0);
    assert!(energy(&samples, 1) > 1.0);
    assert_eq!(shared.pending.load(Ordering::Acquire), 0);
}

#[test]
fn extra_psg_admission_counts_pending_retiring_and_failed_allocation_is_atomic() {
    let shared = Shared::new(48_000, 1);
    let mut renderer = Renderer::new(48_000).unwrap();
    assert!(shared.create_extra_psg(1, Some(0)).is_err());
    assert_eq!(shared.pending.load(Ordering::Acquire), 0);
    for id in 1..=extra_psg::LIMIT as u64 {
        shared.create_extra_psg(id, None).unwrap();
    }
    assert_eq!(shared.extra_psg.counts(), (16, 16));
    assert!(shared
        .create_extra_psg(17, None)
        .unwrap_err()
        .contains("limit"));
    for id in 1..=16 {
        shared.dispose_extra_psg(id).unwrap();
    }
    assert!(shared
        .create_extra_psg(17, None)
        .unwrap_err()
        .contains("limit"));
    assert_eq!(shared.pending.load(Ordering::Acquire), 16);
    render(&mut renderer, &shared, 1);
    shared.create_extra_psg(17, None).unwrap();
    assert_eq!(shared.extra_psg.counts(), (1, 1));
}

#[test]
fn extra_psg_final_core_release_waits_for_control_reclamation() {
    let shared = Shared::new(48_000, 1);
    let mut renderer = Renderer::new(48_000).unwrap();
    shared.create_extra_psg(1, None).unwrap();
    let target = shared.extra_psg.get(1, 0).unwrap();
    let weak = Arc::downgrade(&target);
    drop(target);
    render(&mut renderer, &shared, 1);
    shared.dispose_extra_psg(1).unwrap();
    render(&mut renderer, &shared, 1);
    assert!(
        weak.upgrade().is_some(),
        "callback keeps the final control registry guard"
    );
    shared.extra_psg.collect_retired().unwrap();
    assert!(
        weak.upgrade().is_none(),
        "only control reclamation releases the core"
    );
}

#[test]
fn extra_psg_global_stop_purges_installed_and_pending_instances_then_can_recreate() {
    for install in [false, true] {
        let shared = Shared::new(48_000, 1);
        let mut renderer = Renderer::new(48_000).unwrap();
        shared.create_extra_psg(1, None).unwrap();
        shared.enqueue(1, &tone(1, 0, 400, 0xff)).unwrap();
        shared.enqueue(1, &tone(1, 10_000, 700, 0xff)).unwrap();
        if install {
            render(&mut renderer, &shared, 1000);
        }
        shared.extra_psg.deactivate_all().unwrap();
        shared.epoch.fetch_add(1, Ordering::AcqRel);
        let samples = render(&mut renderer, &shared, 2000);
        assert!(samples[1000..].iter().all(|sample| *sample == [0.0; 2]));
        assert_eq!(shared.pending.load(Ordering::Acquire), 0);
        assert_eq!(shared.stopped_epoch.load(Ordering::Acquire), 1);
        shared.extra_psg.collect_retired().unwrap();
        assert_eq!(shared.extra_psg.counts(), (0, 0));
        assert!(shared.enqueue(1, &tone(1, 0, 400, 0xff)).is_err());
        shared.create_extra_psg(2, None).unwrap();
        shared.enqueue(1, &tone(2, 0, 400, 0xff)).unwrap();
        assert!(energy(&render(&mut renderer, &shared, 1000), 0) > 1.0);
    }
}

#[test]
fn extra_psg_owner_cancellation_drops_future_notes_and_preserves_manual_source() {
    let shared = Shared::new(48_000, 1);
    let mut renderer = Renderer::new(48_000).unwrap();
    shared.create_extra_psg(1, None).unwrap();
    shared.create_extra_psg(2, None).unwrap();
    shared.performance("start", 7, None).unwrap();
    let mut owned = tone(1, 0, 400, 0x10);
    owned.extend(tone(1, 10_000, 700, 0x10));
    for command in &mut owned {
        command.performance = Some(7);
        command.owner = Some(1);
    }
    shared.enqueue(1, &owned).unwrap();
    shared.enqueue(1, &tone(2, 0, 800, 0x01)).unwrap();
    render(&mut renderer, &shared, 1000);
    shared.performance("cancel-owner", 7, Some(1)).unwrap();
    let samples = render(&mut renderer, &shared, 11_000);
    assert_eq!(energy(&samples, 0), 0.0);
    assert!(energy(&samples, 1) > 1.0);
    assert!(shared.enqueue(1, &owned).unwrap_err().contains("expired"));
    assert_eq!(shared.extra_psg.counts(), (2, 2));
}

#[test]
fn extra_psg_batch_rejection_does_not_retain_partial_prepared_commands() {
    let shared = Shared::new(48_000, 1);
    let mut renderer = Renderer::new(48_000).unwrap();
    shared.create_extra_psg(1, None).unwrap();
    shared.create_extra_psg(2, None).unwrap();
    let bad = vec![
        command(1, 0, json!({"method":"write","value":128,"id":1})),
        command(2, 0, json!({"method":"write","value":128,"id":1})),
        command(2, 0, json!({"method":"write","value":129,"id":1})),
    ];
    assert!(shared.enqueue(1, &bad).unwrap_err().contains("Duplicate"));
    assert_eq!(shared.pending.load(Ordering::Acquire), 2);
    for id in [1, 2] {
        let state = shared
            .extra_psg
            .get(id, 0)
            .unwrap()
            .psg()
            .unwrap()
            .game_gear_status(0)
            .unwrap();
        assert_eq!(
            serde_json::to_value(state).unwrap()["segapsg"]["pendingCommands"],
            json!([])
        );
    }
    assert_eq!(energy(&render(&mut renderer, &shared, 1000), 0), 0.0);
}

#[test]
fn extra_psg_is_available_in_every_native_mode_and_source_mixing_is_applied() {
    for mode in [
        "genesis",
        "dx7",
        "nes",
        "gameboy",
        "snes",
        "pc-engine",
        "pc98",
        "pc98-26",
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
        "mega-cd",
        "32x",
        "genesis-full-nuked",
        "genesis-nuked",
    ] {
        let (mega_cd, dx7, nes, console) = NativeGenesis::mode_config(Some(mode)).unwrap();
        let shared = Shared::new_modes(48_000, 1, mega_cd, dx7, nes, console);
        let mut renderer = Renderer::new_modes(48_000, mega_cd, dx7, nes, console).unwrap();
        shared.create_extra_psg(1, None).unwrap();
        shared.enqueue(1, &tone(1, 0, 400, 0xff)).unwrap();
        assert!(
            energy(&render(&mut renderer, &shared, 1000), 0) > 1.0,
            "{mode}"
        );
        shared
            .enqueue(
                1,
                &[command(
                    1,
                    0,
                    json!({"method":"mix","volume":0.0,"pan":0.0}),
                )],
            )
            .unwrap();
        let samples = render(&mut renderer, &shared, 1000);
        // Nuked preserves the chip's idle DAC/DC response. Compare against
        // the same unplayed primary core rather than assuming numeric zero.
        let mut baseline = Renderer::new_modes(48_000, mega_cd, dx7, nes, console).unwrap();
        baseline.idle = false;
        let expected = render(&mut baseline, &shared, 2000);
        assert!(
            samples[300..]
                .iter()
                .zip(&expected[1300..])
                .all(|(actual, expected)| (actual[0] - expected[0]).abs() < 1e-7
                    && (actual[1] - expected[1]).abs() < 1e-7),
            "{mode}"
        );
    }
}

#[test]
fn extra_psg_queue_full_sequence_expiry_and_invalid_messages_leave_no_instance() {
    let shared = Shared::new(48_000, 1);
    shared.pending.store(CAPACITY, Ordering::Release);
    assert!(shared
        .create_extra_psg(1, None)
        .unwrap_err()
        .contains("queue is full"));
    assert_eq!(shared.extra_psg.counts(), (0, 0));
    shared.pending.store(0, Ordering::Release);
    *shared.producer.lock().unwrap() = u64::MAX;
    assert!(shared
        .create_extra_psg(1, None)
        .unwrap_err()
        .contains("sequence"));
    assert_eq!(shared.pending.load(Ordering::Acquire), 0);
    *shared.producer.lock().unwrap() = 0;
    shared.create_extra_psg(1, None).unwrap();
    let invalid:Command=serde_json::from_value(json!({"frame":0,"type":"extra-psg","instance":1,"message":{"chip":"ym2413","method":"write","register":0,"value":0}})).unwrap();
    assert!(shared
        .enqueue(1, &[invalid])
        .unwrap_err()
        .contains("only accept Sega PSG"));
    assert_eq!(shared.pending.load(Ordering::Acquire), 1);
}

#[test]
fn extra_psg_held_clock_failure_is_recoverable_and_does_not_change_other_instances() {
    let shared = Shared::new(48_000, 1);
    let mut renderer = Renderer::new(48_000).unwrap();
    shared.create_extra_psg(1, None).unwrap();
    shared.create_extra_psg(2, None).unwrap();
    let audition = command(
        1,
        0,
        json!({"method":"batch","audition":true,"channel":0,"voice":1,"action":"on","commands":[{"method":"write","value":128},{"method":"write","value":25,"psgRegister":0},{"method":"write","value":146}]}),
    );
    shared.enqueue(1, &[audition]).unwrap();
    shared.enqueue(1, &tone(2, 0, 800, 0x01)).unwrap();
    render(&mut renderer, &shared, 1000);
    let clock = command(1, 0, json!({"method":"clock","hz":4_000_000,"id":10}));
    assert_eq!(
        shared.enqueue(1, &[clock]).unwrap_err(),
        extra_psg::CLOCK_HELD
    );
    assert!(energy(&render(&mut renderer, &shared, 1000), 1) > 1.0);
    assert_eq!(shared.extra_psg.counts(), (2, 2));
    shared.enqueue(1,&[command(1,0,json!({"method":"batch","audition":true,"channel":0,"voice":1,"action":"off","commands":[{"method":"key-off","channel":0}]}))]).unwrap();
    render(&mut renderer, &shared, 1);
    shared
        .enqueue(
            1,
            &[command(
                1,
                0,
                json!({"method":"clock","hz":4_000_000,"id":11}),
            )],
        )
        .unwrap();
    render(&mut renderer, &shared, 1);
    let target = shared.extra_psg.get(1, 0).unwrap();
    target.psg().unwrap().wait_applied(11).unwrap();
    assert_eq!(
        serde_json::to_value(
            target
                .psg()
                .unwrap()
                .game_gear_status(renderer.frame)
                .unwrap()
        )
        .unwrap()["segapsg"]["clock"],
        4_000_000
    );
}

#[test]
fn extra_psg_pending_status_barrier_can_be_canceled_without_holding_running_mutex() {
    let rack = NativeGenesis::default();
    let shared = Arc::new(Shared::new(48_000, 1));
    let mut renderer = Renderer::new(48_000).unwrap();
    let (stop, _stopped) = mpsc::channel();
    *rack.running.lock().unwrap() = Some(Running {
        shared: shared.clone(),
        stop,
        thread: None,
    });
    shared.create_extra_psg(1, None).unwrap();
    shared
        .enqueue(
            1,
            &[command(
                1,
                480_000,
                json!({"method":"clock","hz":4_000_000,"id":1}),
            )],
        )
        .unwrap();
    let status_shared = rack.session_shared(1).unwrap();
    let (result_tx, result_rx) = mpsc::channel();
    let waiting = std::thread::spawn(move || {
        result_tx
            .send(status_shared.extra_psg_status(1, Some(1)))
            .unwrap();
    });
    rack.with_session(1, |shared| shared.dispose_extra_psg(1))
        .unwrap();
    render(&mut renderer, &shared, 1);
    let error = result_rx
        .recv_timeout(Duration::from_millis(200))
        .expect("disposal unblocks the pending clock barrier")
        .unwrap_err();
    assert!(error.contains("expired"));
    waiting.join().unwrap();
    // Avoid Running::drop's stop barrier: this test has no background callback.
    let running = rack.running.lock().unwrap().take().unwrap();
    shared.failed.store(true, Ordering::Release);
    drop(running);
}
