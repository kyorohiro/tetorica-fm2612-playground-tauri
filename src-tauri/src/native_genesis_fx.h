#pragma once
#include <stdint.h>

// Postorder bounded tree, matching the shared graph ABI. The caller validates
// descriptors before this control-thread-only constructor is invoked.
struct NativeFxNode { int32_t type, slot, count, children[32]; };
struct NativeFxUnit { int32_t type, slot; double values[8]; };
extern "C" {
void *native_genesis_fx_create(uint32_t rate, const NativeFxNode *nodes, uint32_t node_count,
                               const NativeFxUnit *units, uint32_t unit_count, double beat_seconds);
void native_genesis_fx_destroy(void *);
void native_genesis_fx_sample(void *, float *stereo);
void native_genesis_fx_parameter(void *, int32_t type, int32_t slot, int32_t key, double value, uint32_t frames);
void native_genesis_fx_tempo(void *, double beat_seconds);
}
