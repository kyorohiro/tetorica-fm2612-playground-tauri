# Native DOS PC OPL

YM3526, YM3812 and YMF262 use the same unmodified ymfm OPL/PCM provider already
compiled for Master System and MSX. Each renderer constructs exactly one selected
OPL core; no Genesis, PSG, AY or OPLL instance is created. MSX Audio selects Y8950
separately through the same renderer; these DOS profiles remain unchanged.

Selected DOS PC and MSX Audio's Y8950 now compile the pinned SDK's
`native/reference/tauri/native_genesis_opl.cpp` and header. Independent instances
of all four models use its `native_genesis_extra_opl.cpp` and header. The SDK's
`native_genesis_y8950_memory.h` supplies the shared memory interface. Each SDK
translation unit resolves its own headers; no application forwarding header is
needed. The SDK version, shared OPL/PCM/ADPCM provider and existing archive order
are unchanged. See the [SDK adoption guide](../../docs/native-sdk-ym2612.md).

Y8950 borrows up to 512 mutable 4 KiB pages, for a maximum of 2 MiB. The memory
helper owns the pointer table and interface, not the RAM bytes. Rust retains
page ownership, control-side copy-on-write preparation, lifetime, retirement and
budget accounting. The audio thread is the sole reader/writer of published
pages and preserves the existing memory and upload revision behavior.
The RF5C164 adapter sharing the archive, machine helpers, PSG/AY, scheduling and
mixing remain local to Tauri. No new sample-memory API is introduced here.

`scripts/sync_native_opl.py --source PATH --check` verifies the pinned Browser
wrappers, authoring API, Worklet, inspector and the once-compiled synthesis
provider references. Those snapshots, old local adapters and headers remain as
provenance. `native-opl/` has no live source consumer and is no longer a Cargo
rebuild watch; the SDK adapter/header/memory inputs are watched instead.
Updating the old copies alone does not update the SDK build.
The common timer interface is also checked
against its Browser source. ymfm is BSD-3-Clause; its existing license is in
`../native-master-system/snapshot/LICENSE`.

`native_genesis_opl.cpp` adapts the core to the output sample rate with the shipped
Worklet's exact source sample averaging / hold behavior. YM3526 and YM3812 are
mono; YMF262 folds A+C left and B+D right. External forced mute preserves authored
operator TL values and restores both physical channels of a four-operator pair
on the next note. Ordinary gate-off retains the core's release envelope.

Stop clears gates and pending retriggers while retaining clock, voice settings,
pair mask, rhythm mode/depth and routing. Status and IRQ reads are observational.
The callback allocates and frees nothing; tests use offline PCM only.
