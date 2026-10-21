# Native HuC6280

The `snapshot/third_party/mame-huc6280` files are byte-for-byte copies of the
standalone core compiled into the Browser WASM. Charles MacDonald's MAME
HuC6280 register/tick implementation and its existing limitations are retained.
The BSD-3-Clause license and pinned upstream files are included here.

The native wrapper compiles that source unchanged in one translation unit. A
local `private` access macro exposes only its fixed state for nonadvancing
inspector snapshots. The wrapper adds channel-targeted writes (restoring the
raw channel-select latch), Stop, waveform uploads, and the same outer clock
resampling as `ChipClockResampler`. It does not change synthesis algorithms.
All storage is allocated at construction; writes, snapshots, Stop and per-frame
rendering allocate and free nothing.

From this repository, synchronize explicitly with:

```sh
python3 scripts/sync_native_huc6280.py --source ../..
python3 scripts/sync_native_huc6280.py --check --source ../..
```
