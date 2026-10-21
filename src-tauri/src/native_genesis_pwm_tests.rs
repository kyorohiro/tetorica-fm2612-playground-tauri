//! Integrated 32X native pipeline without opening an output device.
use super::*;
use serde_json::{json, Value};
fn command(frame: u64, message: Value) -> Command {
    serde_json::from_value(json!({"frame":frame,"type":"pwm","message":message})).unwrap()
}
fn tagged(frame: u64, message: Value, owner: u32) -> Command {
    let mut command = command(frame, message);
    command.performance = Some(1);
    command.owner = Some(owner);
    command
}
fn setup() -> (Shared, Renderer) {
    (
        Shared::new_modes(48000, 1, false, false, false, Some("32x")),
        Renderer::new_modes(48000, false, false, false, Some("32x")).unwrap(),
    )
}
fn render(shared: &Shared, renderer: &mut Renderer, frames: usize) -> Vec<[f32; 2]> {
    renderer.prepare(shared);
    let output = (0..frames).map(|_| renderer.sample(shared)).collect();
    shared.frame.store(renderer.frame, Ordering::Release);
    renderer.publish_primary(shared);
    output
}
fn snapshot(shared: &Shared) -> pwm::Status {
    shared
        .pwm
        .as_ref()
        .unwrap()
        .status(shared.frame.load(Ordering::Acquire))
        .unwrap()
}
fn configure(shared: &Shared, owner: Option<u32>) {
    let mut commands = vec![
        command(0, json!({"method":"write","register":0,"value":5})),
        command(0, json!({"method":"write","register":1,"value":4095})),
        command(0, json!({"method":"write","register":4,"value":2147})),
    ];
    if let Some(owner) = owner {
        for command in &mut commands {
            command.performance = Some(1);
            command.owner = Some(owner);
        }
    }
    shared.enqueue(1, &commands).unwrap();
}
#[test]
fn pwm_native_32x_keeps_genesis_core_and_rejects_foreign_chips_atomically() {
    let (shared, mut renderer) = setup();
    assert!(!renderer.core.0.is_null());
    assert!(renderer.pwm.is_some());
    assert!(renderer.rf5c164.is_none());
    assert_eq!(shared.status().mode, "32x");
    for value in [
        json!({"frame":0,"type":"rf5c164-reset"}),
        json!({"frame":0,"type":"snes","message":{"method":"reset"}}),
        json!({"frame":0,"type":"nes","message":{"method":"reset"}}),
    ] {
        let invalid: Command = serde_json::from_value(value).unwrap();
        assert!(shared
            .enqueue(
                1,
                &[
                    command(0, json!({"method":"write","register":1,"value":1000})),
                    invalid
                ]
            )
            .is_err());
    }
    assert_eq!(snapshot(&shared).registers[1], 0);
    let psg: Vec<Command> = [0x85, 0x10, 0x98]
        .into_iter()
        .map(|value| {
            serde_json::from_value(json!({"frame":0,"type":"psg-write","value":value})).unwrap()
        })
        .collect();
    shared.enqueue(1, &psg).unwrap();
    let audio = render(&shared, &mut renderer, 1024);
    assert!(audio.iter().any(|sample| sample[0].abs() > 0.001));
    let dac: Vec<Command> = [(0x2b, 0x80), (0x2a, 200)]
        .into_iter()
        .map(|(register, value)| {
            serde_json::from_value(
                json!({"frame":0,"type":"write","register":register,"value":value}),
            )
            .unwrap()
        })
        .collect();
    shared.enqueue(1, &dac).unwrap();
    configure(&shared, None);
    assert!(render(&shared, &mut renderer, 1024)
        .iter()
        .all(|sample| sample.iter().all(|v| v.is_finite())));
    assert!(snapshot(&shared).left.is_some());
    let genesis = Shared::new(48000, 2);
    assert!(genesis
        .enqueue(2, &[command(0, json!({"method":"reset"}))])
        .is_err());
}
#[test]
fn pwm_native_reservations_keep_absolute_frames_and_due_order() {
    let (shared, mut renderer) = setup();
    shared.enqueue(1,&[command(0,json!({"method":"schedule-writes","id":1,"baseFrame":600,"entries":[{"frame":0,"register":1,"value":1},{"frame":3,"register":2,"value":100},{"frame":3,"register":2,"value":200}]}))]).unwrap();
    render(&shared, &mut renderer, 1);
    let state = snapshot(&shared);
    assert_eq!(state.last_schedule.unwrap().base_frame, 600);
    assert_eq!(state.current_frame, 1);
    assert_eq!(state.queued_writes, 3);
    render(&shared, &mut renderer, 601);
    assert_eq!(snapshot(&shared).registers[1], 1);
    assert_eq!(snapshot(&shared).left_fifo.len(), 0);
    render(&shared, &mut renderer, 1);
    assert_eq!(snapshot(&shared).left_fifo, vec![100, 200]);
}
#[test]
fn pwm_native_clear_schedule_preserves_outer_future_writes() {
    let (shared, mut renderer) = setup();
    shared.enqueue(1,&[command(0,json!({"method":"schedule-writes","entries":[{"frame":500,"register":2,"value":99}]})),command(500,json!({"method":"write","register":2,"value":555}))]).unwrap();
    render(&shared, &mut renderer, 1);
    shared
        .enqueue(1, &[command(0, json!({"method":"clear-schedule"}))])
        .unwrap();
    render(&shared, &mut renderer, 501);
    assert_eq!(snapshot(&shared).left_fifo, vec![555]);
    assert_eq!(snapshot(&shared).queued_writes, 0);
}
#[test]
fn pwm_native_epoch_stop_clears_future_data_but_keeps_clock_cycle_routing() {
    let (shared, mut renderer) = setup();
    configure(&shared, None);
    shared.enqueue(1,&[command(0,json!({"method":"clock","hz":12000000})),command(0,json!({"method":"schedule-writes","entries":[{"frame":5000,"register":4,"value":999}]}))]).unwrap();
    let audio = render(&shared, &mut renderer, 1024);
    assert!(audio.iter().any(|v| v[0].abs() > 0.001));
    shared.epoch.fetch_add(1, Ordering::AcqRel);
    render(&shared, &mut renderer, 1024);
    let state = snapshot(&shared);
    assert_eq!(
        (state.clock, state.registers[0], state.registers[1]),
        (12000000, 5, 4095)
    );
    assert_eq!(state.queued_writes, 0);
    assert!(state.left.is_none() && state.right.is_none());
    assert!(render(&shared, &mut renderer, 6000)
        .iter()
        .all(|sample| *sample == [0.0; 2]));
}
#[test]
fn pwm_native_owner_cancellation_preserves_new_owner_and_invalidates_its_old_reservations() {
    let (shared, mut renderer) = setup();
    shared.performance("start", 1, None).unwrap();
    configure(&shared, Some(1));
    shared.enqueue(1,&[tagged(0,json!({"method":"schedule-writes","entries":[{"frame":5000,"register":4,"value":999}]}),1)]).unwrap();
    render(&shared, &mut renderer, 1024);
    shared
        .enqueue(
            1,
            &[tagged(
                0,
                json!({"method":"write","register":4,"value":2250}),
                2,
            )],
        )
        .unwrap();
    render(&shared, &mut renderer, 128);
    shared.performance("cancel-owner", 1, Some(1)).unwrap();
    let audio = render(&shared, &mut renderer, 6000);
    assert!(audio.iter().any(|sample| sample[0].abs() > 0.001));
    assert_eq!(snapshot(&shared).queued_writes, 0);
    assert_eq!(snapshot(&shared).left, Some(2250));
    shared.performance("cancel-owner", 1, Some(2)).unwrap();
    assert!(render(&shared, &mut renderer, 1024)
        .iter()
        .all(|sample| *sample == [0.0; 2]));
}
#[test]
fn pwm_native_common_master_and_pwm_mixer_are_applied() {
    let (shared, mut renderer) = setup();
    configure(&shared, None);
    render(&shared, &mut renderer, 1024);
    shared
        .enqueue(
            1,
            &[command(0, json!({"method":"mix","volume":0.5,"pan":-1}))],
        )
        .unwrap();
    let audio = render(&shared, &mut renderer, 1024);
    assert!(audio[1000][0].abs() > 0.001);
    assert_eq!(audio[1000][1], 0.0);
    let mute: Command = serde_json::from_value(
        json!({"frame":0,"type":"mix","fmLeft":1,"fmRight":1,"psgLeft":1,"psgRight":1,"master":0}),
    )
    .unwrap();
    shared.enqueue(1, &[mute]).unwrap();
    let audio = render(&shared, &mut renderer, 1024);
    assert_eq!(audio[1000], [0.0; 2]);
}
#[test]
fn pwm_native_worker_reset_before_same_frame_reserved_schedule_does_not_erase_it() {
    let (shared, mut renderer) = setup();
    shared.performance("start", 1, None).unwrap();
    shared.enqueue(1,&[
        tagged(500,json!({"method":"reset","order":1}),1),
        tagged(0,json!({"method":"schedule-writes","order":2,"id":1,"baseFrame":500,"entries":[{"frame":0,"register":0,"value":5},{"frame":0,"register":1,"value":1000},{"frame":0,"register":4,"value":650},{"frame":1000,"register":4,"value":750}]}),1),
    ]).unwrap();
    render(&shared, &mut renderer, 500);
    let before = snapshot(&shared);
    assert_eq!(before.registers[0], 0);
    assert_eq!(before.queued_writes, 4);
    let audio = render(&shared, &mut renderer, 600);
    let state = snapshot(&shared);
    assert_eq!(state.registers[0], 5);
    assert_eq!(state.registers[1], 1000);
    assert_eq!(state.left, Some(650));
    assert_eq!(state.queued_writes, 1);
    assert!(audio.iter().any(|sample| sample[0].abs() > 0.001));
    shared.enqueue(1,&[tagged(1200,json!({"method":"clear-schedule","order":3}),1),tagged(0,json!({"method":"schedule-writes","order":4,"id":2,"baseFrame":1200,"entries":[{"frame":0,"register":4,"value":850}]}),1)]).unwrap();
    render(&shared, &mut renderer, 300);
    assert_eq!(snapshot(&shared).left, Some(850));
    assert_eq!(snapshot(&shared).queued_writes, 0);
}
#[test]
fn pwm_native_full_32x_keeps_rf5c164_mixing_and_stop_reset() {
    let shared = Shared::new_modes(48000, 1, true, false, false, Some("32x"));
    let mut renderer = Renderer::new_modes(48000, true, false, false, Some("32x")).unwrap();
    assert!(renderer.pwm.is_some() && renderer.rf5c164.is_some() && !renderer.core.0.is_null());
    configure(&shared, None);
    let mut commands:Vec<Command>=vec![serde_json::from_value(json!({"frame":0,"type":"rf5c164-load-memory","offset":0,"data":[128,192,254,192,128,64,126,64,255]})).unwrap()];
    for (register, value) in [
        (7, 0xc0),
        (0, 128),
        (1, 255),
        (2, 0),
        (3, 8),
        (4, 0),
        (5, 0),
        (6, 0),
    ] {
        commands.push(
            serde_json::from_value(
                json!({"frame":0,"type":"rf5c164-write","register":register,"value":value}),
            )
            .unwrap(),
        );
    }
    commands.push(
        serde_json::from_value(json!({"frame":0,"type":"rf5c164-key-on","channel":0})).unwrap(),
    );
    shared.enqueue(1, &commands).unwrap();
    assert!(render(&shared, &mut renderer, 1024)
        .iter()
        .any(|sample| sample[0].abs() > 0.001));
    shared
        .enqueue(1, &[command(0, json!({"method":"mix","volume":0,"pan":0}))])
        .unwrap();
    let rf_only = render(&shared, &mut renderer, 1024);
    assert!(rf_only[512..].iter().any(|sample| sample[0].abs() > 0.001));
    shared.epoch.fetch_add(1, Ordering::AcqRel);
    render(&shared, &mut renderer, 1024);
    assert!(snapshot(&shared).left.is_none());
    let rf = serde_json::to_value(shared.rf5c164.status(48000, renderer.frame).unwrap()).unwrap();
    assert_eq!(rf["mask"], 255);
    shared
        .enqueue(
            1,
            &[serde_json::from_value(json!({"frame":0,"type":"reset"})).unwrap()],
        )
        .unwrap();
    render(&shared, &mut renderer, 1);
    assert_eq!(snapshot(&shared).registers[0], 0);
}
