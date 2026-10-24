# Native MSX

This directory retains the deployed AY-3-8910 WASM synthesis sources, Browser
Worklet/control references and notices as provenance. The active AY core and
header now come from the pinned SDK's `vendor/mame-ay8910/`. They compile once
in the existing `tetorica_native_msx` archive. Independent AY instances also use
the SDK's `native/reference/tauri/native_genesis_ay8910.cpp` and matching header.
All four adopted files are byte-identical to their old copies; the SDK version
and source pin are unchanged. The Native wrapper has no
audio-device, AudioContext, Worker, or JavaScript-runtime dependency.

Selected MSX retains its local `src/native_genesis_msx.cpp` machine adapter and
AY resampler. It uses the SDK AY core, while independent instances retain the
SDK wrapper's own tick/resampling path. The local `NativeOpllCore`, machine
mixing and Rust scheduling/ownership are unchanged. See the
[SDK adoption guide](../../docs/native-sdk-ym2612.md).

AY runs at its original 1,789,773 Hz creation clock. Clock changes use the same
phase-preserving average/hold resampling as `ChipClockResampler`; tone, noise,
all sixteen envelope shapes, shape-write retrigger, and I/O registers keep the
deployed core's semantics. Volume-zero gates suppress the same normalized DAC
DC baseline as the Worklet. Stop clears active volume registers while retaining
configured levels, pitch, noise, mixer, envelope and I/O state.

YM2413 is the exact same `native_genesis_opll.h` renderer used by Master System,
with the already adopted SDK OPL/PCM provider compiled only once. Its
mono output is separate from AY before the Rust per-chip mixer. Reset restores
a power-on snapshot allocated at construction; callback operations allocate and
free no storage.

AY-3-8910 is BSD-3-Clause, copyright Couriersud; see
`snapshot/third_party/mame-ay8910/LICENSE`. YM2413/ymfm is BSD-3-Clause,
copyright Aaron Giles; see `../native-master-system/snapshot/LICENSE`.

Synchronize from the source repository with
`python3 scripts/sync_native_msx.py --source ../..` and verify with `--check`.
This updates/verifies the legacy reference snapshots, not the active SDK build.
The old local AY wrapper/core copies are retained. Cargo watches the SDK AY
directory and wrapper/header, and the local machine adapter, instead of this
reference directory. Its existing bundled license path remains available.
