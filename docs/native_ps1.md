# Native PlayStation SPU connection

The primary `ps1` mode and independent `createSoundChip('ps1')` instances use
one compiled copy of the pinned SDK's `vendor/mame-ps1-spu` sources. There is
no second application-owned copy of the C++ sound core here. The fixed SDK
archive and provenance are recorded in `src-tauri/soundchip-sdk.json`.

`src-tauri/src/native_genesis_extra_ps1.rs` owns each independent C ABI handle. The primary
wrapper in `src-tauri/src/native_genesis_ps1.rs` shares that implementation. All instances
have separate 512 KiB sample RAM, 24 voice registers, decoded sample caches,
ADSR state, and private approximate reverb buffers. The core runs at 44,100 Hz;
the host interpolates to the output device rate and applies the existing
smoothed Mixer volume/pan. No extra calibration gain is applied.

## Commands and snapshots

The native command action is `ps1` for the primary device or `extra-ps1` with an
`instance` ID for an independent device. The message methods are:

- `write`: even byte `register` offset 0–1022 and a 16-bit `value`.
- `batch`: at most 512 register writes, optional `channel` (0–23), `voice` lease,
  and `action` (`on`, `off`, or `edit`). Audition batches require a voice ID.
- `load-memory`: `address`, owned byte `data`, optional audition channel/voice.
- `reset`: optional `clearRam` (false by default).
- `stop-voices` / `all-notes-off`, `mix`, and `cancel-main-owner`.
- `barrier`: optional `ram:true` plus a command ID for an explicit RAM capture.

An immutable append beyond the applied upload high-water mark is permitted
while a manual note is held. Replacing an existing held waveform requires its
current audition channel/voice lease. Per-channel noise/FM/reverb masks merge
only that channel's bit, preserving sibling voices. Shared effects edits cannot
disturb a different manually held voice.

The status commands `native_genesis_ps1_status` and
`native_genesis_extra_ps1_status` accept `after` to wait for an applied command.
Ordinary snapshots contain 512 16-bit registers, enabled/held voices, native
frame count, and pending command IDs. RAM is omitted by default. `ram:true`
requires the ID of an applied RAM capture barrier; RAM, registers, and counters
then describe the same callback boundary. Retained captures expire after
30 seconds and share the 8 MiB pending-command budget.

Transport Stop clears voices and reverb tails while keeping sample RAM and
safe authored registers. KON/KOFF, ENVX, ENDX, and RAM data-port side effects
are never replayed during restoration. Reset clears voice/effect settings;
only `clearRam:true` clears the sample bank. Main/Worker cancellation and stale
voice releases preserve unrelated manual notes and independent instances.

## Verification and limits

`cargo test --locked --manifest-path src-tauri/Cargo.toml ps1` exercises the
actual SDK core without opening an audio device. It covers all 24 voices,
44.1/48/96 kHz host rates and partition equivalence, ownership, memory bounds,
coherent captures, reset/Stop, two independent SPUs, and disposal.

This remains an experimental CPU-free instrument connection. It does not run
PSF files, a PlayStation CPU, CD audio/XA, or custom hardware reverb algorithms.
The inherited SPU core allocates decoded ADPCM caches on cold playback and
private reverb storage on reset/preset changes. The host's Rust command payloads
are prepared and retained off the callback, but this does not make the retained
C++ core allocation-free. Long-sample cold-start latency and physical listening
remain useful checks. Core limitations and BSD notices are included with the
pinned SDK's `vendor/mame-ps1-spu` source.
