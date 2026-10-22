// Exact Browser/WASM Sega PSG and ymfm OPLL cores; no device or JS runtime.
#include "native_genesis_master_system.h"
#include "segapsg.h"
#include "ymfm_opl.h"
#include "ymfm_wasm_interface.h"
#include <algorithm>
#include <array>
#include <cmath>
#include <cstring>
#include <vector>

namespace {
constexpr uint32_t CLOCK = 3579545;
const uint8_t PHYSICAL[5] = {6, 7, 8, 8, 7};
const uint8_t VOLUME_REGISTER[5] = {0x36, 0x37, 0x38, 0x38, 0x37};
const uint8_t VOLUME_SHIFT[5] = {0, 0, 4, 0, 4};
struct NativeMasterSystem {
    ymfm_wasm_interface intf;
    ymfm::ym2413 opll;
    SegaPSG psg;
    uint32_t rate, psg_clock = CLOCK, opll_clock = CLOCK, psg_mute_mask = 0, opll_mute_mask = 0;
    uint64_t opll_remainder = 0;
    double psg_remainder = 0, psg_left = 0, psg_right = 0, last_opll = 0;
    std::array<uint8_t, 64> registers{};
    std::array<uint16_t, 3> periods{};
    std::array<uint8_t, 4> attenuation{{15, 15, 15, 15}}, levels{};
    std::vector<uint8_t> power_on;
    uint32_t pending = 0, forced_channels = 0x1ff;
    uint8_t muted_drums = 31, latch = 0, noise_mode = 0;
    bool silent = true;
    explicit NativeMasterSystem(uint32_t sr) : opll(intf), psg(sr, CLOCK), rate(sr) {
        opll.reset();
        // Browser Ym2413.reset() recreates its core to reset free-running LFO
        // and envelope counters. Restore a preallocated power-on snapshot here
        // instead of destroying/constructing a chip on the audio callback.
        ymfm::ymfm_saved_state saved(power_on, true); opll.save_restore(saved); update_mute();
    }
    void raw_opll(uint8_t reg, uint8_t value) { opll.write(0, reg); opll.write(1, value); }
    void update_mute() { opll.set_mute_mask(opll_mute_mask | forced_channels); }
    void volume(uint8_t reg) {
        uint8_t value = registers[reg];
        if (registers[0x0e] & 32) for (uint8_t i = 0; i < 5; ++i) if ((muted_drums & (1 << i)) && VOLUME_REGISTER[i] == reg)
            value = uint8_t((value & ~(15 << VOLUME_SHIFT[i])) | (15 << VOLUME_SHIFT[i]));
        raw_opll(reg, value);
    }
    void restore(uint8_t channel) { forced_channels &= ~(1u << channel); update_mute(); volume(0x30 + channel); silent = false; }
    void opll_write(uint8_t reg, uint8_t value) {
        if (reg >= 64) return;
        const uint8_t old = registers[reg]; registers[reg] = value;
        if (reg >= 0x20 && reg <= 0x28) {
            pending &= ~(1u << (reg - 0x20));
            if ((value & 16) && !((registers[0x0e] & 32) && reg >= 0x26)) restore(reg - 0x20);
        }
        if (reg == 0x0e) {
            if ((old ^ value) & 32) for (uint8_t i = 0x36; i <= 0x38; ++i) volume(i);
            for (uint8_t i = 0; i < 5; ++i) if ((old ^ value) & (16 >> i)) pending &= ~(1u << (9 + i));
            if (value & 32) for (uint8_t i = 0; i < 5; ++i) if (value & (16 >> i)) {
                muted_drums &= ~(1 << i); restore(PHYSICAL[i]);
            }
        }
        if (reg >= 0x30 && reg <= 0x38) volume(reg); else raw_opll(reg, value);
    }
    void psg_write(uint8_t value, bool author = true) {
        if (value & 128) latch = (value >> 4) & 7;
        const uint8_t channel = latch >> 1;
        if (latch & 1) { attenuation[channel] = value & 15; if (author && (value & 15) != 15) levels[channel] = value & 15; }
        else if (channel == 3) noise_mode = value & 7;
        else if (value & 128) periods[channel] = (periods[channel] & 0x3f0) | (value & 15);
        else periods[channel] = (periods[channel] & 15) | ((value & 63) << 4);
        psg.write(value);
    }
    void key_off(uint8_t voice) {
        if (voice < 4) { psg_write(0x9f | (voice << 5), false); return; }
        if (voice < 13) {
            const uint8_t channel = voice - 4; pending &= ~(1u << channel);
            registers[0x20 + channel] &= ~16; raw_opll(0x20 + channel, registers[0x20 + channel]);
            forced_channels |= 1u << channel; update_mute();
        } else if (voice < 18) {
            const uint8_t index = voice - 13, bit = 16 >> index, physical = PHYSICAL[index];
            pending &= ~(1u << (9 + index)); registers[0x0e] &= ~bit; raw_opll(0x0e, registers[0x0e]);
            muted_drums |= 1 << index; volume(VOLUME_REGISTER[index]);
            bool other = false;
            for (uint8_t i = 0; i < 5; ++i) if (PHYSICAL[i] == physical && (registers[0x0e] & (16 >> i))) other = true;
            if (!other) { forced_channels |= 1u << physical; update_mute(); }
        }
    }
    void audition(uint8_t voice) {
        if (voice < 4) { psg_write(0x90 | (voice << 5) | levels[voice], false); return; }
        if (voice >= 18) return;
        key_off(voice); pending |= 1u << (voice - 4);
    }
    void stop(uint8_t chip) {
        if (chip == 0 || chip == 2) { for (uint8_t voice = 0; voice < 4; ++voice) key_off(voice); psg_left = psg_right = 0; }
        if (chip == 1 || chip == 2) {
            pending = 0; for (uint8_t voice = 4; voice < 13; ++voice) key_off(voice);
            registers[0x0e] &= 224; raw_opll(0x0e, registers[0x0e]); muted_drums = 31;
            for (uint8_t reg = 0x36; reg <= 0x38; ++reg) volume(reg);
            silent = true; last_opll = 0;
        }
    }
    void reset(uint8_t chip) {
        if (chip == 0 || chip == 2) {
            psg.reset(); psg.set_mute_mask(psg_mute_mask); periods.fill(0); attenuation.fill(15); levels.fill(0);
            latch = noise_mode = 0; psg_remainder = psg_left = psg_right = 0;
        }
        if (chip == 1 || chip == 2) {
            ymfm::ymfm_saved_state saved(power_on, false); opll.save_restore(saved);
            intf.irq_asserted = false; intf.timer_remaining[0] = intf.timer_remaining[1] = -1;
            registers.fill(0); forced_channels = 0x1ff; muted_drums = 31; update_mute();
            pending = 0; opll_remainder = 0; last_opll = 0; silent = true;
        }
    }
    static float normalize(int32_t value) { return float(std::max(-32768, std::min(32767, value))) / 32768.0f; }
    void sample(float *pl, float *pr, float *fl, float *fr) {
        // Preserve the fixed base-clock core and its phase across clock edits,
        // exactly as Browser ChipClockResampler does, including held samples.
        psg_remainder += double(psg_clock) / CLOCK;
        const uint32_t psg_count = uint32_t(std::floor(psg_remainder)); psg_remainder -= psg_count;
        if (psg_count) {
            double l = 0, r = 0; float a, b;
            for (uint32_t i = 0; i < psg_count; ++i) { psg.generate(&a, &b, 1); l += a; r += b; }
            psg_left = l / psg_count; psg_right = r / psg_count;
        }
        *pl = float(psg_left); *pr = float(psg_right);
        opll_remainder += opll.sample_rate(opll_clock);
        const uint32_t count = uint32_t(opll_remainder / rate); opll_remainder -= uint64_t(count) * rate;
        if (count) {
            double sum = 0;
            for (uint32_t i = 0; i < count; ++i) {
                ymfm::ym2413::output_data output; opll.generate(&output); intf.advance_sample(opll);
                int32_t mix = 0; for (uint32_t j = 0; j < ymfm::ym2413::OUTPUTS; ++j) mix += output.data[j];
                sum += normalize(mix);
            }
            last_opll = sum / count;
        }
        *fl = *fr = silent ? 0 : float(last_opll);
        // One real OPLL interval must see key-off before a preview retrigger.
        if (count && pending) {
            const uint32_t starts = pending; pending = 0;
            for (uint8_t i = 0; i < 14; ++i) if (starts & (1u << i)) {
                if (i < 9) opll_write(0x20 + i, registers[0x20 + i] | 16);
                else if (registers[0x0e] & 32) opll_write(0x0e, registers[0x0e] | (16 >> (i - 9)));
            }
        }
    }
};
NativeMasterSystem *core(void *p) { return static_cast<NativeMasterSystem *>(p); }
}
extern "C" {
void *native_master_system_create(uint32_t rate) {
    if (rate < 8000 || rate > 384000) return nullptr;
    try { return new NativeMasterSystem(rate); } catch (...) { return nullptr; }
}
void native_master_system_destroy(void *p) { delete core(p); }
void native_master_system_reset(void *p, uint8_t chip) { core(p)->reset(chip); }
void native_master_system_stop(void *p, uint8_t chip) { core(p)->stop(chip); }
void native_master_system_key_off(void *p, uint8_t voice) { core(p)->key_off(voice); }
void native_master_system_audition_key_on(void *p, uint8_t voice) { core(p)->audition(voice); }
void native_master_system_write(void *p, uint8_t chip, uint8_t reg, uint8_t value) { if (chip == 0) core(p)->psg_write(value); else if (chip == 1) core(p)->opll_write(reg, value); }
void native_master_system_sample(void *p, float *pl, float *pr, float *fl, float *fr) { core(p)->sample(pl, pr, fl, fr); }
int native_master_system_set_clock(void *p, uint8_t chip, uint32_t clock) {
    if (clock < 100000 || clock > 100000000 || chip > 1) return 0;
    if (chip == 0) core(p)->psg_clock = clock; else core(p)->opll_clock = clock; return 1;
}
void native_master_system_set_mute_mask(void *p, uint8_t chip, uint32_t mask) {
    auto *h = core(p);
    if (chip == 0) { h->psg_mute_mask = mask & 15; h->psg.set_mute_mask(h->psg_mute_mask); h->psg_left = h->psg_right = 0; }
    else if (chip == 1) { h->opll_mute_mask = mask & 0x1ff; h->update_mute(); h->last_opll = 0; }
}
uint8_t native_master_system_read(void *p, uint8_t offset) { return core(p)->opll.read(offset); }
uint8_t native_master_system_read_status(void *p) { return core(p)->opll.read_status(); }
uint8_t native_master_system_get_irq(void *p) { return core(p)->intf.irq_asserted ? 1 : 0; }
void native_master_system_get_state(void *p, NativeMasterSystemState *s) {
    const auto *h = core(p); std::memcpy(s->registers, h->registers.data(), 64); std::memcpy(s->periods, h->periods.data(), 6);
    std::memcpy(s->attenuation, h->attenuation.data(), 4); std::memcpy(s->levels, h->levels.data(), 4);
    s->psg_clock = h->psg_clock; s->opll_clock = h->opll_clock; s->psg_mute_mask = h->psg_mute_mask; s->opll_mute_mask = h->opll_mute_mask;
    s->latch = h->latch; s->noise_mode = h->noise_mode; s->status = core(p)->opll.read_status(); s->irq = h->intf.irq_asserted ? 1 : 0;
}
}
