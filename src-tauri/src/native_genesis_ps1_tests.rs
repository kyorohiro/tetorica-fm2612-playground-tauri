//! Offline native PS1 tests: actual fixed SDK core, no output device.
use super::*;
use serde_json::{json, Value};
fn write(register: u16, value: u16) -> Value {
    json!({"method":"write","register":register,"value":value})
}
fn cmd(value: Value) -> Command {
    serde_json::from_value(json!({"type":"ps1","frame":0,"message":value})).unwrap()
}
fn send(s: &Shared, values: Vec<Value>) {
    s.enqueue(1, &values.into_iter().map(cmd).collect::<Vec<_>>())
        .unwrap();
}
fn render(s: &Shared, r: &mut Renderer, n: usize) -> Vec<[f32; 2]> {
    r.prepare(s);
    let out = (0..n).map(|_| r.sample(s)).collect();
    r.publish_primary(s);
    out
}
fn wave() -> Vec<u8> {
    let mut bytes = vec![0; 64];
    for block in 0..4 {
        bytes[block * 16] = 4;
        bytes[block * 16 + 1] = if block == 3 {
            3
        } else if block == 0 {
            6
        } else {
            0
        };
        for i in 0..14 {
            let nib = |frame: usize| {
                ((7.0 * (2.0 * std::f64::consts::PI * frame as f64 / 112.0).sin()).round() as i32
                    & 15) as u8
            };
            bytes[block * 16 + i + 2] =
                nib(block * 28 + i * 2) | (nib(block * 28 + i * 2 + 1) << 4);
        }
    }
    bytes
}
fn note(ch: u8, voice: u64, audition: bool) -> Value {
    let base = ch as u16 * 16;
    json!({"method":"batch","channel":ch,"voice":voice,"audition":audition,"action":"on","commands":[write(base,0x3000),write(base+2,0x3000),write(base+4,0x1000),write(base+6,0x200),write(base+8,0xf),write(base+10,0),write(base+14,0x200),write(0x188+if ch>=16{2}else{0},1<<(ch%16))]})
}
fn setup(rate: u32) -> (Shared, Renderer) {
    let s = Shared::new_modes(rate, 1, false, false, false, Some("ps1"));
    let r = Renderer::new_modes(rate, false, false, false, Some("ps1")).unwrap();
    send(
        &s,
        vec![
            json!({"method":"load-memory","address":0x1000,"data":wave()}),
            write(0x180, 0x3fff),
            write(0x182, 0x3fff),
            write(0x1aa, 0xc000),
        ],
    );
    (s, r)
}
fn peak(pcm: &[[f32; 2]]) -> f32 {
    assert!(pcm.iter().flatten().all(|v| v.is_finite()));
    pcm.iter().flatten().fold(0.0f32, |p, v| p.max(v.abs()))
}
#[test]
fn ps1_primary_initializes_only_its_native_core_and_plays_all_24_voices() {
    assert_eq!(
        NativeGenesis::mode_config(Some("ps1")).unwrap(),
        (false, false, false, Some("ps1"))
    );
    let (s, mut r) = setup(48000);
    assert!(r.core.0.is_null());
    assert!(r.ps1.is_some());
    assert!(r.snes.is_none());
    for ch in 0..24 {
        send(&s, vec![note(ch, ch as u64 + 1, false)]);
        assert!(peak(&render(&s, &mut r, 400)) > 0.01);
    }
    assert_eq!(s.status().mode, "ps1");
    let a = s.ps1.as_ref().unwrap().status(99).unwrap();
    assert_eq!(a.ps1.enabled, 0xffffff);
    assert_eq!(a.ps1.registers.len(), 512);
    assert_eq!(a.frame, 99);
    assert!(serde_json::to_value(a).unwrap().get("ps1").is_none());
}
#[test]
fn ps1_manual_lease_protects_program_registers_masks_and_stale_release() {
    let (s, mut r) = setup(48000);
    send(&s, vec![note(23, 4, true)]);
    render(&s, &mut r, 200);
    let off = |id| json!({"method":"batch","channel":23,"voice":id,"audition":true,"action":"off","commands":[write(0x18e,128)]});
    send(
        &s,
        vec![
            note(23, 5, false),
            write(23 * 16, 0),
            write(0x18e, 255),
            off(3),
        ],
    );
    render(&s, &mut r, 100);
    let a = s.ps1.as_ref().unwrap().status(0).unwrap();
    assert_eq!(a.ps1.held, [23]);
    assert_eq!(a.ps1.registers[23 * 8], 0x3000);
    assert!(peak(&render(&s, &mut r, 100)) > 0.01);
    send(&s, vec![off(4)]);
    render(&s, &mut r, 128);
    assert!(s
        .ps1
        .as_ref()
        .unwrap()
        .status(0)
        .unwrap()
        .ps1
        .held
        .is_empty());
}
#[test]
fn ps1_stop_preserves_authored_registers_and_ram_but_clears_tails() {
    let (s, mut r) = setup(48000);
    send(&s, vec![note(0, 1, false)]);
    assert!(peak(&render(&s, &mut r, 500)) > 0.01);
    let before = s.ps1.as_ref().unwrap().status(0).unwrap().ps1.registers;
    s.epoch.fetch_add(1, Ordering::AcqRel);
    assert_eq!(peak(&render(&s, &mut r, 1000)[900..]), 0.0);
    let after = s.ps1.as_ref().unwrap().status(0).unwrap();
    for index in [0, 1, 2, 3, 4, 5, 7, 0xc0, 0xc1] {
        assert_eq!(after.ps1.registers[index], before[index]);
    }
    send(&s, vec![note(0, 2, false)]);
    assert!(peak(&render(&s, &mut r, 500)) > 0.01);
}
#[test]
fn ps1_capture_is_explicit_completed_owned_ram_and_reset_clear_is_controlled() {
    let (s, mut r) = setup(48000);
    render(&s, &mut r, 1);
    send(&s, vec![json!({"id":11,"method":"barrier","ram":true})]);
    render(&s, &mut r, 1);
    let a = s
        .ps1
        .as_ref()
        .unwrap()
        .status_after(0, Some(11), true)
        .unwrap();
    assert_eq!(&a.ps1.ram.unwrap()[0x1000..0x1040], wave());
    assert!(s.ps1.as_ref().unwrap().status(0).unwrap().ps1.ram.is_none());
    send(
        &s,
        vec![
            json!({"method":"reset","clearRam":true}),
            json!({"id":12,"method":"barrier","ram":true}),
        ],
    );
    render(&s, &mut r, 1);
    assert!(s
        .ps1
        .as_ref()
        .unwrap()
        .status_after(0, Some(12), true)
        .unwrap()
        .ps1
        .ram
        .unwrap()
        .iter()
        .all(|v| *v == 0));
}
#[test]
fn ps1_bounds_fail_before_mutation() {
    for value in [
        write(1, 0),
        write(0x400, 0),
        write(0, 0x8000),
        write(6, 1),
        write(0x1ac, 0),
        json!({"method":"load-memory","address":524288,"data":[1]}),
        json!({"method":"batch","channel":24,"action":"on","commands":[write(0,1)]}),
        json!({"method":"batch","channel":0,"action":"edit","commands":[write(16,1)]}),
    ] {
        let m: extra_ps1::Message = serde_json::from_value(value).unwrap();
        assert!(m.prepare().is_err());
    }
}
#[test]
fn ps1_host_resampling_keeps_duration_pitch_and_partition_determinism() {
    for rate in [44100, 48000, 96000] {
        let (s, mut r) = setup(rate);
        send(&s, vec![note(0, 1, false)]);
        let pcm = render(&s, &mut r, rate as usize);
        let crossings = pcm
            .windows(2)
            .filter(|p| p[0][0] <= 0.0 && p[1][0] > 0.0)
            .count();
        assert!(
            (crossings as f64 - 44100.0 / 112.0).abs() < 3.0,
            "{rate}: {crossings}"
        );
        let (s2, mut r2) = setup(rate);
        send(&s2, vec![note(0, 1, false)]);
        let mut chunked = Vec::new();
        let mut remaining = rate as usize;
        while remaining > 0 {
            let take = remaining.min(137);
            chunked.extend(render(&s2, &mut r2, take));
            remaining -= take;
        }
        assert_eq!(pcm, chunked);
    }
}
#[test]
fn ps1_independent_instances_mix_and_dispose_without_mutating_primary() {
    let (s, mut r) = setup(48000);
    s.create_extra_ps1(1).unwrap();
    s.create_extra_ps1(2).unwrap();
    let mut commands = Vec::new();
    for instance in 1..=2 {
        for message in [
            json!({"method":"load-memory","address":0x1000,"data":wave()}),
            write(0x180, 0x3fff),
            write(0x182, 0x3fff),
            write(0x1aa, 0xc000),
            note(0, instance, false),
        ] {
            commands.push(
                serde_json::from_value(
                    json!({"type":"extra-ps1","instance":instance,"frame":0,"message":message}),
                )
                .unwrap(),
            );
        }
    }
    s.enqueue(1, &commands).unwrap();
    assert!(peak(&render(&s, &mut r, 1000)) > 0.01);
    s.dispose_extra_ps1(1).unwrap();
    assert!(peak(&render(&s, &mut r, 1000)) > 0.01);
    assert_eq!(s.ps1.as_ref().unwrap().status(0).unwrap().ps1.enabled, 0);
    s.dispose_extra_ps1(2).unwrap();
    assert_eq!(peak(&render(&s, &mut r, 1000)[900..]), 0.0);
}
#[test]
fn ps1_append_uploads_preserve_held_wave_and_own_wave_edits_require_current_voice() {
    let (s, mut r) = setup(48000);
    send(&s, vec![note(0, 77, true)]);
    render(&s, &mut r, 200);
    send(
        &s,
        vec![json!({"id":20,"method":"load-memory","address":0x1040,"data":wave()})],
    );
    render(&s, &mut r, 1);
    s.ps1.as_ref().unwrap().wait_applied(20).unwrap();
    send(
        &s,
        vec![
            json!({"id":21,"method":"load-memory","address":0x1000,"data":wave(),"audition":true,"channel":0,"voice":76}),
        ],
    );
    render(&s, &mut r, 1);
    assert!(s.ps1.as_ref().unwrap().wait_applied(21).is_err());
    send(
        &s,
        vec![
            json!({"id":22,"method":"load-memory","address":0x1000,"data":wave(),"audition":true,"channel":0,"voice":77}),
        ],
    );
    render(&s, &mut r, 1);
    s.ps1.as_ref().unwrap().wait_applied(22).unwrap();
    assert_eq!(s.ps1.as_ref().unwrap().status(0).unwrap().ps1.held, [0]);
}
#[test]
fn ps1_channel_batches_merge_reverb_sends_and_owner_global_fx_respects_other_manual_notes() {
    let (s, mut r) = setup(48000);
    send(&s, vec![note(1, 91, true)]);
    render(&s, &mut r, 200);
    send(
        &s,
        vec![
            json!({"method":"batch","channel":1,"voice":91,"audition":true,"action":"edit","commands":[write(0x198,2)]}),
            json!({"method":"batch","channel":0,"action":"edit","commands":[write(0x198,1)]}),
            json!({"method":"batch","voice":90,"audition":true,"action":"edit","commands":[write(0x184,123)]}),
        ],
    );
    render(&s, &mut r, 1);
    let a = s.ps1.as_ref().unwrap().status(0).unwrap();
    assert_eq!(a.ps1.registers[0x198 / 2], 3);
    assert_eq!(a.ps1.registers[0x184 / 2], 0);
    send(
        &s,
        vec![
            json!({"method":"batch","voice":91,"audition":true,"action":"edit","commands":[write(0x184,123)]}),
        ],
    );
    render(&s, &mut r, 1);
    assert_eq!(
        s.ps1.as_ref().unwrap().status(0).unwrap().ps1.registers[0x184 / 2],
        123
    );
}
#[test]
fn ps1_worker_cancellation_releases_only_worker_voice_and_removes_future_commands() {
    let (s, mut r) = setup(48000);
    s.performance("start", 7, None).unwrap();
    send(&s, vec![note(0, 1, true)]);
    let mut c = cmd(note(1, 2, false));
    c.performance = Some(7);
    c.owner = Some(1);
    s.enqueue(1, &[c]).unwrap();
    render(&s, &mut r, 100);
    let mut future = cmd(json!({"id":40,"method":"write","register":20,"value":2345}));
    future.frame = 8000;
    future.performance = Some(7);
    future.owner = Some(1);
    s.enqueue(1, &[future]).unwrap();
    s.performance("cancel-owner", 7, Some(1)).unwrap();
    render(&s, &mut r, 1);
    assert!(s.ps1.as_ref().unwrap().wait_applied(40).is_err());
    let a = s.ps1.as_ref().unwrap().status(0).unwrap();
    assert_eq!(a.ps1.enabled, 1);
    assert_eq!(a.ps1.held, [0]);
}
#[test]
fn ps1_ram_barrier_is_coherent_even_when_registers_change_before_control_reads() {
    let (s, mut r) = setup(48000);
    render(&s, &mut r, 1);
    send(
        &s,
        vec![
            write(4, 0x1234),
            json!({"id":30,"method":"barrier","ram":true}),
            write(4, 0x2345),
        ],
    );
    render(&s, &mut r, 200);
    assert_eq!(
        s.ps1.as_ref().unwrap().status(0).unwrap().ps1.registers[2],
        0x2345
    );
    let capture = s
        .ps1
        .as_ref()
        .unwrap()
        .status_after(0, Some(30), true)
        .unwrap();
    assert_eq!(capture.ps1.registers[2], 0x1234);
    assert!(capture.ps1.native_frames < 200);
}

#[test]
fn ps1_idle_inspector_can_edit_adsr_and_global_effects_without_a_held_voice() {
    let (s, mut r) = setup(48000);
    send(
        &s,
        vec![
            json!({"method":"batch","channel":0,"audition":true,"action":"edit","commands":[write(8,0x1234)]}),
            json!({"method":"batch","audition":true,"action":"edit","commands":[write(0x184,123)]}),
        ],
    );
    render(&s, &mut r, 1);
    let a = s.ps1.as_ref().unwrap().status(0).unwrap();
    assert_eq!(a.ps1.registers[4], 0x1234);
    assert_eq!(a.ps1.registers[0xc2], 123);
}
