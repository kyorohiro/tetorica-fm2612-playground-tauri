use super::*;

fn command(frame: u64, action: CommandAction) -> Command {
    Command {
        frame,
        performance: None,
        owner: None,
        action,
    }
}
fn write(frame: u64, register: u8, value: u8) -> Command {
    command(
        frame,
        CommandAction::Rf5c164Write {
            register,
            value,
            audition: false,
        },
    )
}
fn setup(frame: u64, channel: u8, pan: u8) -> Vec<Command> {
    vec![
        write(frame, 7, 0xc0 | channel),
        write(frame, 0, 128),
        write(frame, 1, pan),
        write(frame, 2, 0),
        write(frame, 3, 8),
        write(frame, 4, 0),
        write(frame, 5, 0),
        write(frame, 6, 0),
        command(
            frame,
            CommandAction::Rf5c164KeyOn {
                channel,
                voice: None,
            },
        ),
    ]
}
fn wave() -> Command {
    command(
        0,
        CommandAction::Rf5c164LoadMemory {
            offset: 0,
            data: vec![0x80, 0xc0, 0xfe, 0xc0, 0x80, 0x40, 0x7e, 0x40, 0xff],
        },
    )
}
fn render(renderer: &mut Renderer, shared: &Shared, frames: usize) -> Vec<[f32; 2]> {
    renderer.prepare(shared);
    let pcm = (0..frames).map(|_| renderer.sample(shared)).collect();
    shared.frame.store(renderer.frame, Ordering::Release);
    pcm
}
fn snapshot(shared: &Shared) -> serde_json::Value {
    serde_json::to_value(
        shared
            .rf5c164
            .status(shared.rate, shared.frame.load(Ordering::Acquire))
            .unwrap(),
    )
    .unwrap()
}

#[test]
fn native_megacd_is_opt_in_and_batch_admission_is_atomic() {
    let genesis = Shared::new(48000, 1);
    assert!(Renderer::new(48000).unwrap().rf5c164.is_none());
    assert!(genesis
        .enqueue(1, &[wave()])
        .unwrap_err()
        .contains("Mega CD"));
    assert_eq!(genesis.pending.load(Ordering::Acquire), 0);
    let shared = Shared::new_mode(48000, 2, true);
    assert!(shared.enqueue(2, &[wave(), write(0, 9, 0)]).is_err());
    assert!(shared.queue.is_empty());
    assert!(shared.rf_memory.lock().unwrap().is_empty());
    let uploads = [
        command(
            0,
            CommandAction::Rf5c164LoadMemory {
                offset: 0,
                data: vec![0; 40000],
            },
        ),
        command(
            0,
            CommandAction::Rf5c164LoadMemory {
                offset: 0,
                data: vec![0; 40000],
            },
        ),
    ];
    assert!(shared
        .enqueue(2, &uploads)
        .unwrap_err()
        .contains("upload limit"));
    assert!(shared.queue.is_empty());
    for hz in [99_999, 100_000_001] {
        assert!(shared
            .enqueue(2, &[command(0, CommandAction::Rf5c164Clock { hz })])
            .is_err());
    }
    assert!(shared
        .enqueue(
            2,
            &[command(
                0,
                CommandAction::Rf5c164KeyOn {
                    channel: 8,
                    voice: None
                }
            )]
        )
        .is_err());
    assert!(shared
        .enqueue(
            2,
            &[command(
                0,
                CommandAction::Rf5c164LoadMemory {
                    offset: 65535,
                    data: vec![1, 2]
                }
            )]
        )
        .is_err());
}

#[test]
fn native_megacd_applied_inspector_and_timing_follow_callback_not_admission() {
    for rate in [44100, 48000, 96000] {
        let shared = Shared::new_mode(rate, 1, true);
        let mut renderer = Renderer::new_mode(rate, true).unwrap();
        let mut events = vec![wave()];
        events.extend(setup(1000, 0, 0x0f));
        shared.enqueue(1, &events).unwrap();
        assert_eq!(snapshot(&shared)["channels"][0]["step"], 0);
        let before = render(&mut renderer, &shared, 1000);
        assert!(before.iter().all(|v| *v == [0.0; 2]));
        assert_eq!(snapshot(&shared)["mask"], 255);
        let signal = render(&mut renderer, &shared, 2000);
        assert!(signal.iter().any(|v| v[0].abs() > 0.01));
        assert!(signal.iter().all(|v| v[1] == 0.0));
        let applied = snapshot(&shared);
        assert_eq!(applied["clock"], rf5c164::CLOCK);
        assert_eq!(applied["channels"][0]["step"], 2048);
        assert_eq!(applied["channels"][0]["active"], true);
        assert_eq!(shared.late.load(Ordering::Acquire), 0);
        // Completed uploads are moved into a retirement queue; the callback
        // never releases the final allocation. A subsequent control admission
        // reclaims both references outside the callback.
        assert_eq!(shared.retired_rf.len(), 1);
        assert_eq!(shared.rf_memory.lock().unwrap().len(), 1);
        shared.enqueue(1, &[]).unwrap();
        assert!(shared.retired_rf.is_empty());
        assert!(shared.rf_memory.lock().unwrap().is_empty());
    }
}

