//! Entire Native callback pipeline, offline: no CPAL device or stream.
use super::*;
use serde_json::{json, Value};
const MODES: [&str; 3] = ["gameboy", "snes", "huc6280"];
fn command(kind: &str, frame: u64, message: Value) -> Command {
    serde_json::from_value(json!({"frame":frame,"type":kind,"message":message})).unwrap()
}
fn tagged(kind: &str, frame: u64, message: Value, owner: u32) -> Command {
    let mut c = command(kind, frame, message);
    c.performance = Some(1);
    c.owner = Some(owner);
    c
}
fn render(shared: &Shared, renderer: &mut Renderer, frames: usize) -> Vec<[f32; 2]> {
    renderer.prepare(shared);
    let output = (0..frames).map(|_| renderer.sample(shared)).collect();
    shared.frame.store(renderer.frame, Ordering::Release);
    renderer.publish_primary(shared);
    output
}
fn enabled(shared: &Shared) -> u8 {
    if let Some(chip) = &shared.gameboy {
        let s = chip.status(0).unwrap();
        s.gates
            .iter()
            .enumerate()
            .fold(0, |mask, (ch, on)| mask | if *on { 1 << ch } else { 0 })
    } else if let Some(chip) = &shared.snes {
        chip.status(0).unwrap().enabled
    } else {
        shared
            .huc6280
            .as_ref()
            .unwrap()
            .status(0)
            .unwrap()
            .registers
            .iter()
            .take(6)
            .enumerate()
            .fold(0, |mask, (ch, r)| {
                mask | if r[4] & 128 != 0 { 1 << ch } else { 0 }
            })
    }
}
fn pending(shared: &Shared) -> Vec<u64> {
    if let Some(chip) = &shared.gameboy {
        chip.status(0).unwrap().pending_commands
    } else if let Some(chip) = &shared.snes {
        chip.status(0).unwrap().pending_commands
    } else {
        shared
            .huc6280
            .as_ref()
            .unwrap()
            .status(0)
            .unwrap()
            .pending_commands
    }
}
fn setup(kind: &str) -> (Shared, Renderer) {
    let shared = Shared::new_modes(48000, 1, false, false, false, Some(kind));
    let mut renderer = Renderer::new_modes(48000, false, false, false, Some(kind)).unwrap();
    let mut messages = Vec::new();
    match kind {
        "gameboy" => {
            for (address, value) in [(22, 128), (20, 0x77), (21, 0xff)] {
                messages.push(json!({"method":"write","address":address,"value":value}));
            }
        }
        "huc6280" => {
            messages.push(json!({"method":"write","register":1,"value":255}));
            for channel in 0..2 {
                messages.push(json!({"method":"waveform","channel":channel,"samples":(0..32).collect::<Vec<_>>()}));
                messages.push(json!({"method":"write","channel":channel,"register":5,"value":255}));
            }
        }
        "snes" => {
            messages.push(json!({"method":"load-memory","address":0x400,"data":[0xb3,0x12,0x34,0x56,0x78,0x9a,0xbc,0xde,0xf0]}));
            messages.push(json!({"method":"load-memory","address":0,"data":[0,4,0,4]}));
            for (register, value) in [
                (0x6c, 0x20),
                (0x0c, 127),
                (0x1c, 127),
                (0x5d, 0),
                (0x5c, 255),
                (0x2c, 0),
                (0x3c, 0),
                (0x2d, 0),
                (0x3d, 0),
                (0x4d, 0),
            ] {
                messages.push(json!({"method":"write","register":register,"value":value}));
            }
            for ch in 0..2 {
                for (offset, value) in [
                    (0, 100),
                    (1, 100),
                    (2, 0),
                    (3, 16),
                    (4, 0),
                    (5, 0),
                    (6, 0),
                    (7, 127),
                ] {
                    messages.push(json!({"method":"write","register":ch*16+offset,"value":value}));
                }
            }
        }
        _ => unreachable!(),
    }
    shared
        .enqueue(
            1,
            &messages
                .into_iter()
                .map(|m| command(kind, 0, m))
                .collect::<Vec<_>>(),
        )
        .unwrap();
    render(&shared, &mut renderer, 1);
    (shared, renderer)
}
fn on(kind: &str, ch: u8, voice: u64, audition: bool) -> Value {
    let commands = match kind {
        "gameboy" => vec![
            json!({"method":"write","address":ch*5+1,"value":128}),
            json!({"method":"write","address":ch*5+2,"value":160}),
            json!({"method":"write","address":ch*5+3,"value":34}),
            json!({"method":"write","address":ch*5+4,"value":135}),
        ],
        "huc6280" => vec![
            json!({"method":"write","register":2,"value":254}),
            json!({"method":"write","register":3,"value":0}),
            json!({"method":"write","register":4,"value":152}),
        ],
        "snes" => vec![json!({"method":"key-on","channel":ch})],
        _ => unreachable!(),
    };
    json!({"method":"batch","channel":ch,"voice":voice,"audition":audition,"action":"on","commands":commands})
}
fn off(kind: &str, ch: u8, voice: u64, audition: bool) -> Value {
    let op = match kind {
        "gameboy" => json!({"method":"write","address":ch*5+2,"value":0}),
        "huc6280" => json!({"method":"key-off","channel":ch}),
        "snes" => json!({"method":"key-off","channel":ch}),
        _ => unreachable!(),
    };
    json!({"method":"batch","channel":ch,"voice":voice,"audition":audition,"action":"off","commands":[op]})
}
#[test]
fn consoles_native_modes_own_only_requested_core_and_reject_foreign_chip_batches_atomically() {
    for kind in MODES {
        let (shared, mut renderer) = setup(kind);
        assert!(renderer.core.0.is_null());
        assert!(renderer.nes.is_none() && renderer.dx7.is_none() && renderer.rf5c164.is_none());
        assert_eq!(renderer.gameboy.is_some(), kind == "gameboy");
        assert_eq!(renderer.snes.is_some(), kind == "snes");
        assert_eq!(renderer.huc6280.is_some(), kind == "huc6280");
        assert_eq!(shared.status().mode, kind);
        for value in [
            json!({"frame":0,"type":"write","register":40,"value":0}),
            json!({"frame":0,"type":"psg-write","value":128}),
            json!({"frame":0,"type":"dx7","message":{"method":"panic"}}),
        ] {
            let other: Command = serde_json::from_value(value).unwrap();
            assert!(shared
                .enqueue(1, &[command(kind, 0, on(kind, 0, 1, false)), other])
                .is_err());
            assert_eq!(shared.pending.load(Ordering::Acquire), 0);
        }
        assert!(render(&shared, &mut renderer, 128)
            .iter()
            .all(|s| *s == [0.0; 2]));
    }
}
#[test]
fn consoles_native_due_frame_notes_wake_audio_and_segmentation_does_not_change_pcm() {
    for kind in MODES {
        let (a, mut left) = setup(kind);
        let (b, mut right) = setup(kind);
        a.enqueue(
            1,
            &[
                command(kind, 200, on(kind, 0, 1, false)),
                command(kind, 6000, off(kind, 0, 1, false)),
            ],
        )
        .unwrap();
        b.enqueue(
            1,
            &[
                command(kind, 200, on(kind, 0, 1, false)),
                command(kind, 6000, off(kind, 0, 1, false)),
            ],
        )
        .unwrap();
        let expected = render(&a, &mut left, 8192);
        let mut actual = Vec::new();
        for frames in [1, 127, 53, 512, 31, 1024, 4096, 2348] {
            actual.extend(render(&b, &mut right, frames));
        }
        assert_eq!(expected, actual, "{kind}: callback segmentation");
        assert!(expected[..199].iter().all(|s| *s == [0.0; 2]));
        assert!(
            expected[512..4000].iter().any(|s| s[0].abs() > 0.001),
            "{kind}: first note must wake idle bus"
        );
        assert_eq!(enabled(&a) & 1, 0);
    }
}
#[test]
fn consoles_native_scoped_stop_cancels_older_same_owner_future_and_keeps_other_owner() {
    for kind in MODES {
        let (shared, mut renderer) = setup(kind);
        shared.performance("start", 1, None).unwrap();
        shared
            .enqueue(
                1,
                &[
                    tagged(kind, 0, on(kind, 0, 1, false), 1),
                    tagged(kind, 0, on(kind, 1, 2, false), 2),
                    tagged(kind, 1000, on(kind, 0, 3, false), 1),
                    tagged(kind, 1200, on(kind, 1, 4, false), 2),
                ],
            )
            .unwrap();
        render(&shared, &mut renderer, 256);
        assert_eq!(enabled(&shared) & 3, 3);
        shared
            .enqueue(1, &[tagged(kind, 0, json!({"method":"stop-voices"}), 1)])
            .unwrap();
        render(&shared, &mut renderer, 1500);
        assert_eq!(enabled(&shared) & 3, 2, "{kind}: scoped Stop");
        assert!(pending(&shared).is_empty());
        shared.performance("cancel-owner", 1, Some(1)).unwrap();
        render(&shared, &mut renderer, 1);
        assert_eq!(enabled(&shared) & 3, 2);
        shared.performance("cancel-owner", 1, Some(2)).unwrap();
        render(&shared, &mut renderer, 1);
        assert_eq!(enabled(&shared) & 3, 0);
    }
}
#[test]
fn consoles_native_global_stop_cancels_pending_releases_all_gates_and_new_note_restarts_same_session(
) {
    for kind in MODES {
        let (shared, mut renderer) = setup(kind);
        let mut future = on(kind, 1, 2, false);
        future["id"] = json!(41);
        shared
            .enqueue(
                1,
                &[
                    command(kind, 0, on(kind, 0, 1, true)),
                    command(kind, 5000, future),
                ],
            )
            .unwrap();
        render(&shared, &mut renderer, 512);
        assert_eq!(enabled(&shared) & 1, 1);
        shared.epoch.fetch_add(1, Ordering::AcqRel);
        render(&shared, &mut renderer, 1024);
        assert_eq!(enabled(&shared), 0);
        assert!(pending(&shared).is_empty());
        assert!(render(&shared, &mut renderer, 4096)
            .iter()
            .all(|s| *s == [0.0; 2]));
        assert_eq!(shared.pending.load(Ordering::Acquire), 0);
        shared
            .enqueue(1, &[command(kind, 0, on(kind, 0, 3, false))])
            .unwrap();
        assert!(
            render(&shared, &mut renderer, 2048)
                .iter()
                .any(|s| s[0].abs() > 0.001),
            "{kind}: restart after Stop"
        );
        assert_eq!(shared.session, 1);
    }
}
#[test]
fn consoles_native_stopvoices_does_not_cancel_shared_sample_bus_and_master_mute_applies() {
    for kind in MODES {
        let (shared, mut renderer) = setup(kind);
        shared
            .load_media(
                media::Kind::Sample,
                "common".into(),
                48000.0,
                vec![vec![0.25; 4096]],
            )
            .unwrap();
        let sample: Command = serde_json::from_value(
            json!({"frame":1500,"type":"sample-play","name":"common","voice":1}),
        )
        .unwrap();
        shared
            .enqueue(
                1,
                &[
                    command(kind, 0, on(kind, 0, 1, true)),
                    command(kind, 1000, on(kind, 1, 2, false)),
                    sample,
                ],
            )
            .unwrap();
        render(&shared, &mut renderer, 256);
        shared
            .enqueue(1, &[command(kind, 0, json!({"method":"stop-voices"}))])
            .unwrap();
        render(&shared, &mut renderer, 1200);
        assert_eq!(enabled(&shared), 0);
        let pcm = render(&shared, &mut renderer, 900);
        assert!(
            pcm.iter().any(|s| s[0] > 0.1),
            "{kind}: common samples survive chip cleanup"
        );
        let mix:Command=serde_json::from_value(json!({"frame":0,"type":"mix","fmLeft":1,"fmRight":1,"psgLeft":1,"psgRight":1,"master":0})).unwrap();
        shared.enqueue(1, &[mix]).unwrap();
        let muted = render(&shared, &mut renderer, 512);
        assert!(
            muted[300..].iter().all(|s| *s == [0.0; 2]),
            "{kind}: common final master"
        );
    }
}
