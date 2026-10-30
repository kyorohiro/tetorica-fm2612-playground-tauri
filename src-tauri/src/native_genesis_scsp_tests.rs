//! Real SDK, scheduler and rack; no device is opened.
use super::*;
use serde_json::{json, Value};
fn write(r: u16, v: u16) -> Value {
    json!({"method":"write","register":r,"value":v})
}
fn send(s: &Shared, messages: Vec<Value>) {
    let c = messages
        .into_iter()
        .map(|m| serde_json::from_value(json!({"type":"scsp","frame":0,"message":m})).unwrap())
        .collect::<Vec<Command>>();
    s.enqueue(1, &c).unwrap();
}
fn render(s: &Shared, r: &mut Renderer, n: usize) -> Vec<[f32; 2]> {
    r.prepare(s);
    let out = (0..n).map(|_| r.sample(s)).collect();
    r.publish_primary(s);
    out
}
fn wave() -> Vec<u8> {
    (0..65)
        .flat_map(|i| {
            let v = (8192. * (std::f64::consts::TAU * (i % 64) as f64 / 64.).sin()).round() as i16;
            v.to_be_bytes()
        })
        .collect()
}
fn note(ch: u8, voice: u64, audition: bool) -> Value {
    let b = ch as u16 * 32;
    json!({"method":"batch","channel":ch,"voice":voice,"audition":audition,"action":"on","commands":[write(b,0x1020),write(b+2,4096),write(b+4,0),write(b+6,64),write(b+8,31),write(b+10,0x3c1f),write(b+12,0),write(b+16,0),write(b+22,0xe000),write(b,0x1820)]})
}
fn off(ch: u8, voice: u64, audition: bool) -> Value {
    json!({"method":"batch","channel":ch,"voice":voice,"audition":audition,"action":"off","commands":[write(ch as u16*32,0x1020)]})
}
fn setup() -> (Shared, Renderer) {
    let s = Shared::new_modes(48000, 1, false, false, false, Some("scsp"));
    let r = Renderer::new_modes(48000, false, false, false, Some("scsp")).unwrap();
    send(
        &s,
        vec![
            json!({"method":"load-memory","address":4096,"data":wave()}),
            write(0x400, 0x020f),
        ],
    );
    (s, r)
}
fn peak(a: &[[f32; 2]]) -> f32 {
    a.iter().flatten().fold(0., |m, v| m.max(v.abs()))
}
#[test]
fn scsp_primary_32_slots_stop_retains_ram_and_restarts() {
    assert_eq!(
        NativeGenesis::mode_config(Some("scsp")).unwrap(),
        (false, false, false, Some("scsp"))
    );
    let (s, mut r) = setup();
    assert!(r.core.0.is_null());
    for ch in 0..32 {
        send(&s, vec![note(ch, ch as u64 + 1, false)]);
        assert!(peak(&render(&s, &mut r, 128)) > 0.01);
    }
    assert_eq!(
        s.scsp.as_ref().unwrap().status(0).unwrap().scsp.enabled,
        0xffffffff
    );
    send(&s, vec![json!({"method":"stop-voices"})]);
    assert_eq!(peak(&render(&s, &mut r, 256)), 0.);
    send(&s, vec![note(31, 40, false)]);
    assert!(peak(&render(&s, &mut r, 256)) > 0.01);
    assert_eq!(s.status().mode, "scsp");
}
#[test]
fn scsp_manual_owner_stale_release_and_release_ram_pins() {
    let (s, mut r) = setup();
    send(&s, vec![note(31, 1, true)]);
    render(&s, &mut r, 128);
    send(&s, vec![off(31, 2, true), note(31, 3, false)]);
    render(&s, &mut r, 128);
    let a = s.scsp.as_ref().unwrap().status(0).unwrap();
    assert_eq!(a.scsp.held, vec![31]);
    assert_eq!(a.scsp.playing_addresses[31], Some(4096));
    assert_eq!(a.scsp.enabled, 1 << 31);
    send(&s, vec![off(31, 1, true)]);
    render(&s, &mut r, 128);
    send(
        &s,
        vec![json!({"id":50,"method":"load-memory","address":4096,"data":[0,0]})],
    );
    render(&s, &mut r, 1);
    assert!(s.scsp.as_ref().unwrap().wait_applied(50).is_err());
    send(
        &s,
        vec![
            json!({"method":"stop-voices"}),
            json!({"id":51,"method":"load-memory","address":4096,"data":[0,0]}),
        ],
    );
    render(&s, &mut r, 1);
    assert!(s.scsp.as_ref().unwrap().wait_applied(51).is_ok());
}
#[test]
fn scsp_extra_rack_actual_core_and_retirement() {
    let (s, mut r) = setup();
    s.create_extra_scsp(1).unwrap();
    render(&s, &mut r, 1);
    let commands = [
        json!({"method":"load-memory","address":4096,"data":wave()}),
        write(0x400, 0x020f),
        note(31, 1, false),
    ]
    .into_iter()
    .map(|m| {
        serde_json::from_value(json!({"type":"extra-scsp","instance":1,"frame":0,"message":m}))
            .unwrap()
    })
    .collect::<Vec<Command>>();
    s.enqueue(1, &commands).unwrap();
    assert!(peak(&render(&s, &mut r, 256)) > 0.01);
    assert_eq!(
        s.extra_scsp_status(1, None, false).unwrap().scsp.enabled,
        1 << 31
    );
    assert_eq!(s.scsp.as_ref().unwrap().status(0).unwrap().scsp.enabled, 0);
    s.dispose_extra_scsp(1).unwrap();
    render(&s, &mut r, 512);
    assert!(s.extra_scsp_status(1, None, false).is_err());
}
#[test]
fn scsp_bad_cross_slot_batch_is_atomic() {
    let (s, mut r) = setup();
    render(&s, &mut r, 1);
    let c:Command=serde_json::from_value(json!({"type":"scsp","frame":0,"message":{"method":"batch","channel":0,"action":"on","commands":[write(32,0x1820)]}})).unwrap();
    assert!(s.enqueue(1, &[c]).is_err());
    assert_eq!(s.pending.load(Ordering::Acquire), 0);
    let mut bad = note(31, 1, false);
    bad["id"] = json!(10);
    bad["commands"][1]["value"] = json!(65535);
    send(&s, vec![bad]);
    render(&s, &mut r, 1);
    let state = s.scsp.as_ref().unwrap();
    assert!(state.wait_applied(10).unwrap_err().contains("PCM RAM"));
    let a = state.status(0).unwrap();
    assert_eq!(a.scsp.enabled, 0);
    assert_eq!(a.scsp.registers[31 * 16 + 1], 0);
}

