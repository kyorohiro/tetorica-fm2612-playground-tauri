//! Primary WonderSwan callback scheduling, using the actual fixed SDK chip.
use super::*;
use serde_json::{json, Value};

fn command(frame: u64, message: Value) -> Command {
    serde_json::from_value(json!({"type":"wonderswan","frame":frame,"message":message})).unwrap()
}
fn send(s: &Shared, messages: Vec<Value>) {
    s.enqueue(
        1,
        &messages
            .into_iter()
            .map(|m| command(0, m))
            .collect::<Vec<_>>(),
    )
    .unwrap();
}
fn render(s: &Shared, r: &mut Renderer, frames: usize) -> Vec<[f32; 2]> {
    r.prepare(s);
    let pcm = (0..frames).map(|_| r.sample(s)).collect();
    r.publish_primary(s);
    pcm
}
fn write(address: u32, value: u8) -> Value {
    json!({"method":"write","address":address,"value":value})
}
fn note(channel: u8, voice: u64, audition: bool) -> Value {
    let mut commands:Vec<_>=(0..16).map(|i|json!({"method":"memory","address":u32::from(channel)*16+i,"value":if i<8 {0x11}else{0xee},"channel":channel})).collect();
    commands.extend([
        write(0x80 + u32::from(channel) * 2, 0),
        write(0x81 + u32::from(channel) * 2, 7),
        write(0x88 + u32::from(channel), 0xff),
        json!({"method":"write","address":0x90,"value":1u8<<channel,"controlMask":1u8<<channel}),
    ]);
    json!({"method":"batch","channel":channel,"voice":voice,"audition":audition,"action":"on","commands":commands})
}
fn setup() -> (Shared, Renderer) {
    let s = Shared::new_modes(48000, 1, false, false, false, Some("wonderswan"));
    let r = Renderer::new_modes(48000, false, false, false, Some("wonderswan")).unwrap();
    send(&s, vec![write(0x91, 8)]);
    (s, r)
}
#[test]
fn wonderswan_primary_mode_owns_only_requested_core_and_serializes_direct_status() {
    assert_eq!(
        NativeGenesis::mode_config(Some("wonderswan")).unwrap(),
        (false, false, false, Some("wonderswan"))
    );
    let (s, mut r) = setup();
    assert!(r.core.0.is_null());
    assert!(r.huc6280.is_none());
    assert!(r.wonderswan.is_some());
    send(&s, vec![note(0, 1, false)]);
    let pcm = render(&s, &mut r, 4096);
    assert!(pcm.iter().any(|p| p[0].abs() > 0.01));
    assert_eq!(s.status().mode, "wonderswan");
    let status = serde_json::to_value(s.wonderswan.as_ref().unwrap().status(7).unwrap()).unwrap();
    assert_eq!(status["registers"][16], 1);
    assert_eq!(status["memory"][0], 17);
    assert_eq!(status["frame"], 7);
    assert!(status.get("wonderswan").is_none());
}
#[test]
fn wonderswan_primary_manual_lease_blocks_stale_program_and_wrong_voice_release() {
    let (s, mut r) = setup();
    send(&s, vec![note(0, 9, true)]);
    render(&s, &mut r, 100);
    send(
        &s,
        vec![
            note(0, 10, false),
            write(0x90, 0),
            json!({"method":"batch","channel":0,"voice":8,"audition":true,"action":"off","commands":[{"method":"key-off","channel":0}]}),
        ],
    );
    render(&s, &mut r, 1);
    let a = s.wonderswan.as_ref().unwrap().status(0).unwrap();
    assert_eq!(a.wonderswan.held, [0]);
    assert_eq!(a.wonderswan.registers[16] & 1, 1);
    send(
        &s,
        vec![
            json!({"method":"batch","channel":0,"voice":9,"audition":true,"action":"off","commands":[{"method":"key-off","channel":0}]}),
        ],
    );
    render(&s, &mut r, 1);
    assert!(s
        .wonderswan
        .as_ref()
        .unwrap()
        .status(0)
        .unwrap()
        .wonderswan
        .held
        .is_empty());
}
#[test]
fn wonderswan_primary_worker_cancel_releases_only_its_channel() {
    let (s, mut r) = setup();
    s.performance("start", 7, None).unwrap();
    send(&s, vec![note(0, 1, true)]);
    let mut c = command(0, note(1, 2, false));
    c.performance = Some(7);
    c.owner = Some(1);
    s.enqueue(1, &[c]).unwrap();
    render(&s, &mut r, 100);
    s.performance("cancel-owner", 7, Some(1)).unwrap();
    render(&s, &mut r, 1);
    let a = s.wonderswan.as_ref().unwrap().status(0).unwrap();
    assert_eq!(a.wonderswan.registers[16] & 15, 1);
    assert_eq!(a.wonderswan.held, [0]);
}
#[test]
fn wonderswan_primary_mixed_family_enqueue_fails_before_applied_register_change() {
    let (s, mut r) = setup();
    let other: Command =
        serde_json::from_value(json!({"type":"write","frame":0,"register":40,"value":0})).unwrap();
    assert!(s
        .enqueue(1, &[command(0, note(0, 1, false)), other])
        .is_err());
    render(&s, &mut r, 1);
    assert_eq!(
        s.wonderswan
            .as_ref()
            .unwrap()
            .status(0)
            .unwrap()
            .wonderswan
            .registers[16]
            & 15,
        0
    );
}

