# Native Genesis source snapshot

YM2612 now builds from the pinned SDK distribution under
`../vendor/tetorica-soundchip-native/`; see
[the SDK adoption guide](../../docs/native-sdk-ym2612.md). The files in this
directory remain a checked legacy snapshot and supply the existing Sega PSG
integration and shared headers. Updating this snapshot does not update the
SDK-backed YM2612 build. The shared OPN/SSG/ADPCM provider is built once from
the SDK; other application adapters continue to consume it.

These files are copied without synthesis changes from the parent hello_ymfm
repository (`src/` and `wasm/`) on 2026-10-21. They preserve the same YM2612
reset, timer/IRQ interface and Sega PSG behavior as the Playground WASM build.
The source is included here so this Tauri repository builds independently.
`native_genesis_core.cpp` adds device-rate conversion and mixing around these
wrappers; no JavaScript or Web Audio PCM transfer is involved.

When updating the upstream sound cores, synchronize these files together and
run the hardware-free native_genesis tests. Original copyright/license headers
are retained in each source.

From the Tauri repository root:

```sh
python3 scripts/sync_native_genesis.py --check
python3 scripts/sync_native_genesis.py --check --source /path/to/hello_ymfm
python3 scripts/sync_native_genesis.py --source /path/to/hello_ymfm
node scripts/check_native_genesis_core.mjs /path/to/hello_ymfm
```

`sources.json` records each upstream path and SHA-256. The comparison test
compiles an offline native renderer and compares FM pitch and every output PCM
sample against the actual Playground WASM cores and Worklet resampling method
at 44.1, 48 and 96 kHz; it does not open an audio device.
