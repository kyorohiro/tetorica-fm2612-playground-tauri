// The same pinned ymfm OPNB core as Browser/WASM, with its output-rate adapter.
#include "native_genesis_ym2610b.h"
#include "ymfm_opn.h"
#include "ymfm_wasm_interface.h"
#include <algorithm>
#include <array>
#include <cstring>
#include <vector>

namespace {
constexpr uint32_t ROM_SIZE = 0x1000000;
struct Interface : ymfm_wasm_interface {
    // Reserve both independent ROM spaces once, on the control thread.
    std::vector<uint8_t> rom[2] = {std::vector<uint8_t>(ROM_SIZE, 0), std::vector<uint8_t>(ROM_SIZE, 0)};
    uint8_t ymfm_external_read(ymfm::access_class type, uint32_t address) override {
        if (type != ymfm::ACCESS_ADPCM_A && type != ymfm::ACCESS_ADPCM_B) return 0;
        const auto &bank = rom[type == ymfm::ACCESS_ADPCM_B ? 1 : 0];
        return address < bank.size() ? bank[address] : 0;
    }
};
struct Chip : ymfm::ym2610 {
    Chip(Interface &intf, bool variant) : ym2610(intf, variant ? 0x3f : 0x36) {}
    // Both YM2610 status reads are observational (unlike OPNA's status_hi).
    void clear_output_history() { m_last_fm.clear(); }
};
struct NativeYm2610B {
    Interface intf;
    Chip chip;
    uint32_t output_rate, clock = 8000000, source_mute_mask = 0;
    const bool variant;
    uint64_t remainder = 0;
    double last_left = 0, last_right = 0;
    std::array<uint8_t, 512> registers{};
    std::array<bool, 512> written{};
    std::array<uint8_t, 2> frequency_latch{};

