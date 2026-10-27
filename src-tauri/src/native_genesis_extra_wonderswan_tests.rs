//! Exercise the real chip through the aggregate scheduler without a device.
use super::*;
use serde_json::{json, Value};

fn command(instance: u64, frame: u64, message: Value) -> Command {
    serde_json::from_value(
        json!({"type":"extra-wonderswan","instance":instance,"frame":frame,"message":message}),
    )
    .unwrap()
}
fn send(shared: &Shared, instance: u64, messages: Vec<Value>) {
    shared
        .enqueue(
            1,
            &messages
                .into_iter()
                .map(|m| command(instance, 0, m))
                .collect::<Vec<_>>(),
        )
        .unwrap()
}
fn render(shared: &Shared, renderer: &mut Renderer, frames: usize) -> Vec<[f32; 2]> {
    let pcm = (0..frames)
        .map(|i| {
            if i % 128 == 0 {
                renderer.prepare(shared)
            };
            renderer.sample(shared)
        })
        .collect();
    renderer.prepare(shared);
    pcm
}
fn state(shared: &Shared, instance: u64) -> extra_wonderswan::ChipStatus {
    shared
        .extra_wonderswan_status(instance, None)
        .unwrap()
        .wonderswan
}
fn write(address: u32, value: u8) -> Value {
    json!({"method":"write","address":address,"value":value})
}
fn group(mut message: Value, value: u64) -> Value {
    message["group"] = json!(value);
    message
}
fn tone(ch: u8) -> Vec<Value> {
    let mut commands:Vec<_>=(0..16).map(|i|json!({"method":"memory","address":u32::from(ch)*16+i,"value":if i<8 {0x22}else{0xdd},"channel":ch})).collect();
    commands.extend([
        write(0x80 + u32::from(ch) * 2, 0),
        write(0x81 + u32::from(ch) * 2, 7),
        write(0x88 + u32::from(ch), 0xbb),
        json!({"method":"write","address":0x90,"value":1u8<<ch,"channel":ch,"controlMask":1u8<<ch}),
    ]);
    commands
}
fn batch(ch: u8, voice: u64, action: &str, commands: Vec<Value>, manual: bool) -> Value {
    json!({"method":"batch","channel":ch,"voice":voice,"action":action,"commands":commands,"audition":manual})
}
fn off(ch: u8, voice: u64, manual: bool) -> Value {
    batch(
        ch,
        voice,
        "off",
        vec![json!({"method":"key-off","channel":ch})],
        manual,
    )
}
fn new() -> (Shared, Renderer) {
    let shared = Shared::new(48000, 1);
    let renderer = Renderer::new(48000).unwrap();
    shared.create_extra_wonderswan(1, None).unwrap();
    send(&shared, 1, vec![write(0x91, 8)]);
    (shared, renderer)
}
fn energy(pcm: &[[f32; 2]]) -> f32 {
    pcm.iter().map(|p| p[0].abs() + p[1].abs()).sum()
}