#[test]
fn scsp_rust_instance_fits_common_structure_allowance() {
    let bytes = std::mem::size_of::<extra_scsp::Renderer>()
        + std::mem::size_of::<extra_scsp::Shared>()
        + std::mem::size_of::<extra_psg::Instance>();
    assert!(bytes <= 64 * 1024);
}
fn cmd(value: Value) -> Command {
    serde_json::from_value(json!({"type":"scsp","frame":0,"message":value})).unwrap()
}
#[test]
fn scsp_worker_cancel_keeps_manual_and_retires_future() {
    let (s, mut r) = setup();
    s.performance("start", 7, None).unwrap();
    send(&s, vec![note(0, 1, true)]);
    let mut c = cmd(note(31, 2, false));
    c.performance = Some(7);
    c.owner = Some(1);
    s.enqueue(1, &[c]).unwrap();
    render(&s, &mut r, 100);
    let mut future = cmd(
        json!({"id":40,"method":"batch","action":"edit","channel":31,"commands":[write(31*32+16,0x800)]}),
    );
    future.frame = 8000;
    future.performance = Some(7);
    future.owner = Some(1);
    s.enqueue(1, &[future]).unwrap();
    s.performance("cancel-owner", 7, Some(1)).unwrap();
    render(&s, &mut r, 1);
    let state = s.scsp.as_ref().unwrap();
    assert!(state.wait_applied(40).is_err());
    let a = state.status(0).unwrap();
    assert_eq!(a.scsp.enabled, 1);
    assert_eq!(a.scsp.held, [0]);
}
#[test]
fn scsp_main_group_cancel_preserves_replacement_and_other_group() {
    let (s, mut r) = setup();
    let mut a = note(0, 1, false);
    a["group"] = json!(1);
    let mut b = note(31, 2, false);
    b["group"] = json!(2);
    send(&s, vec![a, b]);
    render(&s, &mut r, 64);
    send(
        &s,
        vec![json!({"method":"cancel-main-owner","targetGroup":1})],
    );
    render(&s, &mut r, 1);
    assert_eq!(
        s.scsp.as_ref().unwrap().status(0).unwrap().scsp.enabled,
        1 << 31
    );
    let mut future = cmd(note(0, 4, false));
    future.frame = 4000;
    if let CommandAction::Scsp { message } = &mut future.action {
        message.group = Some(1);
        message.id = Some(100);
    }
    s.enqueue(1, &[future]).unwrap();
    send(
        &s,
        vec![json!({"method":"cancel-main-owner","targetGroup":1})],
    );
    render(&s, &mut r, 5000);
    assert!(s.scsp.as_ref().unwrap().wait_applied(100).is_err());
}
#[test]
fn scsp_capture_is_coherent_and_duplicate_ids_are_atomic() {
    let (s, mut r) = setup();
    render(&s, &mut r, 1);
    send(
        &s,
        vec![
            json!({"id":2,"method":"barrier","ram":true}),
            write(0x400, 0x0201),
        ],
    );
    render(&s, &mut r, 128);
    let state = s.scsp.as_ref().unwrap();
    let cap = state.status_after(0, Some(2), true).unwrap();
    assert_eq!(cap.scsp.registers[512], 0x020f);
    assert_eq!(cap.scsp.ram.unwrap()[4096..4226], wave());
    assert_eq!(state.status(0).unwrap().scsp.registers[512], 0x0201);
    let a = cmd(json!({"id":40,"method":"barrier"}));
    let b = cmd(json!({"id":40,"method":"barrier"}));
    assert!(s.enqueue(1, &[a, b]).is_err());
    assert_eq!(s.pending.load(Ordering::Acquire), 0);
}
#[test]
fn scsp_callback_partitioning_and_timestamps_match() {
    for rate in [44100, 48000, 96000] {
        let run = |blocks: &[usize]| {
            let s = Shared::new_modes(rate, 1, false, false, false, Some("scsp"));
            let mut r = Renderer::new_modes(rate, false, false, false, Some("scsp")).unwrap();
            send(
                &s,
                vec![
                    json!({"method":"load-memory","address":4096,"data":wave()}),
                    write(0x400, 0x020f),
                ],
            );
            let mut on = cmd(note(31, 1, false));
            on.frame = 113;
            let mut off = cmd(off(31, 1, false));
            off.frame = 1701;
            s.enqueue(1, &[on, off]).unwrap();
            let mut out = Vec::new();
            let mut i = 0;
            while out.len() < 4096 {
                let n = blocks[i % blocks.len()].min(4096 - out.len());
                out.extend(render(&s, &mut r, n));
                i += 1;
            }
            out
        };
        let a = run(&[128]);
        assert!(peak(&a) > 0.01);
        assert_eq!(a, run(&[1, 7, 63, 257, 3, 1024, 17]));
    }
}
#[test]
fn scsp_reset_cancels_prior_future_and_restores_master() {
    let (s, mut r) = setup();
    render(&s, &mut r, 1);
    let mut future = cmd(note(31, 1, false));
    future.frame = 8000;
    if let CommandAction::Scsp { message } = &mut future.action {
        message.id = Some(100);
    }
    s.enqueue(1, &[future]).unwrap();
    send(&s, vec![json!({"method":"reset","clearRam":true})]);
    render(&s, &mut r, 1024);
    let state = s.scsp.as_ref().unwrap();
    assert!(state.wait_applied(100).is_err());
    assert_eq!(state.status(0).unwrap().scsp.registers[512], 0x020f);
    assert_eq!(state.status(0).unwrap().scsp.enabled, 0);
}
