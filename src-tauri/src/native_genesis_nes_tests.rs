use super::*;
use serde_json::{json, Value};

fn command(frame: u64, message: Value) -> Command {
    serde_json::from_value(json!({"frame":frame,"type":"nes","message":message})).unwrap()
}
fn setup() -> (Shared, Renderer) {
    (
        Shared::new_config(48000, 1, false, false, true),
        Renderer::new_config(48000, false, false, true).unwrap(),
    )
}
fn render(shared: &Shared, renderer: &mut Renderer, frames: usize) -> Vec<[f32; 2]> {
    renderer.prepare(shared);
    let output = (0..frames).map(|_| renderer.sample(shared)).collect();
    shared.frame.store(renderer.frame, Ordering::Release);
    renderer.publish_primary(shared);
    output
}
fn snapshot(shared: &Shared) -> nes::Status {
    shared
        .nes
        .as_ref()
        .unwrap()
        .status(shared.frame.load(Ordering::Acquire))
        .unwrap()
}
fn pulse(channel: u8, period: u16, voice: u64, audition: bool) -> Value {
    let base = 0x4000 + channel as u16 * 4;
    json!({"method":"batch","channel":channel,"voice":voice,"audition":audition,"action":"on","commands":[
        {"method":"write","address":base,"value":0xbf},
        {"method":"write","address":base+1,"value":8},
        {"method":"write","address":base+2,"value":period&255},
        {"method":"enable","channel":channel,"enabled":true},
        {"method":"write","address":base+3,"value":8|(period>>8)}]})
}
fn off(channel: u8, voice: u64, audition: bool) -> Value {
    json!({"method":"batch","channel":channel,"voice":voice,"audition":audition,"action":"off","commands":[{"method":"enable","channel":channel,"enabled":false}]})
}
fn tagged(frame: u64, message: Value, owner: u32) -> Command {
    let mut c = command(frame, message);
    c.performance = Some(1);
    c.owner = Some(owner);
    c
}

#[test]
fn nes_native_mode_owns_only_nes_core_and_rejects_other_primary_commands() {
    let (shared, mut renderer) = setup();
    assert!(renderer.core.0.is_null());
    assert!(renderer.dx7.is_none());
    assert!(renderer.rf5c164.is_none());
    assert!(renderer.nes.is_some());
    assert_eq!(shared.status().mode, "nes");
    assert_eq!(shared.status().nes_clock, Some(nes::CLOCK));
    for value in [
        json!({"frame":0,"type":"write","register":40,"value":0}),
        json!({"frame":0,"type":"psg-write","value":128}),
        json!({"frame":0,"type":"dx7","message":{"method":"panic"}}),
        json!({"frame":0,"type":"rf5c164-reset"}),
    ] {
        let command: Command = serde_json::from_value(value).unwrap();
        assert!(shared.enqueue(1, &[command]).is_err());
    }
    assert!(render(&shared, &mut renderer, 128)
        .iter()
        .all(|v| *v == [0.0; 2]));
    let genesis = Shared::new(48000, 1);
    assert!(genesis
        .enqueue(1, &[command(0, json!({"method":"reset"}))])
        .is_err());
}

#[test]
fn nes_native_due_frame_batch_status_and_admission_are_atomic() {
    let (shared, mut renderer) = setup();
    let mut start = pulse(0, 253, 1, false);
    start["id"] = json!(10);
    shared.enqueue(1, &[command(500, start)]).unwrap();
    assert_eq!(snapshot(&shared).pending_commands, vec![10]);
    assert_eq!(snapshot(&shared).enabled, 0);
    render(&shared, &mut renderer, 500);
    assert_eq!(snapshot(&shared).enabled, 0);
    let audio = render(&shared, &mut renderer, 4096);
    let state = snapshot(&shared);
    assert_eq!(state.registers[0], 0xbf);
    assert_eq!(state.registers[2], 253);
    assert_eq!(state.enabled & 1, 1);
    assert!(state.pending_commands.is_empty());
    assert!(audio.iter().any(|s| s[0].abs() > 0.001));
    assert!(audio.iter().all(|s| s.iter().all(|v| v.is_finite())));
    let before = state.registers;
    assert!(shared.enqueue(1,&[command(0,json!({"id":11,"method":"write","address":16384,"value":1})),command(0,json!({"id":12,"method":"batch","channel":0,"action":"edit","commands":[{"method":"write","address":16388,"value":2}]}))]).is_err());
    assert_eq!(shared.pending.load(Ordering::Acquire), 0);
    render(&shared, &mut renderer, 1);
    assert_eq!(snapshot(&shared).registers, before);
}

