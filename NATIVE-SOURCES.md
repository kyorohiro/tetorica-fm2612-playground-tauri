# Rebuilding the Native application

`native-source-bundle/application-sources.zip` accompanies this build. It contains
the matching Rust and C/C++ application sources, native library snapshots,
Cargo/npm lockfiles, build scripts, Tauri configuration, icons, desktop bridge,
audio-sidecar sources and the exact imported frontend. `SOURCE-MANIFEST.json`
records their SHA-256 hashes and the frontend release archive hash.

The complete pinned `tetorica-soundchip-native` source distribution is included
under `src-tauri/vendor/tetorica-soundchip-native/`, together with the separate
import receipt `src-tauri/soundchip-sdk.json`. Its source manifests, provenance,
licenses and notices are preserved. YM2612, the selected/additional YM2203,
YM2608 and shared YM2610/YM2610B adapters, and the common OPN/SSG/ADPCM provider
build from that snapshot. The additional YM2151 adapter and shared OPM provider
also use the SDK. The independent OKIM6258 adapter uses the SDK, and its shared
decoder is generated from the SDK source into Cargo's `OUT_DIR`. A guarded
constructor-only substitution preserves Tauri's existing thread-safe one-time
table initialization; decoder arithmetic and the pinned SDK bytes are unchanged.
The generation recipe is in `src-tauri/build.rs` and needs no Python/CMake step.
X68000's machine adapter, sample ownership and mixing remain in Tauri; its
FM uses the SDK OPM provider and its OKI shares the generated decoder.
The independent YM2413 adapter and common OPL/PCM provider also use the SDK.
Selected and independent YM3526/YM3812/YMF262/Y8950 adapters and their Y8950
memory helper now also use the SDK. Y8950 borrows mutable 4 KiB RAM pages owned
by Rust; ownership, lifetime, control-side copy-on-write, scheduling and budgets
remain application responsibilities. The shared AY-3-8910 core and independent
AY adapter also use the SDK, compiled once in the existing MSX archive.
The shared Sega PSG core/header and raw C ABI now use the SDK's
`vendor/ymfm/segapsg.cpp` and `wasm/segapsg_wasm.cpp`, compiled once in the existing
`tetorica_native_genesis` archive for Genesis/Nuked, Master System, Game Gear and
additional PSG instances. The SDK wrapper resolves `vendor/ymfm/chip_state.h`
through the SDK include path.
Selected RF5C164 adapters, their decoder and raw C ABI also build from the SDK
in `tetorica_native_megacd`; the decoder is compiled once. The independent RF
adapter builds from the SDK in `tetorica_native_opl` and retains its own renderer,
64 KiB RAM and per-voice held-output history. Selected and independent instances
each own their RAM and copy uploads into it. Rust retains immutable upload
payload ownership, scheduling, admission, lifetime and memory budgets.
Master System/MSX machine adapters and their `NativeOpllCore` helper, selected
MSX AY resampling, the local `NativePsgCore` helper and Genesis mixing/DC/gains
remain local;
shared OPL/PCM is compiled
once with the existing archive order and common ADPCM support.
Other application adapters retain their existing
source snapshots. The
[SDK adoption guide](docs/native-sdk-ym2612.md) lists the
consumed paths, pinned archive hash and deliberate offline update procedure.
A separate SDK checkout or sound-core download is not required to rebuild this
application from its matching source ZIP.
YM2608's project rhythm ROM remains under `src-tauri/native-ym2608/snapshot/web/`
and is uploaded by the Rust host; adopting the SDK adapter does not move that asset.
Selected YM2610/YM2610B instances retain their owned dense ADPCM ROM banks;
additional instances borrow immutable sparse ROM pages prepared and retained
by the Rust host. ROM data preparation and ownership remain in the application.

Nuked OPN2 and SNES-SPC are **LGPL-2.1-or-later**. Their licenses, copyright
notices, provenance and corresponding modified sources are under
`src-tauri/native-nuked/` and `src-tauri/native-snes/`. These components are
statically linked. SNES-SPC supplies both the CPU-free S-DSP instruments and the
full SPC700 file player. The application sources and build material are provided so
that you can modify either library and rebuild/relink the application with it.
Other code retains its existing license; the individual component notices apply.
This distribution does not prohibit modification for your own use or reverse
engineering to debug those modifications as permitted by LGPL 2.1 section 6.

## Build with a modified library

1. Extract `application-sources.zip` and enter `tetorica-playground-source`.
2. Install Node.js 22 or later, Python 3, Rust (CI uses 1.91.1), a C/C++17
   compiler and the platform dependencies listed below.
3. Modify the actual source consumed by `src-tauri/build.rs`:
   Nuked sources under `src-tauri/native-nuked/`, or
   `src-tauri/native-snes/snapshot/third_party/snes-spc/snes_spc/` for SNES.
   Do not run a source refresh command after editing; it would replace your edits.
4. Run `npm ci`, then `npm run build -- -- --locked` to create the application
   bundle. This prepares the optional Audify runtime and matching source ZIP.
   `npm run dev` runs a development build. An unsigned local build can be used
   to test the modified sound core without the distributor's signing key.

The exact Nuked source paths and its build recipe are documented in
`src-tauri/native-nuked/README.md`; the standalone SNES rebuild recipe is
`src-tauri/native-snes/rebuild.sh`. Snapshot verification checks the distributed
sources; a deliberate modification will of course change the recorded hashes.

Windows uses the Visual Studio C++ Build Tools and WebView2. macOS uses Xcode
Command Line Tools. Ubuntu 24.04 uses `libwebkit2gtk-4.1-dev`,
`libayatana-appindicator3-dev`, `librsvg2-dev`, `patchelf`, `xdg-utils`,
`libasound2-dev`, `libpulse-dev`, `libjack-jackd2-dev`, `cmake`, and `ninja-build`.
The included `.github/workflows/build-desktop.yml` contains the CI build recipe.
Cargo and npm download their locked dependencies on the first build; dependencies
remain under their own licenses. The compiler and operating system are not bundled.

The source ZIP is generated from the same working tree during Tauri's normal
pre-build step. It excludes local credentials, Git history, installed dependencies,
compiled output and the source ZIP itself. To regenerate it independently:

```sh
python3 scripts/prepare_native_sources.py
```
