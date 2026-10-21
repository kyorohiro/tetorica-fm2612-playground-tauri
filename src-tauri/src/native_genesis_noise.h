#pragma once
#include <stdint.h>
extern "C" {
void *native_genesis_noise_create(uint32_t rate);
void native_genesis_noise_destroy(void *);
void native_genesis_noise_reset(void *);
int32_t native_genesis_noise_voice_create(void *, int32_t slot, int32_t type, uint32_t seed);
int32_t native_genesis_noise_set(void *, int32_t slot, int32_t parameter, double value, int32_t frames);
void native_genesis_noise_filter(void *, int32_t slot, int32_t mode);
void native_genesis_noise_action(void *, int32_t slot, int32_t action);
int32_t native_genesis_noise_active(void *, int32_t slot);
void native_genesis_noise_mix(void *, float *stereo);
}
