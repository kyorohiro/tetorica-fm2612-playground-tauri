//! Offline independent YM2612 rack tests. Never opens an audio device.
use super::*;
use serde_json::{json, Value};
fn command(instance: u64, frame: u64, message: Value) -> Command {
    serde_json::from_value(
        json!({"frame":frame,"type":"extra-ym2612","instance":instance,"message":message}),
    )
    .unwrap()
}
fn write(port: u8, register: u8, value: u8) -> Value {
    json!({"method":"write","port":port,"register":register,"value":value})
}
fn fm(ch: u8, high: u8, low: u8, pan: u8) -> Vec<Value> {
    let port = ch / 3;
    let offset = ch % 3;
    let mut commands = Vec::new();
    for slot in [0, 4, 8, 12] {
        for (register, value) in [
            (0x30 + slot + offset, 1),
            (0x40 + slot + offset, if slot == 12 { 48 } else { 127 }),
            (0x50 + slot + offset, 31),
            (0x60 + slot + offset, 0),
            (0x70 + slot + offset, 0),
            (0x80 + slot + offset, 15),
        ] {
            commands.push(write(port, register, value));
        }
    }
    commands.extend([
        write(port, 0xb0 + offset, 7),
        write(port, 0xb4 + offset, pan),
        write(port, 0xa4 + offset, high),
        write(port, 0xa0 + offset, low),
        write(0, 0x28, 0xf0 | if ch < 3 { ch } else { ch + 1 }),
    ]);
    commands
}
fn start(shared: &Shared, id: u64, ch: u8, pan: u8, owner: Option<Owner>) {
    let mut commands: Vec<_> = fm(ch, 0x22, 0x69, pan)
        .into_iter()
        .map(|v| command(id, 0, v))
        .collect();
    for command in &mut commands {
        command.owner = owner.map(|v| v.owner);
        command.performance = owner.map(|v| v.performance);
    }
    shared.enqueue(1, &commands).unwrap();
}
fn render(renderer: &mut Renderer, shared: &Shared, n: usize) -> Vec<[f32; 2]> {
    // Emulate ordinary 128-frame Native callbacks, including publishing
    // delayed audition key edges and timer state at the following boundary.
    let samples = (0..n)
        .map(|index| {
            if index % 128 == 0 {
                renderer.prepare(shared);
            }
            renderer.sample(shared)
        })
        .collect();
    renderer.prepare(shared);
    samples
}
fn energy(samples: &[[f32; 2]]) -> f32 {
    samples.iter().map(|s| s[0].abs() + s[1].abs()).sum()
}
fn snapshot(shared: &Shared, id: u64) -> ym2612::ChipStatus {
    shared.extra_ym2612_status(id, None).unwrap().ym2612
}
fn audition(shared: &Shared, id: u64, ch: u8, voice: u64, action: &str, commands: Vec<Value>) {
    shared.enqueue(1,&[command(id,0,json!({"method":"batch","channel":ch,"voice":voice,"action":action,"audition":true,"commands":commands}))]).unwrap();
}