#[test]
fn nes_native_manual_leases_block_future_patch_and_protect_global_enable_bits() {
    let (shared, mut renderer) = setup();
    shared.performance("start", 1, None).unwrap();
    shared
        .enqueue(
            1,
            &[
                tagged(500, json!({"method":"write","address":0x4000,"value":1}), 1),
                tagged(500, json!({"method":"write","address":0x4002,"value":7}), 1),
                tagged(500, json!({"method":"write","address":0x4015,"value":2}), 1),
                command(0, pulse(0, 253, 21, true)),
            ],
        )
        .unwrap();
    render(&shared, &mut renderer, 700);
    let state = snapshot(&shared);
    assert_eq!(state.registers[0], 0xbf);
    assert_eq!(state.registers[2], 253);
    assert_eq!(state.enabled & 3, 3);
    shared.performance("cancel-owner", 1, Some(1)).unwrap();
    render(&shared, &mut renderer, 1);
    assert_eq!(snapshot(&shared).enabled & 3, 1);
    shared
        .enqueue(
            1,
            &[
                command(0, pulse(0, 100, 22, true)),
                command(0, off(0, 21, true)),
            ],
        )
        .unwrap();
    render(&shared, &mut renderer, 1);
    assert_eq!(snapshot(&shared).enabled & 1, 1);
    assert_eq!(snapshot(&shared).registers[2], 100);
    shared.enqueue(1, &[command(0, off(0, 22, true))]).unwrap();
    render(&shared, &mut renderer, 1);
    assert_eq!(snapshot(&shared).enabled & 1, 0);
}

#[test]
fn nes_native_stale_performance_release_does_not_stop_new_channel_owner() {
    let (shared, mut renderer) = setup();
    shared.performance("start", 1, None).unwrap();
    shared
        .enqueue(
            1,
            &[
                tagged(0, pulse(0, 253, 1, false), 1),
                tagged(500, pulse(0, 100, 2, false), 2),
                tagged(700, off(0, 1, false), 1),
            ],
        )
        .unwrap();
    render(&shared, &mut renderer, 800);
    assert_eq!(snapshot(&shared).enabled & 1, 1);
    assert_eq!(snapshot(&shared).registers[2], 100);
    shared.performance("cancel-owner", 1, Some(1)).unwrap();
    render(&shared, &mut renderer, 1);
    assert_eq!(snapshot(&shared).enabled & 1, 1);
    shared.performance("cancel-owner", 1, Some(2)).unwrap();
    render(&shared, &mut renderer, 1);
    assert_eq!(snapshot(&shared).enabled & 1, 0);
}

#[test]
fn nes_native_completed_manual_dmc_is_not_restarted_by_other_channel_mask() {
    let (shared, mut renderer) = setup();
    shared.enqueue(1,&[command(0,json!({"method":"load-memory","address":0xc000,"data":[85]})),command(0,json!({"method":"batch","channel":4,"voice":55,"audition":true,"action":"on","commands":[{"method":"enable","channel":4,"enabled":false},{"method":"write","address":0x4010,"value":15},{"method":"write","address":0x4011,"value":64},{"method":"write","address":0x4012,"value":0},{"method":"write","address":0x4013,"value":0},{"method":"enable","channel":4,"enabled":true}]}))]).unwrap();
    render(&shared, &mut renderer, 4096);
    assert!(!snapshot(&shared).dmc_active);
    shared
        .enqueue(
            1,
            &[command(
                0,
                json!({"method":"write","address":0x4015,"value":1}),
            )],
        )
        .unwrap();
    render(&shared, &mut renderer, 1);
    assert!(!snapshot(&shared).dmc_active);
    assert_eq!(snapshot(&shared).enabled & 16, 0);
}

