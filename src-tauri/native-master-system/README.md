# Native Master System

Master System combines the existing project Sega PSG core with ymfm YM2413
(OPLL). Both default clocks are 3,579,545 Hz. Both produce mono duplicated to
left/right before the separate chip mixer gains/pan and global effects.
Game Gear's stereo register is deliberately unavailable in this mode.

The synthesis sources are exact snapshots of the shipped Browser/WASM sources.
`scripts/sync_native_master_system.py --check --source /path/to/hello_ymfm`
verifies the pinned sources and the common ymfm/PSG provider. The provider is
compiled only once by `native-genesis`; the OPLL wrapper/archive precedes it in
GNU static archive order. The OPLL translation unit also includes the PCM
implementation used by other classes in that upstream translation unit.

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
