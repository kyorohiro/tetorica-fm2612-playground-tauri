# Native SNES S-DSP and SPC file player

The accurate `SPC_DSP` from SNES-SPC 0.9.0 is compiled directly for CPU-free
BRR instruments and captured DSP/RAM timelines. It is the same modified library
used by the Browser WASM backend. The independent SPC file player additionally
runs its full SPC700 CPU and timers. No Nintendo IPL ROM, game samples, game
music, or external voice bank is included.

Upstream: https://github.com/blarggs-audio-libraries/snes_spc
Revision: `ec8ee2bbe30451614c1d02a83f7af1c97d497d45`.
Copyright Shay Green. License: **LGPL-2.1-or-later**, see `LICENSE` and original
notices. The full modified source, original documentation and license are in
`snapshot/third_party/snes-spc`. `sources.json` pins every byte. Local changes
are documented by that directory's `README.md`.

The Native wrapper is `../src/native_genesis_snes.cpp` and its C ABI header.
An exact copy of both, plus the offline fixture, accompanies the library in
`snapshot/application/src-tauri/src`; those copies are checked by the manifest.
It matches the fixed 32,000 Hz DSP output and `SpcResampler` linear interpolation
(one source frame per pull) with double intermediates and float32 output. Stop
resets execution and retains applied RAM/register settings without KON; reset
clears RAM. The existing JavaScript SnesSynth reuploads its samples on reset.
Captured timelines retain the original four-frame FIFO and 1,024,000 Hz ordering.

The full file player is `../src/native_genesis_spc.cpp` and its C ABI header.
It runs the same `SNES_SPC` CPU/timers and accurate DSP as the Browser file player,
including its 128-source-frame read-ahead and output interpolation. Original
files and prepared seek Cores are owned by the host on the control thread.
The callback plays or pauses an installed Core, changes its output mute mask,
captures analysis snapshots and collects bounded DSP-write observations without
allocation. File load, restart and seek emulation run before installation.
ID666 metadata does not impose an automatic end or fade. The file wrapper,
header and synthetic offline fixture are also pinned under `snapshot/application`.

To rebuild the complete application with a modified library:

```sh
cargo build --manifest-path src-tauri/Cargo.toml
```

The ordinary Tauri Rust/application sources, Cargo lockfile and `build.rs` are
provided alongside this component. `build.rs` compiles the wrapper and
`SPC_DSP.cpp`, `SNES_SPC.cpp`, `SNES_SPC_misc.cpp` and `SNES_SPC_state.cpp` using
the snapshot include directory. No external download is
required for the SNES component. This source distribution permits rebuilding
and relinking the application with a changed LGPL component.

To rebuild just the LGPL component from this bundled directory on its own:

```sh
sh rebuild.sh /tmp/tetorica-native-snes-rebuild
```

This produces `libtetorica_native_snes.a` and runs the offline fixed-state and
allocation checks, using only bundled sources and a C++17 compiler. The component
archive can replace the one produced by the application's Cargo `build.rs`.

Verify the bundled source without the parent checkout:

```sh
python3 scripts/sync_native_snes.py --check
```

To refresh deliberately from the parent repository, supply `--source ../..`.
Reference snapshots of the WASM wrapper, build script and JavaScript resampler
are included for matching the Browser behavior; the DSP files are not rewritten.
