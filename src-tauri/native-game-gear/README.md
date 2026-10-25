# Native Game Gear

One standalone Sega PSG core, using the pinned SDK's `vendor/ymfm/segapsg.cpp`
compiled once in the shared `tetorica_native_genesis` archive. No Genesis,
YM2612 or OPLL instance is created. The local machine adapter and shared
`src/native_genesis_psg.h` (`NativePsgCore`) retain rate conversion and controls;
Master System and additional PSG instances use the same application helper.
The SDK version and source bytes are unchanged. See the
[SDK adoption guide](../../docs/native-sdk-ym2612.md).

`scripts/sync_native_game_gear.py --source PATH --check` verifies the pinned
Browser API, inspector, Worklet and resampler against their shared PSG provider.
These snapshots remain provenance/comparison inputs. Refreshing them or the
old `native-genesis/segapsg.cpp` does not update the active SDK core.

The eight stereo register bits route each of the four channels independently:
bits 0..3 to right and 4..7 to left. Stop silences current and held PCM immediately,
retaining stereo routing, selected clock, authored periods/noise mode and the
remembered attenuation used for Keyboard previews. It does not reset oscillators
or the noise LFSR. Reset matches the Browser chip, including stereo mask `0xff`.

Clock edits retain the original creation-clock core and fractional resampling
phase, using the shipped Worklet's sample averaging and hold rules. Callback
operations allocate and free nothing. Validation renders PCM offline and never
opens an audio device; Master System parity is checked after the extraction.
