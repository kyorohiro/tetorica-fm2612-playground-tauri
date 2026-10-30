# SCSP Native integration

SCSP primary and additional instruments now run on the Native callback, with
Main/Worker commands, PCM upload, the 32-slot inspector, Hold/PC Keyboard and
PCM/EG editing. Select Saturn (SCSP) and Native after rebuilding this app with
the imported frontend. Web Audio remains available.

The primary wrapper and extra rack share `native_genesis_extra_scsp.rs` and the
previously verified Rust `Core`. The SDK pin, raw C++ and WASM are unchanged.
The existing bounded queues, 16-slot/64 MiB admission, completion barriers,
owner invalidation and off-callback retirement are used. The raw core has a
2 MiB charge (checked in the C++ parity fixture); Rust/shared/renderer structure
sizes fit the common 64 KiB allowance. Pending/capture payloads are charged separately.

Only supported PCM slot/master operations enter the callback. Batches are
confined to their declared slot. Masks, key actions and final PCM bounds/loop
pitch are validated before mutation. KEYONEX still scans all 32 slots. Each slot
has an identity-safe manual lease and a separate playing RAM range. Release
retains that range until retrigger/Stop/reset. Overlapping uploads are rejected.
Status publishes actual playing addresses and small registers/metadata; full
RAM is copied only for an explicit capture barrier.

Stop clears DSP and converter look-ahead while retaining RAM. Reset restores
master and retires older reservations, applying immediately even in timed Worker
playback. Clear reset invalidates previous sample handles. Lifecycle fences
keep old sessions/instances and stale manual Off commands away from replacements.
Additional instances have the same original initial sine as the primary.

The JS endpoint owns authored presets and stages PCM. It does not generate audio
or allocate an AudioContext. Ordinary key controls do not copy the full RAM.
Worker upload responses carry resource metadata on the same port. Editor Apply
is blocked while code playback is active/reserved; hardware playing ranges
provide an additional protection independent of requested gate bits.

Future note batches are immutable and capture their preset at admission. Later
edits do not rewrite a queued note. Inspector settings are authored parameters,
not envelope meters; actual sample references are observed separately.

Tests include actual SDK/Rust rack scheduling, all slots, timestamp/block
equivalence, manual/stale owners, release RAM, scoped cancellation, coherent
capture, duplicate IDs and reset barriers. The independent Rust/Browser parity
runner still verifies 96 cases / 5,589,524 stereo frames exactly. Browser UI
tests mock only RPC and prohibit Web Audio construction. These offline tests do
not verify physical output, real-device interruptions or callback deadlines.
The matching source ZIP includes this adapter and the complete licensed SDK.
No publication, commit or remote CI was performed by this change.