#[test]
fn native_megacd_semantic_keys_preserve_other_channels_and_manual_replacements() {
    let shared = Shared::new_mode(48000, 1, true);
    let mut renderer = Renderer::new_mode(48000, true).unwrap();
    shared.performance("start", 5, None).unwrap();
    let mut events = vec![wave()];
    events.extend(setup(0, 0, 0x0f));
    for event in &mut events {
        event.performance = Some(5);
        event.owner = Some(1);
    }
    events.push(Command {
        frame: 5000,
        performance: Some(5),
        owner: Some(1),
        action: CommandAction::Rf5c164KeyOn {
            channel: 0,
            voice: None,
        },
    });
    shared.enqueue(1, &events).unwrap();
    render(&mut renderer, &shared, 512);
    shared.enqueue(1, &setup(0, 1, 0xf0)).unwrap();
    render(&mut renderer, &shared, 512);
    assert_eq!(snapshot(&shared)["mask"], 252);
    shared
        .enqueue(
            1,
            &[command(
                1200,
                CommandAction::Rf5c164KeyOff {
                    channel: 0,
                    voice: None,
                },
            )],
        )
        .unwrap();
    render(&mut renderer, &shared, 512);
    assert_eq!(snapshot(&shared)["mask"], 253);
    // A manual audition replaces CH0 ownership. Cancelling the old loop must
    // retain both manual channels and remove its future queued note.
    shared
        .enqueue(
            1,
            &[command(
                0,
                CommandAction::Rf5c164KeyOn {
                    channel: 0,
                    voice: None,
                },
            )],
        )
        .unwrap();
    render(&mut renderer, &shared, 128);
    shared.performance("cancel-owner", 5, Some(1)).unwrap();
    render(&mut renderer, &shared, 6000);
    assert_eq!(snapshot(&shared)["mask"], 252);
    assert_eq!(shared.pending.load(Ordering::Acquire), 0);
    // New owned CH0 is released by cancellation, while manual CH1 survives.
    shared.performance("start", 6, None).unwrap();
    shared
        .enqueue(
            1,
            &[Command {
                frame: 0,
                performance: Some(6),
                owner: Some(2),
                action: CommandAction::Rf5c164KeyOn {
                    channel: 0,
                    voice: None,
                },
            }],
        )
        .unwrap();
    render(&mut renderer, &shared, 128);
    shared.performance("cancel-owner", 6, Some(2)).unwrap();
    let sound = render(&mut renderer, &shared, 128);
    assert_eq!(snapshot(&shared)["mask"], 253);
    assert!(sound.iter().any(|v| v[1].abs() > 0.01));
}

