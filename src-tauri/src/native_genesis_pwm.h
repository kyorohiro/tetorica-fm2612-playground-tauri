// SPDX-License-Identifier: BSD-3-Clause
#pragma once
#include <cstdint>
struct NativePwmState {
    uint32_t registers[5];
    uint32_t clock;
    uint32_t cycle;
    uint32_t left;
    uint32_t right;
    uint32_t left_valid;
    uint32_t right_valid;
    uint32_t left_size;
    uint32_t right_size;
    uint32_t left_fifo[3];
    uint32_t right_fifo[3];
    uint32_t timer_tick;
    double interrupt_count;
    double until_tick;
};
extern "C" {
void *native_pwm_create(uint32_t rate);
void native_pwm_destroy(void *ptr);
void native_pwm_reset(void *ptr);
void native_pwm_stop(void *ptr);
void native_pwm_write(void *ptr, uint32_t reg, uint32_t value);
int native_pwm_set_clock(void *ptr, uint32_t hz);
void native_pwm_sample(void *ptr, float *left, float *right);
void native_pwm_get_state(void *ptr, NativePwmState *out);
}