#[test]
fn extra_ym2612_rack_mixes_two_fm_and_psg_without_touching_primary_or_each_other() {
    for engine in ["ymfm", "nuked"] {
        let shared = Shared::new(48_000, 1);
        let mut renderer = Renderer::new(48_000).unwrap();
        shared.create_extra_psg(1, None).unwrap();
        shared
            .create_extra_ym2612_engine(2, None, Some(engine))
            .unwrap();
        shared
            .create_extra_ym2612_engine(3, Some(8_000_000), Some(engine))
            .unwrap();
        start(&shared, 2, 0, 0x80, None);
        start(&shared, 3, 3, 0x40, None);
        let psg:Vec<Command>=[0x80,0x19,0x92].into_iter().map(|value|serde_json::from_value(json!({"frame":0,"type":"extra-psg","instance":1,"message":{"chip":"segapsg","method":"write","value":value}})).unwrap()).collect();
        shared.enqueue(1, &psg).unwrap();
        assert!(energy(&render(&mut renderer, &shared, 2000)) > 2.);
        assert_eq!(renderer.fm_owners, [None; 6]);
        assert_eq!(renderer.psg_owners, [None; 4]);
        let a = snapshot(&shared, 2);
        let b = snapshot(&shared, 3);
        assert_eq!(a.clock, ym2612::CLOCK);
        assert_eq!(b.clock, 8_000_000);
        assert_eq!(a.key_masks, [15, 0, 0, 0, 0, 0]);
        assert_eq!(b.key_masks, [0, 0, 0, 15, 0, 0]);
        assert_eq!(a.registers[0][0xa0], 0x69);
        assert_eq!(b.registers[1][0xa0], 0x69);
        shared
            .enqueue(1, &[command(2, 0, json!({"method":"reset"}))])
            .unwrap();
        render(&mut renderer, &shared, 1);
        assert_eq!(snapshot(&shared, 2).key_masks, [0; 6]);
        assert_eq!(snapshot(&shared, 3).key_masks, [0, 0, 0, 15, 0, 0]);
        assert_eq!(
            shared
                .extra_psg
                .get(1, 0)
                .unwrap()
                .psg()
                .unwrap()
                .game_gear_status(0)
                .unwrap()
                .segapsg
                .periods[0],
            400
        );
    }
}
#[test]
fn extra_ym2612_and_psg_share_limit_and_monotonic_identity_without_protocol_aliases() {
    for engine in ["ymfm", "nuked"] {
        let shared = Shared::new(48_000, 1);
        let mut renderer = Renderer::new(48_000).unwrap();
        for id in 1..=16 {
            if id % 2 == 0 {
                shared
                    .create_extra_ym2612_engine(id, None, Some(engine))
                    .unwrap();
            } else {
                shared.create_extra_psg(id, None).unwrap();
            }
        }
        assert!(shared
            .create_extra_ym2612_engine(17, None, Some(engine))
            .unwrap_err()
            .contains("limit"));
        assert!(shared
            .create_extra_psg(17, None)
            .unwrap_err()
            .contains("limit"));
        assert!(shared
            .enqueue(1, &[command(1, 0, write(0, 0x22, 1))])
            .unwrap_err()
            .contains("not a YM2612"));
        assert!(shared.dispose_extra_psg(2).unwrap_err().contains("match"));
        assert!(shared
            .dispose_extra_ym2612(1)
            .unwrap_err()
            .contains("match"));
        assert_eq!(shared.extra_psg.counts(), (16, 16));
        for id in 1..=16 {
            if id % 2 == 0 {
                shared.dispose_extra_ym2612(id).unwrap();
            } else {
                shared.dispose_extra_psg(id).unwrap();
            }
        }
        assert!(shared
            .create_extra_ym2612_engine(17, None, Some(engine))
            .unwrap_err()
            .contains("limit"));
        render(&mut renderer, &shared, 1);
        shared
            .create_extra_ym2612_engine(17, None, Some(engine))
            .unwrap();
        assert!(shared
            .create_extra_psg(16, None)
            .unwrap_err()
            .contains("increasing"));
        assert_eq!(shared.extra_psg.counts(), (1, 1));
    }
}
#[test]
fn extra_ym2612_disposal_invalidates_future_events_and_only_control_drops_core() {
    for engine in ["ymfm", "nuked"] {
        let shared = Shared::new(48_000, 1);
        let mut renderer = Renderer::new(48_000).unwrap();
        shared
            .create_extra_ym2612_engine(1, None, Some(engine))
            .unwrap();
        let target = shared.extra_psg.get(1, 0).unwrap();
        let weak = Arc::downgrade(&target);
        drop(target);
        shared
            .enqueue(1, &[command(1, 10_000, write(0, 0x22, 15))])
            .unwrap();
        render(&mut renderer, &shared, 1);
        shared.dispose_extra_ym2612(1).unwrap();
        shared.dispose_extra_ym2612(1).unwrap();
        shared
            .create_extra_ym2612_engine(2, None, Some(engine))
            .unwrap();
        assert!(shared
            .enqueue(1, &[command(1, 0, write(0, 0x22, 1))])
            .unwrap_err()
            .contains("expired"));
        render(&mut renderer, &shared, 11_000);
        assert!(weak.upgrade().is_some());
        assert_eq!(snapshot(&shared, 2).registers[0][0x22], 0);
        assert_eq!(shared.pending.load(Ordering::Acquire), 0);
        shared.extra_psg.collect_retired().unwrap();
        assert!(weak.upgrade().is_none());
    }
}
#[test]
fn extra_ym2612_admission_rolls_back_other_instance_and_other_chip_commands() {
    for engine in ["ymfm", "nuked"] {
        let shared = Shared::new(48_000, 1);
        let mut renderer = Renderer::new(48_000).unwrap();
        shared.create_extra_psg(1, None).unwrap();
        shared
            .create_extra_ym2612_engine(2, None, Some(engine))
            .unwrap();
        let psg:Command=serde_json::from_value(json!({"frame":0,"type":"extra-psg","instance":1,"message":{"chip":"segapsg","method":"write","value":128,"id":1}})).unwrap();
        let bad = [
            psg,
            command(
                2,
                0,
                json!({"method":"write","port":0,"register":0x22,"value":1,"id":1}),
            ),
            command(
                2,
                0,
                json!({"method":"write","port":0,"register":0x22,"value":2,"id":1}),
            ),
        ];
        assert!(shared.enqueue(1, &bad).unwrap_err().contains("Duplicate"));
        assert_eq!(shared.pending.load(Ordering::Acquire), 2);
        assert!(snapshot(&shared, 2).pending_commands.is_empty());
        render(&mut renderer, &shared, 1);
        assert_eq!(snapshot(&shared, 2).registers[0][0x22], 0);
        assert!(shared
            .extra_psg
            .get(1, 0)
            .unwrap()
            .psg()
            .unwrap()
            .game_gear_status(0)
            .unwrap()
            .segapsg
            .pending_commands
            .is_empty());
    }
}
#[test]
fn extra_ym2612_owner_cancellation_preserves_manual_voice_and_stops_timer_csm() {
    for engine in ["ymfm", "nuked"] {
        let shared = Shared::new(48_000, 1);
        let mut renderer = Renderer::new(48_000).unwrap();
        shared
            .create_extra_ym2612_engine(1, None, Some(engine))
            .unwrap();
        shared.performance("start", 7, None).unwrap();
        let owner = Some(Owner {
            performance: 7,
            owner: 1,
        });
        start(&shared, 1, 2, 0xc0, owner);
        let mut csm = command(1, 0, write(0, 0x27, 0x85));
        csm.performance = Some(7);
        csm.owner = Some(1);
        shared.enqueue(1, &[csm]).unwrap();
        audition(&shared, 1, 3, 99, "on", fm(3, 0x22, 0x69, 0xc0));
        let mut future = command(1, 10_000, write(0, 0x27, 0x85));
        future.performance = Some(7);
        future.owner = Some(1);
        shared.enqueue(1, &[future]).unwrap();
        render(&mut renderer, &shared, 2000);
        shared.performance("cancel-owner", 7, Some(1)).unwrap();
        render(&mut renderer, &shared, 11_000);
        let s = snapshot(&shared, 1);
        assert_eq!(s.key_masks[2], 0);
        assert_eq!(s.registers[0][0x27] & 0x81, 0);
        assert_eq!(s.key_masks[3], 15);
        assert_eq!(s.held, [3]);
        assert_eq!(shared.pending.load(Ordering::Acquire), 0);
    }
}
#[test]
fn extra_ym2612_manual_lease_blocks_program_edits_and_recovers_held_clock_error() {
    for engine in ["ymfm", "nuked"] {
        let shared = Shared::new(48_000, 1);
        let mut renderer = Renderer::new(48_000).unwrap();
        shared
            .create_extra_ym2612_engine(1, None, Some(engine))
            .unwrap();
        shared
            .create_extra_ym2612_engine(2, None, Some(engine))
            .unwrap();
        // Schedule clock before the manual lease is applied: rejection must also
        // occur on callback, with an observed, recoverable command outcome.
        shared
            .enqueue(
                1,
                &[command(
                    1,
                    1000,
                    json!({"method":"clock","hz":8_000_000,"id":1}),
                )],
            )
            .unwrap();
        audition(&shared, 1, 0, 1, "on", fm(0, 0x22, 0x69, 0xc0));
        render(&mut renderer, &shared, 2000);
        assert!(shared
            .extra_ym2612_status(1, Some(1))
            .unwrap_err()
            .contains("manually held additional YM2612"));
        assert_eq!(snapshot(&shared, 1).clock, ym2612::CLOCK);
        shared
            .enqueue(
                1,
                &[
                    command(1, 0, write(0, 0x40, 1)),
                    command(1, 0, json!({"method":"reset"})),
                ],
            )
            .unwrap();
        render(&mut renderer, &shared, 1);
        assert_eq!(snapshot(&shared, 1).held, [0]);
        assert_eq!(snapshot(&shared, 1).registers[0][0x40], 127);
        assert!(shared
            .enqueue(
                1,
                &[command(
                    1,
                    0,
                    json!({"method":"clock","hz":8_000_000,"id":2})
                )]
            )
            .unwrap_err()
            .contains("manually held"));
        audition(
            &shared,
            1,
            0,
            1,
            "off",
            vec![json!({"method":"key-off","channel":0})],
        );
        render(&mut renderer, &shared, 1);
        shared
            .enqueue(
                1,
                &[command(
                    1,
                    0,
                    json!({"method":"clock","hz":8_000_000,"id":3}),
                )],
            )
            .unwrap();
        render(&mut renderer, &shared, 1);
        assert_eq!(
            shared.extra_ym2612_status(1, Some(3)).unwrap().ym2612.clock,
            8_000_000
        );
        assert_eq!(snapshot(&shared, 2).clock, ym2612::CLOCK);
    }
}
#[test]
fn extra_ym2612_status_barrier_disposal_does_not_hold_native_running_mutex() {
    for engine in ["ymfm", "nuked"] {
        let rack = NativeGenesis::default();
        let shared = Arc::new(Shared::new(48_000, 1));
        let mut renderer = Renderer::new(48_000).unwrap();
        let (stop, _) = mpsc::channel();
        *rack.running.lock().unwrap() = Some(Running {
            shared: shared.clone(),
            stop,
            thread: None,
        });
        shared
            .create_extra_ym2612_engine(1, None, Some(engine))
            .unwrap();
        shared
            .enqueue(
                1,
                &[command(
                    1,
                    480_000,
                    json!({"method":"clock","hz":8_000_000,"id":1}),
                )],
            )
            .unwrap();
        let status_shared = rack.session_shared(1).unwrap();
        let (tx, rx) = mpsc::channel();
        let waiting = std::thread::spawn(move || {
            tx.send(status_shared.extra_ym2612_status(1, Some(1)))
                .unwrap()
        });
        rack.with_session(1, |s| s.dispose_extra_ym2612(1)).unwrap();
        render(&mut renderer, &shared, 1);
        assert!(rx
            .recv_timeout(Duration::from_millis(200))
            .unwrap()
            .unwrap_err()
            .contains("expired"));
        waiting.join().unwrap();
        let running = rack.running.lock().unwrap().take().unwrap();
        shared.failed.store(true, Ordering::Release);
        drop(running);
    }
}
#[test]
fn extra_ym2612_stop_create_race_retires_old_epoch_before_new_rack_admission() {
    for engine in ["ymfm", "nuked"] {
        let shared = Arc::new(Shared::new(48_000, 1));
        let mut renderer = Renderer::new(48_000).unwrap();
        shared
            .create_extra_ym2612_engine(1, None, Some(engine))
            .unwrap();
        start(&shared, 1, 0, 0xc0, None);
        render(&mut renderer, &shared, 1000);
        let stopping = shared.clone();
        let stopped = std::thread::spawn(move || stopping.stop());
        let start = Instant::now();
        while shared.epoch.load(Ordering::Acquire) == 0 {
            assert!(start.elapsed() < Duration::from_secs(1));
            std::thread::yield_now();
        }
        let creating = shared.clone();
        let created =
            std::thread::spawn(move || creating.create_extra_ym2612_engine(2, None, Some(engine)));
        render(&mut renderer, &shared, 2000);
        stopped.join().unwrap().unwrap();
        created.join().unwrap().unwrap();
        render(&mut renderer, &shared, 1);
        assert!(shared.extra_psg.get(1, 1).is_err());
        assert!(shared.extra_psg.get(2, 1).is_ok());
        assert_eq!(shared.extra_psg.counts(), (1, 1));
    }
}
#[test]
fn extra_ym2612_all_native_modes_use_common_source_mixer_fx_and_individual_disposal() {
    for engine in ["ymfm", "nuked"] {
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
            let (mega_cd, dx7, nes, console) = NativeGenesis::mode_config(Some(mode)).unwrap();
            let shared = Shared::new_modes(48_000, 1, mega_cd, dx7, nes, console);
            let mut renderer = Renderer::new_modes(48_000, mega_cd, dx7, nes, console).unwrap();
            shared
                .create_extra_ym2612_engine(1, None, Some(engine))
                .unwrap();
            start(&shared, 1, 0, 0xc0, None);
            let sounding = render(&mut renderer, &shared, 1000);
            assert!(energy(&sounding) > 1., "{mode}");
            shared
                .enqueue(
                    1,
                    &[command(1, 0, json!({"method":"mix","volume":0.,"pan":0.}))],
                )
                .unwrap();
            let muted = render(&mut renderer, &shared, 1000);
            let mut baseline = Renderer::new_modes(48_000, mega_cd, dx7, nes, console).unwrap();
            baseline.idle = false;
            let expected = render(&mut baseline, &shared, 2000);
            assert!(
                muted[300..]
                    .iter()
                    .zip(&expected[1300..])
                    .all(|(a, b)| (a[0] - b[0]).abs() < 1e-7 && (a[1] - b[1]).abs() < 1e-7),
                "{mode}"
            );
            shared.dispose_extra_ym2612(1).unwrap();
            render(&mut renderer, &shared, 1);
            shared.extra_psg.collect_retired().unwrap();
            assert_eq!(shared.extra_psg.counts(), (0, 0));
        }
    }
}
#[test]
fn extra_ym2612_snapshot_tracks_committed_shared_latch_timers_and_mute_controls() {
    for engine in ["ymfm", "nuked"] {
        let shared = Shared::new(48_000, 1);
        let mut renderer = Renderer::new(48_000).unwrap();
        shared
            .create_extra_ym2612_engine(1, None, Some(engine))
            .unwrap();
        let messages = [
            write(0, 0xa4, 0x21),
            write(1, 0xa0, 0x69),
            write(0, 0x24, 255),
            write(0, 0x25, 3),
            write(0, 0x27, 5),
            json!({"method":"source-mute","mask":1}),
            json!({"method":"channel-mute","mask":32}),
        ];
        shared
            .enqueue(
                1,
                &messages
                    .into_iter()
                    .map(|v| command(1, 0, v))
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        render(&mut renderer, &shared, 2000);
        renderer.prepare(&shared);
        let s = snapshot(&shared, 1);
        assert_eq!(s.registers[1][0xa4], 0x21);
        assert_eq!(s.registers[1][0xa0], 0x69);
        assert_eq!(s.source_mute_mask, 1);
        assert_eq!(s.channel_mute_mask, 32);
        assert!(s.status & 1 != 0 && s.irq);
    }
}

#[test]
fn extra_ym2612_validation_and_failed_creation_leave_bounded_state_unchanged() {
    for engine in ["ymfm", "nuked"] {
        eprintln!("extra-chip-memory-layout: instance={} ym2612_shared={} session_shared={} prepared_arc={} queue_slot_estimate={} queue_heap={} bytes", std::mem::size_of::<extra_psg::Instance>(), std::mem::size_of::<ym2612::Shared>(), std::mem::size_of::<Shared>(), std::mem::size_of::<Arc<ym2612::Prepared>>(), std::mem::size_of::<usize>() + std::mem::size_of::<Arc<ym2612::Prepared>>(), CAPACITY * (std::mem::size_of::<usize>() + std::mem::size_of::<Arc<ym2612::Prepared>>()));
        let shared = Shared::new(48_000, 1);
        for message in [
            json!({"method":"write","port":2,"register":0,"value":1}),
            json!({"method":"key-off","channel":6}),
            json!({"method":"clock","hz":99999}),
            json!({"method":"clock","hz":100000001}),
            json!({"method":"source-mute","mask":2}),
            json!({"method":"channel-mute","mask":64}),
            json!({"method":"mix","volume":4.1,"pan":0.}),
            json!({"method":"mix","volume":1.,"pan":1.1}),
            json!({"method":"reset","id":0}),
            json!({"method":"batch","channel":0,"voice":1,"action":"on","audition":true,"commands":[{"method":"write","port":1,"register":0x40,"value":1}]}),
            json!({"method":"batch","channel":0,"action":"on","audition":true,"commands":[{"method":"key-off","channel":0}]}),
        ] {
            assert!(serde_json::from_value::<ym2612::Message>(message)
                .unwrap()
                .prepare()
                .is_err());
        }
        assert!(shared
            .create_extra_ym2612_engine(1, Some(99999), Some(engine))
            .is_err());
        assert_eq!(shared.extra_psg.counts(), (0, 0));
        assert_eq!(shared.pending.load(Ordering::Acquire), 0);
        shared.pending.store(CAPACITY, Ordering::Release);
        assert!(shared
            .create_extra_ym2612_engine(1, None, Some(engine))
            .unwrap_err()
            .contains("queue is full"));
        shared.pending.store(0, Ordering::Release);
        *shared.producer.lock().unwrap() = u64::MAX;
        assert!(shared
            .create_extra_ym2612_engine(1, None, Some(engine))
            .unwrap_err()
            .contains("sequence"));
        assert_eq!(shared.pending.load(Ordering::Acquire), 0);
        *shared.producer.lock().unwrap() = 0;
        shared
            .create_extra_ym2612_engine(1, None, Some(engine))
            .unwrap();
        let s = shared.extra_ym2612_status(1, None).unwrap();
        assert_eq!(
            s.ym2612.registers.iter().map(Vec::len).collect::<Vec<_>>(),
            [256, 256]
        );
        assert_eq!(s.ym2612.key_masks, [0; 6]);
        assert_eq!(s.sample_rate, 48_000);
        assert!(shared
            .enqueue(1, &[command(1, MAX_SAFE_INTEGER + 1, write(0, 0x22, 1))])
            .is_err());
    }
}
#[test]
fn extra_ym2612_stop_voices_cancels_future_program_and_dac_but_keeps_manual_voice() {
    for engine in ["ymfm", "nuked"] {
        let shared = Shared::new(48_000, 1);
        let mut renderer = Renderer::new(48_000).unwrap();
        shared
            .create_extra_ym2612_engine(1, None, Some(engine))
            .unwrap();
        audition(&shared, 1, 0, 1, "on", fm(0, 0x22, 0x69, 0xc0));
        shared
            .enqueue(
                1,
                &[
                    command(1, 0, write(0, 0x2b, 128)),
                    command(1, 0, write(0, 0x2a, 200)),
                    command(1, 10_000, write(0, 0x2b, 128)),
                ],
            )
            .unwrap();
        render(&mut renderer, &shared, 1000);
        assert_eq!(snapshot(&shared, 1).registers[0][0x2b], 128);
        shared
            .enqueue(1, &[command(1, 0, json!({"method":"stop-voices"}))])
            .unwrap();
        render(&mut renderer, &shared, 11_000);
        let s = snapshot(&shared, 1);
        assert_eq!(s.held, [0]);
        assert_eq!(s.key_masks[0], 15);
        assert_eq!(s.registers[0][0x2b] & 128, 0);
        assert_eq!(shared.pending.load(Ordering::Acquire), 0);
        shared
            .enqueue(
                1,
                &[command(
                    1,
                    0,
                    json!({"method":"stop-voices","audition":true}),
                )],
            )
            .unwrap();
        render(&mut renderer, &shared, 1);
        assert!(snapshot(&shared, 1).held.is_empty());
    }
}
#[test]
fn extra_ym2612_signal_reaches_common_native_fx_chain() {
    for engine in ["ymfm", "nuked"] {
        let shared = Shared::new(48_000, 1);
        let mut renderer = Renderer::new(48_000).unwrap();
        shared
            .create_extra_ym2612_engine(1, None, Some(engine))
            .unwrap();
        start(&shared, 1, 0, 0xc0, None);
        assert!(energy(&render(&mut renderer, &shared, 1000)) > 1.);
        let message=serde_json::from_value::<FxMessage>(json!({"op":"chain","children":[{"type":"gain","slot":0}],"units":[{"type":"gain","slot":0,"values":{"gain":0.}}],"beatSeconds":0.5})).unwrap();
        let graph = shared
            .fx
            .lock()
            .unwrap()
            .command(message, 48_000)
            .unwrap()
            .unwrap();
        shared.enqueue_special(Action::Fx(graph)).unwrap();
        // Graph replacement crossfades for 5 ms before the new gain is exact.
        assert!(render(&mut renderer, &shared, 1000)[300..]
            .iter()
            .all(|v| *v == [0.; 2]));
        assert_eq!(snapshot(&shared, 1).key_masks[0], 15);
    }
}
#[test]
fn extra_ym2612_channel_edits_stay_atomic_and_stale_manual_off_cannot_release_replacement() {
    for engine in ["ymfm", "nuked"] {
        let shared = Shared::new(48_000, 1);
        let mut renderer = Renderer::new(48_000).unwrap();
        shared
            .create_extra_ym2612_engine(1, None, Some(engine))
            .unwrap();
        // These are the actual inspector global edit packets: DAC belongs to
        // CH6 and mode/special frequencies to CH3; LFO has no physical channel.
        let global = |channel, commands: Vec<Value>| {
            command(
                1,
                0,
                json!({"method":"batch","channel":channel,"audition":true,"action":"edit","commands":commands}),
            )
        };
        shared
            .enqueue(
                1,
                &[
                    global(5, vec![write(0, 0x2b, 128), write(0, 0x2a, 200)]),
                    global(
                        2,
                        vec![
                            write(0, 0x27, 0x40),
                            write(0, 0xac, 0x22),
                            write(0, 0xa8, 0x69),
                        ],
                    ),
                    global(0, vec![write(0, 0x22, 9)]),
                ],
            )
            .unwrap();
        render(&mut renderer, &shared, 1);
        let globals = snapshot(&shared, 1);
        assert_eq!(globals.registers[0][0x2b], 128);
        assert_eq!(globals.registers[0][0x2a], 200);
        assert_eq!(globals.registers[0][0x27], 0x40);
        assert_eq!(globals.registers[0][0xa8], 0x69);
        assert_eq!(globals.registers[0][0xac], 0x22);
        assert_eq!(globals.registers[0][0x22], 9);
        // Full preset/operator edit stays channel-local; LFO is an explicit global
        // edit. A mixed physical-channel group is rejected before any register.
        let mut edit = fm(3, 0x22, 0x69, 0xc0);
        edit.pop();
        edit.push(write(0, 0x22, 8));
        audition(&shared, 1, 3, 1, "edit", edit);
        render(&mut renderer, &shared, 1);
        assert_eq!(snapshot(&shared, 1).registers[1][0x40], 127);
        assert_eq!(snapshot(&shared, 1).registers[0][0x22], 8);
        let mixed = json!({"method":"batch","channel":3,"action":"edit","audition":true,"commands":[{"method":"write","port":1,"register":0x40,"value":1},{"method":"write","port":0,"register":0x40,"value":1}]});
        assert!(shared
            .enqueue(1, &[command(1, 0, mixed)])
            .unwrap_err()
            .contains("physical voice"));
        render(&mut renderer, &shared, 1);
        assert_eq!(snapshot(&shared, 1).registers[1][0x40], 127);
        audition(&shared, 1, 3, 1, "on", fm(3, 0x22, 0x69, 0xc0));
        audition(&shared, 1, 3, 2, "on", fm(3, 0x23, 0x31, 0xc0));
        audition(
            &shared,
            1,
            3,
            1,
            "off",
            vec![json!({"method":"key-off","channel":3})],
        );
        render(&mut renderer, &shared, 1000);
        assert_eq!(snapshot(&shared, 1).key_masks[3], 15);
        assert_eq!(snapshot(&shared, 1).held, [3]);
        assert_eq!(snapshot(&shared, 1).registers[1][0xa0], 0x31);
        audition(
            &shared,
            1,
            3,
            2,
            "off",
            vec![json!({"method":"key-off","channel":3})],
        );
        render(&mut renderer, &shared, 1);
        assert!(snapshot(&shared, 1).held.is_empty());
    }
}

#[test]
fn extra_ym2612_csm_and_normal_ch3_keep_independent_worker_owners_in_both_cancel_orders() {
    for engine in ["ymfm", "nuked"] {
        for cancelled in [1, 2] {
            let shared = Shared::new(48_000, 1);
            let mut renderer = Renderer::new(48_000).unwrap();
            shared
                .create_extra_ym2612_engine(1, None, Some(engine))
                .unwrap();
            shared.performance("start", 7, None).unwrap();
            let owner = |n| {
                Some(Owner {
                    performance: 7,
                    owner: n,
                })
            };
            start(&shared, 1, 2, 0xc0, owner(1));
            let mut csm = command(1, 0, write(0, 0x27, 0x8f));
            csm.performance = Some(7);
            csm.owner = Some(1);
            shared.enqueue(1, &[csm]).unwrap();
            // A second loop's normal key edge must not steal Timer A's lease.
            start(&shared, 1, 2, 0xc0, owner(2));
            render(&mut renderer, &shared, 1000);
            shared
                .performance("cancel-owner", 7, Some(cancelled))
                .unwrap();
            render(&mut renderer, &shared, 1000);
            let s = snapshot(&shared, 1);
            if cancelled == 1 {
                assert_eq!(s.registers[0][0x27] & 0x81, 0);
                assert_eq!(s.key_masks[2], 15);
                assert_eq!(s.registers[0][0x27] & 0x0a, 0x0a);
            } else {
                assert_eq!(s.registers[0][0x27] & 0x81, 0x81);
                assert_eq!(s.key_masks[2], 0);
            }
            shared
                .performance("cancel-owner", 7, Some(if cancelled == 1 { 2 } else { 1 }))
                .unwrap();
            render(&mut renderer, &shared, 1000);
            let s = snapshot(&shared, 1);
            assert_eq!(s.registers[0][0x27] & 0x81, 0);
            assert_eq!(s.key_masks[2], 0);
        }
    }
}
#[test]
fn extra_ym2612_cancel_csm_preserves_normal_ch3_lease_and_manual_guard() {
    for engine in ["ymfm", "nuked"] {
        let shared = Shared::new(48_000, 1);
        let mut renderer = Renderer::new(48_000).unwrap();
        shared
            .create_extra_ym2612_engine(1, None, Some(engine))
            .unwrap();
        shared.performance("start", 7, None).unwrap();
        let owner = Some(Owner {
            performance: 7,
            owner: 1,
        });
        start(&shared, 1, 2, 0xc0, owner);
        shared
            .enqueue(1, &[command(1, 0, write(0, 0x27, 0x85))])
            .unwrap();
        shared
            .enqueue(1, &[command(1, 0, json!({"method":"cancel-csm"}))])
            .unwrap();
        render(&mut renderer, &shared, 1000);
        assert_eq!(snapshot(&shared, 1).registers[0][0x27] & 0x81, 0);
        assert_eq!(snapshot(&shared, 1).key_masks[2], 15);
        // The ordinary key source still belongs to its worker after CSM clears.
        shared.performance("cancel-owner", 7, Some(1)).unwrap();
        render(&mut renderer, &shared, 1000);
        assert_eq!(snapshot(&shared, 1).key_masks[2], 0);
        shared.performance("start", 8, None).unwrap();
        let mut csm = command(1, 0, write(0, 0x27, 0x85));
        csm.performance = Some(8);
        csm.owner = Some(1);
        shared.enqueue(1, &[csm]).unwrap();
        audition(&shared, 1, 2, 99, "on", fm(2, 0x22, 0x69, 0xc0));
        render(&mut renderer, &shared, 1000);
        // A manual retrigger forces a low physical gate and suspends CSM. Model
        // an explicit inspector edit enabling CSM while that same key stays held.
        audition(&shared, 1, 2, 99, "edit", vec![write(0, 0x27, 0x85)]);
        render(&mut renderer, &shared, 1);
        let mut wrong_owner = command(1, 0, json!({"method":"cancel-csm"}));
        wrong_owner.performance = Some(8);
        wrong_owner.owner = Some(2);
        shared.enqueue(1, &[wrong_owner]).unwrap();
        render(&mut renderer, &shared, 1);
        assert_eq!(snapshot(&shared, 1).registers[0][0x27] & 0x81, 0x81);
        // Main's ownerless source cancellation can release the tracked program
        // CSM source without releasing the independent held normal CH3 tone.
        shared
            .enqueue(1, &[command(1, 0, json!({"method":"cancel-csm"}))])
            .unwrap();
        render(&mut renderer, &shared, 1000);
        let direct = snapshot(&shared, 1);
        assert_eq!(direct.registers[0][0x27] & 0x81, 0);
        assert_eq!(direct.key_masks[2], 15);
        assert_eq!(direct.held, [2]);
        // Expired-owner cleanup after direct cancellation is idempotent and also
        // cannot release the independently held normal CH3 tone.
        shared.performance("cancel-owner", 8, Some(1)).unwrap();
        render(&mut renderer, &shared, 1000);
        let s = snapshot(&shared, 1);
        assert_eq!(s.registers[0][0x27] & 0x81, 0);
        assert_eq!(s.key_masks[2], 15);
        assert_eq!(s.held, [2]);
    }
}

#[test]
fn extra_ym2612_cancel_csm_guards_manual_csm_source_and_untracked_manual_edit() {
    for engine in ["ymfm", "nuked"] {
        for tracked in [false, true] {
            let shared = Shared::new(48_000, 1);
            let mut renderer = Renderer::new(48_000).unwrap();
            shared
                .create_extra_ym2612_engine(1, None, Some(engine))
                .unwrap();
            audition(&shared, 1, 2, 99, "on", fm(2, 0x22, 0x69, 0xc0));
            render(&mut renderer, &shared, 1000);
            if tracked {
                let mut enable = write(0, 0x27, 0x85);
                enable["audition"] = json!(true);
                shared.enqueue(1, &[command(1, 0, enable)]).unwrap();
            } else {
                audition(&shared, 1, 2, 99, "edit", vec![write(0, 0x27, 0x85)]);
            }
            render(&mut renderer, &shared, 1);
            shared
                .enqueue(1, &[command(1, 0, json!({"method":"cancel-csm"}))])
                .unwrap();
            render(&mut renderer, &shared, 1000);
            let guarded = snapshot(&shared, 1);
            assert_eq!(guarded.registers[0][0x27] & 0x81, 0x81);
            assert_eq!(guarded.key_masks[2], 15);
            assert_eq!(guarded.held, [2]);
            shared
                .enqueue(
                    1,
                    &[command(
                        1,
                        0,
                        json!({"method":"cancel-csm","audition":true}),
                    )],
                )
                .unwrap();
            render(&mut renderer, &shared, 1000);
            let cleared = snapshot(&shared, 1);
            assert_eq!(cleared.registers[0][0x27] & 0x81, 0);
            assert_eq!(cleared.key_masks[2], 15);
            assert_eq!(cleared.held, [2]);
        }
    }
}

#[test]
fn extra_ym2612_engine_defaults_validation_and_owned_metadata_are_atomic() {
    let shared = Shared::new(48_000, 1);
    assert_eq!(
        shared.create_extra_ym2612(1, None).unwrap().ym2612.engine,
        ym2612::Engine::Ymfm
    );
    let counts = shared.extra_psg.counts();
    let budget = shared.extra_psg.memory_used();
    let pending = shared.pending.load(Ordering::Acquire);
    let sequence = *shared.producer.lock().unwrap();
    for invalid in ["", "Nuked", "nuked ", "genesis-nuked", "unsupported"] {
        assert!(shared
            .create_extra_ym2612_engine(2, None, Some(invalid))
            .unwrap_err()
            .contains("engine must be"));
        assert_eq!(shared.extra_psg.counts(), counts);
        assert_eq!(shared.extra_psg.memory_used(), budget);
        assert_eq!(shared.pending.load(Ordering::Acquire), pending);
        assert_eq!(*shared.producer.lock().unwrap(), sequence);
    }
    assert_eq!(
        shared
            .create_extra_ym2612_engine(2, None, Some("nuked"))
            .unwrap()
            .ym2612
            .engine,
        ym2612::Engine::Nuked
    );
    let mut renderer = Renderer::new(48_000).unwrap();
    for id in [1, 2] {
        shared
            .enqueue(
                1,
                &[
                    command(id, 0, json!({"method":"reset"})),
                    command(id, 0, json!({"method":"clock","hz":8_000_000})),
                ],
            )
            .unwrap();
    }
    render(&mut renderer, &shared, 128);
    for (id, engine) in [(1, "ymfm"), (2, "nuked")] {
        let status = serde_json::to_value(shared.extra_ym2612_status(id, None).unwrap()).unwrap();
        assert_eq!(status["ym2612"]["engine"], engine);
        assert_eq!(status["ym2612"]["clock"], 8_000_000);
    }
}

#[test]
fn extra_ym2612_ymfm_nuked_mixed_output_equals_sum_of_independent_engines() {
    let shared = Shared::new(48_000, 1);
    let mut renderer = Renderer::new(48_000).unwrap();
    let mut separate = Vec::new();
    for (id, engine, channel, pan) in [(1, "ymfm", 0, 0x80), (2, "nuked", 5, 0x40)] {
        shared
            .create_extra_ym2612_engine(id, None, Some(engine))
            .unwrap();
        start(&shared, id, channel, pan, None);
        let single = Shared::new(48_000, 1);
        let mut renderer = Renderer::new(48_000).unwrap();
        single
            .create_extra_ym2612_engine(1, None, Some(engine))
            .unwrap();
        start(&single, 1, channel, pan, None);
        separate.push(render(&mut renderer, &single, 4096));
    }
    let combined = render(&mut renderer, &shared, 4096);
    assert!(separate.iter().all(|v| energy(v) > 1.));
    assert!(separate[0]
        .iter()
        .zip(&separate[1])
        .any(|(a, b)| (a[0] - b[0]).abs() > 1e-5));
    for ((a, b), actual) in separate[0].iter().zip(&separate[1]).zip(&combined) {
        for channel in 0..2 {
            assert!((a[channel] + b[channel] - actual[channel]).abs() < 1e-7);
        }
    }
    // Selecting one instance's reset/clock/disposal cannot change the other's identity.
    shared
        .enqueue(1, &[command(1, 0, json!({"method":"reset"}))])
        .unwrap();
    shared.dispose_extra_ym2612(1).unwrap();
    render(&mut renderer, &shared, 128);
    assert_eq!(snapshot(&shared, 2).engine, ym2612::Engine::Nuked);
    assert_eq!(snapshot(&shared, 2).key_masks[5], 15);
    assert_eq!(snapshot(&shared, 2).clock, ym2612::CLOCK);
}