#[test]
fn nes_native_stop_keeps_ram_wave_modulation_patch_and_clock_reset_clears_ram() {
    let (shared, mut renderer) = setup();
    let mut ops = vec![
        json!({"method":"write","address":0x4023,"value":2}),
        json!({"method":"write","address":0x4083,"value":128}),
        json!({"method":"write","address":0x4089,"value":130}),
    ];
    for i in 0..64 {
        ops.push(json!({"method":"write","address":0x4040+i,"value":i}));
    }
    ops.push(json!({"method":"write","address":0x4089,"value":2}));
    ops.push(json!({"method":"write","address":0x4080,"value":155}));
    ops.push(json!({"method":"write","address":0x4087,"value":128}));
    for i in 0..32 {
        ops.push(json!({"method":"write","address":0x4088,"value":i%8}));
    }
    shared
        .enqueue(
            1,
            &[
                command(
                    0,
                    json!({"method":"load-memory","address":0xc000,"data":[1,2,3,4]}),
                ),
                command(
                    0,
                    json!({"method":"batch","channel":5,"action":"edit","commands":ops}),
                ),
                command(0, json!({"method":"clock","hz":2_000_000})),
                command(0, pulse(0, 253, 1, false)),
            ],
        )
        .unwrap();
    render(&shared, &mut renderer, 1024);
    let before = snapshot(&shared);
    assert_eq!(before.fds_gain, 27);
    assert_eq!(before.fds_master, 2);
    shared.epoch.fetch_add(1, Ordering::AcqRel);
    render(&shared, &mut renderer, 1024);
    let after = snapshot(&shared);
    assert_eq!(after.enabled, 0);
    assert!(!after.dmc_active);
    assert!(!after.fds_active);
    assert_eq!(after.clock, 2_000_000.0);
    assert_eq!(after.fds_gain, 27);
    assert_eq!(after.fds_master, 2);
    assert_eq!(&after.registers[0x40..0x80], &before.registers[0x40..0x80]);
    assert_eq!(after.modulation, before.modulation);
    assert_eq!(renderer.nes.as_ref().unwrap().memory_byte(0xc001), 2);
    assert_eq!(after.memory_ranges.len(), 1);
    assert!(render(&shared, &mut renderer, 1024)
        .iter()
        .all(|s| *s == [0.0; 2]));
    shared
        .enqueue(1, &[command(0, json!({"method":"reset"}))])
        .unwrap();
    render(&shared, &mut renderer, 1);
    let reset = snapshot(&shared);
    assert!(reset.memory_ranges.is_empty());
    assert_eq!(renderer.nes.as_ref().unwrap().memory_byte(0xc001), 0);
    assert_eq!(reset.clock, 2_000_000.0);
    assert!(reset.registers[0x40..0x80].iter().all(|v| *v == 0));
}

#[test]
fn nes_native_upload_retirement_guards_control_memory_for_apply_and_cancel() {
    let (shared, mut renderer) = setup();
    shared
        .enqueue(
            1,
            &[command(
                1000,
                json!({"id":1,"method":"load-memory","address":0,"data":vec![91;65536]}),
            )],
        )
        .unwrap();
    let event = shared.queue.pop().unwrap();
    let guard = match &*event.action.borrow() {
        Action::Nes(command) => command.clone(),
        _ => unreachable!(),
    };
    shared.queue.push(event).unwrap();
    render(&shared, &mut renderer, 1001);
    assert!(Arc::strong_count(&guard) >= 3);
    assert_eq!(renderer.nes.as_ref().unwrap().memory_byte(12345), 91);
    assert!(snapshot(&shared).pending_commands.is_empty());
    assert_eq!(Arc::strong_count(&guard), 1);
    shared
        .enqueue(
            1,
            &[command(
                5000,
                json!({"id":2,"method":"load-memory","address":0,"data":[42]}),
            )],
        )
        .unwrap();
    let event = shared.queue.pop().unwrap();
    let cancelled = match &*event.action.borrow() {
        Action::Nes(command) => command.clone(),
        _ => unreachable!(),
    };
    shared.queue.push(event).unwrap();
    shared.epoch.fetch_add(1, Ordering::AcqRel);
    render(&shared, &mut renderer, 1);
    assert!(Arc::strong_count(&cancelled) >= 3);
    assert_eq!(renderer.nes.as_ref().unwrap().memory_byte(0), 91);
    assert!(snapshot(&shared).pending_commands.is_empty());
    assert_eq!(Arc::strong_count(&cancelled), 1);
}

