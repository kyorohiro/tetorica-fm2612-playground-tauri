# Native Nuked OPN2

The selected Nuked engine uses the same LGPL-2.1-or-later Nuked OPN2 1.0.12
synthesis C source and wrapper as the shipped Browser WASM. Its companion header
and upstream README still label their version 1.0.9; the pinned content hashes
are authoritative. Copyright Alexey Khokholov
(Nuke.YKT), 2017–2022. Read the included [LICENSE](LICENSE) and original source
notice under `snapshot/third_party/nuked-opn2/`.

`scripts/sync_native_nuked.py --source PATH --check` verifies every pinned source
hash against the Browser checkout. The upstream synthesis and wrapper are copied
unchanged. `native_genesis_nuked.cpp` only prefixes their exported symbols and
initializes the process-global YM2612 chip type once, before a core can render.
Later constructors therefore do not write a shared chip type during rendering.

`native_genesis_create_nuked(rate)` selects Nuked FM and the common PSG. Existing
`native_genesis_create(rate)` continues to select ymfm. Both use the same opaque
Native core, register transport, PSG, mix, Stop and destruction entry points;
the Nuked path constructs no hidden ymfm YM2612 instance.

Each raw address or data write clocks 24 internal cycles, matching the Browser
wrapper's write-latch semantics. A generated source sample clocks another 24
cycles, sums the multiplexed outputs and scales by 11, at master clock / 144.
Master clocks remain 7,670,454 Hz FM and 3,579,545 Hz PSG. Nuked's natural reset
idle DC is retained exactly as the Browser Worklet emits it. Explicit Native
Stop restores the authored patch in frequency-latch-safe order and suppresses
PCM until another FM/CSM, DAC or PSG trigger. Rendering uses the Worklet's double
sample averaging, held samples, 0.9 FM / 0.35 PSG mix and rounded 5 ms ramps. FM, PSG and master retain independent ramps when
the combined transport edits just one source target.

The whole application source/relink materials and LGPL notices are packaged by
the desktop release preparation. Offline fixtures never open audio devices.

The Native symbol adapter `src/native_genesis_nuked.cpp` is also provided under
the GNU LGPL, version 2.1 or later; see its source notice and this LICENSE.
