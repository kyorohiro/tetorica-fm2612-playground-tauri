// Exact Browser/WASM Sega PSG and ymfm OPLL cores; no device or JS runtime.
#include "native_genesis_master_system.h"
#include "segapsg.h"
#include "native_genesis_opll.h"
#include <array>
#include <cmath>
#include <cstring>

namespace {
constexpr uint32_t CLOCK = 3579545;
struct NativeMasterSystem : NativeOpllCore {
    SegaPSG psg;
    uint32_t psg_clock = CLOCK, psg_mute_mask = 0;
    double psg_remainder = 0, psg_left = 0, psg_right = 0;
    std::array<uint16_t, 3> periods{};
    std::array<uint8_t, 4> attenuation{{15, 15, 15, 15}}, levels{};
    uint8_t latch = 0, noise_mode = 0;
    explicit NativeMasterSystem(uint32_t sr) : NativeOpllCore(sr), psg(sr, CLOCK) {}
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
        NativeOpllCore::key_off(voice - 4);
    }
    void audition(uint8_t voice) {
        if (voice < 4) { psg_write(0x90 | (voice << 5) | levels[voice], false); return; }
        NativeOpllCore::audition(voice - 4);
    }
    void stop(uint8_t chip) {
        if (chip == 0 || chip == 2) { for (uint8_t voice = 0; voice < 4; ++voice) key_off(voice); psg_left = psg_right = 0; }
        if (chip == 1 || chip == 2) stop_opll();
    }
    void reset(uint8_t chip) {
        if (chip == 0 || chip == 2) {
            psg.reset(); psg.set_mute_mask(psg_mute_mask); periods.fill(0); attenuation.fill(15); levels.fill(0);
            latch = noise_mode = 0; psg_remainder = psg_left = psg_right = 0;
        }
        if (chip == 1 || chip == 2) reset_opll();
    }
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
        sample_opll(fl, fr);
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
