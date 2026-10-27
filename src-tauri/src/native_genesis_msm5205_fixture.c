// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026, kyorohiro
#include "msm5205_pcm.h"
#include <stddef.h>
/* Compiled C ABI oracle: catches mismatched Rust layout, and compares the full
 * renderer path with a direct C driver without opening a physical device. */
size_t native_msm5205_fixture_size(void) { return sizeof(msm5205_pcm_state); }
size_t native_msm5205_fixture_offset(uint32_t index) {
    static const size_t offsets[] = {offsetof(msm5205_pcm_state, source),
        offsetof(msm5205_pcm_state, phase), offsetof(msm5205_pcm_state, gain),
        offsetof(msm5205_pcm_state, fifo), offsetof(msm5205_pcm_state, fifo_count)};
    return index < 5 ? offsets[index] : (size_t)-1;
}
int native_msm5205_fixture_render(uint32_t rate, const uint8_t *data, uint32_t bytes,
                                float *left, float *right, uint32_t frames) {
    msm5205_pcm_state core;
    if (!msm5205_pcm_init(&core, rate, MSM5205_PCM_DEFAULT_CLOCK) ||
        !msm5205_pcm_prepare(&core, data, bytes) ||
        !msm5205_pcm_play(&core, 1, bytes * 2 - 2, 0, 12)) return 0;
    return msm5205_pcm_render(&core, left, right, frames);
}
