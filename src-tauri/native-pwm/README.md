# Native 32X PWM

The Native backend directly compiles the exact shared `wasm/pwm32x_core.h`
used by the browser's WASM endpoint. This is the existing MAME-derived JS
FIFO/timer/routing implementation translated to fixed C++ storage; the BSD
license and source provenance are included unchanged. `sources.json` records
the pinned source hashes.

Build `src/native_genesis_pwm.cpp` with
`native-pwm/snapshot/wasm` in the include path. Compile with
`-ffp-contract=off` to retain the original JavaScript arithmetic on ARM64.

The Native playback adapter uses duty output, gain 1, and the device's output
sample rate, matching `PWM32XPlayback`. Uploads are validated and stably sorted
on the control thread. The callback admits one retained cursor per upload
into a heap allocated before playback, rather than inserting every write.
The bound is one million pending writes and 65,536 active uploads. Audio
callbacks neither allocate scheduled-write storage nor destroy uploaded
command data. Completed/cancelled arrays return to a control-thread retirement
queue. Worker lookahead reservations are admitted immediately with an absolute
output-frame base; they do not block on future playback. Their first writes
become visible after ordinary commands at that base, so a preceding scheduled
Reset cannot erase a later reservation.

Stop clears scheduled writes, FIFOs and held output while preserving the
configured clock, cycle and routing. Reset clears PWM registers and preserves
the monotonically increasing output-frame counter. Owner cancellation clears
only that owner's reservations and silences its held output.

Refresh after a deliberate shared-core change:

```sh
python3 scripts/sync_native_pwm.py --source /path/to/hello_ymfm
python3 scripts/sync_native_pwm.py --check --source /path/to/hello_ymfm
```

Run the C ABI against the synchronous JS reference without an output device:

```sh
node scripts/check_native_pwm.mjs --out=/tmp/native-pwm-parity.json
```

The audit compares routing, disabled modes, cycle 0/1, FIFO overflow and
underflow, held samples, interrupt timing, clock changes, Stop and Reset at
44.1/48/96 kHz. It also checks that wrapper writes/render/state/Stop allocate
and free no heap storage.
