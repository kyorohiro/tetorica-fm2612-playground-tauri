# Native Game Boy APU

The Native backend uses the existing browser `third_party/mame-gameboy`
standalone DMG core. The DSP `.cpp` is byte identical to the pinned browser
source. Its mirrored header adds one friend declaration for fixed-size state
observation and gate-only Stop; no timing, waveform, envelope, sweep, noise,
wave-RAM accessibility, panning or mix algorithm is changed.

The wrapper constructs the core at the output sample rate and the Game Boy's
4,194,304 Hz base clock. Clock changes use the same count/average/hold resampler
as `web/playground_chip_clock.js`. No AudioContext, WASM VM, JS VM, additional
sound chip or playback device is created by this module.

Stop clears channel gates and held output while retaining sound registers,
wave RAM and clock. Reset uses the existing DMG core reset, including its
power-on wave-RAM pattern. The applied snapshot exposes actual core gates and
wave RAM; authored voice values are retained separately across DAC key-off.

Build the snapshot `snapshot/third_party/mame-gameboy/gameboy_apu.cpp` and
`src/native_genesis_gameboy.cpp` with that snapshot directory in the include
path. The source is BSD-3-Clause; see `LICENSE` and the upstream references in
the snapshot README. `sources.json` records SHA-256 hashes.

Refresh deliberately after a browser-core change:

```sh
python3 scripts/sync_native_gameboy.py --source /path/to/hello_ymfm
python3 scripts/sync_native_gameboy.py --check --source /path/to/hello_ymfm
```

Offline Native/WASM PCM parity (no output device):

```sh
node scripts/check_native_gameboy.mjs
```
