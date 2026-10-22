# Native DOS PC OPL

YM3526, YM3812 and YMF262 use the same unmodified ymfm OPL/PCM provider already
compiled for Master System and MSX. Each renderer constructs exactly one selected
OPL core; no Genesis, PSG, AY or OPLL instance is created. MSX Audio selects Y8950
separately through the same renderer; these DOS profiles remain unchanged.

`scripts/sync_native_opl.py --source PATH --check` verifies the pinned Browser
wrappers, authoring API, Worklet, inspector and the once-compiled synthesis
provider under `native-master-system`. The common timer interface is also checked
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
