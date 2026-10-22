# Native MSX

This snapshot pins the deployed AY-3-8910 WASM synthesis sources and Browser
Worklet/control reference. Synthesis is unchanged. The Native wrapper has no
audio-device, AudioContext, Worker, or JavaScript-runtime dependency.

AY runs at its original 1,789,773 Hz creation clock. Clock changes use the same
phase-preserving average/hold resampling as `ChipClockResampler`; tone, noise,
all sixteen envelope shapes, shape-write retrigger, and I/O registers keep the
deployed core's semantics. Volume-zero gates suppress the same normalized DAC
DC baseline as the Worklet. Stop clears active volume registers while retaining
configured levels, pitch, noise, mixer, envelope and I/O state.

YM2413 is the exact same `native_genesis_opll.h` renderer used by Master System,
with the OPLL source snapshot and shared ymfm provider compiled only once. Its
mono output is separate from AY before the Rust per-chip mixer. Reset restores
a power-on snapshot allocated at construction; callback operations allocate and
free no storage.

AY-3-8910 is BSD-3-Clause, copyright Couriersud; see
`snapshot/third_party/mame-ay8910/LICENSE`. YM2413/ymfm is BSD-3-Clause,
copyright Aaron Giles; see `../native-master-system/snapshot/LICENSE`.

Synchronize from the source repository with
`python3 scripts/sync_native_msx.py --source ../..` and verify with `--check`.
