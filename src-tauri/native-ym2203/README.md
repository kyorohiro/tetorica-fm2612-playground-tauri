# Native PC-98 YM2203

One selected YM2203 provides three FM and three SSG voices. It reuses the exact
OPN/SSG provider already compiled once in the `native-genesis` archive; it constructs no
Genesis or YM2608 chip. Existing ymfm BSD-3-Clause notices apply.

Both the selected PC-98 chip and `createSoundChip('ym2203')` now compile
`native/reference/tauri/native_genesis_ym2203.cpp` from the pinned SDK under
`src-tauri/vendor/tetorica-soundchip-native/`. The SDK version and source bytes
are unchanged from the YM2612 adoption. Its OPN/SSG/ADPCM provider is linked
once, in the existing archive order. See the
[SDK adoption guide](../../docs/native-sdk-ym2612.md) for the pin and update flow.

This directory retains the Browser reference snapshot and provenance. The old
`src-tauri/src/native_genesis_ym2203.cpp` and header are retained as reference
copies; editing them does not update the SDK-backed build. Host scheduling,
ownership, additional-instance admission, output and JS APIs remain in Tauri.

`scripts/sync_native_ym2203.py --source PATH --check` verifies pinned Browser
wrappers, engines, Worklets and inspection controls against the shared provider.

The default input clock is 4,000,000 Hz, matching the current Playground. Maximum
fidelity generates at clock/4 regardless of the FM/SSG prescaler. All four integer
output buses are summed and clipped before source sample averaging / hold. The
result is mono and goes to both output channels, matching the shipped WASM.

The adapter retains applied FM frequency latch pairs, channel-3 special mode,
prescaler, operator settings, SSG periods/noise/envelope and I/O registers. FM
forced off masks source channels and physical TL without replacing authored TL;
ordinary register key-off retains release. Manual retrigger waits for a real FM
clock interval. SSG off clears active volume, while retaining its authored level
for preview, including an explicitly retained zero. Stop clears voices and timer
starts and retains their settings and master clock, without resetting synthesis
phases. Reset follows the chip's own semantics and preserves the output-rate
remainder/held sample, matching the primary `ym2203-worklet.js` reset boundary.

The state view is 280 bytes and reads do not advance the chip. Status, IRQ and
SSG data reads use the original core. Source mute bit0 is SSG (the existing WASM
convention), bit1 is FM; three additional channel mute bits select physical FM
channels. Callback operations allocate and free nothing; verification is offline.
