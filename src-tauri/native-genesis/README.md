# Native Genesis source snapshot

YM2612 and the shared Sega PSG core/raw C ABI now build from the pinned SDK distribution under
`../vendor/tetorica-soundchip-native/`; see
[the SDK adoption guide](../../docs/native-sdk-ym2612.md). The files in this
directory retain the checked legacy snapshots and remaining application fallback
inputs, so Cargo still watches the directory. Updating these copies does not
update the SDK-backed YM2612 or Sega PSG build. The SDK's `wasm/segapsg_wasm.cpp`
and `vendor/ymfm/segapsg.cpp` compile once in `tetorica_native_genesis`, shared by
Genesis/Nuked, Master System, Game Gear and additional PSG instances. The common
OPN/SSG/ADPCM provider also builds once from the SDK.

These legacy files were copied without synthesis changes from the parent hello_ymfm
repository (`src/` and `wasm/`) on 2026-10-21. They preserve the same YM2612
reset, timer/IRQ interface and Sega PSG behavior as the Playground WASM build.
The matching SDK sources and retained snapshots let this Tauri repository build
independently. The local `native_genesis_core.cpp` retains Genesis/Nuked gains,
DC centering, PSG mixing and rate conversion. Master System/Game Gear retain
their local machine adapters and `NativePsgCore` helper; adopting the core does
not import a complete machine adapter. No JavaScript or Web Audio PCM transfer
is involved.

The commands below refresh or verify the legacy references; they do not update
the active SDK inputs. Deliberate SDK updates follow the adoption guide. Keep
the hardware-free comparisons and original copyright/license headers.

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
