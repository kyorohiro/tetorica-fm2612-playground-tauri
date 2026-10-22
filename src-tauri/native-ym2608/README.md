# Native YM2608 (PC-98)

The Native adapter uses the same BSD-3-Clause ymfm OPNA implementation and
`ymfm_wasm_interface.h` timer behavior as the Browser/WASM YM2608. Shared ymfm
sources live in `../native-genesis`, are hash-checked by this snapshot, and are
compiled only once. The wrapper archive precedes that provider for GNU linkers.

`snapshot/wasm/ym2608_wasm.cpp` pins the exact Web reference for source provenance;
it is not separately compiled into the Native application. The supplied project
rhythm ROM is byte-identical to the Web copy and is uploaded by the host before
playback. It is not a ROM extracted from Yamaha hardware.

`../src/native_genesis_ym2608.cpp` reserves the 8 KiB rhythm ROM and 2 MiB ADPCM-B
RAM at creation. Register, memory, mute, Stop/reset, inspection and render paths
do not allocate or free. Native output uses `Ym2608AudioEngine`'s normalization,
integer source-rate accumulation, double precision averaging and sample hold.
Native FFI creation catches allocation failures. UI status inspection peeks the
extended status without performing the IRQ-changing hardware status read.

Reset retains uploaded memory, clock and source mute settings. Stop removes
active voices while preserving FM patches, SSG pitch/mixer, rhythm levels,
ADPCM sample ranges/pitch, both memories, clock and mute settings. Explicit
SSG levels must be restored when retriggering after Stop, as after key-off.

Synchronize and verify from the Tauri repository:

```sh
python3 scripts/sync_native_ym2608.py --source ../..
python3 scripts/sync_native_ym2608.py --check --source ../..
```

Offline PCM and callback allocation checks from the root checkout:

```sh
node scripts/check_native_ym2608.mjs --out docs/issues/native_pc98_pcm_validation.json
```

No audio output device is opened by these checks.