#[test]
fn nes_native_pcm_is_identical_across_callback_segmentation_and_master_mix_works() {
    let (a, mut left) = setup();
    let (b, mut right) = setup();
    a.enqueue(
        1,
        &[
            command(200, pulse(0, 253, 1, false)),
            command(6000, off(0, 1, false)),
        ],
    )
    .unwrap();
    b.enqueue(
        1,
        &[
            command(200, pulse(0, 253, 1, false)),
            command(6000, off(0, 1, false)),
        ],
    )
    .unwrap();
    let expected = render(&a, &mut left, 8192);
    let mut actual = Vec::new();
    for frames in [1, 127, 53, 512, 31, 1024, 4096, 2348] {
        actual.extend(render(&b, &mut right, frames));
    }
    assert_eq!(expected, actual);
    let (shared, mut renderer) = setup();
    shared
        .enqueue(1, &[command(0, pulse(0, 253, 1, false))])
        .unwrap();
    assert!(render(&shared, &mut renderer, 4096)
        .iter()
        .any(|s| s[0].abs() > 0.001));
    let mix: Command = serde_json::from_value(
        json!({"frame":0,"type":"mix","fmLeft":1,"fmRight":1,"psgLeft":1,"psgRight":1,"master":0}),
    )
    .unwrap();
    shared.enqueue(1, &[mix]).unwrap();
    let muted = render(&shared, &mut renderer, 512);
    assert!(muted[300..].iter().all(|s| *s == [0.0; 2]));
    assert!(renderer.core.0.is_null());
}

#[test]
fn nes_native_explicit_stop_clears_manual_and_older_nes_future_only() {
    let (shared, mut renderer) = setup();
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
    let mut older = pulse(0, 253, 2, false);
    older["id"] = json!(41);
    shared
        .enqueue(
            1,
            &[
                command(0, pulse(0, 253, 1, true)),
                command(1000, older),
                sample,
            ],
        )
        .unwrap();
    render(&shared, &mut renderer, 256);
    assert_eq!(snapshot(&shared).enabled & 1, 1);
    let mut newer = pulse(1, 100, 3, false);
    newer["id"] = json!(43);
    shared
        .enqueue(
            1,
            &[
                command(0, json!({"id":42,"method":"stop-voices"})),
                command(2200, newer),
            ],
        )
        .unwrap();
    render(&shared, &mut renderer, 800);
    render(&shared, &mut renderer, 1);
    let state = snapshot(&shared);
    assert_eq!(state.enabled, 0);
    assert_eq!(state.pending_commands, vec![43]);
    let common = render(&shared, &mut renderer, 900);
    assert!(common.iter().any(|sample| sample[0] > 0.1));
    render(&shared, &mut renderer, 300);
    assert_eq!(snapshot(&shared).enabled & 3, 2);
}

#[test]
fn nes_native_scoped_stop_keeps_other_owner_and_its_accepted_future() {
    let (shared, mut renderer) = setup();
    shared.performance("start", 1, None).unwrap();
    shared
        .enqueue(
            1,
            &[
                tagged(0, pulse(0, 253, 1, false), 1),
                tagged(0, pulse(1, 100, 2, false), 2),
                tagged(1000, pulse(0, 150, 3, false), 1),
                tagged(1200, pulse(1, 80, 4, false), 2),
            ],
        )
        .unwrap();
    render(&shared, &mut renderer, 256);
    shared
        .enqueue(1, &[tagged(0, json!({"method":"stop-voices"}), 1)])
        .unwrap();
    render(&shared, &mut renderer, 1500);
    assert_eq!(snapshot(&shared).enabled & 3, 2);
    assert_eq!(snapshot(&shared).registers[6], 80);
}

