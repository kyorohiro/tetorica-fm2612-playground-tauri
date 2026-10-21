# Native NES + FDS

`../src/native_genesis_nes.cpp` is the standalone Native mirror of the canonical
`hello_ymfm/third_party/tetorica-nes/nes_core.cpp`. This is a direct C++ port of
the pinned JavaScript NES/FDS implementation, shared with Browser WASM. It does
not substitute a different emulator. The
original source files and source notices are retained under `snapshot/`, with
SHA-256 hashes in `sources.json`.

- JSNES APU: Apache-2.0, revision
  `b8a45d088e922b6abcd87c4eb1b7237919e9d3d2`; see `LICENSE-JSNES` and the
  snapshotted JSNES authors/readme.
- FDS: MIT, FIX94's fixNES revision
  `156fcaca9f4cd9c23d423737994c45cfb05d16ca`; see `LICENSE-FDS` and the
  snapshotted adaptation/readme. No NSFPlay source or voice bank is included.

The port retains the JSNES channel/frame-counter algorithms and NTSC/PAL
periods, current one-CPU-cycle host advancement, nonlinear lookup mixing,
integer high-pass, and the wrapper's 10 Hz DC removal. In particular, the
APU/FDS mix is rounded to float32 **before** the outer filter consumes it, as
in `NesApu.generateStereo`. Explicit helpers preserve JavaScript int32 shifts
and floor behavior; double filter/clock accumulators are retained.

FDS keeps the current JavaScript wave/modulation/envelope/divide-by-16 logic,
2 kHz low-pass, 20 Hz DC filter and approximate mix gain. The output follows
that implementation, rather than claiming calibrated hardware analog balance.

The core owns fixed 64 KiB DMC RAM, fixed FDS memories, fixed lookup tables
and filter state. Creation and destruction run outside audio callbacks. Writes,
RAM copies, reset, gate-off, snapshots and per-frame generation allocate and
free no memory. The offline fixture audits `new`/`delete` during those paths.

Public reset clears RAM and FDS wave/modulation memories and resets filter and
resampler phase. Gate-off preserves applied patch/wave/RAM settings. Clock
changes use the existing `ChipClockResampler` ratio/average/hold algorithm,
keeping the authored base-region core and its phase; they do not recreate it.

From the Tauri repository:

```sh
python3 scripts/sync_native_nes.py --source ../..
python3 scripts/sync_native_nes.py --check --source ../..
```

Synchronizing updates the **C++ mirror**, adapting only the local header include.
`core.json` records canonical and mirrored SHA-256 hashes. The original JavaScript
reference is retained and checked against `sources.json`; it need not match the
current Browser wrapper, which now loads WASM. Native `--check` without `--source`
verifies the local mirror and pinned reference without the root repository.

Replacing the pinned JavaScript reference requires `--refresh-reference`:
review/port any DSP changes and rerun the offline PCM comparison before accepting
the new hashes. From `hello_ymfm`:

```sh
node scripts/check_native_nes.mjs --out /private/tmp/native-nes-parity.json
```

The reference uses JavaScript `NesApu`, with the existing clock resampler when
requested. The fixture compares native float32 stereo output without opening an
audio device. Realtime listening and hardware analog accuracy remain separate
validation work.
