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
fn audition_note(ch: u8, voice: u64) -> Value {
    let mut value = note(ch, voice, true);
    let commands = value["commands"].as_array_mut().unwrap();
    commands.pop();
    commands.push(json!({"method":"audition-key","channel":ch,"on":true}));
    value
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

#[test]
fn ps1_live_ops_require_valid_channels_and_explicit_manual_key_leases() {
    for value in [
        json!({"method":"batch","action":"edit","commands":[{"method":"refresh-envelope","channel":0}]}),
        json!({"method":"batch","channel":0,"action":"edit","commands":[{"method":"refresh-envelope","channel":1}]}),
        json!({"method":"batch","channel":24,"action":"edit","commands":[{"method":"refresh-envelope","channel":24}]}),
        json!({"method":"batch","channel":0,"voice":1,"action":"on","commands":[{"method":"audition-key","channel":0,"on":true}]}),
        json!({"method":"batch","channel":0,"audition":true,"action":"edit","commands":[{"method":"audition-key","channel":0,"on":true}]}),
        json!({"method":"batch","channel":0,"voice":1,"audition":true,"action":"off","commands":[{"method":"audition-key","channel":0,"on":true}]}),
    ] {
        let message: extra_ps1::Message = serde_json::from_value(value).unwrap();
        assert!(message.prepare().is_err());
    }
}

#[test]
fn ps1_live_adsr_refresh_preserves_held_sample_phase_and_voice_lease() {
    let (s, mut r) = setup(44100);
    let (reference, mut reference_renderer) = setup(44100);
    send(&s, vec![audition_note(0, 77)]);
    send(&reference, vec![audition_note(0, 77)]);
    assert_eq!(
        render(&s, &mut r, 1009),
        render(&reference, &mut reference_renderer, 1009)
    );
    // Changing attack after reaching sustain must not retrigger the decoder or
    // reset the held envelope. The raw write is the exact control comparison.
    send(
        &s,
        vec![
            json!({"method":"batch","channel":0,"voice":77,"audition":true,"action":"edit","commands":[write(8,0x7f0f),{"method":"refresh-envelope","channel":0}]}),
        ],
    );
    send(
        &reference,
        vec![
            json!({"method":"batch","channel":0,"voice":77,"audition":true,"action":"edit","commands":[write(8,0x7f0f)]}),
        ],
    );
    assert_eq!(
        render(&s, &mut r, 512),
        render(&reference, &mut reference_renderer, 512)
    );
    assert_eq!(s.ps1.as_ref().unwrap().status(0).unwrap().ps1.held, [0]);
    // A stale keyboard release cannot take the current manual voice away.
    send(
        &s,
        vec![
            json!({"method":"batch","channel":0,"voice":76,"audition":true,"action":"off","commands":[{"method":"audition-key","channel":0,"on":false}]}),
        ],
    );
    assert_eq!(
        render(&s, &mut r, 512),
        render(&reference, &mut reference_renderer, 512)
    );
    assert_eq!(s.ps1.as_ref().unwrap().status(0).unwrap().ps1.held, [0]);
    // A sustain direction/rate edit must actually reach the active envelope;
    // raw writes alone intentionally keep the hardware's existing latch.
    // Linear SR40 lasts about 53 ms, so this 93 ms window includes its fade.
    send(
        &s,
        vec![
            json!({"method":"batch","channel":0,"voice":77,"audition":true,"action":"edit","commands":[write(10,0x4a00),{"method":"refresh-envelope","channel":0}]}),
        ],
    );
    send(
        &reference,
        vec![
            json!({"method":"batch","channel":0,"voice":77,"audition":true,"action":"edit","commands":[write(10,0x4a00)]}),
        ],
    );
    let edited = render(&s, &mut r, 4096);
    let unrefreshed = render(&reference, &mut reference_renderer, 4096);
    assert_eq!(edited[0], unrefreshed[0]);
    let energy = |pcm: &[[f32; 2]]| pcm.iter().flatten().map(|v| v * v).sum::<f32>();
    assert!(
        energy(&edited) < energy(&unrefreshed) * 0.8,
        "edited energy {}, raw energy {}, ENVX {} vs {}",
        energy(&edited),
        energy(&unrefreshed),
        s.ps1.as_ref().unwrap().status(0).unwrap().ps1.registers[6],
        reference
            .ps1
            .as_ref()
            .unwrap()
            .status(0)
            .unwrap()
            .ps1
            .registers[6]
    );
    assert!(
        s.ps1.as_ref().unwrap().status(0).unwrap().ps1.registers[6]
            < reference
                .ps1
                .as_ref()
                .unwrap()
                .status(0)
                .unwrap()
                .ps1
                .registers[6]
    );
}

#[test]
fn ps1_additional_manual_keys_use_the_same_live_core_and_lease_rules() {
    let (s, mut r) = setup(44100);
    s.create_extra_ps1(1).unwrap();
    let enqueue = |messages: Vec<Value>| {
        let commands = messages
            .into_iter()
            .map(|message| {
                serde_json::from_value(
                    json!({"type":"extra-ps1","instance":1,"frame":0,"message":message}),
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        s.enqueue(1, &commands).unwrap();
    };
    enqueue(vec![
        json!({"method":"load-memory","address":0x1000,"data":wave()}),
        write(0x180, 0x3fff),
        write(0x182, 0x3fff),
        write(0x1aa, 0xc000),
        audition_note(23, 88),
    ]);
    assert!(peak(&render(&s, &mut r, 1009)) > 0.01);
    enqueue(vec![
        json!({"method":"batch","channel":23,"voice":88,"audition":true,"action":"edit","commands":[write(23*16+8,0x7f0f),{"method":"refresh-envelope","channel":23}]}),
    ]);
    assert!(peak(&render(&s, &mut r, 200)) > 0.01);
    enqueue(vec![
        json!({"method":"batch","channel":23,"voice":88,"audition":true,"action":"off","commands":[{"method":"audition-key","channel":23,"on":false}]}),
    ]);
    let released = render(&s, &mut r, 512);
    assert!(peak(&released[..64]) > 0.001);
    assert_eq!(peak(&released[256..]), 0.0);
    assert_eq!(s.ps1.as_ref().unwrap().status(0).unwrap().ps1.enabled, 0);
}

#[test]
fn ps1_program_edits_refresh_primary_and_additional_envelopes_without_manual_keys() {
    for additional in [false, true] {
        let (s, mut r) = setup(44100);
        let (reference, mut reference_renderer) = setup(44100);
        let enqueue = |state: &Shared, messages: Vec<Value>| {
            let commands = messages
                .into_iter()
                .map(|message| {
                    if additional {
                        serde_json::from_value(
                            json!({"type":"extra-ps1","instance":1,"frame":0,"message":message}),
                        )
                        .unwrap()
                    } else {
                        cmd(message)
                    }
                })
                .collect::<Vec<_>>();
            state.enqueue(1, &commands).unwrap();
        };
        for state in [&s, &reference] {
            if additional {
                state.create_extra_ps1(1).unwrap();
                enqueue(
                    state,
                    vec![
                        json!({"method":"load-memory","address":0x1000,"data":wave()}),
                        write(0x180, 0x3fff),
                        write(0x182, 0x3fff),
                        write(0x1aa, 0xc000),
                    ],
                );
            }
            enqueue(state, vec![note(0, 1, false)]);
        }
        assert_eq!(
            render(&s, &mut r, 1009),
            render(&reference, &mut reference_renderer, 1009)
        );
        // This is the ordinary program Edit batch produced by setEnvelope(),
        // with neither an audition lease nor a key-on operation.
        enqueue(
            &s,
            vec![
                json!({"method":"batch","channel":0,"action":"edit","commands":[write(10,0x4a00),{"method":"refresh-envelope","channel":0}]}),
            ],
        );
        enqueue(
            &reference,
            vec![
                json!({"method":"batch","channel":0,"action":"edit","commands":[write(10,0x4a00)]}),
            ],
        );
        let changed = render(&s, &mut r, 4096);
        let raw = render(&reference, &mut reference_renderer, 4096);
        let energy = |pcm: &[[f32; 2]]| pcm.iter().flatten().map(|v| v * v).sum::<f32>();
        assert_eq!(changed[0], raw[0]);
        assert!(
            energy(&changed) < energy(&raw) * 0.8,
            "additional={additional}"
        );
        // Release leaves no gate/lease owner, but its RR31 tail still exists.
        // A normal Edit must be allowed to apply RR0 to that active tail.
        for state in [&s, &reference] {
            let mut on = note(0, 2, false);
            for op in on["commands"].as_array_mut().unwrap() {
                if op["register"] == 10 {
                    op["value"] = json!(31);
                }
            }
            enqueue(state, vec![on]);
        }
        let onset = render(&s, &mut r, 1009);
        let reference_onset = render(&reference, &mut reference_renderer, 1009);
        // Ordinary KON retains the interpolator's two prior source samples;
        // these differ because only the first rig's old sustain decayed.
        assert_eq!(&onset[2..], &reference_onset[2..]);
        for state in [&s, &reference] {
            enqueue(
                state,
                vec![
                    json!({"method":"batch","channel":0,"voice":2,"action":"off","commands":[write(0x18c,1)]}),
                ],
            );
        }
        assert_eq!(
            render(&s, &mut r, 256),
            render(&reference, &mut reference_renderer, 256)
        );
        enqueue(
            &s,
            vec![
                json!({"method":"batch","channel":0,"action":"edit","commands":[write(10,0),{"method":"refresh-envelope","channel":0}]}),
            ],
        );
        enqueue(
            &reference,
            vec![json!({"method":"batch","channel":0,"action":"edit","commands":[write(10,0)]})],
        );
        let released = render(&s, &mut r, 512);
        let held = render(&reference, &mut reference_renderer, 512);
        assert!(peak(&held) > 0.01);
        assert!(
            energy(&released) < energy(&held) * 0.1,
            "additional={additional}"
        );
        assert_eq!(peak(&released[128..]), 0.0);
    }
}

#[test]
fn ps1_masked_adsr_writes_and_conditional_refresh_use_applied_words() {
    for additional in [false, true] {
        let (s, mut r) = setup(44100);
        let (reference, mut rr) = setup(44100);
        let enqueue = |state: &Shared, ops: Vec<Value>| {
            let message = json!({"method":"batch","channel":0,"action":"edit","commands":ops});
            let c = if additional {
                serde_json::from_value(
                    json!({"type":"extra-ps1","instance":1,"frame":0,"message":message}),
                )
                .unwrap()
            } else {
                cmd(message)
            };
            state.enqueue(1, &[c]).unwrap();
        };
        for state in [&s, &reference] {
            let messages = if additional {
                state.create_extra_ps1(1).unwrap();
                vec![
                    json!({"method":"load-memory","address":0x1000,"data":wave()}),
                    write(0x180, 0x3fff),
                    write(0x182, 0x3fff),
                    write(0x1aa, 0xc000),
                    note(0, 1, false),
                ]
            } else {
                vec![note(0, 1, false)]
            };
            let commands = messages
                .into_iter()
                .map(|message| {
                    if additional {
                        serde_json::from_value(
                            json!({"type":"extra-ps1","instance":1,"frame":0,"message":message}),
                        )
                        .unwrap()
                    } else {
                        cmd(message)
                    }
                })
                .collect::<Vec<_>>();
            state.enqueue(1, &commands).unwrap();
        }
        assert_eq!(render(&s, &mut r, 1009), render(&reference, &mut rr, 1009));
        let masked = |register, value, mask| json!({"method":"write","register":register,"value":value,"mask":mask});
        let refresh = json!({"method":"refresh-envelope","channel":0,"ifChanged":true});
        // A deferred word already in registers must not be activated by a
        // same-value public edit whose conditional refresh has no new bits.
        for state in [&s, &reference] {
            enqueue(state, vec![masked(10, 0x4f00, 0x5fc0)]);
        }
        render(&s, &mut r, 128);
        render(&reference, &mut rr, 128);
        enqueue(&s, vec![masked(10, 0x4f00, 0x5fc0), refresh.clone()]);
        enqueue(&reference, vec![masked(10, 0x4f00, 0x5fc0)]);
        assert_eq!(render(&s, &mut r, 512), render(&reference, &mut rr, 512));
        // Restore a holding register without changing the currently held
        // cache, then update AR only and retain SL/reserved applied bits.
        for state in [&s, &reference] {
            enqueue(state, vec![write(10, 0x2000)]);
        }
        render(&s, &mut r, 1);
        render(&reference, &mut rr, 1);
        enqueue(&s, vec![masked(8, 0x3d00, 0x7f00), refresh.clone()]);
        enqueue(&reference, vec![masked(8, 0x3d00, 0x7f00)]);
        assert_eq!(render(&s, &mut r, 512), render(&reference, &mut rr, 512));
        enqueue(&s, vec![masked(10, 0x4a00, 0x5fc0), refresh]);
        enqueue(&reference, vec![masked(10, 0x4a00, 0x5fc0)]);
        let edited = render(&s, &mut r, 4096);
        let raw = render(&reference, &mut rr, 4096);
        let energy = |pcm: &[[f32; 2]]| pcm.iter().flatten().map(|v| v * v).sum::<f32>();
        assert_eq!(edited[0], raw[0]);
        assert!(energy(&edited) < energy(&raw) * 0.8);
        let regs = if additional {
            s.extra_ps1_status(1, None, false).unwrap().ps1.registers
        } else {
            s.ps1.as_ref().unwrap().status(0).unwrap().ps1.registers
        };
        assert_eq!(regs[4], 0x3d0f);
        assert_eq!(regs[5], 0x6a00);
    }
}

#[test]
fn ps1_mask_and_conditional_refresh_json_options_are_strict() {
    for mask in [
        json!(null),
        json!(-1),
        json!(65536),
        json!(1.5),
        json!("255"),
    ] {
        let value = json!({"method":"batch","channel":0,"action":"edit","commands":[{"method":"write","register":8,"value":0,"mask":mask}]});
        assert!(serde_json::from_value::<extra_ps1::Message>(value).is_err());
    }
    for value in [
        json!({"method":"batch","channel":0,"action":"edit","commands":[{"method":"write","register":8,"value":0,"mask":0}]}),
        json!({"method":"batch","channel":0,"action":"edit","commands":[{"method":"write","register":0,"value":0,"mask":255}]}),
        json!({"method":"batch","channel":0,"action":"edit","commands":[{"method":"write","register":24,"value":0,"mask":255}]}),
        json!({"method":"batch","action":"edit","commands":[{"method":"write","register":8,"value":0,"mask":255}]}),
    ] {
        assert!(serde_json::from_value::<extra_ps1::Message>(value)
            .unwrap()
            .prepare()
            .is_err());
    }
    for option in [json!(null), json!(1), json!("true")] {
        assert!(serde_json::from_value::<extra_ps1::Message>(json!({"method":"batch","channel":0,"action":"edit","commands":[{"method":"refresh-envelope","channel":0,"ifChanged":option}]})).is_err());
    }
}

#[test]
fn ps1_deferred_release_rate_survives_manual_off_until_the_next_note() {
    for additional in [false, true] {
        for original_rate in [31u16, 0] {
            let next_rate = 31 - original_rate;
            let (s, mut r) = setup(44100);
            let (reference, mut rr) = setup(44100);
            let (live, mut lr) = setup(44100);
            let (program, mut pr) = setup(44100);
            let enqueue = |state: &Shared, messages: Vec<Value>| {
                let commands = messages
                    .into_iter()
                    .map(|message| {
                        if additional {
                            serde_json::from_value(json!({
                                "type":"extra-ps1","instance":1,"frame":0,"message":message
                            }))
                            .unwrap()
                        } else {
                            cmd(message)
                        }
                    })
                    .collect::<Vec<_>>();
                state.enqueue(1, &commands).unwrap();
            };
            let key = |voice, on| json!({
                "method":"batch","channel":0,"voice":voice,"audition":true,
                "action":if on {"on"} else {"off"},
                "commands":[{"method":"audition-key","channel":0,"on":on}]
            });
            for (index, state) in [&s, &reference, &live, &program].into_iter().enumerate() {
                if additional {
                    state.create_extra_ps1(1).unwrap();
                    enqueue(state, vec![
                        json!({"method":"load-memory","address":0x1000,"data":wave()}),
                        write(0x180, 0x3fff),
                        write(0x182, 0x3fff),
                        write(0x1aa, 0xc000),
                    ]);
                }
                let mut on = if index == 3 { note(0, 77, false) } else { audition_note(0, 77) };
                for op in on["commands"].as_array_mut().unwrap() {
                    if op["register"] == 10 {
                        op["value"] = json!(original_rate);
                    }
                }
                enqueue(state, vec![on]);
            }
            let onset = render(&s, &mut r, 1009);
            assert_eq!(onset, render(&reference, &mut rr, 1009));
            assert_eq!(onset, render(&live, &mut lr, 1009));
            // Manual gain ramps and native interpolation history intentionally
            // differ from raw keys. Compare the remaining steady onset.
            assert_eq!(&onset[256..], &render(&program, &mut pr, 1009)[256..]);
            // A deferred inspector edit is only a register write. A live edit
            // also refreshes the existing cache, without keying the voice again.
            enqueue(&s, vec![json!({
                "method":"batch","channel":0,"voice":77,"audition":true,"action":"edit",
                "commands":[write(10,next_rate)]
            })]);
            enqueue(&live, vec![json!({
                "method":"batch","channel":0,"voice":77,"audition":true,"action":"edit",
                "commands":[write(10,next_rate),{"method":"refresh-envelope","channel":0}]
            })]);
            enqueue(&program, vec![json!({
                "method":"batch","channel":0,"action":"edit",
                "commands":[write(10,next_rate)]
            })]);
            let held = render(&s, &mut r, 257);
            assert_eq!(held, render(&reference, &mut rr, 257));
            assert_eq!(held, render(&live, &mut lr, 257));
            assert_eq!(held, render(&program, &mut pr, 257));
            // Stale keys must not steal the held note or change decoder phase.
            enqueue(&s, vec![key(76, false)]);
            let status = |state: &Shared| {
                if additional {
                    state.extra_ps1_status(1, None, false).unwrap().ps1
                } else {
                    state.ps1.as_ref().unwrap().status(0).unwrap().ps1
                }
            };
            assert_eq!(render(&s, &mut r, 1), render(&reference, &mut rr, 1));
            render(&live, &mut lr, 1);
            render(&program, &mut pr, 1);
            assert_eq!(status(&s).held, [0]);
            for state in [&s, &reference, &live] {
                enqueue(state, vec![key(77, false)]);
            }
            enqueue(&program, vec![json!({
                "method":"batch","channel":0,"voice":77,"action":"off",
                "commands":[write(0x18c,1)]
            })]);
            let released = render(&s, &mut r, 512);
            let unchanged = render(&reference, &mut rr, 512);
            let updated = render(&live, &mut lr, 512);
            assert_eq!(released, unchanged, "additional={additional}, old RR={original_rate}: manual off must keep the latched release");
            // Manual off has an intentional 128-native-frame click-prevention
            // floor and native interpolation history; raw RR0 keyOff may
            // finish sooner. The untouched manual
            // reference still checks every frame of that initial transition.
            let programmed = render(&program, &mut pr, 512);
            assert_eq!(&released[256..], &programmed[256..], "manual and program release must keep the same deferred rate after the manual transition");
            if original_rate == 31 {
                assert!(peak(&released[256..]) > 0.01);
                assert_eq!(peak(&updated[256..]), 0.0);
            } else {
                assert_eq!(peak(&released[256..]), 0.0);
                assert!(peak(&updated[256..]) > 0.01);
            }
            assert!(status(&s).held.is_empty());
            assert_eq!(status(&s).enabled, 0);
            // Do not rewrite ADSR here: the next key must latch the earlier
            // deferred register value, not restore the old active cache.
            enqueue(&s, vec![key(78, true)]);
            assert!(peak(&render(&s, &mut r, 1009)) > 0.01);
            assert_eq!(status(&s).held, [0]);
            assert_eq!(status(&s).registers[6], 32767);
            enqueue(&s, vec![key(78, false)]);
            let next_release = render(&s, &mut r, 512);
            if next_rate == 31 {
                assert!(peak(&next_release[256..]) > 0.01);
            } else {
                assert_eq!(peak(&next_release[256..]), 0.0);
            }
        }
    }
}
