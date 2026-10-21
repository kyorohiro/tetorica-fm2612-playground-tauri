use super::*;
#[test]
fn dx7_native_primary_has_no_genesis_chips() {
    let shared = Shared::new_primary(48000, 1, false, true);
    let renderer = Renderer::new_primary(48000, false, true).unwrap();
    assert!(renderer.core.0.is_null());
    assert!(renderer.rf5c164.is_none());
    assert_eq!(shared.status().mode, "dx7");
    assert_eq!(
        shared.dx7.as_ref().unwrap().status(0).unwrap().voices[0],
        dx7::init_voice()
    );
}
fn command(frame: u64, message: serde_json::Value) -> Command {
    serde_json::from_value(serde_json::json!({"frame":frame,"type":"dx7","message":message}))
        .unwrap()
}
fn render(shared: &Shared, renderer: &mut Renderer, frames: usize) -> Vec<[f32; 2]> {
    renderer.prepare(shared);
    let output = (0..frames).map(|_| renderer.sample(shared)).collect();
    shared.frame.store(renderer.frame, Ordering::Release);
    output
}
fn setup() -> (Shared, Renderer) {
    (
        Shared::new_primary(48000, 1, false, true),
        Renderer::new_primary(48000, false, true).unwrap(),
    )
}
fn snapshot(shared: &Shared) -> dx7::Status {
    shared
        .dx7
        .as_ref()
        .unwrap()
        .status(shared.frame.load(Ordering::Acquire))
        .unwrap()
}
#[test]
fn dx7_native_timed_patches_pending_and_failed_batch_are_atomic() {
    let (shared, mut renderer) = setup();
    let mut patch = dx7::init_voice();
    patch[134] = 0;
    shared
        .enqueue(
            1,
            &[command(
                500,
                serde_json::json!({"id":1,"method":"set-preset","voice":patch.as_slice()}),
            )],
        )
        .unwrap();
    assert_eq!(snapshot(&shared).pending_commands, vec![1]);
    render(&shared, &mut renderer, 500);
    assert_eq!(snapshot(&shared).voices[0][134], 31);
    render(&shared, &mut renderer, 1);
    assert_eq!(snapshot(&shared).voices[0][134], 0);
    assert!(snapshot(&shared).pending_commands.is_empty());
    let valid = command(
        600,
        serde_json::json!({"id":2,"method":"select-voice","index":0}),
    );
    let invalid = command(
        600,
        serde_json::json!({"id":3,"method":"set-preset","voice":[255]}),
    );
    assert!(shared.enqueue(1, &[valid, invalid]).is_err());
    assert_eq!(shared.pending.load(Ordering::Acquire), 0);
    assert!(snapshot(&shared).pending_commands.is_empty());
}
#[test]
fn dx7_native_masked_edit_merges_applied_bank_at_due_frame() {
    let (shared, mut renderer) = setup();
    let mut first = dx7::init_voice();
    first[121] = 40;
    let mut second = dx7::init_voice();
    second[135] = 7;
    shared.enqueue(1,&[
        command(100,serde_json::json!({"id":1,"method":"edit-voice","voice":first.as_slice(),"indices":[121]})),
        command(200,serde_json::json!({"id":2,"method":"edit-voice","voice":second.as_slice(),"indices":[135]}))
    ]).unwrap();
    render(&shared, &mut renderer, 201);
    let state = snapshot(&shared);
    assert_eq!(state.voices[0][121], 40);
    assert_eq!(state.voices[0][135], 7);
}
#[test]
fn dx7_native_same_pitch_keyboard_and_performance_have_distinct_leases() {
    let (shared, mut renderer) = setup();
    shared.performance("start", 1, None).unwrap();
    let mut scheduled = command(
        0,
        serde_json::json!({"method":"note-on","note":60,"velocity":50,"voice":1}),
    );
    scheduled.performance = Some(1);
    scheduled.owner = Some(1);
    shared
        .enqueue(
            1,
            &[
                scheduled,
                command(
                    0,
                    serde_json::json!({"method":"note-on","note":60,"velocity":100,"voice":2}),
                ),
            ],
        )
        .unwrap();
    render(&shared, &mut renderer, 1000);
    assert_eq!(snapshot(&shared).active_notes.len(), 2);
    shared
        .enqueue(
            1,
            &[command(
                0,
                serde_json::json!({"method":"note-off","note":60,"voice":2}),
            )],
        )
        .unwrap();
    render(&shared, &mut renderer, 1);
    assert_eq!(snapshot(&shared).active_notes.len(), 1);
    shared.performance("cancel-owner", 1, Some(1)).unwrap();
    render(&shared, &mut renderer, 1);
    assert!(snapshot(&shared).active_notes.is_empty());
}
#[test]
fn dx7_native_audition_blocks_previously_queued_patch_and_same_pitch_note() {
    let (shared, mut renderer) = setup();
    let mut patch = dx7::init_voice();
    patch[134] = 0;
    shared.enqueue(1,&[
        command(500,serde_json::json!({"id":1,"method":"set-preset","voice":patch.as_slice()})),
        command(500,serde_json::json!({"id":2,"method":"note-on","note":60,"velocity":20,"voice":1})),
        command(0,serde_json::json!({"id":3,"method":"note-on","note":60,"velocity":100,"voice":2,"audition":true})),
        command(600,serde_json::json!({"id":4,"method":"all-notes-off"})),
        command(700,serde_json::json!({"id":5,"method":"panic"}))
    ]).unwrap();
    render(&shared, &mut renderer, 800);
    let state = snapshot(&shared);
    assert_eq!(state.voices[0][134], 31);
    assert_eq!(state.active_notes.len(), 1);
    assert!(state.pending_commands.is_empty());
    shared
        .enqueue(
            1,
            &[command(
                0,
                serde_json::json!({"method":"note-off","note":60,"voice":999,"audition":true}),
            )],
        )
        .unwrap();
    render(&shared, &mut renderer, 1);
    assert_eq!(snapshot(&shared).active_notes.len(), 1);
    shared
        .enqueue(
            1,
            &[command(
                0,
                serde_json::json!({"method":"note-off","note":60,"voice":2,"audition":true}),
            )],
        )
        .unwrap();
    render(&shared, &mut renderer, 1);
    assert!(snapshot(&shared).active_notes.is_empty());
}
#[test]
fn dx7_native_release_tails_and_stop_preserve_authored_bank() {
    let (shared, mut renderer) = setup();
    let mut patch = dx7::init_voice();
    patch[121] = 70;
    shared.enqueue(1,&[
        command(0,serde_json::json!({"method":"load-bank","voices":[patch.as_slice(),dx7::init_voice().as_slice()]})),
        command(0,serde_json::json!({"method":"note-on","note":60,"velocity":100,"voice":1})),
        command(4000,serde_json::json!({"method":"note-off","note":60,"voice":1}))
    ]).unwrap();
    let audio = render(&shared, &mut renderer, 5000);
    assert!(audio[4200..4500].iter().any(|v| v[0].abs() > 0.0001));
    shared.epoch.fetch_add(1, Ordering::AcqRel);
    let audio = render(&shared, &mut renderer, 1000);
    assert!(audio[600..].iter().all(|v| *v == [0.0; 2]));
    let state = snapshot(&shared);
    assert_eq!(state.voices.len(), 2);
    assert_eq!(state.voices[0][121], 70);
    assert!(state.active_notes.is_empty());
}
#[test]
fn dx7_native_cancellation_discards_pending_edits_and_retains_keyboard() {
    let (shared, mut renderer) = setup();
    shared.performance("start", 1, None).unwrap();
    let mut pending = command(
        1000,
        serde_json::json!({"id":1,"method":"select-voice","index":1}),
    );
    pending.performance = Some(1);
    pending.owner = Some(1);
    shared.enqueue(1,&[pending,command(0,serde_json::json!({"id":2,"method":"note-on","note":70,"velocity":100,"voice":2}))]).unwrap();
    render(&shared, &mut renderer, 1);
    shared.performance("cancel-owner", 1, Some(1)).unwrap();
    render(&shared, &mut renderer, 1);
    let state = snapshot(&shared);
    assert!(state.pending_commands.is_empty());
    assert_eq!(state.active_notes.len(), 1);
}
#[test]
fn dx7_native_wrong_mode_and_unsafe_voice_parameters_are_rejected() {
    let shared = Shared::new(48000, 1);
    assert!(shared
        .enqueue(
            1,
            &[command(
                0,
                serde_json::json!({"method":"note-on","note":60,"velocity":100})
            )]
        )
        .is_err());
    let (shared, _) = setup();
    let genesis: Command = serde_json::from_value(
        serde_json::json!({"frame":0,"type":"write","register":40,"value":240}),
    )
    .unwrap();
    assert!(shared.enqueue(1, &[genesis]).is_err());
    for message in [
        serde_json::json!({"method":"note-on","note":128}),
        serde_json::json!({"method":"note-on","note":60,"voice":0}),
        serde_json::json!({"method":"load-bank","voices":[]}),
        serde_json::json!({"method":"edit-voice","voice":dx7::init_voice().as_slice(),"indices":[1,1]}),
    ] {
        assert!(shared.enqueue(1, &[command(0, message)]).is_err());
    }
}
#[test]
fn dx7_native_retrigger_layers_release_together_and_stale_off_is_harmless() {
    let (shared, mut renderer) = setup();
    shared
        .enqueue(
            1,
            &[
                command(
                    0,
                    serde_json::json!({"method":"note-on","note":60,"voice":1}),
                ),
                command(
                    0,
                    serde_json::json!({"method":"note-on","note":60,"voice":2}),
                ),
            ],
        )
        .unwrap();
    render(&shared, &mut renderer, 1);
    assert_eq!(snapshot(&shared).active_notes.len(), 2);
    shared
        .enqueue(
            1,
            &[command(
                0,
                serde_json::json!({"method":"note-off","note":60,"voice":1}),
            )],
        )
        .unwrap();
    render(&shared, &mut renderer, 1);
    assert_eq!(snapshot(&shared).active_notes.len(), 2);
    shared
        .enqueue(
            1,
            &[command(
                0,
                serde_json::json!({"method":"note-off","note":60,"voice":2}),
            )],
        )
        .unwrap();
    render(&shared, &mut renderer, 1);
    assert!(snapshot(&shared).active_notes.is_empty());
}
#[test]
fn dx7_native_sixteen_voices_and_released_slot_reuse() {
    let (shared, mut renderer) = setup();
    let commands: Vec<_> = (0..17)
        .map(|index| {
            command(
                0,
                serde_json::json!({"method":"note-on","note":40+index,"voice":index+1}),
            )
        })
        .collect();
    shared.enqueue(1, &commands).unwrap();
    render(&shared, &mut renderer, 1);
    assert_eq!(snapshot(&shared).active_notes.len(), 16);
    shared
        .enqueue(
            1,
            &[
                command(
                    0,
                    serde_json::json!({"method":"note-off","note":40,"voice":1}),
                ),
                command(
                    0,
                    serde_json::json!({"method":"note-on","note":80,"voice":100}),
                ),
            ],
        )
        .unwrap();
    render(&shared, &mut renderer, 1);
    assert_eq!(snapshot(&shared).active_notes.len(), 16);
}
#[test]
fn dx7_native_mixer_matches_stereo_balance_and_ramps_parameters() {
    let (a, mut ra) = setup();
    let (b, mut rb) = setup();
    for shared in [&a, &b] {
        shared
            .enqueue(
                1,
                &[command(
                    0,
                    serde_json::json!({"method":"note-on","note":60,"velocity":100,"voice":1}),
                )],
            )
            .unwrap();
    }
    assert_eq!(render(&a, &mut ra, 1000), render(&b, &mut rb, 1000));
    a.enqueue(
        1,
        &[command(
            0,
            serde_json::json!({"method":"mix","volume":0.5,"pan":0}),
        )],
    )
    .unwrap();
    let changed = render(&a, &mut ra, 500);
    let original = render(&b, &mut rb, 500);
    for i in 0..500 {
        let gain = if i < 240 {
            1.0 - (i + 1) as f32 / 480.0
        } else {
            0.5
        };
        for channel in 0..2 {
            assert!((changed[i][channel] - original[i][channel] * gain).abs() < 0.000001);
        }
    }
    a.enqueue(
        1,
        &[command(
            0,
            serde_json::json!({"method":"mix","volume":0.5,"pan":1}),
        )],
    )
    .unwrap();
    let changed = render(&a, &mut ra, 500);
    let original = render(&b, &mut rb, 500);
    for i in 240..500 {
        assert_eq!(changed[i][0], 0.0);
        assert!((changed[i][1] - original[i][1] * 0.5).abs() < 0.000001);
    }
}
#[test]
fn dx7_native_control_allocations_are_reclaimed_only_after_callback_retirement() {
    let (shared, mut renderer) = setup();
    shared.enqueue(1,&[command(0,serde_json::json!({"id":1,"method":"set-preset","voice":dx7::init_voice().as_slice()}))]).unwrap();
    let event = shared.queue.pop().unwrap();
    let weak = match &*event.action.borrow() {
        Action::Dx7(prepared) => Arc::downgrade(prepared),
        _ => unreachable!(),
    };
    shared.queue.push(event).unwrap();
    assert_eq!(weak.strong_count(), 2); // Control registry plus event.
    render(&shared, &mut renderer, 1);
    assert_eq!(weak.strong_count(), 2); // Registry plus retirement queue, not freed by audio.
    snapshot(&shared); // IPC-side state read reclaims completed control data.
    assert_eq!(weak.strong_count(), 0);
}