fn compact_trace() -> Value {
    json!({"version":1,"clock":3200000,"durationSamples":44100,
        "events":[[0,0,0x91,8],[0,0,0x94,15],[0,0,0x90,34],[0,1,8,171],[0,2,0,6,3,4]],
        "pcmData":[0,10,2,20,2,30]})
}
#[test]
fn extra_wonderswan_compact_timeline_stays_charged_until_replacement_and_cancels_by_group() {
    let (s, mut r) = new();
    render(&s, &mut r, 1);
    s.extra_psg.collect_retired().unwrap();
    let baseline = s.extra_psg.memory_used();
    send(
        &s,
        1,
        vec![json!({"method":"prepare-timeline","trace":compact_trace()})],
    );
    render(&s, &mut r, 1);
    s.extra_psg.collect_retired().unwrap();
    assert!(
        s.extra_psg.memory_used() > baseline,
        "loaded compact bytes lost their budget permit"
    );
    s.enqueue(
        1,
        &[
            command(
                1,
                500,
                json!({"method":"play-timeline","token":1,"group":9}),
            ),
            command(1, 0, json!({"method":"stop-timeline","group":9})),
        ],
    )
    .unwrap();
    render(&s, &mut r, 600);
    assert_eq!(state(&s, 1).timeline.token, 0);
    send(
        &s,
        1,
        vec![json!({"method":"play-timeline","token":2,"group":9})],
    );
    render(&s, &mut r, 4);
    assert!(state(&s, 1).timeline.playing);
    assert_eq!(state(&s, 1).clock, 3200000);
    assert_eq!(state(&s, 1).memory[8], 171);
    send(
        &s,
        1,
        vec![json!({"method":"cancel-main-owner","targetGroup":9})],
    );
    render(&s, &mut r, 1);
    assert!(state(&s, 1).timeline.cancelled);
    assert!(!state(&s, 1).timeline.playing);
    send(&s, 1, vec![json!({"method":"reset"})]);
    render(&s, &mut r, 1);
    s.extra_psg.collect_retired().unwrap();
    assert_eq!(s.extra_psg.memory_used(), baseline);
}
#[test]
fn extra_wonderswan_compact_timeline_manual_clock_and_density_fences_are_recoverable() {
    let (s, mut r) = new();
    send(&s, 1, vec![batch(0, 1, "on", tone(0), true)]);
    render(&s, &mut r, 1);
    assert!(s
        .enqueue(
            1,
            &[command(
                1,
                0,
                json!({"method":"prepare-timeline","trace":compact_trace()})
            )]
        )
        .unwrap_err()
        .contains("manually held"));
    send(&s, 1, vec![off(0, 1, true)]);
    render(&s, &mut r, 1);
    let mut too_dense = compact_trace();
    too_dense["events"] = json!((0..513).map(|_| vec![0, 0, 0x91, 8]).collect::<Vec<_>>());
    assert!(s
        .enqueue(
            1,
            &[command(
                1,
                0,
                json!({"method":"prepare-timeline","trace":too_dense})
            )]
        )
        .unwrap_err()
        .contains("512 writes"));
    send(
        &s,
        1,
        vec![json!({"method":"prepare-timeline","trace":compact_trace()})],
    );
    render(&s, &mut r, 1);
    send(&s, 1, vec![json!({"method":"play-timeline","token":3})]);
    render(&s, &mut r, 1);
    assert!(s
        .enqueue(1, &[command(1, 0, json!({"method":"clock","hz":4000000}))])
        .unwrap_err()
        .contains("timeline"));
    send(&s, 1, vec![json!({"method":"stop-timeline"})]);
    render(&s, &mut r, 1);
    send(&s, 1, vec![json!({"method":"clock","hz":4000000})]);
    render(&s, &mut r, 1);
    assert_eq!(state(&s, 1).clock, 4000000);
}

#[test]
fn extra_wonderswan_instances_creation_clocks_ram_and_mixer_are_independent() {
    let (s, mut r) = new();
    s.create_extra_wonderswan(2, Some(4_000_001)).unwrap();
    send(&s, 1, vec![batch(0, 1, "on", tone(0), false)]);
    send(
        &s,
        2,
        vec![
            write(0x91, 8),
            batch(2, 2, "on", tone(2), false),
            json!({"method":"mix","volume":0.25,"pan":-1.0}),
        ],
    );
    assert!(energy(&render(&s, &mut r, 3000)) > 1.0);
    let a = state(&s, 1);
    let b = state(&s, 2);
    assert_eq!(a.clock, 3_072_000);
    assert_eq!(b.clock, 4_000_001);
    assert_eq!(a.memory[0], 0x22);
    assert_eq!(b.memory[0], 0);
    assert_eq!(b.memory[32], 0x22);
    assert_eq!(a.memory[32], 0);
    assert_eq!(a.registers[16] & 15, 1);
    assert_eq!(b.registers[16] & 15, 4);
}

