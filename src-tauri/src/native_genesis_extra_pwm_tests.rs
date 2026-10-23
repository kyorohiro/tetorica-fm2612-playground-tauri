//! Additional 32X PWM integration without opening an output device.
use super::*;
use serde_json::{json, Value};
fn cmd(instance: u64, frame: u64, message: Value) -> Command {
    serde_json::from_value(
        json!({"type":"extra-pwm","instance":instance,"frame":frame,"message":message}),
    )
    .unwrap()
}
fn send(s: &Shared, instance: u64, messages: Vec<Value>) {
    s.enqueue(
        1,
        &messages
            .into_iter()
            .map(|m| cmd(instance, 0, m))
            .collect::<Vec<_>>(),
    )
    .unwrap();
}
fn render(s: &Shared, r: &mut Renderer, count: usize) -> Vec<[f32; 2]> {
    let result = (0..count)
        .map(|i| {
            if i % 128 == 0 {
                r.prepare(s)
            }
            r.sample(s)
        })
        .collect();
    r.prepare(s);
    s.frame.store(r.frame, Ordering::Release);
    result
}
fn new() -> (Shared, Renderer) {
    let s = Shared::new(48000, 1);
    let r = Renderer::new(48000).unwrap();
    s.create_extra_pwm(1).unwrap();
    (s, r)
}
fn state(s: &Shared, id: u64) -> extra_pwm::ChipStatus {
    s.extra_pwm_status(id, None).unwrap().pwm
}
fn write(register: u8, value: u16) -> Value {
    json!({"method":"write","register":register,"value":value})
}
fn group(mut m: Value, group: u64) -> Value {
    m["group"] = json!(group);
    m
}
fn schedule(id: u64, base: u64, entries: Value) -> Value {
    json!({"method":"schedule-writes","id":id,"baseFrame":base,"entries":entries})
}
fn configure(s: &Shared, id: u64) {
    send(s, id, vec![write(0, 5), write(1, 4095)]);
}
fn energy(p: &[[f32; 2]]) -> f32 {
    p.iter().flatten().map(|v| v.abs()).sum()
}
#[test]
fn extra_pwm_nineteen_kinds_share_sixteen_slots_and_bounded_layout() {
    let (s, mut r) = new();
    s.create_extra_dx7(2).unwrap();
    s.create_extra_huc6280(3, None).unwrap();
    s.create_extra_nes(4, None, true).unwrap();
    s.create_extra_gameboy(5, None).unwrap();
    s.create_extra_psg(6, None).unwrap();
    s.create_extra_ym2612(7, None).unwrap();
    s.create_extra_ym2203(8, None).unwrap();
    s.create_extra_ym2608(9, None).unwrap();
    s.create_extra_ym2610(10, None).unwrap();
    s.create_extra_ym2151(11, None).unwrap();
    s.create_extra_ym2413(12, None).unwrap();
    s.create_extra_ay8910(13, None).unwrap();
    s.create_extra_ym3526(14, None).unwrap();
    s.create_extra_ym3812(15, None).unwrap();
    s.create_extra_y8950(16, None).unwrap();
    assert!(s
        .create_extra_ymf262(17, None)
        .unwrap_err()
        .contains("limit"));
    render(&s, &mut r, 1);
    s.dispose_extra_pwm(1).unwrap();
    render(&s, &mut r, 1);
    s.extra_psg.collect_retired().unwrap();
    s.create_extra_ymf262(17, None).unwrap();
    println!(
        "PWM Instance={} Shared={} Renderer={} Session={} Fixed={} Entry={}",
        std::mem::size_of::<extra_psg::Instance>(),
        std::mem::size_of::<extra_pwm::Shared>(),
        std::mem::size_of::<extra_pwm::Renderer>(),
        std::mem::size_of::<Renderer>(),
        extra_pwm::INSTANCE_BYTES,
        std::mem::size_of::<extra_pwm::Write>()
    );
    assert!(std::mem::size_of::<extra_psg::Instance>() + 27040 < 65536);
}
#[test]
fn extra_pwm_sixteen_instances_fit_shared_memory_budget() {
    let (s, mut r) = new();
    for id in 2..=16 {
        s.create_extra_pwm(id).unwrap();
    }
    assert_eq!(s.extra_psg.memory_used(), 16 * extra_pwm::INSTANCE_BYTES);
    assert!(s.extra_psg.memory_used() < extra_budget::DEFAULT_LIMIT);
    render(&s, &mut r, 1);
    s.extra_psg.deactivate_all().unwrap();
    render(&s, &mut r, 1);
    s.extra_psg.collect_retired().unwrap();
    assert_eq!(s.extra_psg.memory_used(), 0);
}
#[test]
fn extra_pwm_clock_registers_fifo_and_latches_are_independent() {
    let (s, mut r) = new();
    s.create_extra_pwm(2).unwrap();
    configure(&s, 1);
    configure(&s, 2);
    send(
        &s,
        1,
        vec![
            write(2, 1000),
            write(3, 3000),
            json!({"method":"clock","hz":24000000}),
        ],
    );
    send(&s, 2, vec![write(2, 2000)]);
    render(&s, &mut r, 20);
    let a = state(&s, 1);
    let b = state(&s, 2);
    assert_eq!(a.clock, 24000000);
    assert_eq!(b.clock, extra_pwm::CLOCK);
    assert_eq!(a.left, Some(1000));
    assert_eq!(a.right, Some(3000));
    assert_eq!(b.left, Some(2000));
    assert_eq!(b.right, None);
}
#[test]
fn extra_pwm_pcm_matches_primary_at_three_rates_and_exact_boundaries() {
    for rate in [44100, 48000, 96000] {
        let s = Shared::new(rate, 1);
        let mut r = Renderer::new(rate).unwrap();
        s.create_extra_pwm(1).unwrap();
        let p = Shared::new_modes(rate, 1, false, false, false, Some("32x"));
        let mut pr = Renderer::new_modes(rate, false, false, false, Some("32x")).unwrap();
        let messages = [
            (0, write(0, 5)),
            (0, write(1, 1000)),
            (
                0,
                schedule(
                    1,
                    0,
                    json!([{"frame":0,"register":4,"value":500},{"frame":1,"register":2,"value":800},{"frame":17,"register":3,"value":100},{"frame":129,"register":4,"value":900}]),
                ),
            ),
            (500, json!({"method":"clock","hz":20000000})),
            (700, json!({"method":"clear-schedule"})),
        ];
        s.enqueue(
            1,
            &messages
                .iter()
                .map(|(f, m)| cmd(1, *f, m.clone()))
                .collect::<Vec<_>>(),
        )
        .unwrap();
        p.enqueue(
            1,
            &messages
                .iter()
                .map(|(f, m)| {
                    serde_json::from_value(json!({"type":"pwm","frame":f,"message":m})).unwrap()
                })
                .collect::<Vec<_>>(),
        )
        .unwrap();
        let a = render(&s, &mut r, 4096);
        let b = render(&p, &mut pr, 4096);
        assert_eq!(a, b, "rate {rate}");
        assert!(a.iter().flatten().all(|v| v.is_finite()));
    }
}
#[test]
fn extra_pwm_added_late_uses_session_absolute_frame() {
    let s = Shared::new(48000, 1);
    let mut r = Renderer::new(48000).unwrap();
    render(&s, &mut r, 500);
    s.create_extra_pwm(1).unwrap();
    configure(&s, 1);
    send(
        &s,
        1,
        vec![
            json!({"method":"schedule-writes","id":1,"entries":[{"frame":2,"register":2,"value":99}]}),
        ],
    );
    render(&s, &mut r, 1);
    let a = s.extra_pwm_status(1, Some(1)).unwrap().pwm;
    assert_eq!(a.last_schedule.unwrap().base_frame, 500);
    assert_eq!(a.queued_writes, 1);
    render(&s, &mut r, 1);
    assert_eq!(state(&s, 1).left_fifo, [99]);
}
#[test]
fn extra_pwm_future_reservations_ack_admission_and_keep_individual_results() {
    let (s, mut r) = new();
    configure(&s, 1);
    send(
        &s,
        1,
        vec![
            schedule(1, 10000, json!([{"frame":2,"register":2,"value":11}])),
            schedule(2, 20000, json!([{"frame":3,"register":3,"value":22}])),
        ],
    );
    render(&s, &mut r, 1);
    for (id, base) in [(1, 10000), (2, 20000)] {
        let a = s.extra_pwm_status(1, Some(id)).unwrap().pwm;
        let result = a.last_schedule.unwrap();
        assert_eq!((result.id, result.base_frame), (id, base));
        assert_eq!(a.queued_writes, 2);
    }
    assert_eq!(state(&s, 1).last_schedule.unwrap().id, 2);
}
#[test]
fn extra_pwm_stable_sort_and_three_entry_fifo_overflow() {
    let (s, mut r) = new();
    configure(&s, 1);
    send(
        &s,
        1,
        vec![schedule(
            1,
            100,
            json!([{"frame":3,"register":2,"value":4},{"frame":2,"register":2,"value":1},{"frame":2,"register":2,"value":2},{"frame":2,"register":2,"value":3}]),
        )],
    );
    render(&s, &mut r, 104);
    assert_eq!(state(&s, 1).left_fifo, [2, 3, 4]);
}
#[test]
fn extra_pwm_main_cancel_preserves_foreign_queues_fifo_and_latches() {
    let (s, mut r) = new();
    configure(&s, 1);
    send(
        &s,
        1,
        vec![group(write(2, 1000), 1), group(write(3, 3000), 2)],
    );
    render(&s, &mut r, 20);
    send(
        &s,
        1,
        vec![
            group(write(2, 1100), 1),
            group(write(3, 3100), 2),
            group(
                schedule(1, 1000, json!([{"frame":0,"register":2,"value":1200}])),
                1,
            ),
            group(
                schedule(2, 1000, json!([{"frame":0,"register":3,"value":3200}])),
                2,
            ),
        ],
    );
    render(&s, &mut r, 1);
    send(
        &s,
        1,
        vec![json!({"method":"cancel-main-owner","targetGroup":1})],
    );
    render(&s, &mut r, 1);
    let a = state(&s, 1);
    assert_eq!(a.left, None);
    assert_eq!(a.left_fifo, Vec::<u32>::new());
    assert_eq!(a.right, Some(3000));
    assert_eq!(a.right_fifo, [3100]);
    assert_eq!(a.queued_writes, 1);
    assert_eq!(a.registers[0], 5);
    assert_eq!(a.cycle, 4095);
    render(&s, &mut r, 1100);
    assert_eq!(state(&s, 1).right, Some(3200));
    assert_eq!(state(&s, 1).left, None);
}
#[test]
fn extra_pwm_main_cancel_invalidates_only_target_outer_lookahead() {
    let (s, mut r) = new();
    s.enqueue(
        1,
        &[
            cmd(
                1,
                1000,
                group(json!({"method":"clock","hz":20000000,"id":1}), 1),
            ),
            cmd(
                1,
                1000,
                group(json!({"method":"clock","hz":24000000,"id":2}), 2),
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
        .extra_pwm_status(1, Some(1))
        .unwrap_err()
        .contains("cancelled"));
    assert_eq!(state(&s, 1).pending_commands, [2]);
    render(&s, &mut r, 1100);
    assert_eq!(s.extra_pwm_status(1, Some(2)).unwrap().pwm.clock, 24000000);
}
#[test]
fn extra_pwm_worker_cancel_preserves_unowned_and_other_worker_sources() {
    let (s, mut r) = new();
    configure(&s, 1);
    s.performance("start", 7, None).unwrap();
    let tagged = |m, owner| {
        let mut c = cmd(1, 0, m);
        c.performance = Some(7);
        c.owner = Some(owner);
        c
    };
    s.enqueue(
        1,
        &[
            tagged(write(2, 1000), 1),
            tagged(write(3, 3000), 2),
            tagged(
                schedule(1, 1000, json!([{"frame":0,"register":2,"value":1100}])),
                1,
            ),
            tagged(
                schedule(2, 1000, json!([{"frame":0,"register":3,"value":3100}])),
                2,
            ),
        ],
    )
    .unwrap();
    render(&s, &mut r, 20);
    s.performance("cancel-owner", 7, Some(1)).unwrap();
    render(&s, &mut r, 1);
    let a = state(&s, 1);
    assert_eq!(a.left, None);
    assert_eq!(a.right, Some(3000));
    assert_eq!(a.queued_writes, 1);
    s.performance("stop", 7, None).unwrap();
    render(&s, &mut r, 1);
    assert_eq!(state(&s, 1).right, None);
    assert_eq!(state(&s, 1).queued_writes, 0);
}
#[test]
fn extra_pwm_clear_schedule_keeps_other_group_and_existing_latched_output() {
    let (s, mut r) = new();
    configure(&s, 1);
    send(&s, 1, vec![group(write(2, 1000), 1)]);
    render(&s, &mut r, 20);
    send(
        &s,
        1,
        vec![
            group(
                schedule(1, 0, json!([{"frame":1000,"register":2,"value":1100}])),
                1,
            ),
            group(
                schedule(2, 0, json!([{"frame":1000,"register":3,"value":3100}])),
                2,
            ),
        ],
    );
    render(&s, &mut r, 1);
    send(
        &s,
        1,
        vec![group(json!({"method":"clear-schedule","order":10}), 1)],
    );
    render(&s, &mut r, 1);
    let a = state(&s, 1);
    assert_eq!(a.queued_writes, 1);
    assert_eq!(a.left, Some(1000));
}
#[test]
fn extra_pwm_stop_silences_preserves_clock_registers_and_cancels_future() {
    let (s, mut r) = new();
    configure(&s, 1);
    send(
        &s,
        1,
        vec![write(4, 1000), json!({"method":"clock","hz":24000000})],
    );
    assert!(energy(&render(&s, &mut r, 100)) > 0.);
    s.enqueue(
        1,
        &[cmd(1, 1000, json!({"method":"clock","hz":20000000,"id":1}))],
    )
    .unwrap();
    send(
        &s,
        1,
        vec![
            schedule(2, 1000, json!([{"frame":0,"register":4,"value":2000}])),
            json!({"method":"stop-voices"}),
        ],
    );
    assert!(energy(&render(&s, &mut r, 1200)) < 1e-5);
    let a = state(&s, 1);
    assert_eq!(a.clock, 24000000);
    assert_eq!(a.registers[0], 5);
    assert_eq!(a.cycle, 4095);
    assert_eq!(a.queued_writes, 0);
    assert!(s
        .extra_pwm_status(1, Some(1))
        .unwrap_err()
        .contains("cancelled"));
}
#[test]
fn extra_pwm_reset_preserves_clock_and_mixer_but_clears_chip_state() {
    let (s, mut r) = new();
    configure(&s, 1);
    send(
        &s,
        1,
        vec![
            write(4, 1000),
            json!({"method":"clock","hz":24000000}),
            json!({"method":"mix","volume":0.0,"pan":0.5}),
        ],
    );
    render(&s, &mut r, 300);
    send(&s, 1, vec![json!({"method":"reset"})]);
    render(&s, &mut r, 1);
    let a = state(&s, 1);
    assert_eq!(a.clock, 24000000);
    assert_eq!(a.registers[0], 0);
    assert_eq!(a.cycle, 0);
    assert_eq!(a.left, None);
    assert_eq!(a.queued_writes, 0);
    configure(&s, 1);
    send(&s, 1, vec![write(4, 1000)]);
    assert!(energy(&render(&s, &mut r, 100)) < 1e-5);
}
#[test]
fn extra_pwm_invalid_transaction_and_duplicate_ids_are_atomic() {
    let (s, mut r) = new();
    render(&s, &mut r, 1);
    let initial = state(&s, 1).registers;
    for m in [
        write(5, 1),
        json!({"method":"clock","hz":99999}),
        json!({"method":"mix","volume":5.0,"pan":0.0}),
        json!({"method":"cancel-main-owner","targetGroup":0}),
        schedule(
            1,
            MAX_SAFE_INTEGER,
            json!([{"frame":1,"register":2,"value":0}]),
        ),
    ] {
        assert!(s
            .enqueue(1, &[cmd(1, 0, write(0, 5)), cmd(1, 0, m)])
            .is_err());
    }
    assert!(s
        .enqueue(
            1,
            &[cmd(
                1,
                1,
                json!({"method":"cancel-main-owner","targetGroup":1})
            )]
        )
        .is_err());
    let c = || cmd(1, 1000, json!({"method":"clock","hz":24000000,"id":1}));
    assert!(s.enqueue(1, &[c(), c()]).unwrap_err().contains("Duplicate"));
    render(&s, &mut r, 1);
    assert_eq!(state(&s, 1).registers, initial);
    assert!(state(&s, 1).pending_commands.is_empty());
}
#[test]
fn extra_pwm_payload_budget_includes_pending_and_retiring_uploads() {
    let mut s = Shared::new(48000, 1);
    s.extra_psg = extra_psg::Shared::with_memory_limit(extra_pwm::INSTANCE_BYTES + 800);
    let mut r = Renderer::new(48000).unwrap();
    s.create_extra_pwm(1).unwrap();
    let upload = || {
        cmd(
            1,
            0,
            schedule(
                1,
                1000,
                json!(vec![json!({"frame":0,"register":2,"value":1}); 32]),
            ),
        )
    };
    assert!(s.enqueue(1, &[upload()]).unwrap_err().contains("budget"));
    assert_eq!(s.extra_psg.memory_used(), extra_pwm::INSTANCE_BYTES);
    send(
        &s,
        1,
        vec![schedule(
            1,
            1000,
            json!([{"frame":0,"register":2,"value":1}]),
        )],
    );
    render(&s, &mut r, 1);
    s.extra_psg.collect_retired().unwrap();
    assert!(s.extra_psg.memory_used() > extra_pwm::INSTANCE_BYTES);
    send(&s, 1, vec![json!({"method":"clear-schedule"})]);
    render(&s, &mut r, 1);
    s.extra_psg.collect_retired().unwrap();
    assert_eq!(s.extra_psg.memory_used(), extra_pwm::INSTANCE_BYTES);
}
#[test]
fn extra_pwm_dispose_stop_and_same_public_id_leave_clean_monotonic_instance() {
    let (s, mut r) = new();
    configure(&s, 1);
    send(
        &s,
        1,
        vec![
            write(4, 1000),
            schedule(1, 100000, json!([{"frame":0,"register":4,"value":2000}])),
        ],
    );
    render(&s, &mut r, 100);
    s.dispose_extra_pwm(1).unwrap();
    render(&s, &mut r, 1);
    s.extra_psg.collect_retired().unwrap();
    assert_eq!(s.extra_psg.memory_used(), 0);
    assert!(s.create_extra_pwm(1).is_err());
    assert!(s.enqueue(1, &[cmd(1, 0, write(4, 2000))]).is_err());
    s.create_extra_pwm(2).unwrap();
    assert_eq!(state(&s, 2).registers, [0, 0, 0x4000, 0x4000, 0x4000]);
    assert_eq!(state(&s, 2).clock, extra_pwm::CLOCK);
}
#[test]
fn extra_pwm_completion_wait_does_not_hold_control_and_history_expires() {
    use std::{sync::mpsc, time::Duration};
    let (s, mut r) = new();
    let s = Arc::new(s);
    s.enqueue(
        1,
        &[cmd(1, 1000, json!({"method":"clock","hz":24000000,"id":1}))],
    )
    .unwrap();
    send(
        &s,
        1,
        (2..=601)
            .map(|id| json!({"method":"barrier","id":id}))
            .collect(),
    );
    render(&s, &mut r, 1);
    state(&s, 1);
    assert!(s
        .extra_pwm_status(1, Some(2))
        .unwrap_err()
        .contains("expired"));
    assert!(s.extra_pwm_status(1, Some(601)).is_ok());
    let (tx, rx) = mpsc::channel();
    let waiting = s.clone();
    let task = std::thread::spawn(move || tx.send(waiting.extra_pwm_status(1, Some(1))).unwrap());
    assert!(rx.recv_timeout(Duration::from_millis(20)).is_err());
    send(&s, 1, vec![write(0, 5)]);
    render(&s, &mut r, 1200);
    assert_eq!(
        rx.recv_timeout(Duration::from_millis(200))
            .unwrap()
            .unwrap()
            .pwm
            .clock,
        24000000
    );
    task.join().unwrap();
}
#[test]
fn extra_pwm_failed_other_family_retain_does_not_commit_main_cancel() {
    let (s, mut r) = new();
    s.create_extra_okim6258(2, None).unwrap();
    s.enqueue(
        1,
        &[cmd(
            1,
            1000,
            group(json!({"method":"clock","hz":24000000,"id":1}), 42),
        )],
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
                cmd(1, 0, json!({"method":"cancel-main-owner","targetGroup":42})),
                oki(json!({"method":"clock","hz":4000000}))
            ]
        )
        .unwrap_err()
        .contains("manually held"));
    render(&s, &mut r, 1200);
    assert_eq!(s.extra_pwm_status(1, Some(1)).unwrap().pwm.clock, 24000000);
}

