# Shared Native FX snapshot

`scripts/sync_native_genesis_fx.py --source ../..` snapshots the existing
`native/audio_effect/*.c` implementation and records its SHA-256 hashes.
`--check` verifies both the source hashes and generated member text.

The generated `.inc` files are included inside one C++ instance class. The
conversion removes sibling declarations/includes, changes file-static storage
into instance members, scopes colliding state names, and translates the C
compound-literal/calloc syntax. DSP formulas and algorithms are unchanged.
This provides independent prebuilt graphs while retaining the Web Audio DSP.

Only the control thread creates/destroys instances and their delay buffers.
The audio callback owns processing and parameter updates for installed graphs.
Built-in FX have the same 32-node / 8-slot-per-type limits as the shared API.
Arbitrary JavaScript `liveFx` functions are deliberately unsupported.
