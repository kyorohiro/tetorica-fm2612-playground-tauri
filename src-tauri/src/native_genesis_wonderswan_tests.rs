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