#[test]
fn native_megacd_stop_preserves_ram_patch_clock_and_silences_future_commands() {
    let shared = Shared::new_mode(48000, 1, true);
    let mut renderer = Renderer::new_mode(48000, true).unwrap();
    let mut events = vec![
        wave(),
        command(0, CommandAction::Rf5c164Clock { hz: 10_000_000 }),
    ];
    events.extend(setup(0, 0, 0xff));
    events.push(command(
        9000,
        CommandAction::Rf5c164LoadMemory {
            offset: 0,
            data: vec![0; 9],
        },
    ));
    events.push(command(
        9000,
        CommandAction::Rf5c164KeyOn {
            channel: 0,
            voice: None,
        },
    ));
    shared.enqueue(1, &events).unwrap();
    assert!(render(&mut renderer, &shared, 1000)
        .iter()
        .any(|v| v[0].abs() > 0.01));
    shared.epoch.store(1, Ordering::Release);
    let stopped = render(&mut renderer, &shared, 10000);
    assert!(stopped[480..].iter().all(|v| *v == [0.0; 2]));
    assert_eq!(shared.stopped_epoch.load(Ordering::Acquire), 1);
    assert_eq!(shared.pending.load(Ordering::Acquire), 0);
    assert_eq!(snapshot(&shared)["mask"], 255);
    assert_eq!(snapshot(&shared)["channels"][0]["step"], 2048);
    assert_eq!(snapshot(&shared)["clock"], 10_000_000);
    shared
        .enqueue(
            1,
            &[command(
                0,
                CommandAction::Rf5c164KeyOn {
                    channel: 0,
                    voice: None,
                },
            )],
        )
        .unwrap();
    assert!(render(&mut renderer, &shared, 1000)
        .iter()
        .any(|v| v[0].abs() > 0.01));
    // Explicit RF reset clears registers while preserving waveform RAM.
    let mut events = vec![command(0, CommandAction::Rf5c164Reset)];
    events.extend(setup(0, 0, 0xff));
    shared.enqueue(1, &events).unwrap();
    assert!(render(&mut renderer, &shared, 1000)
        .iter()
        .any(|v| v[0].abs() > 0.01));
}

#[test]
fn native_megacd_mix_and_master_are_in_shared_output_path() {
    let shared = Shared::new_mode(48000, 1, true);
    let mut renderer = Renderer::new_mode(48000, true).unwrap();
    let mut events = vec![wave()];
    events.extend(setup(0, 0, 0xff));
    shared.enqueue(1, &events).unwrap();
    assert!(render(&mut renderer, &shared, 1000)
        .iter()
        .any(|v| v[0].abs() > 0.01));
    shared
        .enqueue(
            1,
            &[command(
                0,
                CommandAction::Rf5c164Mix {
                    left: 0.0,
                    right: 1.0,
                },
            )],
        )
        .unwrap();
    let pan = render(&mut renderer, &shared, 1000);
    assert!(pan[240..].iter().all(|v| v[0] == 0.0));
    assert!(pan[240..].iter().any(|v| v[1].abs() > 0.01));
    shared
        .enqueue(
            1,
            &[command(
                0,
                CommandAction::Mix {
                    fm_left: 1.0,
                    fm_right: 1.0,
                    psg_left: 1.0,
                    psg_right: 1.0,
                    master: 0.0,
                },
            )],
        )
        .unwrap();
    assert!(render(&mut renderer, &shared, 1000)[240..]
        .iter()
        .all(|v| *v == [0.0; 2]));
}

#[test]
fn native_megacd_stale_keyoff_uses_applied_voice_and_owner() {
    let shared = Shared::new_mode(48000, 1, true);
    let mut renderer = Renderer::new_mode(48000, true).unwrap();
    let mut events = vec![wave()];
    events.extend(setup(0, 0, 0xff));
    events.push(command(
        0,
        CommandAction::Rf5c164KeyOn {
            channel: 0,
            voice: Some(1),
        },
    ));
    events.push(command(
        1500,
        CommandAction::Rf5c164KeyOff {
            channel: 0,
            voice: Some(1),
        },
    ));
    shared.enqueue(1, &events).unwrap();
    render(&mut renderer, &shared, 512);
    shared.performance("start", 2, None).unwrap();
    shared
        .enqueue(
            1,
            &[Command {
                frame: 1000,
                performance: Some(2),
                owner: Some(1),
                action: CommandAction::Rf5c164KeyOn {
                    channel: 0,
                    voice: Some(2),
                },
            }],
        )
        .unwrap();
    render(&mut renderer, &shared, 1500);
    assert_eq!(
        snapshot(&shared)["mask"],
        254,
        "stale manual release must not cut newer Worker note"
    );
    // An owned future release likewise cannot cut a newer manual audition.
    shared
        .enqueue(
            1,
            &[
                Command {
                    frame: 3000,
                    performance: Some(2),
                    owner: Some(1),
                    action: CommandAction::Rf5c164KeyOff {
                        channel: 0,
                        voice: Some(2),
                    },
                },
                command(
                    0,
                    CommandAction::Rf5c164KeyOn {
                        channel: 0,
                        voice: Some(3),
                    },
                ),
            ],
        )
        .unwrap();
    render(&mut renderer, &shared, 1500);
    assert_eq!(snapshot(&shared)["mask"], 254);
    shared
        .enqueue(
            1,
            &[command(
                0,
                CommandAction::Rf5c164KeyOff {
                    channel: 0,
                    voice: Some(3),
                },
            )],
        )
        .unwrap();
    render(&mut renderer, &shared, 1);
    assert_eq!(snapshot(&shared)["mask"], 255);
    shared
        .enqueue(
            1,
            &[command(
                0,
                CommandAction::Rf5c164KeyOn {
                    channel: 0,
                    voice: Some(0),
                },
            )],
        )
        .unwrap_err();
}