#[test]
fn extra_pwm_scope_table_full_schedule_and_fifo_boundary_never_allocates_a_control_tag() {
    let (s, mut r) = new();
    configure(&s, 1);
    send(
        &s,
        1,
        (1..=extra_pwm::CURSORS as u64)
            .map(|id| {
                group(
                    schedule(id, 100000, json!([{"frame":0,"register":2,"value":1000}])),
                    id,
                )
            })
            .collect(),
    );
    send(
        &s,
        1,
        vec![group(write(2, 1000), 10001), group(write(3, 3000), 10002)],
    );
    render(&s, &mut r, 20);
    send(
        &s,
        1,
        vec![
            group(write(2, 1100), 10003),
            group(write(2, 1200), 10004),
            group(write(2, 1300), 10005),
            group(write(3, 3100), 10006),
            group(write(3, 3200), 10007),
            group(write(3, 3300), 10008),
            group(json!({"method":"barrier"}), 10010),
            group(json!({"method":"clock","hz":24000000}), 10011),
            group(write(2, 1400), 10009),
        ],
    );
    render(&s, &mut r, 1);
    assert_eq!(state(&s, 1).left_fifo, [1200, 1300, 1400]);
    assert_eq!(state(&s, 1).queued_writes, extra_pwm::CURSORS);
    assert!(s
        .enqueue(
            1,
            &[cmd(
                1,
                0,
                group(
                    schedule(2000, 100000, json!([{"frame":0,"register":2,"value":1}])),
                    2000
                )
            )]
        )
        .unwrap_err()
        .contains("1024 batches"));
    let mut admission_ns = [0u128; 32];
    for (index, elapsed) in admission_ns.iter_mut().enumerate() {
        send(
            &s,
            1,
            vec![group(write(2, 1400 + index as u16), 20000 + index as u64)],
        );
        r.prepare(&s);
        let start = std::time::Instant::now();
        let _sample = r.sample(&s);
        *elapsed = start.elapsed().as_nanos();
    }
    admission_ns.sort_unstable();
    println!("PWM debug callback new-scope 1024 active batches / 32 trials: median={} ns p97={} ns max={} ns", admission_ns[16], admission_ns[30], admission_ns[31]);
    let (entry, scope, completion) = extra_pwm::Renderer::layout_bytes();
    let fixed = std::mem::size_of::<extra_psg::Instance>()
        + 168
        + 2 * CAPACITY * 16
        + extra_pwm::CURSORS * entry
        + (extra_pwm::CURSORS + 9) * scope
        + 512 * completion;
    println!(
        "PWM fixedactualupper={} reserved={} entry={} scope={} completion={}",
        fixed,
        extra_pwm::INSTANCE_BYTES,
        entry,
        scope,
        completion
    );
    assert!(fixed < extra_pwm::INSTANCE_BYTES);
}
#[test]
fn extra_pwm_ordered_reset_keeps_later_reservation_scope_until_it_runs() {
    let (s, mut r) = new();
    configure(&s, 1);
    // A future reservation is admitted now but logically occurs after Reset.
    send(
        &s,
        1,
        vec![group(
            json!({"method":"schedule-writes","id":1,"order":20,"baseFrame":100,"entries":[{"frame":0,"register":0,"value":5},{"frame":0,"register":1,"value":4095},{"frame":0,"register":2,"value":1100}]}),
            7,
        )],
    );
    s.enqueue(
        1,
        &[cmd(1, 100, group(json!({"method":"reset","order":10}), 7))],
    )
    .unwrap();
    render(&s, &mut r, 110);
    let a = state(&s, 1);
    assert_eq!(a.queued_writes, 0);
    assert_eq!(a.left, Some(1100));
}
#[test]
fn extra_pwm_completed_reservation_history_returns_requested_base() {
    let (s, mut r) = new();
    send(
        &s,
        1,
        vec![schedule(1, 100, json!([])), schedule(2, 200, json!([]))],
    );
    render(&s, &mut r, 1);
    state(&s, 1);
    for (id, base) in [(1, 100), (2, 200)] {
        let result = s
            .extra_pwm_status(1, Some(id))
            .unwrap()
            .pwm
            .last_schedule
            .unwrap();
        assert_eq!((result.id, result.base_frame), (id, base));
    }
}

#[test]
fn extra_pwm_grouped_explicit_reset_resets_hardware_but_keeps_foreign_future_scope() {
    let (s, mut r) = new();
    configure(&s, 1);
    send(
        &s,
        1,
        vec![
            group(write(4, 1000), 1),
            group(
                schedule(
                    1,
                    1000,
                    json!([{"frame":0,"register":0,"value":5},{"frame":0,"register":1,"value":4095},{"frame":0,"register":3,"value":3100}]),
                ),
                2,
            ),
        ],
    );
    render(&s, &mut r, 20);
    send(&s, 1, vec![group(json!({"method":"reset","order":10}), 1)]);
    render(&s, &mut r, 1);
    let a = state(&s, 1);
    assert_eq!(a.registers[0], 0);
    assert_eq!(a.cycle, 0);
    assert_eq!(a.left, None);
    assert_eq!(a.right, None);
    assert_eq!(a.queued_writes, 3);
    render(&s, &mut r, 1100);
    let a = state(&s, 1);
    assert_eq!(a.right, Some(3100));
    assert_eq!(a.queued_writes, 0);
}
