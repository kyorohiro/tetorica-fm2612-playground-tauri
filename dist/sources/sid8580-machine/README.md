# SID Player 8580 preview source kit

The player uses the two WASMs in ../../generated/sid8580-machine/. Their
matching receipt is also copied here as sid8580-machine-build.json. The
compiled sources below retain their repository-relative paths. The original
BSD C64 host and zlib CPU/CIA/VIC devices are unchanged; the marked generated
host links the pinned MIT sidera SID through direct WASM function imports.
Both research filter-fit flags are OFF in the player. This is an approximate
8580 preview, not real-hardware or listening certification.

To rebuild from this directory, install Node.js, Python3, Rust1.91.1 with
wasm32-unknown-unknown, C11/C++17 Clang and Emscripten (the recorded build uses
6.0.6). Then run:

    sh scripts/build_sid8580_machine.sh build/rebuild
    node scripts/sid8580_machine_artifacts.mjs --check build/rebuild
    node scripts/prepare_sid8580_parser.mjs --check

The build produces two WASMs and a separate native validation executable.
Only the two WASMs are distributed in the player. The full compiler versions,
link recipe, 99 compiled input hashes, output hashes and import contract are
recorded in sid8580-machine-build.json. source-kit.json authenticates every
copied source and notice. Equal compiler profiles can be compared byte for
byte with the recorded artifacts; other toolchains require separate functional
checks. Packaging checks current source hashes and WASM imports without calling
installed compilers, so a browser-only CI can create this distribution.

Browser modules are readable at ../../sid-player/ and ../../sid8580-lab/.
The generated PSID layout view and its original source/generator are included
here too; the canonical CLI parser retains its original validation policy.

The native host/wrappers are BSD-3-Clause, the chip devices are zlib and the
pinned sidera sources are MIT. Original and marked adapted sources remain
separate. Rust standard-library notices and conservative Emscripten runtime
notices are copied unmodified with their provenance; their complete notices
do not imply every optional component is linked. No GPL SID reference engine,
native binary, C64 firmware, game music or PCM is included. User-loaded files
are never package inputs.
