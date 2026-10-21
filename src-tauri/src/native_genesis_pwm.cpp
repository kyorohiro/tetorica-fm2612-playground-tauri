// SPDX-License-Identifier: BSD-3-Clause
// The exact browser/WASM PWM core, directly owned by the native audio callback.
#include "native_genesis_pwm.h"
#include "pwm32x_core.h"
#include <new>
using tetorica::pwm32x::Core;
extern "C" {
void *native_pwm_create(uint32_t rate) {
    if (!rate || rate > 1000000) return nullptr;
    return new (std::nothrow) Core(23011361, rate, 1, true);
}
void native_pwm_destroy(void *ptr) { delete static_cast<Core *>(ptr); }
void native_pwm_reset(void *ptr) { static_cast<Core *>(ptr)->reset(); }
void native_pwm_stop(void *ptr) {
    auto *core = static_cast<Core *>(ptr);
    auto state = core->state();
    state.left_fifo.size = state.right_fifo.size = 0;
    state.left_valid = state.right_valid = false;
    state.left = state.right = 0;
    core->load_state(state);
}
void native_pwm_write(void *ptr, uint32_t reg, uint32_t value) {
    static_cast<Core *>(ptr)->write(reg, value);
}
int native_pwm_set_clock(void *ptr, uint32_t hz) {
    if (hz < 100000 || hz > 100000000) return 0;
    static_cast<Core *>(ptr)->set_clock(hz);
    return 1;
}
void native_pwm_sample(void *ptr, float *left, float *right) {
    static_cast<Core *>(ptr)->sample(*left, *right);
}
void native_pwm_get_state(void *ptr, NativePwmState *out) {
    const auto *core = static_cast<Core *>(ptr);
    const auto state = core->state();
    for (uint32_t reg = 0; reg < 5; ++reg) out->registers[reg] = core->read(reg);
    out->clock = static_cast<uint32_t>(core->clock());
    out->cycle = state.cycle;
    out->left = state.left;
    out->right = state.right;
    out->left_valid = state.left_valid;
    out->right_valid = state.right_valid;
    out->left_size = state.left_fifo.size;
    out->right_size = state.right_fifo.size;
    for (uint32_t i = 0; i < 3; ++i) {
        out->left_fifo[i] = state.left_fifo.values[i];
        out->right_fifo[i] = state.right_fifo.values[i];
    }
    out->timer_tick = state.timer_tick;
    out->interrupt_count = state.interrupt_count;
    out->until_tick = state.until_tick;
}
}