fn compact_trace() -> Value {
    json!({"version":1,"clock":3072000,"durationSamples":20,
        "events":[[0,0,0x91,8],[0,0,0x94,15],[0,0,0x90,34],[0,1,7,165],[0,2,0,6,3,4],[4,0,0x89,99]],
        "pcmData":[0,10,2,20,2,30]})
}
#[test]
fn wonderswan_primary_compact_timeline_matches_exact_sample_writes_at_both_rates() {
    for rate in [44100, 48000] {
        let a = Shared::new_modes(rate, 1, false, false, false, Some("wonderswan"));
        let b = Shared::new_modes(rate, 1, false, false, false, Some("wonderswan"));
        let mut ra = Renderer::new_modes(rate, false, false, false, Some("wonderswan")).unwrap();
        let mut rb = Renderer::new_modes(rate, false, false, false, Some("wonderswan")).unwrap();
        send(
            &a,
            vec![json!({"method":"prepare-timeline","trace":compact_trace()})],
        );
        render(&a, &mut ra, 1);
        render(&b, &mut rb, 1);
        a.enqueue(
            1,
            &[command(300, json!({"method":"play-timeline","token":8}))],
        )
        .unwrap();
        let frame = |sample: u64| 300 + (sample * u64::from(rate)).div_ceil(44100);
        let mut expected = vec![
            command(300, write(0x91, 8)),
            command(300, write(0x94, 15)),
            command(300, write(0x90, 34)),
            command(300, json!({"method":"memory","address":7,"value":165})),
        ];
        for (sample, value) in [(0, 10), (2, 20), (4, 30), (4, 99)] {
            expected.push(command(frame(sample), write(0x89, value)));
        }
        expected.push(command(frame(20), write(0x90, 32)));
        b.enqueue(1, &expected).unwrap();
        let actual = render(&a, &mut ra, 1000);
        let reference = render(&b, &mut rb, 1000);
        assert_eq!(actual, reference, "compact PCM changed at {rate} Hz");
        let state = a.wonderswan.as_ref().unwrap().status(0).unwrap().wonderswan;
        assert_eq!(state.memory[7], 165);
        assert_eq!(state.registers[9], 99);
        assert!(state.timeline.ended);
        assert!(!state.timeline.playing);
        assert_eq!(state.timeline.token, 8);
    }
}
#[test]
fn wonderswan_primary_compact_timeline_owner_cancel_retires_future_play_and_active_pcm() {
    let (s, mut r) = setup();
    send(
        &s,
        vec![json!({"method":"prepare-timeline","trace":compact_trace(),"group":9})],
    );
    render(&s, &mut r, 1);
    s.enqueue(
        1,
        &[
            command(500, json!({"method":"play-timeline","token":1,"group":9})),
            command(0, json!({"method":"stop-timeline","group":9})),
        ],
    )
    .unwrap();
    render(&s, &mut r, 600);
    let state = s.wonderswan.as_ref().unwrap().status(0).unwrap().wonderswan;
    assert_eq!(state.timeline.token, 0);
    assert!(!state.timeline.playing);
    assert_eq!(state.registers[16] & 15, 0);
    s.performance("start", 7, None).unwrap();
    let mut start = command(0, json!({"method":"play-timeline","token":2}));
    start.performance = Some(7);
    start.owner = Some(1);
    s.enqueue(1, &[start]).unwrap();
    render(&s, &mut r, 3);
    s.performance("cancel-owner", 7, Some(1)).unwrap();
    render(&s, &mut r, 1);
    let state = s.wonderswan.as_ref().unwrap().status(0).unwrap().wonderswan;
    assert!(state.timeline.cancelled);
    assert!(!state.timeline.playing);
    assert_eq!(state.registers[16] & 15, 0);
}
