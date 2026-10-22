//! Selected Nuked Genesis/32X pipeline. Never opens an output device.
use super::*;
use serde_json::{json, Value};
fn command(frame: u64, v: Value) -> Command {
    let mut v = v;
    v["frame"] = json!(frame);
    serde_json::from_value(v).unwrap()
}
fn write(frame: u64, port: u8, register: u8, value: u8) -> Command {
    command(
        frame,
        json!({"type":"write","port":port,"register":register,"value":value}),
    )
}
fn setup(full: bool) -> (Shared, Renderer) {
    let mode = if full { "32x-nuked" } else { "genesis-nuked" };
    let s = Shared::new_modes(48000, 1, full, false, false, Some(mode));
    let r = Renderer::new_modes(48000, full, false, false, Some(mode)).unwrap();
    r.publish_primary(&s);
    (s, r)
}
fn render(s: &Shared, r: &mut Renderer, n: usize) -> Vec<[f32; 2]> {
    r.prepare(s);
    let out = (0..n).map(|_| r.sample(s)).collect();
    s.frame.store(r.frame, Ordering::Release);
    r.publish_primary(s);
    out
}
fn mix(fm: f32, psg: f32, master: f32) -> Command {
    command(
        0,
        json!({"type":"mix","fmLeft":fm,"fmRight":fm,"psgLeft":psg,"psgRight":psg,"master":master}),
    )
}
fn fm(frame: u64, ch: u8) -> Vec<Command> {
    let port = u8::from(ch >= 3);
    let offset = ch % 3;
    let mut commands = Vec::new();
    for slot in [0, 4, 8, 12] {
        for (reg, value) in [
            (0x30 + slot + offset, 1),
            (0x40 + slot + offset, if slot == 12 { 42 } else { 127 }),
            (0x50 + slot + offset, 31),
            (0x60 + slot + offset, 0),
            (0x70 + slot + offset, 0),
            (0x80 + slot + offset, 15),
        ] {
            commands.push(write(frame, port, reg, value))
        }
    }
    for (reg, value) in [
        (0xb0 + offset, 7),
        (0xb4 + offset, 0xc0),
        (0xa4 + offset, 0x22),
        (0xa0 + offset, 0x69),
    ] {
        commands.push(write(frame, port, reg, value))
    }
    commands.push(write(
        frame,
        0,
        0x28,
        0xf0 | (ch % 3 + if ch >= 3 { 4 } else { 0 }),
    ));
    commands
}
fn psg(frame: u64) -> Vec<Command> {
    [0x85, 0x10, 0x9a]
        .into_iter()
        .map(|value| command(frame, json!({"type":"psg-write","value":value})))
        .collect()
}
fn rf(frame: u64) -> Vec<Command> {
    let mut out = vec![command(
        0,
        json!({"type":"rf5c164-load-memory","offset":0,"data":[128,208,224,208,128,80,96,80,255]}),
    )];
    for (reg, value) in [
        (7, 0xc0),
        (0, 100),
        (1, 0xff),
        (2, 0),
        (3, 8),
        (4, 0),
        (5, 0),
        (6, 0),
    ] {
        out.push(command(
            frame,
            json!({"type":"rf5c164-write","register":reg,"value":value}),
        ))
    }
    out.push(command(frame, json!({"type":"rf5c164-key-on","channel":0})));
    out
}
fn pwm(frame: u64) -> Vec<Command> {
    [(0,5),(1,4095),(2,2147),(3,2147)].into_iter().map(|(register,value)|command(frame,json!({"type":"pwm","message":{"method":"write","register":register,"value":value}}))).collect()
}
fn varies(pcm: &[[f32; 2]]) -> bool {
    let first = pcm[0][0];
    pcm.iter().any(|p| (p[0] - first).abs() > 0.00001)
}
fn tagged(mut c: Command, owner: u32) -> Command {
    c.performance = Some(1);
    c.owner = Some(owner);
    c
}
#[test]
fn nuked_native_mode_policy_is_explicit_and_status_identifies_selected_engine() {
    for (mode, full, console) in [
        ("genesis-nuked", false, "genesis-nuked"),
        ("genesis-full-nuked", true, "32x-nuked"),
    ] {
        assert_eq!(
            NativeGenesis::mode_config(Some(mode)).unwrap(),
            (full, false, false, Some(console))
        );
        let (s, r) = setup(full);
        assert!(!r.core.0.is_null());
        assert_eq!(s.status().mode, mode);
        assert_eq!(s.status().engine, Some("nuked"));
        assert_eq!(s.status().ym_clock, 7_670_454);
        assert_eq!(s.status().psg_clock, 3_579_545);
        assert_eq!(r.rf5c164.is_some(), full);
        assert_eq!(r.pwm.is_some(), full);
        assert!(r.dx7.is_none() && r.nes.is_none() && r.opl.is_none() && r.ym2203.is_none());
        assert_eq!(serde_json::to_value(s.status()).unwrap()["engine"], "nuked");
    }
    let normal = Shared::new(48000, 1);
    assert_eq!(normal.status().engine, Some("ymfm"));
    assert_eq!(
        Shared::new_modes(48000, 1, false, true, false, None)
            .status()
            .engine,
        None
    );
    for invalid in [
        "nuked",
        "32x-nuked",
        "genesis-full-unknown",
        "genesis-nuked-extra",
    ] {
        assert!(NativeGenesis::mode_config(Some(invalid)).is_err())
    }
}
#[test]
fn nuked_native_constructor_generates_actual_different_fm_and_repeatable_selected_pcm() {
    let mut a = Core::new_nuked(48000).unwrap();
    let mut b = Core::new_nuked(48000).unwrap();
    let mut ymfm = Core::new(48000).unwrap();
    for command in fm(0, 0) {
        let action = command.validate().unwrap();
        a.apply(action.clone());
        b.apply(action.clone());
        ymfm.apply(action)
    }
    let p: Vec<_> = (0..4096).map(|_| a.sample()).collect();
    let q: Vec<_> = (0..4096).map(|_| b.sample()).collect();
    let other: Vec<_> = (0..4096).map(|_| ymfm.sample()).collect();
    assert_eq!(p, q);
    assert!(varies(&p[512..]));
    assert!(p
        .iter()
        .zip(&other)
        .any(|(a, b)| (a[0] - b[0]).abs() > 0.0001));
    for rate in [0, 7999, 384001, u32::MAX] {
        assert!(Core::new_nuked(rate).is_err())
    }
}
#[test]
fn nuked_native_full_composite_is_opt_in_and_foreign_admission_is_atomic() {
    let (base, mut br) = setup(false);
    assert!(base.enqueue(1, &rf(0)).is_err());
    assert!(base.enqueue(1, &pwm(0)).is_err());
    assert_eq!(base.pending.load(Ordering::Acquire), 0);
    assert!(render(&base, &mut br, 128).iter().all(|p| *p == [0.; 2]));
    let (full, _) = setup(true);
    let invalid = vec![
        fm(0, 0).remove(0),
        command(0, json!({"type":"snes","message":{"method":"reset"}})),
    ];
    assert!(full.enqueue(1, &invalid).is_err());
    assert!(full.queue.is_empty());
    assert_eq!(full.pending.load(Ordering::Acquire), 0);
    assert!(full.enqueue(1, &[write(0, 2, 0x30, 1)]).is_err());
    assert_eq!(full.rf5c164.clock(), 12_500_000);
    assert_eq!(full.pwm.as_ref().unwrap().clock(), 23_011_361);
}
#[test]
fn nuked_native_fm_psg_rf_pwm_mix_equals_independent_selected_sources() {
    let mut separate = Vec::new();
    for source in 0..4 {
        let (s, mut r) = setup(true);
        let mut commands = match source {
            0 => fm(0, 0),
            1 => psg(0),
            2 => rf(0),
            _ => pwm(0),
        };
        commands.push(mix(
            if source == 0 { 0.25 } else { 0. },
            if source == 1 { 0.15 } else { 0. },
            1.,
        ));
        if source == 2 {
            commands.push(command(
                0,
                json!({"type":"rf5c164-mix","left":0.25,"right":0}),
            ))
        }
        if source == 3 {
            commands.push(command(
                0,
                json!({"type":"pwm","message":{"method":"mix","volume":0.5,"pan":1}}),
            ))
        }
        s.enqueue(1, &commands).unwrap();
        separate.push(render(&s, &mut r, 4096))
    }
    let (s, mut r) = setup(true);
    let mut commands = fm(0, 0);
    commands.extend(psg(0));
    commands.extend(rf(0));
    commands.extend(pwm(0));
    commands.extend([
        mix(0.25, 0.15, 0.5),
        command(0, json!({"type":"rf5c164-mix","left":0.25,"right":0})),
        command(
            0,
            json!({"type":"pwm","message":{"method":"mix","volume":0.5,"pan":1}}),
        ),
    ]);
    s.enqueue(1, &commands).unwrap();
    let combined = render(&s, &mut r, 4096);
    for source in &separate {
        assert!(source[512..]
            .iter()
            .any(|p| p[0].abs() + p[1].abs() > 0.0001))
    }
    for i in 512..4096 {
        for ch in 0..2 {
            let expected = separate.iter().fold(0., |sum, source| sum + source[i][ch]) * 0.5;
            assert!(
                (combined[i][ch] - expected).abs() < 2e-7,
                "frame{i} channel{ch}: {} != {expected}",
                combined[i][ch]
            )
        }
    }
}
#[test]
fn nuked_native_timestamp_order_and_callback_segmentation_are_exact() {
    let (s, mut r) = setup(true);
    let (t, mut q) = setup(true);
    let mut commands = fm(100, 0);
    commands.extend(psg(200));
    commands.extend(rf(500));
    commands.extend(pwm(700));
    commands.extend([
        write(1500, 0, 0x28, 0),
        command(2000, json!({"type":"psg-write","value":0x9f})),
        command(2500, json!({"type":"rf5c164-key-off","channel":0})),
        command(
            3000,
            json!({"type":"pwm","message":{"method":"stop-voices"}}),
        ),
    ]);
    s.enqueue(1, &commands).unwrap();
    t.enqueue(1, &commands).unwrap();
    let all = render(&s, &mut r, 4096);
    let mut chunked = Vec::new();
    for n in [33, 467, 1, 999, 2596] {
        chunked.extend(render(&t, &mut q, n))
    }
    assert_eq!(all, chunked);
    assert!(all[..100].iter().all(|p| *p == [0.; 2]));
    assert!(varies(&all[512..1500]));
    assert_eq!(s.pending.load(Ordering::Acquire), 0);
}
#[test]
fn nuked_native_dac_bank_and_packed_streams_use_same_selected_core() {
    for full in [false, true] {
        let (s, mut r) = setup(full);
        let data: Vec<f32> = (0..512)
            .map(|i| if i % 8 < 4 { 64. } else { 192. })
            .collect();
        s.load_media(media::Kind::Dac, "voice".into(), 22050., vec![data])
            .unwrap();
        s.enqueue(
            1,
            &[command(
                100,
                json!({"type":"dac-play","name":"voice","pan":"left"}),
            )],
        )
        .unwrap();
        let pcm = render(&s, &mut r, 2048);
        assert!(varies(&pcm[400..1000]));
        assert!(pcm[400..1000].iter().any(|p| (p[0] - p[1]).abs() > 0.01));
        assert!(r.dac.is_none());
        let mut packed = Vec::new();
        for frame in [0u32, 100, 200, 300] {
            packed.extend(frame.to_le_bytes().into_iter().map(f32::from));
            packed.push(if frame % 200 == 0 { 64. } else { 192. })
        }
        s.load_media(
            media::Kind::DacStream,
            "packed".into(),
            44100.,
            vec![packed],
        )
        .unwrap();
        s.enqueue(
            1,
            &[
                write(0, 1, 0xb6, 0xc0),
                write(0, 0, 0x2b, 0x80),
                command(0, json!({"type":"dac-stream-play","name":"packed"})),
            ],
        )
        .unwrap();
        let pcm = render(&s, &mut r, 1024);
        assert!(varies(&pcm[10..300]));
        assert_eq!(s.dac_streams.load(Ordering::Acquire), 0);
    }
}
#[test]
fn nuked_native_owner_cancel_discards_future_commands_and_keeps_other_psg_owner() {
    let (s, mut r) = setup(true);
    s.active_performance.store(1, Ordering::Release);
    let mut commands: Vec<_> = fm(0, 0).into_iter().map(|c| tagged(c, 1)).collect();
    commands.extend(psg(0).into_iter().map(|c| tagged(c, 2)));
    commands.extend(fm(2500, 1).into_iter().map(|c| tagged(c, 1)));
    commands.extend(rf(2500).into_iter().map(|c| tagged(c, 1)));
    commands.extend(pwm(2500).into_iter().map(|c| tagged(c, 1)));
    s.enqueue(1, &commands).unwrap();
    render(&s, &mut r, 512);
    s.cancelled_owners[0].store(true, Ordering::Release);
    s.invalidation.fetch_add(1, Ordering::Release);
    let pcm = render(&s, &mut r, 4096);
    assert_eq!(r.fm_owners[0], None);
    assert_eq!(r.fm_owners[1], None);
    assert_eq!(
        r.psg_owners[0],
        Some(Owner {
            performance: 1,
            owner: 2
        })
    );
    assert!(varies(&pcm[1024..]));
    assert_eq!(s.pending.load(Ordering::Acquire), 0);
    let pwm = s.pwm.as_ref().unwrap().status(0).unwrap();
    assert_eq!(pwm.registers[0], 0);
    assert_eq!(pwm.registers[1], 0);
}
#[test]
fn nuked_native_epoch_stop_is_silent_retains_patch_and_restarts_selected_full_composite() {
    let (s, mut r) = setup(true);
    let mut commands = fm(0, 0);
    commands.extend(psg(0));
    commands.extend(rf(0));
    commands.extend(pwm(0));
    commands.extend(fm(5000, 1));
    s.enqueue(1, &commands).unwrap();
    assert!(varies(&render(&s, &mut r, 1024)));
    s.epoch.fetch_add(1, Ordering::Release);
    let stopped = render(&s, &mut r, 6000);
    assert!(stopped[1000..].iter().all(|p| *p == [0.; 2]));
    assert_eq!(s.stopped_epoch.load(Ordering::Acquire), 1);
    assert_eq!(s.pending.load(Ordering::Acquire), 0);
    assert_eq!(s.status().engine, Some("nuked"));
    s.enqueue(1, &[write(0, 0, 0x28, 0xf0)]).unwrap();
    assert!(varies(&render(&s, &mut r, 2048)[512..]));
}
#[test]
fn nuked_native_common_fx_master_and_recording_capture_final_selected_pcm() {
    let (dry, mut dr) = setup(true);
    let (wet, mut wr) = setup(true);
    let mut commands = fm(0, 0);
    commands.extend(psg(0));
    commands.extend(rf(0));
    commands.extend(pwm(0));
    commands.push(mix(0.15, 0.1, 1.));
    commands.push(command(
        0,
        json!({"type":"rf5c164-mix","left":0.1,"right":0.1}),
    ));
    commands.push(command(
        0,
        json!({"type":"pwm","message":{"method":"mix","volume":0.1,"pan":0}}),
    ));
    dry.enqueue(1, &commands).unwrap();
    wet.enqueue(1, &commands).unwrap();
    let fx:FxMessage=serde_json::from_value(json!({"op":"chain","children":[{"type":"gain","slot":0}],"units":[{"type":"gain","slot":0,"values":{"gain":0.5}}],"beatSeconds":0.5})).unwrap();
    let graph = wet.fx.lock().unwrap().command(fx, 48000).unwrap().unwrap();
    wet.enqueue_special(Action::Fx(graph)).unwrap();
    wet.enqueue(1, &[mix(0.15, 0.1, 0.5)]).unwrap();
    wet.record_start(0.02).unwrap();
    let expected = render(&dry, &mut dr, 2048);
    let actual = render(&wet, &mut wr, 2048);
    for i in 512..2048 {
        for ch in 0..2 {
            assert!((actual[i][ch] - expected[i][ch] * 0.25).abs() < 1e-7)
        }
    }
    let capture = wet.record_stop().unwrap();
    assert_eq!(capture.frames, 960);
    for ch in 0..2 {
        assert_eq!(
            capture.channels[ch],
            actual[..960].iter().map(|p| p[ch]).collect::<Vec<_>>()
        )
    }
}

