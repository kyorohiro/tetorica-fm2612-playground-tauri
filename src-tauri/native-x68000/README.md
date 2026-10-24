# Native X68000 reference sources

The YM2151 uses the same ymfm OPM source as the Browser WASM. The OKIM6258 uses
the same MAME-derived standalone decoder as the Browser WASM, with flags 12
(4-bit ADPCM, 12-bit output), not Oki6258AudioEngine's default 10-bit setting.
The X68000 pair defaults to a 4 MHz FM clock and an 8 MHz ADPCM clock.

`snapshot/` preserves exact Browser source, output-rate / stream references,
and BSD notices. YM2151 now uses the pinned SDK's
`native/src/ymfm_opm_provider.cpp`; additional YM2151 instances compile its
`native/reference/tauri/native_genesis_ym2151.cpp` and matching header.
The selected X68000 still uses the local `src/native_genesis_x68000.cpp` machine
adapter, with its YM2151 core supplied by the same SDK OPM provider.
The provider is compiled once, with SDK include directories before the legacy
snapshots. The existing archive order is retained for GNU static linkers.
Common ymfm support, including ADPCM, continues to use the already pinned SDK.
The SDK version and source bytes are unchanged. See the
[SDK adoption guide](../../docs/native-sdk-ym2612.md).

The machine adapter, OKIM6258 decoder, additional OKI adapter and sample handling
remain local to Tauri. This directory still supplies the OKI implementation and
remains a Cargo rebuild watch. Its Browser snapshots and hashes, and the old
local YM2151 wrapper/provider sources, are retained as provenance; editing those
reference copies or running the sync script alone does not update the SDK inputs.

The generated `okim6258.cpp` differs from its pinned snapshot only by initializing
the shared ADPCM difference table once, with C++ thread-safe static initialization.
The Browser constructor otherwise rewrites that global table on every new chip,
which can race an already-running Native callback. Decoding remains identical.

The host adapter `src/native_genesis_x68000.cpp` mirrors Browser FM averaging /
hold resampling, immediate force-key-off with authored total-level restoration,
independent AM/PM depth state, and Oki6258Stream's low-nibble-first FIFO feeding,
fractional VCLK phase, repeat predictor reset and end-of-sample silence.
Output planes remain separate so Rust can apply each chip's own mixer settings.
Manual previews release the channel for the next ordinary native FM interval
before re-key-on, so even a fully decayed percussive envelope retriggers. No PCM
is generated or discarded in a command handler; later release / Stop cancels it.

Two 16 MiB OKI sample banks are allocated during creation. Uploads are sequential
64 KiB chunks into the inactive bank; commit swaps it only after every byte has
arrived, stops the old voice and retains the new sample for later playback.
Abort / partial uploads leave the committed sample and active voice untouched.
Stop retains patches, KC/KF, LFO depths, clocks, sample, pan and divider; OKI reset
retains the sample / clock and restores divider 512 / pan 0, as Browser does.
No callback command, render, Stop, reset, or inspection allocates or frees memory.

Synchronize and verify from the Tauri repository root:

```sh
python3 scripts/sync_native_x68000.py --source ../..
python3 scripts/sync_native_x68000.py --check --source ../..
```

The parent repository's `scripts/check_native_x68000.mjs` compares each independent
output against the actual deployed WASM at 44.1 / 48 / 96 kHz. The C++ fixture also
checks synchronous status / IRQ, full 16 MiB atomic sample ownership, independent
reset and Stop semantics, and zero callback allocations / frees. All tests are
offline; no audio device is opened. This is not a full X68000 CPU / DMA emulator.

Licenses: ymfm by Aaron Giles, BSD-3-Clause (`snapshot/LICENSE` and source notices);
MAME-derived OKIM6258 by Barry Rodewald, BSD-3-Clause
(`snapshot/third_party/mame-okim6258/LICENSE`).
