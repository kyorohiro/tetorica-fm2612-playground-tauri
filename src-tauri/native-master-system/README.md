# Native Master System

Master System combines the existing project Sega PSG core with ymfm YM2413
(OPLL). Both default clocks are 3,579,545 Hz. Both produce mono duplicated to
left/right before the separate chip mixer gains/pan and global effects.
Game Gear's stereo register is deliberately unavailable in this mode.

The shared OPL provider and PCM support now compile from the pinned SDK's
`native/src/ymfm_opl_provider.cpp` and `vendor/ymfm/ymfm_pcm.cpp`. They are
compiled once in the existing archive order. The SDK version and source bytes
are unchanged. Independent `createSoundChip('ym2413')` instances also compile
the SDK's `native/reference/tauri/native_genesis_ym2413.cpp` and matching header.

Selected Master System and MSX renderers still use the application's
`src/native_genesis_opll.h` (`NativeOpllCore`) and local machine adapters.
The independent SDK adapter has its own renderer; it does not replace that
shared application helper. The local `NativePsgCore` helper, machine controls and
selected MSX AY resampling remain application responsibilities;
the shared AY core, independent AY adapter and OPL/Y8950 adapters now use the
pinned SDK alongside the shared OPL/PCM and ADPCM providers. No whole-machine
adapter is imported from the SDK. See the
[SDK adoption guide](../../docs/native-sdk-ym2612.md).

This directory retains exact snapshots of the shipped Browser/WASM sources.
`scripts/sync_native_master_system.py --check --source /path/to/hello_ymfm`
verifies the pinned sources and the common ymfm/PSG references. The legacy
directory is provenance-only and no longer a Cargo rebuild watch; active SDK
inputs and local machine/helper sources are watched instead. Updating these
snapshots or the old local YM2413 wrapper/provider does not update the SDK build.
The common Sega PSG core now compiles from the pinned SDK's
`vendor/ymfm/segapsg.cpp`, once in the existing `tetorica_native_genesis` archive.
Genesis uses the SDK's raw C ABI wrapper; Master System and Game Gear retain
their local machine wrappers and `NativePsgCore` rate conversion. Updating the
old PSG snapshot does not update that SDK build.

The adapter mirrors Browser PSG clock-resampling phase and OPLL source-rate
averaging/hold, including clocks changed during playback. Stop retains patches,
periods, noise mode, clocks and authored levels; it silences gates without
reconstructing chips or resetting the PSG LFSR. Reset clears synthesis state.
OPLL reset restores a power-on snapshot allocated at construction, matching
Browser `Ym2413.reset()` core recreation without callback construction/free.
Manual OPLL retriggers wait for one ordinary source interval, then key on;
Stop/reset/off cancel that pending key on. Rhythm voices retain their shared
physical operator relationships.

No audio device, callback allocation/free, JavaScript runtime or callback chip
constructor is needed. `native_genesis_master_system_fixture.cpp` exercises the
ABI offline; `scripts/check_native_master_system.mjs` compares both chip planes
against the actual deployed WASM binaries at 44.1/48/96 kHz.

ymfm is copyright Aaron Giles, BSD-3-Clause. See the complete source headers and
`snapshot/LICENSE`. Sega PSG is the project's BSD-3-Clause implementation,
shared with Genesis. Existing application resources ship the ymfm BSD notice.
