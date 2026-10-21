# Shared noise DSP snapshot

`scripts/sync_native_genesis_noise.py --source ../..` snapshots
`native/audio_effect/noise.c`. `--check` verifies its SHA-256 hash and generated
member source against the upstream implementation.

Only include/extern declarations and storage syntax are changed. Removing
file-static storage makes the same C functions and voice state instance-owned
C++ members. No filter, random-number, ramp or envelope formula is changed.

Each renderer owns one instance with 32 fixed voices. Preparation/destruction
happens off the callback; reset, control and stereo mixing allocate no memory.
The one-frame gain buffer is an internal DSP buffer, not an AudioNode or bridge.