#[test]
fn extra_wonderswan_manual_voice_rejects_stale_gate_mode_and_physical_ram_writes() {
    let (s, mut r) = new();
    send(&s, 1, vec![batch(1, 41, "on", tone(1), true)]);
    render(&s, &mut r, 100);
    send(
        &s,
        1,
        vec![
            write(0x90, 0xff),
            write(0x82, 123),
            write(0x8f, 8),
            write(0x91, 0),
            json!({"method":"memory","address":16,"value":255,"channel":0}),
            json!({"method":"reset"}),
            json!({"method":"stop-voices"}),
            off(1, 42, true),
        ],
    );
    render(&s, &mut r, 1);
    let a = state(&s, 1);
    assert_eq!(a.held, [1]);
    assert_eq!(a.registers[16] & 0x22, 2);
    assert_eq!(a.registers[2], 0);
    assert_eq!(a.registers[15], 0);
    assert_eq!(a.registers[17], 8);
    assert_eq!(a.memory[16], 0x22);
    send(&s, 1, vec![off(1, 41, true)]);
    render(&s, &mut r, 1);
    assert!(state(&s, 1).held.is_empty());
    assert_eq!(state(&s, 1).registers[16] & 2, 0);
}

#[test]
fn extra_wonderswan_control_mask_preserves_sibling_modes_and_authored_projection() {
    let (s, mut r) = new();
    send(
        &s,
        1,
        vec![
            group(batch(0, 1, "on", tone(0), false), 1),
            batch(1, 7, "on", tone(1), true),
        ],
    );
    render(&s, &mut r, 100);
    let a = state(&s, 1);
    assert_eq!(a.registers[16] & 15, 3);
    assert_eq!(a.authored_registers[16] & 15, 1);
    send(
        &s,
        1,
        vec![group(
            json!({"method":"write","address":0x90,"value":0x40,"channel":2,"controlMask":0x44}),
            2,
        )],
    );
    render(&s, &mut r, 1);
    let a = state(&s, 1);
    assert_eq!(a.registers[16] & 0x43, 0x43);
    assert_eq!(a.authored_registers[16] & 15, 1);
    send(&s, 1, vec![off(1, 7, true)]);
    render(&s, &mut r, 1);
    assert_eq!(state(&s, 1).registers[16] & 15, 1);
}

#[test]
fn extra_wonderswan_group_cancellation_and_stale_reset_preserve_siblings() {
    let (s, mut r) = new();
    send(
        &s,
        1,
        vec![
            group(batch(0, 1, "on", tone(0), false), 1),
            group(batch(3, 2, "on", tone(3), false), 2),
        ],
    );
    render(&s, &mut r, 100);
    send(
        &s,
        1,
        vec![
            group(json!({"method":"key-off","channel":0}), 2),
            group(json!({"method":"reset"}), 2),
            json!({"method":"cancel-main-owner","targetGroup":2}),
        ],
    );
    render(&s, &mut r, 1);
    let a = state(&s, 1);
    assert_eq!(a.registers[16] & 15, 1);
    assert_eq!(a.memory[0], 0x22);
    assert_eq!(a.memory[48], 0x22);
}

#[test]
fn extra_wonderswan_stop_retains_ram_patch_clock_and_source_mute_reset_clears_ram() {
    let (s, mut r) = new();
    send(
        &s,
        1,
        vec![
            batch(0, 1, "on", tone(0), false),
            json!({"method":"clock","hz":4_000_000}),
            json!({"method":"source-mute","mask":1}),
        ],
    );
    assert!(energy(&render(&s, &mut r, 1000)) < 1e-6);
    send(&s, 1, vec![json!({"method":"stop-voices"})]);
    render(&s, &mut r, 1);
    let a = state(&s, 1);
    assert_eq!(a.clock, 4_000_000);
    assert_eq!(a.source_mute_mask, 1);
    assert_eq!(a.memory[0], 0x22);
    assert_eq!(a.registers[1], 7);
    assert_eq!(a.registers[16] & 15, 0);
    send(&s, 1, vec![json!({"method":"reset"})]);
    render(&s, &mut r, 1);
    let a = state(&s, 1);
    assert_eq!(a.clock, 4_000_000);
    assert_eq!(a.source_mute_mask, 1);
    assert!(a.memory.iter().all(|b| *b == 0));
}

