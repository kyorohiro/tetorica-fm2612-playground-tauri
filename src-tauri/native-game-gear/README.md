# Native Game Gear

One standalone Sega PSG core, using the same unmodified synthesis provider
already compiled by `native-genesis`. No Genesis, YM2612 or OPLL instance is
created. The shared `NativePsgCore` adapter is also used by Master System.

`scripts/sync_native_game_gear.py --source PATH --check` verifies the pinned
Browser API, inspector, Worklet and resampler against their shared PSG provider.

The eight stereo register bits route each of the four channels independently:
bits 0..3 to right and 4..7 to left. Stop silences current and held PCM immediately,
retaining stereo routing, selected clock, authored periods/noise mode and the
remembered attenuation used for Keyboard previews. It does not reset oscillators
or the noise LFSR. Reset matches the Browser chip, including stereo mask `0xff`.

Clock edits retain the original creation-clock core and fractional resampling
phase, using the shipped Worklet's sample averaging and hold rules. Callback
operations allocate and free nothing. Validation renders PCM offline and never
opens an audio device; Master System parity is checked after the extraction.
