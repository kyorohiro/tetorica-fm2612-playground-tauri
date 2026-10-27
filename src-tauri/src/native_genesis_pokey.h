// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 kyorohiro
#pragma once
#include <cstddef>
#include <cstdint>
struct NativePokeyState { uint8_t registers[16], reads[16], levels[4]; uint32_t clock; };
extern "C" {
size_t native_pokey_core_size();
void *native_pokey_create(uint32_t rate,uint32_t clock);
void native_pokey_destroy(void*);
void native_pokey_reset(void*);
void native_pokey_write(void*,uint32_t,uint8_t);
void native_pokey_key_off(void*,uint32_t);
void native_pokey_stop(void*);
void native_pokey_set_mute_mask(void*,uint32_t);
void native_pokey_set_retained_level(void*,uint32_t,uint8_t);
int32_t native_pokey_set_clock(void*,uint32_t);
void native_pokey_sample(void*,float*,float*);
void native_pokey_raw_sample(void*,float*,float*);
void native_pokey_get_state(void*,NativePokeyState*);
}