    NativeYm2610B(uint32_t rate, bool full) : chip(intf, full), output_rate(rate), variant(full) { reset(); }
    void reset() {
        chip.reset();
        remainder = 0; last_left = last_right = 0;
        registers.fill(0); written.fill(false); frequency_latch.fill(0);
        for (auto reg : {0xb4, 0xb5, 0xb6, 0x1b4, 0x1b5, 0x1b6}) registers[reg] = 0xc0;
        for (uint32_t reg = 0x108; reg <= 0x10d; ++reg) registers[reg] = 0xdf;
        registers[0x1c] = 0;
        chip.set_adpcm_mute((source_mute_mask & 2) != 0, (source_mute_mask & 4) != 0);
    }
    void write(uint8_t port, uint8_t reg, uint8_t value) {
        if (port > 1) return;
        const uint32_t base = port * 256;
        // ymfm commits a high/low frequency pair on the low-byte write. Keep
        // the applied pair for inspection and latch-safe Stop restoration.
        if ((reg & 0xf0) == 0xa0 && (reg & 3) != 3) {
            auto &latch = frequency_latch[(reg >> 3) & 1];
            if (reg & 4) latch = value & 0x3f;
            else {
                registers[base + reg] = value;
                registers[base + (reg | 4)] = latch;
                written[base + reg] = written[base + (reg | 4)] = true;
            }
        } else {
            registers[base + reg] = value;
            written[base + reg] = true;
        }
        chip.write(port * 2, reg); chip.write(port * 2 + 1, value);
    }
    void key_off(uint8_t voice) {
        if (voice < 6) write(0, 0x28, voice < 3 ? voice : voice + 1);
        else if (voice < 9) write(0, 8 + voice - 6, 0);
        else if (voice < 15) write(1, 0, 0x80 | (1 << (voice - 9)));
        else if (voice == 15) write(0, 0x10, 1);
    }
    void stop() {
        const auto saved = registers;
        const auto valid = written;
        const auto saved_latch = frequency_latch;
        reset(); chip.clear_output_history();
        for (uint32_t index = 0; index < 512; ++index) {
            if (!valid[index]) continue;
            const uint8_t port = index >> 8, reg = index & 255;
            if ((reg >= 0xa0 && reg <= 0xaf) || index == 0x10 || index == 0x18 || index == 0x28 ||
                index == 0x100 || (index >= 8 && index <= 10)) continue;
            uint8_t value = saved[index];
            if (index == 0x27) value &= 0xc0; // Stop timers/CSM triggers.
            write(port, reg, value);
        }
        for (uint8_t port = 0; port < 2; ++port)
            for (uint8_t base : {0xa0, 0xa8}) for (uint8_t ch = 0; ch < 3; ++ch) {
                const uint8_t low = base + ch, high = low + 4;
                if (valid[port * 256 + high]) write(port, high, saved[port * 256 + high]);
                if (valid[port * 256 + low]) write(port, low, saved[port * 256 + low]);
            }
        write(0, 0xa4, saved_latch[0]); write(0, 0xac, saved_latch[1]);
        // Explicitly stop all non-FM sections; their ROM ranges and pitch remain.
        for (uint8_t voice = 6; voice < 16; ++voice) key_off(voice);
    }
    static float normalize(int32_t value) {
        return static_cast<float>(std::max(-32768, std::min(32767, value))) / 32768.0f;
    }
    void sample(float *left, float *right) {
        // Match Ym2610BAudioEngine exactly: normalize/clamp each native sample,
        // then average in double precision, holding the prior value on upsample.
        remainder += chip.sample_rate(clock);
        const uint32_t count = static_cast<uint32_t>(remainder / output_rate);
        remainder -= static_cast<uint64_t>(count) * output_rate;
        if (count) {
            double l = 0, r = 0;
            for (uint32_t n = 0; n < count; ++n) {
                Chip::output_data output;
                chip.generate(&output); intf.advance_sample(chip);
                const int32_t ssg = source_mute_mask & 1 ? 0 : output.data[2];
                l += normalize(output.data[0] + ssg); r += normalize(output.data[1] + ssg);
            }
            last_left = l / count; last_right = r / count;
        }
        *left = static_cast<float>(last_left); *right = static_cast<float>(last_right);
    }
};
NativeYm2610B *core(void *ptr) { return static_cast<NativeYm2610B *>(ptr); }
int transfer(std::vector<uint8_t> &target, const uint8_t *bytes, uint32_t offset, uint32_t size) {
    if (offset > target.size() || size > target.size() - offset || (size && !bytes)) return 0;
    if (size) std::memcpy(target.data() + offset, bytes, size);
    return 1;
}
}
extern "C" {
void *native_ym2610b_create(uint32_t rate, uint8_t variant) {
    if (rate < 8000 || rate > 384000 || variant > 1) return nullptr;
    try { return new NativeYm2610B(rate, variant != 0); } catch (...) { return nullptr; }
}
void native_ym2610b_destroy(void *ptr) { delete core(ptr); }
void native_ym2610b_reset(void *ptr) { core(ptr)->reset(); }
void native_ym2610b_stop(void *ptr) { core(ptr)->stop(); }
void native_ym2610b_key_off(void *ptr, uint8_t voice) { core(ptr)->key_off(voice); }
void native_ym2610b_write(void *ptr, uint8_t port, uint8_t reg, uint8_t value) { core(ptr)->write(port, reg, value); }
void native_ym2610b_sample(void *ptr, float *left, float *right) { core(ptr)->sample(left, right); }
int native_ym2610b_set_clock(void *ptr, uint32_t clock) {
    if (clock < 100000 || clock > 100000000) return 0;
    core(ptr)->clock = clock; return 1;
}
void native_ym2610b_set_source_mute_mask(void *ptr, uint32_t mask) {
    auto *h = core(ptr); h->source_mute_mask = mask & 7;
    h->chip.set_adpcm_mute((mask & 2) != 0, (mask & 4) != 0);
}
int native_ym2610b_load_rom(void *ptr, uint8_t type, const uint8_t *bytes, uint32_t offset, uint32_t size) {
    return type < 2 ? transfer(core(ptr)->intf.rom[type], bytes, offset, size) : 0;
}
void native_ym2610b_clear_rom(void *ptr, uint8_t type) {
    if (type < 2) { auto &bank = core(ptr)->intf.rom[type]; std::fill(bank.begin(), bank.end(), 0); }
}
int native_ym2610b_copy_rom(void *ptr, uint8_t type, uint8_t *bytes, uint32_t offset, uint32_t size) {
    if (type > 1) return 0;
    const auto &bank = core(ptr)->intf.rom[type];
    if (offset > bank.size() || size > bank.size() - offset || (size && !bytes)) return 0;
    if (size) std::memcpy(bytes, bank.data() + offset, size);
    return 1;
}
uint8_t native_ym2610b_read(void *ptr, uint8_t offset) { return core(ptr)->chip.read(offset); }
uint8_t native_ym2610b_read_status(void *ptr) { return core(ptr)->chip.read_status(); }
uint8_t native_ym2610b_read_status_hi(void *ptr) { return core(ptr)->chip.read_status_hi(); }
uint8_t native_ym2610b_get_irq(void *ptr) { return core(ptr)->intf.irq_asserted ? 1 : 0; }
void native_ym2610b_get_state(void *ptr, NativeYm2610BState *state) {
    auto *h = core(ptr);
    std::memcpy(state->registers, h->registers.data(), h->registers.size());
    state->clock = h->clock; state->source_mute_mask = h->source_mute_mask;
    state->status = h->chip.read_status(); state->status_hi = h->chip.read_status_hi();
    state->irq = h->intf.irq_asserted ? 1 : 0; state->variant = h->variant ? 1 : 0;
}
}
