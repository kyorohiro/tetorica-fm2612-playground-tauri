// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 kyorohiro
// Application speaker/transport adapter. The pinned SDK raw DSP is unchanged.
// Speaker centering and outer clock conversion match the Browser worklet.
#include "native_genesis_wonderswan.h"
#include "wonderswan.h"
#include <cmath>
#include <cstring>
#include <new>

namespace {
struct NativeWonderSwan {
    WonderSwan core;
    NativeWonderSwanState state{};
    const uint32_t base_clock, engine_rate;
    uint64_t speaker_phase = 0;
    double ratio = 1, remainder = 0;
    float held_left = 0, held_right = 0;
    double speaker_left = 0, speaker_right = 0;
    double target_left[4]{}, target_right[4]{};
    double channel_left[4]{}, channel_right[4]{};
    bool wave_ready[4]{};
    uint16_t cold_position[4]{};

    NativeWonderSwan(uint32_t rate, uint32_t clock)
        : core(clock, rate), base_clock(clock), engine_rate(clock / 128) {
        state.clock = clock;
        state.sample_rate = rate;
    }
    void reset() {
        const auto clock = state.clock, rate = state.sample_rate;
        const auto mute = state.mute_mask;
        core.reset();
        state = NativeWonderSwanState{};
        state.clock = clock; state.sample_rate = rate; state.mute_mask = mute;
        for (unsigned ch = 0; ch < 4; ++ch) core.mute(ch, (mute & (1u << ch)) != 0);
        speaker_phase = 0;
        remainder = held_left = held_right = speaker_left = speaker_right = 0;
        std::memset(target_left, 0, sizeof(target_left));
        std::memset(target_right, 0, sizeof(target_right));
        std::memset(channel_left, 0, sizeof(channel_left));
        std::memset(channel_right, 0, sizeof(channel_right));
        std::memset(wave_ready, 0, sizeof(wave_ready));
        std::memset(cold_position, 0, sizeof(cold_position));
    }
    void center_targets() {
        const auto *r = state.registers;
        const auto control = r[16];
        for (unsigned ch = 0; ch < 4; ++ch) {
            double left = 0, right = 0;
            if ((r[17] & 8) && (control & (1u << ch))) {
                if (ch == 1 && (control & 32)) {
                    if (r[20] & 12) left = (r[20] & 8) ? 127.5 : 63.5;
                    if (r[20] & 3) right = (r[20] & 2) ? 127.5 : 63.5;
                } else {
                    double midpoint = 0;
                    if (ch == 3 && (control & 128)) {
                        if (r[14] & 16) midpoint = 7.5;
                    } else {
                        const auto address = (uint32_t(r[15]) << 6) + ch * 16;
                        for (unsigned i = 0; i < 16; ++i) {
                            const auto value = state.memory[(address + i) & 0x3fff];
                            midpoint += (value & 15) + (value >> 4);
                        }
                        midpoint /= 32;
                    }
                    left = midpoint * (r[8 + ch] >> 4);
                    right = midpoint * (r[8 + ch] & 15);
                }
            }
            target_left[ch] = left / 1024;
            target_right[ch] = right / 1024;
        }
    }
    void write(uint32_t address, uint8_t value, uint8_t mask) {
        if (address < 0x80 || address > 0x9f) return;
        if (address == 0x90) value = (state.registers[16] & uint8_t(~mask)) | (value & mask);
        core.writeRegister(address, value);
        state.registers[address - 0x80] = address == 0x8e ? (value & uint8_t(~8)) : value;
        center_targets();
    }
    void memory(uint32_t address, uint8_t value) {
        if (address >= 0x4000) return;
        core.writeMemory(address, value);
        state.memory[address] = value;
        center_targets();
    }
    void speaker_sum() {
        speaker_left = speaker_right = 0;
        for (unsigned ch = 0; ch < 4; ++ch) if (!(state.mute_mask & (1u << ch))) {
            speaker_left += channel_left[ch]; speaker_right += channel_right[ch];
        }
    }
    void speaker_tick() {
        const auto *r = state.registers;
        const auto control = r[16];
        for (unsigned ch = 0; ch < 4; ++ch) {
            channel_left[ch] = channel_right[ch] = 0;
            if (!(r[17] & 8) || !(control & (1u << ch))) continue;
            const bool pcm = ch == 1 && (control & 32);
            // The cold raw core emits its first fetched waveform one tick later.
            if (pcm || wave_ready[ch]) {
                channel_left[ch] = target_left[ch]; channel_right[ch] = target_right[ch];
            }
            if (!pcm && !wave_ready[ch]) {
                const auto period = 2048 - (r[ch * 2] | ((r[ch * 2 + 1] & 7) << 8));
                cold_position[ch] += 128;
                if (cold_position[ch] >= period) wave_ready[ch] = true;
            }
        }
        speaker_sum();
    }
    void internal_sample(float &left, float &right) {
        core.generate(&left, &right, 1);
        speaker_phase += engine_rate;
        while (speaker_phase >= state.sample_rate) {
            speaker_phase -= state.sample_rate;
            speaker_tick();
        }
        const bool silent = !(state.registers[16] & 15) || !(state.registers[17] & 8);
        // Match the worklet's Float32Array assignment before outer averaging.
        left = silent ? 0 : float(double(left) - speaker_left);
        right = silent ? 0 : float(double(right) - speaker_right);
    }
    void sample(float &left, float &right) {
        remainder += ratio;
        const auto count = uint32_t(std::floor(remainder));
        remainder -= count;
        if (count) {
            double sum_left = 0, sum_right = 0;
            for (uint32_t i = 0; i < count; ++i) {
                float l, r; internal_sample(l, r); sum_left += l; sum_right += r;
            }
            held_left = float(sum_left / count); held_right = float(sum_right / count);
        }
        left = held_left; right = held_right;
    }
};
static_assert(sizeof(NativeWonderSwanState) == 16428, "Rust/C++ WonderSwan state layout");
}
extern "C" {
void *native_wonderswan_create(uint32_t rate, uint32_t clock) {
    if (clock < 100000 || !WonderSwan::validConfiguration(clock, rate)) return nullptr;
    return new (std::nothrow) NativeWonderSwan(rate, clock);
}
size_t native_wonderswan_core_size() { return sizeof(NativeWonderSwan); }
void native_wonderswan_destroy(void *ptr) { delete static_cast<NativeWonderSwan *>(ptr); }
void native_wonderswan_reset(void *ptr) { if (ptr) static_cast<NativeWonderSwan *>(ptr)->reset(); }
void native_wonderswan_write(void *ptr, uint32_t address, uint8_t value, uint8_t mask) {
    if (ptr) static_cast<NativeWonderSwan *>(ptr)->write(address, value, mask);
}
void native_wonderswan_write_memory(void *ptr, uint32_t address, uint8_t value) {
    if (ptr) static_cast<NativeWonderSwan *>(ptr)->memory(address, value);
}
uint8_t native_wonderswan_read_register(const void *ptr, uint32_t address) {
    return ptr && address >= 0x80 && address <= 0x9f ? static_cast<const NativeWonderSwan *>(ptr)->state.registers[address - 0x80] : 0;
}
uint8_t native_wonderswan_read_memory(const void *ptr, uint32_t address) {
    return ptr && address < 0x4000 ? static_cast<const NativeWonderSwan *>(ptr)->state.memory[address] : 0;
}
void native_wonderswan_key_off(void *ptr, uint32_t channel) {
    if (!ptr || channel > 3) return;
    auto &p = *static_cast<NativeWonderSwan *>(ptr);
    p.write(0x90, 0, uint8_t(1u << channel));
    if (!(p.state.registers[16] & 15)) p.held_left = p.held_right = 0;
}
void native_wonderswan_stop(void *ptr) {
    if (!ptr) return;
    auto &p = *static_cast<NativeWonderSwan *>(ptr);
    p.write(0x90, 0, 15); p.held_left = p.held_right = 0;
}
void native_wonderswan_set_mute_mask(void *ptr, uint32_t mask) {
    if (!ptr || mask > 15) return;
    auto &p = *static_cast<NativeWonderSwan *>(ptr);
    p.state.mute_mask = uint8_t(mask);
    for (unsigned ch = 0; ch < 4; ++ch) p.core.mute(ch, (mask & (1u << ch)) != 0);
    p.speaker_sum();
    if (mask == 15) p.held_left = p.held_right = 0;
}
int32_t native_wonderswan_set_clock(void *ptr, uint32_t hz) {
    if (!ptr || hz < 100000 || hz > 100000000) return 0;
    auto &p = *static_cast<NativeWonderSwan *>(ptr);
    p.state.clock = hz; p.ratio = double(hz) / p.base_clock; return 1;
}
void native_wonderswan_sample(void *ptr, float *left, float *right) {
    if (!left || !right) return;
    if (ptr) static_cast<NativeWonderSwan *>(ptr)->sample(*left, *right);
    else *left = *right = 0;
}
void native_wonderswan_raw_sample(void *ptr, float *left, float *right) {
    if (!left || !right) return;
    if (ptr) static_cast<NativeWonderSwan *>(ptr)->core.generate(left, right, 1);
    else *left = *right = 0;
}
void native_wonderswan_get_state(const void *ptr, NativeWonderSwanState *out) {
    if (!out) return;
    if (ptr) *out = static_cast<const NativeWonderSwan *>(ptr)->state;
    else std::memset(out, 0, sizeof(*out));
}
}
