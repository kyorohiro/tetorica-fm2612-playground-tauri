//! Offline harness for the exact callback module; binary protocol is private to
//! check_native_scsp.mjs. No audio device or Tauri host is involved.
#[path = "native_genesis_scsp_core.rs"]
mod core;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    io::{self, Read, Write},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};
static HOT: AtomicBool = AtomicBool::new(false);
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static FREES: AtomicU64 = AtomicU64::new(0);
struct Allocator;
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if HOT.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        System.alloc(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        if HOT.load(Ordering::Relaxed) {
            FREES.fetch_add(1, Ordering::Relaxed);
        }
        System.dealloc(p, l)
    }
}
#[global_allocator]
static ALLOCATOR: Allocator = Allocator;
extern "C" {
    fn scsp_probe_enable(on: i32);
    fn scsp_probe_new() -> u64;
    fn scsp_probe_delete() -> u64;
}
fn word(input: &mut &[u8]) -> u32 {
    let (a, b) = input.split_at(4);
    *input = b;
    u32::from_le_bytes(a.try_into().unwrap())
}
struct Event {
    frame: usize,
    kind: u32,
    address: u32,
    value: u16,
    mask: u16,
    bytes: Vec<u8>,
}
fn main() {
    let mut input = Vec::new();
    io::stdin().read_to_end(&mut input).unwrap();
    let mut input = input.as_slice();
    let rate = word(&mut input);
    let frames = word(&mut input) as usize;
    let events_len = word(&mut input);
    let blocks_len = word(&mut input);
    assert!(frames <= 10_000_000 && events_len <= 100_000 && blocks_len > 0 && blocks_len <= 1000);
    let blocks: Vec<_> = (0..blocks_len).map(|_| word(&mut input) as usize).collect();
    assert!(blocks.iter().all(|n| *n > 0));
    let events: Vec<_> = (0..events_len)
        .map(|_| {
            let frame = word(&mut input) as usize;
            let kind = word(&mut input);
            let address = word(&mut input);
            let value = word(&mut input) as u16;
            let mask = word(&mut input) as u16;
            let len = word(&mut input) as usize;
            let (data, tail) = input.split_at(len);
            input = tail;
            Event {
                frame,
                kind,
                address,
                value,
                mask,
                bytes: data.to_vec(),
            }
        })
        .collect();
    assert!(input.is_empty() && events.iter().all(|e| e.frame <= frames));
    assert!(events.windows(2).all(|e| e[0].frame <= e[1].frame));
    let mut chip = core::Core::new(rate).unwrap();
    assert_eq!(chip.output_rate(), rate);
    let mut left = vec![0f32; frames];
    let mut right = vec![0f32; frames];
    let mut at = 0;
    let mut event = 0;
    let mut block = 0;
    unsafe { scsp_probe_enable(1) };
    HOT.store(true, Ordering::SeqCst);
    loop {
        while event < events.len() && events[event].frame == at {
            let e = &events[event];
            match e.kind {
                1 => chip.write(e.address, e.value, e.mask).unwrap(),
                2 => chip.load_ram(e.address, &e.bytes).unwrap(),
                3 => chip.reset(false),
                4 => chip.reset(true),
                5 => chip.stop_voices(),
                _ => panic!("Unknown fixture event"),
            }
            event += 1;
        }
        if at == frames {
            break;
        }
        let end = (at + blocks[block % blocks.len()])
            .min(frames)
            .min(events.get(event).map_or(frames, |e| e.frame));
        chip.render(&mut left[at..end], &mut right[at..end])
            .unwrap();
        at = end;
        block += 1;
    }
    HOT.store(false, Ordering::SeqCst);
    unsafe { scsp_probe_enable(0) };
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    for (l, r) in left.iter().zip(&right) {
        stdout.write_all(&l.to_le_bytes()).unwrap();
        stdout.write_all(&r.to_le_bytes()).unwrap();
    }
    for count in [
        chip.native_frames(),
        chip.output_frames(),
        ALLOCS.load(Ordering::Relaxed),
        FREES.load(Ordering::Relaxed),
        unsafe { scsp_probe_new() },
        unsafe { scsp_probe_delete() },
    ] {
        stdout.write_all(&count.to_le_bytes()).unwrap();
    }
    for at in (0..=0x402).step_by(2).chain([0x408]) {
        stdout
            .write_all(&chip.read(at).unwrap().to_le_bytes())
            .unwrap();
    }
    let mut ram = vec![0; core::RAM_BYTES];
    chip.read_ram(0, &mut ram).unwrap();
    stdout.write_all(&ram).unwrap();
}
