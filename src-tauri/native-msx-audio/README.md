# Native MSX Audio

The existing MSX YM2413 / AY-3-8910 pair is combined with one selected Y8950
renderer. `native_opl_create(rate, 3)` constructs Y8950 only. Its unmodified ymfm
OPL/PCM synthesis provider is already compiled once by `native-master-system`;
MSX's AY provider is also reused without another copy or hidden DOS OPL core.

`scripts/sync_native_msx_audio.py --source PATH --check` verifies the pinned
Browser Y8950 wrapper, public OPL API, Worklet, inspector, timer interface and
shared providers. Existing ymfm and AY BSD-3-Clause licenses remain in their
provider directories and the app resources.

The scope is the current Web `OPLSynth` FM/rhythm API: nine mono FM voices,
five hardware rhythm gates, authored operators/TL, clock changes, forced mute,
delayed manual retrigger, silent Stop, timers, status and IRQ. Rust supplies
independent mixing and scheduling for all three MSX Audio chips.

No new ADPCM RAM/upload API is exposed. Raw Y8950 register writes still reach
the same ymfm chip. Default external memory and I/O read zero and discard writes,
just as the shipped WASM wrapper does before sample memory is loaded. Y8950
timers advance by 72 master clocks per source sample. Status merges live Delta-T
flags into the FM status/IRQ through the same core; data/status reads preserve
the original wrapper's semantics. Stop mirrors the shipped Worklet's FM/rhythm
gate and output mute behavior; it does not introduce a new ADPCM transport.

The pinned Y8950 data-port reader maps addresses `0x09` / `0x1a` to ADPCM
`read(2)` / `read(19)`, which return zero. The ADPCM memory-read side effect
exists only at `read(8)`, so the current snapshot port reads do not consume data.
The fixture compares frequent port polling against an unpolled identical core
through raw Delta-T controls, proving identical PCM and status sequences.

PCM/status validation runs the actual deployed Y8950 WASM Worklet offline,
including raw I/O/Delta-T default-memory controls, ordinary register reads,
timers/IRQ and Stop/reset. Callback operations allocate and free nothing.
