// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 kyorohiro
#pragma once
#include <stddef.h>
#include <stdint.h>

struct NativeWonderSwanState {
    uint8_t registers[32];
    uint8_t memory[16384];
    uint32_t clock;
    uint32_t sample_rate;
    uint8_t mute_mask;
    uint8_t padding[3];
};

#ifdef __cplusplus
extern "C" {
#endif
void *native_wonderswan_create(uint32_t rate, uint32_t initial_clock);
size_t native_wonderswan_core_size(void);
void native_wonderswan_destroy(void *ptr);
void native_wonderswan_reset(void *ptr);
void native_wonderswan_stop(void *ptr);
void native_wonderswan_key_off(void *ptr, uint32_t channel);
void native_wonderswan_write(void *ptr, uint32_t address, uint8_t value, uint8_t control_mask);
void native_wonderswan_write_memory(void *ptr, uint32_t address, uint8_t value);
uint8_t native_wonderswan_read_register(const void *ptr, uint32_t address);
uint8_t native_wonderswan_read_memory(const void *ptr, uint32_t address);
void native_wonderswan_set_mute_mask(void *ptr, uint32_t mask);
int32_t native_wonderswan_set_clock(void *ptr, uint32_t hz);
void native_wonderswan_sample(void *ptr, float *left, float *right);
void native_wonderswan_get_state(const void *ptr, struct NativeWonderSwanState *out);
// Diagnostic raw core output; use a separate instance from speaker playback.
void native_wonderswan_raw_sample(void *ptr, float *left, float *right);
#ifdef __cplusplus
}
#endif