#[test]
fn extra_wonderswan_scheduled_clock_reservation_blocks_new_manual_auditions() {
    let (s, mut r) = new();
    s.enqueue(
        1,
        &[command(
            1,
            1000,
            json!({"method":"clock","hz":4_000_000,"id":1}),
        )],
    )
    .unwrap();
    let mut audition = batch(0, 2, "on", tone(0), true);
    audition["id"] = json!(2);
    send(&s, 1, vec![audition]);
    render(&s, &mut r, 100);
    assert!(s
        .extra_wonderswan_status(1, Some(2))
        .unwrap_err()
        .contains("scheduled"));
    assert!(state(&s, 1).held.is_empty());
    render(&s, &mut r, 1100);
    assert_eq!(
        s.extra_wonderswan_status(1, Some(1))
            .unwrap()
            .wonderswan
            .clock,
        4_000_000
    );
    send(&s, 1, vec![batch(0, 3, "on", tone(0), true)]);
    render(&s, &mut r, 1);
    assert!(s
        .enqueue(
            1,
            &[command(1, 0, json!({"method":"clock","hz":3_000_000}))]
        )
        .unwrap_err()
        .contains("manually held"));
}

#[test]
fn extra_wonderswan_worker_owner_cancel_reclaims_future_payload_without_manual_release() {
    let (s, mut r) = new();
    s.performance("start", 7, None).unwrap();
    send(&s, 1, vec![batch(0, 1, "on", tone(0), true)]);
    let mut c = command(
        1,
        100000,
        json!({"method":"batch","channel":2,"voice":2,"action":"on","commands":tone(2),"id":10}),
    );
    c.performance = Some(7);
    c.owner = Some(1);
    s.enqueue(1, &[c]).unwrap();
    render(&s, &mut r, 100);
    s.performance("cancel-owner", 7, Some(1)).unwrap();
    render(&s, &mut r, 1);
    assert!(s
        .extra_wonderswan_status(1, Some(10))
        .unwrap_err()
        .contains("cancelled"));
    assert_eq!(state(&s, 1).held, [0]);
    assert_eq!(state(&s, 1).registers[16] & 15, 1);
    s.extra_psg.reserve_payloads(std::iter::empty()).unwrap();
    assert_eq!(s.extra_psg.memory_used(), extra_budget::INSTANCE_BYTES);
}

#[test]
fn extra_wonderswan_invalid_ranges_and_cross_channel_batches_are_transactional() {
    let (s, mut r) = new();
    for m in [
        write(0x7f, 0),
        write(0xa0, 0),
        json!({"method":"memory","address":16384,"value":0}),
        json!({"method":"memory","address":0,"value":0,"channel":4}),
        json!({"method":"source-mute","mask":16}),
        json!({"method":"clock","hz":99999}),
        batch(0, 1, "on", vec![write(0x82, 1)], false),
        batch(
            0,
            1,
            "on",
            vec![json!({"method":"write","address":0x90,"value":3,"controlMask":3})],
            false,
        ),
    ] {
        assert!(s
            .enqueue(1, &[command(1, 0, write(0x88, 255)), command(1, 0, m)])
            .is_err());
    }
    render(&s, &mut r, 1);
    assert_eq!(state(&s, 1).registers[8], 0);
}