fn csm(frame: u64) -> Vec<Command> {
    let mut patch = fm(frame, 2);
    patch.pop();
    patch.extend([
        write(frame, 0, 0x24, 255),
        write(frame, 0, 0x25, 3),
        write(frame, 0, 0x27, 0x85),
    ]);
    patch
}
#[test]
fn nuked_native_cold_csm_source_is_cancelled_and_future_csm_does_not_restart() {
    for nuked in [false, true] {
        let (s, mut r) = if nuked {
            setup(false)
        } else {
            (Shared::new(48000, 1), Renderer::new(48000).unwrap())
        };
        s.active_performance.store(1, Ordering::Release);
        let mut commands: Vec<_> = csm(0).into_iter().map(|c| tagged(c, 1)).collect();
        commands.extend(csm(5000).into_iter().map(|c| tagged(c, 1)));
        s.enqueue(1, &commands).unwrap();
        let before = render(&s, &mut r, 2048);
        assert!(varies(&before[512..]));
        assert_eq!(
            r.csm_owner,
            Some(Owner {
                performance: 1,
                owner: 1
            })
        );
        assert_eq!(r.fm_key_masks[2], 0);
        s.cancelled_owners[0].store(true, Ordering::Release);
        s.invalidation.fetch_add(1, Ordering::Release);
        render(&s, &mut r, 12000);
        assert_eq!(r.csm_owner, None);
        assert_eq!(r.fm_mode_register & 0x81, 0);
        assert_eq!(r.fm_key_masks[2], 0);
        assert_eq!(s.pending.load(Ordering::Acquire), 0);
        let tail = render(&s, &mut r, 1024);
        let low = tail.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min);
        let high = tail.iter().map(|p| p[0]).fold(f32::NEG_INFINITY, f32::max);
        assert!(
            high - low < 0.00001,
            "CSM continued after owner cancellation: {low}..{high}"
        );
    }
}
#[test]
fn nuked_native_csm_owner_cancellation_preserves_explicit_manual_ch3_gate() {
    let (s, mut r) = setup(false);
    s.active_performance.store(1, Ordering::Release);
    let commands: Vec<_> = csm(0).into_iter().map(|c| tagged(c, 1)).collect();
    s.enqueue(1, &commands).unwrap();
    render(&s, &mut r, 1024);
    s.enqueue(1, &[write(0, 0, 0x28, 0xf2)]).unwrap();
    render(&s, &mut r, 128);
    assert_eq!(r.fm_key_masks[2], 15);
    assert_eq!(r.fm_owners[2], None);
    s.cancelled_owners[0].store(true, Ordering::Release);
    s.invalidation.fetch_add(1, Ordering::Release);
    let after = render(&s, &mut r, 2048);
    assert_eq!(r.csm_owner, None);
    assert_eq!(r.fm_mode_register & 0x81, 0);
    assert_eq!(r.fm_key_masks[2], 15);
    assert!(varies(&after[512..]));
}
#[test]
fn nuked_native_all_six_fm_channels_generate_through_selected_engine() {
    for channel in 0..6 {
        let (s, mut r) = setup(false);
        let mut commands = fm(0, channel);
        commands.push(mix(0.25, 0., 1.));
        s.enqueue(1, &commands).unwrap();
        assert!(
            varies(&render(&s, &mut r, 2048)[512..]),
            "FM channel{channel}"
        );
    }
}

#[test]
fn nuked_native_channel3_special_mode_does_not_acquire_a_csm_source_lease() {
    let (s, mut r) = setup(false);
    s.active_performance.store(1, Ordering::Release);
    let mut commands = csm(0);
    *commands.last_mut().unwrap() = write(0, 0, 0x27, 0xc5);
    let commands: Vec<_> = commands.into_iter().map(|c| tagged(c, 1)).collect();
    s.enqueue(1, &commands).unwrap();
    let cold = render(&s, &mut r, 2048);
    assert_eq!(r.csm_owner, None);
    assert_eq!(r.fm_key_masks[2], 0);
    assert!(!varies(&cold[512..]));
    s.cancelled_owners[0].store(true, Ordering::Release);
    s.invalidation.fetch_add(1, Ordering::Release);
    let after = render(&s, &mut r, 2048);
    assert_eq!(r.fm_mode_register, 0xc5);
    assert!(!varies(&after[512..]));
}