#[test]
fn nes_native_scoped_stop_preserves_unowned_edit_enabled_voice() {
    let (shared, mut renderer) = setup();
    shared.performance("start", 1, None).unwrap();
    // An inspector edit preserves ownership; enabling a previously unowned
    // channel through such an edit deliberately leaves no script lease.
    let unowned = json!({"method":"batch","channel":1,"action":"edit","commands":[
        {"method":"write","address":0x4004,"value":191},
        {"method":"write","address":0x4005,"value":8},
        {"method":"write","address":0x4006,"value":100},
        {"method":"enable","channel":1,"enabled":true},
        {"method":"write","address":0x4007,"value":8}
    ]});
    shared
        .enqueue(
            1,
            &[tagged(0, pulse(0, 253, 1, false), 1), command(0, unowned)],
        )
        .unwrap();
    render(&shared, &mut renderer, 512);
    assert_eq!(snapshot(&shared).enabled & 3, 3);
    shared
        .enqueue(1, &[tagged(0, json!({"method":"stop-voices"}), 1)])
        .unwrap();
    let audio = render(&shared, &mut renderer, 512);
    assert_eq!(snapshot(&shared).enabled & 3, 2);
    assert!(audio.iter().any(|sample| sample[0].abs() > 0.001));
}

#[test]
fn nes_native_fds_configured_gain_edits_are_atomic_while_silent_and_lease_guarded() {
    let (shared, mut renderer) = setup();
    let edit = |gain, voice, audition| json!({"method":"batch","channel":5,"voice":voice,"audition":audition,"action":"edit","fdsGain":gain,"fdsMaster":2,"commands":[{"method":"write","address":0x4080,"value":128}]});
    shared
        .enqueue(1, &[command(0, edit(19, Value::Null, false))])
        .unwrap();
    render(&shared, &mut renderer, 1);
    assert_eq!(snapshot(&shared).fds_gain, 19);
    assert!(!snapshot(&shared).fds_active);
    shared.enqueue(1,&[command(0,json!({"method":"batch","channel":5,"voice":77,"audition":true,"action":"on","fdsGain":19,"commands":[{"method":"write","address":0x4023,"value":2},{"method":"write","address":0x4080,"value":147},{"method":"write","address":0x4082,"value":0},{"method":"write","address":0x4083,"value":68}]})),command(0,edit(7,json!(66),true))]).unwrap();
    render(&shared, &mut renderer, 1);
    assert_eq!(snapshot(&shared).fds_gain, 19);
    assert!(snapshot(&shared).fds_active);
    shared
        .enqueue(1, &[command(0, edit(23, json!(77), true))])
        .unwrap();
    render(&shared, &mut renderer, 1);
    assert_eq!(snapshot(&shared).fds_gain, 23);
}

#[test]
fn nes_native_ui_applied_barrier_waits_for_callback_without_holding_control_registry() {
    let (shared, mut renderer) = setup();
    let shared = Arc::new(shared);
    shared
        .enqueue(
            1,
            &[command(
                0,
                json!({"id":101,"method":"write","address":0x4000,"value":191}),
            )],
        )
        .unwrap();
    let callback = shared.clone();
    let thread = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(10));
        render(&callback, &mut renderer, 1);
    });
    shared.nes.as_ref().unwrap().wait_applied(101).unwrap();
    assert_eq!(snapshot(&shared).registers[0], 191);
    thread.join().unwrap();
    // Status collection already retired the command; an acknowledged barrier
    // remains valid without retaining completed command allocations forever.
    shared.nes.as_ref().unwrap().wait_applied(101).unwrap();
    assert!(shared.nes.as_ref().unwrap().wait_applied(102).is_err());
}