#[test]
fn extra_wonderswan_slot_limit_disposal_and_budget_reclamation_use_shared_rack() {
    assert!(
        std::mem::size_of::<extra_psg::Instance>()
            + std::mem::size_of::<extra_wonderswan::Shared>()
            + std::mem::size_of::<extra_wonderswan::Renderer>()
            + extra_wonderswan::core_bytes()
            < 64 * 1024
    );
    let (s, mut r) = new();
    for id in 2..=16 {
        s.create_extra_wonderswan(id, None).unwrap();
    }
    assert!(s.create_extra_psg(17, None).unwrap_err().contains("limit"));
    // Retire the setup output-enable command before counting instance storage.
    render(&s, &mut r, 1);
    s.extra_psg.reserve_payloads(std::iter::empty()).unwrap();
    assert_eq!(s.extra_psg.memory_used(), 16 * extra_budget::INSTANCE_BYTES);
    render(&s, &mut r, 1);
    s.dispose_extra_wonderswan(1).unwrap();
    render(&s, &mut r, 1);
    s.extra_psg.collect_retired().unwrap();
    s.create_extra_psg(17, None).unwrap();
    assert_eq!(s.extra_psg.memory_used(), 16 * extra_budget::INSTANCE_BYTES);
}

#[test]
fn extra_wonderswan_group_cancel_releases_only_its_clock_reservation() {
    let (s, mut r) = new();
    s.enqueue(
        1,
        &[
            command(
                1,
                100000,
                group(json!({"method":"clock","hz":4_000_000,"id":1}), 1),
            ),
            command(
                1,
                100000,
                group(json!({"method":"clock","hz":3_000_000,"id":2}), 2),
            ),
        ],
    )
    .unwrap();
    send(
        &s,
        1,
        vec![json!({"method":"cancel-main-owner","targetGroup":1})],
    );
    render(&s, &mut r, 1);
    assert!(s
        .extra_wonderswan_status(1, Some(1))
        .unwrap_err()
        .contains("cancelled"));
    assert_eq!(state(&s, 1).pending_commands, [2]);
    assert_eq!(state(&s, 1).pending_clocks, 1);
    send(
        &s,
        1,
        vec![json!({"method":"cancel-main-owner","targetGroup":2})],
    );
    render(&s, &mut r, 1);
    assert_eq!(state(&s, 1).pending_clocks, 0);
    send(&s, 1, vec![batch(0, 3, "on", tone(0), true)]);
    render(&s, &mut r, 1);
    assert_eq!(state(&s, 1).held, [0]);
}

#[test]
fn extra_wonderswan_failed_other_family_retain_rolls_back_clock_and_payload() {
    let (s, mut r) = new();
    s.create_extra_huc6280(2, None).unwrap();
    let huc = |message: Value| {
        serde_json::from_value::<Command>(
            json!({"type":"extra-huc6280","instance":2,"frame":0,"message":message}),
        )
        .unwrap()
    };
    s.enqueue(1,&[huc(json!({"method":"batch","channel":0,"voice":1,"audition":true,"action":"on","commands":[{"method":"write","register":4,"value":159}]}))]).unwrap();
    render(&s, &mut r, 1);
    assert!(s
        .enqueue(
            1,
            &[
                command(1, 1000, json!({"method":"clock","hz":4_000_000,"id":1})),
                huc(json!({"method":"clock","hz":4_000_000}))
            ]
        )
        .unwrap_err()
        .contains("manually held"));
    render(&s, &mut r, 1);
    assert_eq!(state(&s, 1).pending_clocks, 0);
    send(&s, 1, vec![batch(0, 2, "on", tone(0), true)]);
    render(&s, &mut r, 1);
    assert_eq!(state(&s, 1).held, [0]);
    s.extra_psg.reserve_payloads(std::iter::empty()).unwrap();
    assert_eq!(s.extra_psg.memory_used(), 2 * extra_budget::INSTANCE_BYTES);
}
