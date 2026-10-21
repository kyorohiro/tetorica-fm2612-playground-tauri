# Native SNES S-DSP

The accurate `SPC_DSP` from SNES-SPC 0.9.0 is compiled directly for CPU-free
BRR instruments and captured DSP/RAM timelines. It is the same modified library
used by the Browser WASM backend. No SPC700 CPU is run here; no Nintendo IPL ROM,
game samples, game music, or external voice bank is included.

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

To rebuild the complete application with a modified library:

```sh
cargo build --manifest-path src-tauri/Cargo.toml
```

The ordinary Tauri Rust/application sources, Cargo lockfile and `build.rs` are
provided alongside this component. `build.rs` compiles the wrapper and
`SPC_DSP.cpp` using the snapshot include directory. No external download is
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
