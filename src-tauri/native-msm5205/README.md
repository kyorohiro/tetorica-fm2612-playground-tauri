# Shared MSM5205 ADPCM transport

This directory contains a verbatim, hash-pinned copy of `hello_ymfm/native/msm5205`:

- `msm5205_pcm.c` / `msm5205_pcm.h`: allocation-free MSM5205 decoder and bounded
  PC Engine packed-ADPCM RAM playback driver, shared with Browser WASM.
- `LICENSE`: BSD-3-Clause notices for the upstream-derived decoder and this host.
- `NOTICE.md`: upstream provenance, adaptation scope and local changes.

The application compiles this local source as C11, with no downloads or parent
checkout. This application module does not change the fixed sound-chip SDK.
`python3 scripts/sync_native_msm5205.py --check` verifies all source and license
bytes offline. To update deliberately, pass `--source /path/to/hello_ymfm`;
`--check --source` compares the two copies without changing them.

The control thread owns immutable encoded ADPCM (at most 64 KiB per sample).
The existing audio callback advances decoding and playback on the same clock as
PSG and CD-DA. Offset and length are high-first **nibbles**, not byte addresses.
The rate register is 0–15, with 15 (32 kHz at 1.536 MHz) as the default. Stop and
EOF output strict silence. The RAM playback driver does not implement a PC
Engine CPU, CD controller, DMA or SCSI interface. Existing VGM JS decoding is
unchanged. Raw register writes explicitly select the separate legacy raw mode.

Source storage is retained by a control-thread registry, so replacing or clearing
it on the audio callback never destroys the last allocation reference. Rendering
allocates no memory and takes no locks. Rust integration, source pin, build and
license accompany the complete application source bundle.
