//! Device-free full SPC700 integration, lifetimes, ownership and observer bounds.
use super::*;
use serde_json::{json, Value};
fn command(instance: u64, message: Value) -> Command {
    serde_json::from_value(
        json!({"type":"extra-spc","instance":instance,"frame":0,"message":message}),
    )
    .unwrap()
}
fn send(s: &Shared, instance: u64, messages: Vec<Value>) -> Result<(), String> {
    s.enqueue(
        1,
        &messages
            .into_iter()
            .map(|m| command(instance, m))
            .collect::<Vec<_>>(),
    )
}
fn render(s: &Shared, r: &mut Renderer, count: usize) -> Vec<[f32; 2]> {
    let out = (0..count)
        .map(|i| {
            if i % 128 == 0 {
                r.prepare(s)
            }
            r.sample(s)
        })
        .collect();
    r.prepare(s);
    s.frame.store(r.frame, Ordering::Release);
    out
}
fn new() -> (Shared, Renderer) {
    let s = Shared::new(48000, 1);
    let r = Renderer::new(48000).unwrap();
    s.create_extra_spc(1).unwrap();
    (s, r)
}
fn synthetic(marker: u8) -> Vec<u8> {
    let mut b = vec![0; 0x10200];
    let signature = b"SNES-SPC700 Sound File Data v0.30";
    b[..signature.len()].copy_from_slice(signature);
    b[0x21] = 0x1a;
    b[0x22] = 0x1a;
    b[0x23] = 0x1b;
    b[0x24] = 30;
    b[0x26] = 2;
    b[0x2b] = 0xef;
    b[0x300..0x30a].copy_from_slice(&[0x8f, 0, 0xf2, 0xe8, 0x20, 0xc4, 0xf3, 0xbc, 0x2f, 0xfb]);
    b[0x1101] = 0x20;
    b[0x1103] = 0x20;
    b[0x2100..0x2109].copy_from_slice(&[0xc3, 0x13, 0x57, 0x75, 0x31, 0xfd, 0xb9, 0x9b, 0xdf]);
    b[0x700] = marker;
    for (reg, v) in [
        (0, 64),
        (1, 48),
        (3, 16),
        (7, 127),
        (0x0c, 127),
        (0x1c, 127),
        (0x4c, 1),
        (0x5d, 0x10),
        (0x6c, 0x20),
    ] {
        b[0x10100 + reg] = v;
    }
    b
}
fn load(s: &Shared, r: &mut Renderer, id: u64, marker: u8) {
    send(
        s,
        id,
        vec![json!({"method":"load-spc","data":synthetic(marker)})],
    )
    .unwrap();
    render(s, r, 1);
}
fn state(s: &Shared, id: u64) -> extra_spc::PlayerState {
    s.extra_spc_status(id, None).unwrap().spc
}
fn snapshot(s: &Shared, r: &mut Renderer, id: u64, c: u64) -> extra_spc::Snapshot {
    send(s, id, vec![json!({"method":"snapshot","id":c})]).unwrap();
    render(s, r, 1);
    s.extra_spc_status(id, Some(c)).unwrap().snapshot.unwrap()
}
fn scoped(mut m: Value, session: u64, owner: u64) -> Value {
    m["sourceOwner"] = json!({"session":session,"owner":owner});
    m
}
fn energy(out: &[[f32; 2]]) -> f32 {
    out.iter().flatten().map(|v| v.abs()).sum()
}
fn observe(s: &Shared, r: &mut Renderer, epoch: u64) {
    send(
        s,
        1,
        vec![json!({"method":"observe","enabled":true,"epoch":epoch,"intervalMs":32})],
    )
    .unwrap();
    render(s, r, 1);
}
fn seek_begin(s: &Shared, r: &mut Renderer, id: u64, scope: Option<(u64, u64)>) {
    let mut m = json!({"method":"seek-begin","seekId":id});
    if let Some((a, b)) = scope {
        m = scoped(m, a, b)
    }
    send(s, 1, vec![m]).unwrap();
    render(s, r, 1);
}
#[test]
fn extra_spc_blank_state_and_twenty_one_kinds_share_sixteen_slots() {
    let (s, mut r) = new();
    let a = state(&s, 1);
    assert!(a.ready);
    assert!(!a.loaded && !a.playing && !a.paused);
    assert_eq!(a.native_sample_rate, 32000);
    s.create_extra_snes(2).unwrap();
    s.create_extra_pwm(3).unwrap();
    s.create_extra_dx7(4).unwrap();
    s.create_extra_huc6280(5, None).unwrap();
    s.create_extra_nes(6, None, true).unwrap();
    s.create_extra_gameboy(7, None).unwrap();
    s.create_extra_psg(8, None).unwrap();
    s.create_extra_ym2612(9, None).unwrap();
    s.create_extra_ym2203(10, None).unwrap();
    s.create_extra_ym2608(11, None).unwrap();
    s.create_extra_ym2610(12, None).unwrap();
    s.create_extra_ym2151(13, None).unwrap();
    s.create_extra_ym2413(14, None).unwrap();
    s.create_extra_ay8910(15, None).unwrap();
    s.create_extra_ym3526(16, None).unwrap();
    assert!(s
        .create_extra_ym3812(17, None)
        .unwrap_err()
        .contains("limit"));
    render(&s, &mut r, 1);
    s.dispose_extra_spc(1).unwrap();
    render(&s, &mut r, 1);
    s.extra_psg.collect_retired().unwrap();
    s.create_extra_ym3812(17, None).unwrap();
}
#[test]
fn extra_spc_sixteen_empty_and_loaded_players_fit_shared_budget_and_release() {
    let (s, mut r) = new();
    for id in 2..=16 {
        s.create_extra_spc(id).unwrap();
    }
    assert_eq!(s.extra_psg.memory_used(), 16 * extra_spc::INSTANCE_BYTES);
    for id in 1..=16 {
        load(&s, &mut r, id, id as u8)
    }
    let used = s.extra_psg.memory_used();
    assert!(used < extra_budget::DEFAULT_LIMIT);
    assert!(used > 16 * extra_spc::INSTANCE_BYTES);
    s.extra_psg.deactivate_all().unwrap();
    render(&s, &mut r, 1);
    s.extra_psg.collect_retired().unwrap();
    assert_eq!(s.extra_psg.memory_used(), 0);
}
#[test]
fn extra_spc_actual_cpu_audio_ram_and_mute_are_independent() {
    let (s, mut r) = new();
    s.create_extra_spc(2).unwrap();
    load(&s, &mut r, 1, 42);
    load(&s, &mut r, 2, 84);
    send(&s, 1, vec![json!({"method":"play"})]).unwrap();
    assert!(energy(&render(&s, &mut r, 4096)) > 0.1);
    assert_eq!(snapshot(&s, &mut r, 1, 1).ram[0x600], 42);
    assert_eq!(snapshot(&s, &mut r, 2, 1).ram[0x600], 84);
    assert!(state(&s, 1).rendered_native_frames > 0);
    assert_eq!(state(&s, 2).rendered_native_frames, 0);
    send(&s, 1, vec![json!({"method":"mute","mask":255})]).unwrap();
    render(&s, &mut r, 128);
    assert_eq!(state(&s, 1).mute_mask, 255);
    assert_eq!(state(&s, 2).mute_mask, 0);
}
#[test]
fn extra_spc_pause_freezes_position_then_resume_preserves_resampler() {
    let (s, mut r) = new();
    load(&s, &mut r, 1, 1);
    send(&s, 1, vec![json!({"method":"play"})]).unwrap();
    render(&s, &mut r, 2048);
    send(&s, 1, vec![json!({"method":"pause"})]).unwrap();
    render(&s, &mut r, 1);
    let a = state(&s, 1);
    assert!(a.paused);
    assert_eq!(energy(&render(&s, &mut r, 2048)), 0.);
    let b = state(&s, 1);
    assert_eq!(a.playback_native_frames, b.playback_native_frames);
    assert_eq!(a.rendered_native_frames, b.rendered_native_frames);
    send(&s, 1, vec![json!({"method":"play"})]).unwrap();
    render(&s, &mut r, 128);
    assert!(state(&s, 1).playback_native_frames > a.playback_native_frames);
}
#[test]
fn extra_spc_stop_reloads_original_and_preserves_latest_mute() {
    let (s, mut r) = new();
    load(&s, &mut r, 1, 99);
    send(&s, 1, vec![json!({"method":"play"})]).unwrap();
    render(&s, &mut r, 1024);
    send(
        &s,
        1,
        vec![json!({"method":"stop"}), json!({"method":"mute","mask":11})],
    )
    .unwrap();
    render(&s, &mut r, 1);
    let a = state(&s, 1);
    assert!(a.loaded && a.paused);
    assert_eq!(a.rendered_native_frames, 0);
    assert_eq!(a.mute_mask, 11);
    assert_eq!(snapshot(&s, &mut r, 1, 1).ram[0x600], 99);
}
#[test]
fn extra_spc_snapshot_is_captured_once_at_its_barrier() {
    let (s, mut r) = new();
    load(&s, &mut r, 1, 7);
    send(
        &s,
        1,
        vec![
            json!({"method":"snapshot","id":1}),
            json!({"method":"play"}),
        ],
    )
    .unwrap();
    render(&s, &mut r, 2048);
    let c = s.extra_spc_status(1, Some(1)).unwrap().snapshot.unwrap();
    assert_eq!(c.native_frames, 0);
    assert_eq!(c.playback_native_frames, 0.);
    assert_eq!(c.ram.len(), 65536);
    assert_eq!(c.dsp.len(), 128);
    assert!(state(&s, 1).rendered_native_frames > 0);
}
#[test]
fn extra_spc_failed_load_releases_prepared_memory_and_keeps_old_file() {
    let (s, mut r) = new();
    load(&s, &mut r, 1, 9);
    s.extra_psg.collect_retired().unwrap();
    let before = s.extra_psg.memory_used();
    let e = send(
        &s,
        1,
        vec![json!({"method":"load-spc","data":vec![0u8;0x10200]})],
    )
    .unwrap_err();
    assert!(e.starts_with("Native SPC load rejected:"));
    s.extra_psg.collect_retired().unwrap();
    assert_eq!(s.extra_psg.memory_used(), before);
    assert_eq!(snapshot(&s, &mut r, 1, 1).ram[0x600], 9);
}
#[test]
fn extra_spc_seek_progress_snapshot_end_and_latest_mute() {
    let (s, mut r) = new();
    load(&s, &mut r, 1, 1);
    seek_begin(&s, &mut r, 10, None);
    let a = s.extra_spc_seek_step(1, 10, 16000, None).unwrap();
    assert!(a.spc.paused);
    assert_eq!(a.spc.rendered_native_frames, 16000);
    assert_eq!(a.spc.playback_native_frames, 16000.);
    send(&s, 1, vec![json!({"method":"mute","mask":31})]).unwrap();
    render(&s, &mut r, 1);
    let c = snapshot(&s, &mut r, 1, 1);
    assert_eq!(c.native_frames, 16000);
    assert_eq!(c.mute_mask, 31);
    s.extra_spc_seek_step(1, 10, 1000, None).unwrap();
    send(
        &s,
        1,
        vec![json!({"method":"seek-end","seekId":10,"resume":true,"id":2})],
    )
    .unwrap();
    render(&s, &mut r, 128);
    s.extra_spc_status(1, Some(2)).unwrap();
    let b = state(&s, 1);
    assert!(b.playing);
    assert!(b.playback_native_frames >= 17000.);
    assert_eq!(b.mute_mask, 31);
}
#[test]
fn extra_spc_duplicate_seek_end_cannot_mutate_installed_core() {
    let (s, mut r) = new();
    load(&s, &mut r, 1, 1);
    seek_begin(&s, &mut r, 10, None);
    s.extra_spc_seek_step(1, 10, 400, None).unwrap();
    send(
        &s,
        1,
        vec![
            json!({"method":"seek-end","seekId":10,"resume":true,"id":1}),
            json!({"method":"seek-end","seekId":10,"resume":false,"id":2}),
        ],
    )
    .unwrap();
    render(&s, &mut r, 128);
    assert!(s.extra_spc_status(1, Some(1)).unwrap().spc.playing);
    assert!(s
        .extra_spc_status(1, Some(2))
        .unwrap_err()
        .contains("cancelled"));
    assert!(state(&s, 1).playing);
    assert!(send(
        &s,
        1,
        vec![json!({"method":"seek-end","seekId":10,"resume":false})]
    )
    .unwrap_err()
    .contains("cancelled"));
}
#[test]
fn extra_spc_pause_during_seek_keeps_partial_progress() {
    let (s, mut r) = new();
    load(&s, &mut r, 1, 1);
    seek_begin(&s, &mut r, 10, None);
    s.extra_spc_seek_step(1, 10, 3200, None).unwrap();
    send(&s, 1, vec![json!({"method":"pause"})]).unwrap();
    render(&s, &mut r, 1);
    let a = state(&s, 1);
    assert!(a.paused);
    assert_eq!(a.playback_native_frames, 3200.);
    assert!(s.extra_spc_seek_step(1, 10, 1, None).is_err());
}
#[test]
fn extra_spc_play_during_seek_keeps_partial_progress() {
    let (s, mut r) = new();
    load(&s, &mut r, 1, 1);
    seek_begin(&s, &mut r, 10, None);
    s.extra_spc_seek_step(1, 10, 3200, None).unwrap();
    send(&s, 1, vec![json!({"method":"play"})]).unwrap();
    render(&s, &mut r, 128);
    let a = state(&s, 1);
    assert!(a.playing);
    assert!(a.playback_native_frames >= 3200.);
}
#[test]
fn extra_spc_load_invalidates_stale_seek_and_dispose_fences_same_id() {
    let (s, mut r) = new();
    load(&s, &mut r, 1, 1);
    seek_begin(&s, &mut r, 10, None);
    s.extra_spc_seek_step(1, 10, 100, None).unwrap();
    load(&s, &mut r, 1, 2);
    assert!(s.extra_spc_seek_step(1, 10, 1, None).is_err());
    assert_eq!(snapshot(&s, &mut r, 1, 1).ram[0x600], 2);
    s.dispose_extra_spc(1).unwrap();
    render(&s, &mut r, 1);
    s.extra_psg.collect_retired().unwrap();
    assert!(s.create_extra_spc(1).unwrap_err().contains("increasing"));
    s.create_extra_spc(2).unwrap();
    assert!(!state(&s, 2).loaded);
}
#[test]
fn extra_spc_owner_cancel_preserves_other_owner_and_rejects_late_commands() {
    let (s, mut r) = new();
    load(&s, &mut r, 1, 1);
    send(&s, 1, vec![scoped(json!({"method":"play"}), 1, 10)]).unwrap();
    render(&s, &mut r, 128);
    send(
        &s,
        1,
        vec![json!({"method":"cancel-owner","session":1,"owner":20})],
    )
    .unwrap();
    render(&s, &mut r, 128);
    assert!(state(&s, 1).playing);
    send(
        &s,
        1,
        vec![json!({"method":"cancel-owner","session":1,"owner":10})],
    )
    .unwrap();
    render(&s, &mut r, 128);
    assert!(state(&s, 1).paused);
    assert!(send(&s, 1, vec![scoped(json!({"method":"play"}), 1, 10)])
        .unwrap_err()
        .contains("cancelled"));
    send(&s, 1, vec![scoped(json!({"method":"play"}), 2, 10)]).unwrap();
    render(&s, &mut r, 128);
    assert!(state(&s, 1).playing);
}
#[test]
fn extra_spc_owner_cancel_during_seek_keeps_partial_position() {
    let (s, mut r) = new();
    load(&s, &mut r, 1, 1);
    seek_begin(&s, &mut r, 10, Some((1, 10)));
    let scope = Some(extra_spc::Scope {
        session: 1,
        owner: 10,
    });
    s.extra_spc_seek_step(1, 10, 6400, scope).unwrap();
    send(
        &s,
        1,
        vec![json!({"method":"cancel-owner","session":1,"owner":10})],
    )
    .unwrap();
    render(&s, &mut r, 1);
    assert_eq!(state(&s, 1).playback_native_frames, 6400.);
    assert!(state(&s, 1).paused);
}
#[test]
fn extra_spc_performance_cancel_is_local_and_new_session_recovers() {
    let (s, mut r) = new();
    s.create_extra_spc(2).unwrap();
    load(&s, &mut r, 1, 1);
    load(&s, &mut r, 2, 2);
    for id in 1..=2 {
        send(&s, id, vec![scoped(json!({"method":"play"}), 3, 10)]).unwrap();
    }
    render(&s, &mut r, 128);
    send(
        &s,
        1,
        vec![json!({"method":"cancel-performance","session":3})],
    )
    .unwrap();
    render(&s, &mut r, 128);
    assert!(state(&s, 1).paused);
    assert!(state(&s, 2).playing);
    assert!(send(&s, 1, vec![scoped(json!({"method":"play"}), 3, 11)]).is_err());
    send(&s, 1, vec![scoped(json!({"method":"play"}), 4, 11)]).unwrap();
    render(&s, &mut r, 1);
    assert!(state(&s, 1).playing);
}
#[test]
fn extra_spc_owner_capacity_projects_whole_transaction_without_partial_cancel() {
    let (s, mut r) = new();
    load(&s, &mut r, 1, 1);
    for owner in 1..=1023 {
        send(
            &s,
            1,
            vec![json!({"method":"cancel-owner","session":1,"owner":owner})],
        )
        .unwrap();
        render(&s, &mut r, 1);
    }
    let err = send(
        &s,
        1,
        vec![
            json!({"method":"cancel-owner","session":1,"owner":1024}),
            json!({"method":"cancel-owner","session":1,"owner":1025}),
        ],
    )
    .unwrap_err();
    assert!(err.contains("owner limit"));
    send(&s, 1, vec![scoped(json!({"method":"play"}), 1, 1024)]).unwrap();
    render(&s, &mut r, 1);
    assert!(state(&s, 1).playing);
}
#[test]
fn extra_spc_observer_is_off_until_requested_and_emits_real_cpu_writes() {
    let (s, mut r) = new();
    load(&s, &mut r, 1, 1);
    send(&s, 1, vec![json!({"method":"play"})]).unwrap();
    render(&s, &mut r, 128);
    assert!(s.extra_spc_events(1, 0).unwrap().batches.is_empty());
    observe(&s, &mut r, 7);
    render(&s, &mut r, 128);
    let e = s.extra_spc_events(1, 7).unwrap();
    assert!(!e.failed);
    assert_eq!(e.reset.unwrap().dsp.len(), 128);
    assert!(!e.batches.is_empty());
    assert!(e.batches.iter().any(|p| p.count > 0));
    for p in e.batches {
        assert_eq!(p.writes.len(), p.count * 3);
        assert!(p.writes.chunks(3).all(|w| w[1] == 0.));
        assert!(p.writes.chunks(3).all(|w| w[0] <= p.clock as f64));
    }
}
#[test]
fn extra_spc_observer_backpressure_is_four_packets_and_ack_resumes() {
    let (s, mut r) = new();
    load(&s, &mut r, 1, 1);
    observe(&s, &mut r, 5);
    send(&s, 1, vec![json!({"method":"play"})]).unwrap();
    let mut seq = Vec::new();
    for _ in 0..8 {
        if seq.len() == 4 {
            break;
        }
        render(&s, &mut r, 128);
        seq.extend(
            s.extra_spc_events(1, 5)
                .unwrap()
                .batches
                .into_iter()
                .map(|p| p.sequence),
        );
    }
    assert_eq!(seq.len(), 4);
    render(&s, &mut r, 512);
    assert!(s.extra_spc_events(1, 5).unwrap().batches.is_empty());
    send(
        &s,
        1,
        vec![json!({"method":"event-ack","epoch":5,"sequence":seq[0]})],
    )
    .unwrap();
    render(&s, &mut r, 1);
    assert!(!s.extra_spc_events(1, 5).unwrap().batches.is_empty());
}
#[test]
fn extra_spc_observer_overflow_resyncs_without_changing_audio() {
    let (s, mut r) = new();
    load(&s, &mut r, 1, 1);
    observe(&s, &mut r, 5);
    send(&s, 1, vec![json!({"method":"play"})]).unwrap();
    assert!(energy(&render(&s, &mut r, 8192)) > 1.);
    let e = s.extra_spc_events(1, 5).unwrap();
    assert!(e
        .batches
        .iter()
        .any(|p| p.dropped_writes > 0 && p.dsp.as_ref().is_some_and(|v| v.len() == 128)));
    assert!(state(&s, 1).playing);
}
#[test]
fn extra_spc_observer_seek_suspends_then_restarts_baseline_and_stale_epoch_ignored() {
    let (s, mut r) = new();
    load(&s, &mut r, 1, 1);
    observe(&s, &mut r, 4);
    seek_begin(&s, &mut r, 10, None);
    assert!(s.extra_spc_events(1, 0).unwrap().suspended);
    s.extra_spc_seek_step(1, 10, 100, None).unwrap();
    send(
        &s,
        1,
        vec![json!({"method":"seek-end","seekId":10,"resume":false,"eventEpoch":8})],
    )
    .unwrap();
    render(&s, &mut r, 1);
    assert!(s.extra_spc_events(1, 4).unwrap().batches.is_empty());
    let e = s.extra_spc_events(1, 8).unwrap();
    assert_eq!(e.reset.unwrap().clock, 3200);
}
#[test]
fn extra_spc_snapshot_limit_releases_completed_undelivered_on_owner_cancel() {
    let (s, mut r) = new();
    load(&s, &mut r, 1, 1);
    send(
        &s,
        1,
        (1..=16)
            .map(|id| scoped(json!({"method":"snapshot","id":id}), 1, 10))
            .collect(),
    )
    .unwrap();
    render(&s, &mut r, 1);
    assert!(send(&s, 1, vec![json!({"method":"snapshot","id":17})])
        .unwrap_err()
        .contains("snapshot limit"));
    send(
        &s,
        1,
        vec![json!({"method":"cancel-owner","session":1,"owner":10})],
    )
    .unwrap();
    render(&s, &mut r, 1);
    snapshot(&s, &mut r, 1, 18);
}
#[test]
fn extra_spc_bounds_snapshot_work_across_rack_and_releases_failed_prepare() {
    let (s, mut r) = new();
    s.create_extra_spc(2).unwrap();
    load(&s, &mut r, 1, 1);
    load(&s, &mut r, 2, 2);
    send(
        &s,
        1,
        (1..=16)
            .map(|id| json!({"method":"snapshot","id":id}))
            .collect(),
    )
    .unwrap();
    assert!(send(&s, 2, vec![json!({"method":"snapshot","id":20})])
        .unwrap_err()
        .contains("across shared rack"));
    render(&s, &mut r, 1);
    for id in 1..=16 {
        s.extra_spc_status(1, Some(id)).unwrap();
    }
    snapshot(&s, &mut r, 2, 21);
}
#[test]
fn extra_spc_rejects_musical_scheduling_and_invalid_arguments() {
    let (s, mut r) = new();
    let mut c = json!({"type":"extra-spc","instance":1,"frame":0,"message":{"method":"barrier"}});
    c["frame"] = json!(100);
    assert!(s.enqueue(1, &[serde_json::from_value(c).unwrap()]).is_err());
    assert!(send(&s, 1, vec![json!({"method":"load-spc","data":[0]})])
        .unwrap_err()
        .contains("size"));
    load(&s, &mut r, 1, 1);
    seek_begin(&s, &mut r, 1, None);
    assert!(s.extra_spc_seek_step(1, 1, 16001, None).is_err());
    assert!(send(
        &s,
        1,
        vec![json!({"method":"observe","enabled":true,"epoch":1,"intervalMs":1})]
    )
    .is_err());
}
#[test]
fn extra_spc_memory_layout_includes_fixed_queues_core_and_control_headroom() {
    let (shared, renderer, prepared, core, heap) = extra_spc::layout();
    println!("SPC Shared={shared} Renderer={renderer} Prepared={prepared} CoreHeap={core} FixedHeap={heap} Instance={} Session={} Reserve={}",std::mem::size_of::<extra_psg::Instance>(),std::mem::size_of::<Renderer>(),extra_spc::INSTANCE_BYTES);
    assert_eq!(core, 204624);
    assert!(
        shared + renderer + heap + std::mem::size_of::<extra_psg::Instance>() + 65536
            < extra_spc::INSTANCE_BYTES
    );
    assert!(std::mem::size_of::<extra_psg::Instance>() + 27040 < 65536);
}
#[test]
fn extra_spc_observe_interval_update_preserves_generation_and_pending_ack_credits() {
    let (s, mut r) = new();
    load(&s, &mut r, 1, 1);
    observe(&s, &mut r, 5);
    send(&s, 1, vec![json!({"method":"play"})]).unwrap();
    let mut seq = Vec::new();
    for _ in 0..8 {
        if seq.len() == 4 {
            break;
        }
        render(&s, &mut r, 128);
        seq.extend(
            s.extra_spc_events(1, 5)
                .unwrap()
                .batches
                .into_iter()
                .map(|p| p.sequence),
        );
    }
    send(
        &s,
        1,
        vec![json!({"method":"observe","enabled":true,"epoch":5,"intervalMs":64})],
    )
    .unwrap();
    render(&s, &mut r, 512);
    let e = s.extra_spc_events(1, 5).unwrap();
    assert!(e.reset.is_none() && e.batches.is_empty());
    send(
        &s,
        1,
        vec![json!({"method":"event-ack","epoch":5,"sequence":seq[0]})],
    )
    .unwrap();
    render(&s, &mut r, 1);
    assert!(!s.extra_spc_events(1, 5).unwrap().batches.is_empty());
}
#[test]
fn extra_spc_callback_install_capture_observe_and_retire_fit_audio_stack() {
    const ENV: &str = "TETORICA_EXTRA_SPC_SMALL_STACK";
    if std::env::var_os(ENV).is_none() {
        let result=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","native_genesis::extra_spc_tests::extra_spc_callback_install_capture_observe_and_retire_fit_audio_stack","--nocapture"]).env(ENV,"1").output().unwrap();
        assert!(
            result.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        return;
    }
    let (s, mut r) = new();
    load(&s, &mut r, 1, 1);
    let s = Arc::new(s);
    observe(&s, &mut r, 1);
    send(
        &s,
        1,
        vec![
            json!({"method":"snapshot","id":1}),
            json!({"method":"play"}),
        ],
    )
    .unwrap();
    let shared = s.clone();
    let mut r = Box::new(r);
    r = std::thread::Builder::new()
        .stack_size(64 * 1024)
        .spawn(move || {
            r.prepare(&shared);
            for _ in 0..256 {
                r.sample(&shared);
            }
            r.prepare(&shared);
            r
        })
        .unwrap()
        .join()
        .unwrap();
    assert!(s.extra_spc_status(1, Some(1)).unwrap().snapshot.is_some());
    send(&s, 1, vec![json!({"method":"stop"})]).unwrap();
    let shared = s.clone();
    let r = std::thread::Builder::new()
        .stack_size(64 * 1024)
        .spawn(move || {
            r.prepare(&shared);
            r.sample(&shared);
            r.prepare(&shared);
            r
        })
        .unwrap()
        .join()
        .unwrap();
    assert!(state(&s, 1).paused);
    drop(r);
}
#[test]
fn extra_spc_observer_does_not_change_pcm_or_resampler() {
    fn play(enabled: bool) -> Vec<[f32; 2]> {
        let (s, mut r) = new();
        load(&s, &mut r, 1, 1);
        send(
            &s,
            1,
            vec![json!({"method":"observe","enabled":enabled,"epoch":1,"intervalMs":32})],
        )
        .unwrap();
        render(&s, &mut r, 1);
        send(&s, 1, vec![json!({"method":"play"})]).unwrap();
        render(&s, &mut r, 4096)
    }
    assert_eq!(play(false), play(true));
}
#[test]
fn extra_spc_cancelled_prepared_load_cannot_replace_applied_file() {
    let (s, mut r) = new();
    load(&s, &mut r, 1, 1);
    send(
        &s,
        1,
        vec![scoped(
            json!({"method":"load-spc","data":synthetic(2),"id":1}),
            1,
            10,
        )],
    )
    .unwrap();
    send(
        &s,
        1,
        vec![json!({"method":"cancel-owner","session":1,"owner":10})],
    )
    .unwrap();
    render(&s, &mut r, 1);
    assert!(s
        .extra_spc_status(1, Some(1))
        .unwrap_err()
        .contains("cancelled"));
    assert_eq!(snapshot(&s, &mut r, 1, 2).ram[0x600], 1);
}
#[test]
fn extra_spc_failed_other_family_retain_does_not_commit_cancel_or_discard_capture() {
    let (s, mut r) = new();
    s.create_extra_okim6258(2, None).unwrap();
    load(&s, &mut r, 1, 1);
    seek_begin(&s, &mut r, 10, Some((1, 10)));
    let scope = Some(extra_spc::Scope {
        session: 1,
        owner: 10,
    });
    s.extra_spc_seek_step(1, 10, 200, None.or(scope)).unwrap();
    send(
        &s,
        1,
        vec![scoped(json!({"method":"snapshot","id":1}), 1, 10)],
    )
    .unwrap();
    let oki = |m| {
        serde_json::from_value::<Command>(
            json!({"type":"extra-okim6258","instance":2,"frame":0,"message":m}),
        )
        .unwrap()
    };
    s.enqueue(1,&[oki(json!({"method":"batch","channel":0,"voice":1,"audition":true,"action":"on","commands":[{"method":"write","register":0,"value":2}]}))]).unwrap();
    render(&s, &mut r, 1);
    assert!(s
        .enqueue(
            1,
            &[
                command(1, json!({"method":"cancel-owner","session":1,"owner":10})),
                oki(json!({"method":"clock","hz":4000000}))
            ]
        )
        .unwrap_err()
        .contains("manually held"));
    assert!(s.extra_spc_status(1, Some(1)).unwrap().snapshot.is_some());
    assert_eq!(
        s.extra_spc_seek_step(1, 10, 1, scope)
            .unwrap()
            .spc
            .playback_native_frames,
        201.
    );
}
#[test]
fn extra_spc_after_wait_releases_command_fifo_and_private_seek_is_control_only() {
    let (s, mut r) = new();
    load(&s, &mut r, 1, 1);
    let s = Arc::new(s);
    send(&s, 1, vec![json!({"method":"barrier","id":1})]).unwrap();
    let waiter = s.clone();
    let thread = std::thread::spawn(move || waiter.extra_spc_status(1, Some(1)));
    send(&s, 1, vec![json!({"method":"mute","mask":5})]).unwrap();
    render(&s, &mut r, 1);
    assert_eq!(thread.join().unwrap().unwrap().spc.mute_mask, 5);
    seek_begin(&s, &mut r, 10, None);
    let stepper = s.clone();
    let thread = std::thread::spawn(move || stepper.extra_spc_seek_step(1, 10, 16000, None));
    send(&s, 1, vec![json!({"method":"mute","mask":7})]).unwrap();
    render(&s, &mut r, 1);
    thread.join().unwrap().unwrap();
    assert_eq!(state(&s, 1).mute_mask, 7);
}
#[test]
fn extra_spc_queued_load_then_stop_uses_new_file_separate_or_same_batch() {
    for same_batch in [false, true] {
        let (s, mut r) = new();
        load(&s, &mut r, 1, 1);
        let m = json!({"method":"load-spc","data":synthetic(2),"id":1});
        let stop = json!({"method":"stop","id":2});
        if same_batch {
            send(&s, 1, vec![m, stop]).unwrap();
        } else {
            send(&s, 1, vec![m]).unwrap();
            send(&s, 1, vec![stop]).unwrap();
        }
        render(&s, &mut r, 128);
        assert!(s.extra_spc_status(1, Some(2)).unwrap().spc.paused);
        let c = snapshot(&s, &mut r, 1, 3);
        assert_eq!(c.native_frames, 0);
        assert_eq!(c.ram[0x600], 2);
    }
}
#[test]
fn extra_spc_queued_first_load_snapshot_and_seek_use_accepted_file() {
    let (s, mut r) = new();
    send(
        &s,
        1,
        vec![
            json!({"method":"load-spc","data":synthetic(9),"id":1}),
            json!({"method":"snapshot","id":2}),
            json!({"method":"seek-begin","seekId":10,"id":3}),
        ],
    )
    .unwrap();
    render(&s, &mut r, 1);
    assert_eq!(
        s.extra_spc_status(1, Some(2))
            .unwrap()
            .snapshot
            .unwrap()
            .ram[0x600],
        9
    );
    s.extra_spc_seek_step(1, 10, 50, None).unwrap();
    let c = snapshot(&s, &mut r, 1, 4);
    assert_eq!(c.ram[0x600], 9);
    assert_eq!(c.native_frames, 50);
}
#[test]
fn extra_spc_cancelled_pending_file_cannot_return_through_dependent_stop_or_seek() {
    for seek in [false, true] {
        let (s, mut r) = new();
        load(&s, &mut r, 1, 1);
        send(
            &s,
            1,
            vec![scoped(
                json!({"method":"load-spc","data":synthetic(2),"id":1}),
                1,
                10,
            )],
        )
        .unwrap();
        send(
            &s,
            1,
            vec![if seek {
                json!({"method":"seek-begin","seekId":10,"id":2})
            } else {
                json!({"method":"stop","id":2})
            }],
        )
        .unwrap();
        send(
            &s,
            1,
            vec![json!({"method":"cancel-owner","session":1,"owner":10})],
        )
        .unwrap();
        render(&s, &mut r, 1);
        assert!(s
            .extra_spc_status(1, Some(2))
            .unwrap_err()
            .contains("cancelled"));
        assert_eq!(snapshot(&s, &mut r, 1, 3).ram[0x600], 1);
        if seek {
            assert!(s.extra_spc_seek_step(1, 10, 1, None).is_err());
        }
    }
}
#[test]
fn extra_spc_private_seek_observes_admitted_mute_before_device_callback() {
    let (s, mut r) = new();
    load(&s, &mut r, 1, 1);
    seek_begin(&s, &mut r, 10, None);
    send(&s, 1, vec![json!({"method":"mute","mask":255})]).unwrap();
    assert_eq!(
        s.extra_spc_seek_step(1, 10, 100, None)
            .unwrap()
            .spc
            .mute_mask,
        255
    );
    send(
        &s,
        1,
        vec![
            json!({"method":"mute","mask":17}),
            json!({"method":"snapshot","id":1}),
        ],
    )
    .unwrap();
    render(&s, &mut r, 1);
    assert_eq!(
        s.extra_spc_status(1, Some(1))
            .unwrap()
            .snapshot
            .unwrap()
            .mute_mask,
        17
    );
}
#[test]
fn extra_spc_cancelled_pending_file_cannot_escape_via_private_snapshot_or_consumed_seek() {
    for method in ["snapshot", "play", "pause", "seek-end"] {
        let (s, mut r) = new();
        load(&s, &mut r, 1, 1);
        send(
            &s,
            1,
            vec![scoped(
                json!({"method":"load-spc","data":synthetic(2),"id":1}),
                1,
                10,
            )],
        )
        .unwrap();
        send(
            &s,
            1,
            vec![json!({"method":"seek-begin","seekId":10,"id":2})],
        )
        .unwrap();
        s.extra_spc_seek_step(1, 10, 100, None).unwrap();
        let m = if method == "seek-end" {
            json!({"method":method,"seekId":10,"resume":true,"id":3})
        } else {
            json!({"method":method,"id":3})
        };
        send(&s, 1, vec![m]).unwrap();
        send(
            &s,
            1,
            vec![json!({"method":"cancel-owner","session":1,"owner":10})],
        )
        .unwrap();
        render(&s, &mut r, 1);
        assert!(
            s.extra_spc_status(1, Some(3))
                .unwrap_err()
                .contains("cancelled"),
            "{method}"
        );
        assert_eq!(snapshot(&s, &mut r, 1, 4).ram[0x600], 1, "{method}");
        assert_eq!(state(&s, 1).rendered_native_frames, 0, "{method}");
    }
}
#[test]
fn extra_spc_same_batch_consumed_seek_keeps_flat_pending_file_dependency() {
    let (s, mut r) = new();
    load(&s, &mut r, 1, 1);
    send(
        &s,
        1,
        vec![
            scoped(
                json!({"method":"load-spc","data":synthetic(2),"id":1}),
                1,
                10,
            ),
            json!({"method":"seek-begin","seekId":10,"id":2}),
            json!({"method":"play","id":3}),
        ],
    )
    .unwrap();
    send(
        &s,
        1,
        vec![json!({"method":"cancel-owner","session":1,"owner":10})],
    )
    .unwrap();
    render(&s, &mut r, 1);
    assert!(s
        .extra_spc_status(1, Some(3))
        .unwrap_err()
        .contains("cancelled"));
    assert_eq!(snapshot(&s, &mut r, 1, 4).ram[0x600], 1);
}
