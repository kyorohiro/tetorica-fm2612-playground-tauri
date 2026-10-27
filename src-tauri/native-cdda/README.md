# Shared CD-DA PCM transport

These files are an unmodified copy of `hello_ymfm/native/cdda`:

- `cdda_pcm.c`: allocation-free PCM16LE stereo transport and resampler.
- `cdda_pcm.h`: the shared C API, also used by the browser WASM module.
- `LICENSE`: BSD-3-Clause terms for this application-owned implementation.

The application compiles this local copy as C11. It does not download a core or
require a parent checkout. `sources.json` pins every source/header/license byte.
This new application module does not change the fixed sound-chip SDK version.

Verify offline with `python3 scripts/sync_native_cdda.py --check`. To deliberately
update from a local source checkout, use
`python3 scripts/sync_native_cdda.py --source /path/to/hello_ymfm`, then review the
source changes and refreshed hashes. `--check --source` additionally compares
against that checkout without changing files.

The Rust host owns immutable source storage and queues all control changes onto
the existing device callback. Rendering allocates no memory, acquires no mutex,
and never destroys source storage. The complete Rust integration, build script,
local source pin and license are included in the application's source bundle.
