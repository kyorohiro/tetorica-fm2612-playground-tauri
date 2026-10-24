# Native YM2610 / YM2610B (Neo Geo)

The Native adapter uses the same BSD-3-Clause ymfm OPNB implementation,
variant channel mask and `ymfm_wasm_interface.h` timers as Browser/WASM.
Both selected and additional instances compile
`native/reference/tauri/native_genesis_ym2610b.cpp` and its matching header from
the pinned SDK under `src-tauri/vendor/tetorica-soundchip-native/`.
The SDK pin is unchanged. Shared OPN/SSG/ADPCM sources also come from the SDK,
and the common provider is compiled once. The OPN wrapper
archives precede that provider so GNU linkers resolve all references.
See the [SDK adoption guide](../../docs/native-sdk-ym2612.md) for paths and updates.

`snapshot/wasm/ym2610b_wasm.cpp` pins the exact Browser reference for source
provenance; it is not compiled separately into the Native application. The
wrapper does not change any synthesis algorithms. Variant 0 selects original
YM2610 physical FM slots 1, 2, 4, 5; variant 1 enables all six as YM2610B.
SSG, six ADPCM-A voices and ADPCM-B remain available in both variants.

The SDK adapter's selected instances reserve independent dense 16 MiB ADPCM-A
and 16 MiB ADPCM-B ROM banks at construction. Upload, per-bank clear, inspection,
register writes, source mute, Stop/reset and sample paths do not allocate or
free memory. Input bounds and bank numbers are checked before copying.
Construction catches allocation failures before returning across Rust FFI.

Additional instances instead borrow immutable 64 KiB pages in two sparse 16 MiB
address spaces. The Rust control side prepares copy-on-write page versions,
retains their owners and budget guards, and schedules pointer publication.
The callback installs pointers without allocating or copying multi-MiB ROMs.
Page lifetime, ownership, retirement and sample preparation remain host duties;
unmapped pages read as zero. The SDK adoption does not move ROM assets or the
Rust page-management implementation.

This directory retains Browser snapshots and provenance. The old
`../src/native_genesis_ym2610b.cpp` and header remain reference copies;
editing them or running the sync script alone does not update the SDK adapter.

Native output exactly follows `Ym2610BAudioEngine`: normalize and clamp each
native source sample, accumulate integer source rate, average using double
precision, and hold the previous sample during upsampling. Both hardware
status reads are observational, including timer IRQ and ADPCM end flags.

Chip reset preserves uploaded ROMs, clock and source mute, matching
`YM2610BSynth.reset()`. The VGM-specific `Ym2610BAudioEngine.reset()` separately
clears ROMs, so callers wanting that behavior explicitly clear both banks.
Stop immediately silences all voices, clears timer/CSM triggers and preserves
FM patches/frequency pairs, SSG pitch/mixer, A sample ranges/levels/pan,
B range/Delta-N/level/pan, ROMs, clock and source mute. SSG levels are restored
by the host on a later key-on, just as after SSG key-off.

Synchronize and verify from the Tauri repository:

```sh
python3 scripts/sync_native_ym2610b.py --source ../..
python3 scripts/sync_native_ym2610b.py --check --source ../..
```

Offline PCM, variant/status/EOS and callback allocation checks from the root:

```sh
node scripts/check_native_ym2610b.mjs --out docs/issues/native_neogeo_pcm_validation.json
```

The fixture deliberately uses deterministic generated sample bytes. These
checks do not open an audio output device.
