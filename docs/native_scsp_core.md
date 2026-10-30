# SCSP Native callback core — 2026-10-30

The first Native integration layer is compiled into the application: an exclusively
owned Rust `Core` calls the same pinned SCSP C ABI as the Browser WASM build.
SCSP is **not yet selectable as a Native backend**. Timestamped admission,
manual leases, RPC status, primary/extra rack integration and recovery are the
next layer; the existing UI correctly remains Web Audio only for SCSP.

`src-tauri/src/native_genesis_scsp_core.rs` provides masked register access,
caller-owned bounded RAM upload/readback, reset, immediate Stop, one host sample
and caller-owned stereo block rendering. Construction allocates the SDK core;
processing, register writes, RAM operations and reset require no Rust allocation
or C++ `new`/`delete`. The object can move to a callback thread and is not `Sync`.
It must have one exclusive owner. The caller must prepare and validate complete
transactions and protect playing sample memory before applying these primitives.

SCSP runs at 44,100 Hz. Host rates 8,000–384,000 Hz use the exact Browser
`Pcm44100Resampler` phase, two-sample initial look-ahead and double interpolation
followed by Float32 rounding. At 44,100 Hz the path directly pulls one sample
without interpolation/look-ahead. Block partitioning does not change output.
Reset clears the converter and its counters. Stop also resets DSP/registers,
restores master `0x020f`, and retains RAM, matching `ScspSynth.stopVoices`.
Authored presets and sample metadata are owned by the transport above this layer.

The SDK was imported through `scripts/import_soundchip_sdk.py` from the same
476-file unpublished 0.1.7 source candidate as the frontend repository:
archive SHA256 `efe7daa49a97dad00de9745e92aed9a16820cde2ae89e55a0222523dee7974a1`,
base commit `f9f76df545e56fa4195cf345a9de507ae3bcb39b`.
No immutable vendor file was directly edited. The importer verifies SCSP's full
source/recipe/fixture/license closure while retaining historical snapshot checks.
The newer pin also contains the frontend's previous PS1 host-PCM/headroom SDK
additions; this application does not enable those new APIs in this change.

## Reproduce without a device

From the parent frontend repository:

```sh
node scripts/check_native_scsp.mjs --out docs/issues/native_scsp_core_01.json
```

Or from this repository, passing that frontend source explicitly:

```sh
node scripts/check_native_scsp.mjs --source /path/to/hello_ymfm --out /tmp/scsp.json
cargo test --manifest-path src-tauri/Cargo.toml native_genesis::scsp_core
python3 -m unittest discover -s tests -p test_soundchip_sdk.py
```

The fixture compiles the **actual Rust callback module** plus the fixed SDK
C++, and compares every sample with real Browser `ScspSynth`, `Scsp` WASM and
the shared PCM resampler. All slot registers, master/monitor, full 512 KiB RAM
and native/output frame counts must also match. Regular and irregular partitions
must produce the same PCM hash. Input files are hashed before and after the run.

96 cases / 5,589,524 stereo frames passed exactly on macOS ARM64. The 8/44.1/48/
96/384 kHz matrix covers PCM8/16, one-shots, forward loops, 32 slots/clipping,
pitch, pan, named EG, isolated retrigger, live RAM/masked writes, Stop, reset and
10-second runs at 44.1/48/96 kHz. Stop and one-shot tails are explicitly silent.
Measured Rust alloc/dealloc and C++ new/delete counts are zero during processing,
including commands/reset. Direct C malloc/free are not intercepted. The two
Rust unit tests cover atomic bounds rejection, instance isolation, clocks and
RAM retention/clear. No device, AudioContext, deadline or hardware fidelity
claim is made by these offline tests.

The test-only allocation probe and fixture are not linked into the application.
No publication or frontend/SDK release is needed for this intermediate layer.

Full Cargo regression: **802 passed, 0 failed, 4 existing ignored tests**.
SDK importer Python: 14 passed; Native source Python: 10 passed. All 24 source
snapshots were verified offline. The matching 2,112-file application source ZIP
was regenerated and its full manifest/contents verified against the checkout.