#[test]
fn native_megacd_keyboard_base_is_applied_at_due_frame_and_survives_audition_and_stop() {
    let shared = Shared::new_mode(48000, 1, true);
    let mut renderer = Renderer::new_mode(48000, true).unwrap();
    let mut events = vec![wave()];
    events.extend(setup(0, 0, 0xff));
    events.push(write(2000, 2, 128));
    events.push(write(2000, 3, 16));
    shared.enqueue(1, &events).unwrap();
    render(&mut renderer, &shared, 512);
    assert_eq!(snapshot(&shared)["channels"][0]["configuredStep"], 2048);
    let audition = |register, value| {
        command(
            0,
            CommandAction::Rf5c164Write {
                register,
                value,
                audition: true,
            },
        )
    };
    shared
        .enqueue(1, &[audition(2, 0), audition(3, 32)])
        .unwrap();
    render(&mut renderer, &shared, 512);
    assert_eq!(snapshot(&shared)["channels"][0]["step"], 8192);
    assert_eq!(snapshot(&shared)["channels"][0]["configuredStep"], 2048);
    render(&mut renderer, &shared, 1000);
    assert_eq!(snapshot(&shared)["channels"][0]["configuredStep"], 4224);
    shared.epoch.store(1, Ordering::Release);
    render(&mut renderer, &shared, 1024);
    assert_eq!(snapshot(&shared)["channels"][0]["configuredStep"], 4224);
    assert_eq!(snapshot(&shared)["mask"], 255);
    let json = serde_json::json!({"type":"rf5c164-key-on","frame":0,"channel":0,"voice":123});
    assert!(matches!(
        serde_json::from_value::<Command>(json)
            .unwrap()
            .validate()
            .unwrap(),
        Action::RfKey(0, true, Some(123))
    ));
    let json = serde_json::json!({"type":"rf5c164-write","frame":0,"register":2,"value":7});
    assert!(matches!(
        serde_json::from_value::<Command>(json)
            .unwrap()
            .validate()
            .unwrap(),
        Action::RfWrite(2, 7, false)
    ));
    assert!(NativeGenesis::default()
        .start_mode(Some("unsupported-test-mode"))
        .is_err());
}

#[test]
fn native_megacd_common_fx_and_recording_receive_rf_output() {
    let reference = Shared::new_mode(48000, 1, true);
    let shared = Shared::new_mode(48000, 2, true);
    let mut dry = Renderer::new_mode(48000, true).unwrap();
    let mut wet = Renderer::new_mode(48000, true).unwrap();
    let mut events = vec![wave()];
    events.extend(setup(0, 0, 0xff));
    reference.enqueue(1, &events).unwrap();
    shared.enqueue(2, &events).unwrap();
    let message=serde_json::from_value::<FxMessage>(serde_json::json!({"op":"chain","children":[{"type":"gain","slot":0}],"units":[{"type":"gain","slot":0,"values":{"gain":0.5}}],"beatSeconds":0.5})).unwrap();
    let graph = shared
        .fx
        .lock()
        .unwrap()
        .command(message, 48000)
        .unwrap()
        .unwrap();
    shared.enqueue_special(Action::Fx(graph)).unwrap();
    shared.record_start(0.03).unwrap();
    let expected = render(&mut dry, &reference, 2048);
    let actual = render(&mut wet, &shared, 2048);
    for (dry, wet) in expected[1000..].iter().zip(&actual[1000..]) {
        for channel in 0..2 {
            assert!((wet[channel] - dry[channel] * 0.5).abs() < 1e-7);
        }
    }
    let recorded = shared.record_stop().unwrap();
    assert_eq!(recorded.frames, 1440);
    assert_eq!(
        recorded.channels[0],
        actual[..1440].iter().map(|v| v[0]).collect::<Vec<_>>()
    );
    assert_eq!(
        recorded.channels[1],
        actual[..1440].iter().map(|v| v[1]).collect::<Vec<_>>()
    );
}
