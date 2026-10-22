// The same pinned ymfm OPNA core as Browser/WASM, with its output-rate adapter.
#include "native_genesis_ym2608.h"
#include "ymfm_opn.h"
#include "ymfm_wasm_interface.h"
#include <algorithm>
#include <array>
#include <cstring>
#include <vector>

namespace {
constexpr uint32_t ROM_SIZE = 0x2000, MEMORY_SIZE = 0x200000;
struct Interface : ymfm_wasm_interface {
    // Allocate once during create, never while applying a callback command.
    std::vector<uint8_t> rom = std::vector<uint8_t>(ROM_SIZE, 0);
    std::vector<uint8_t> memory = std::vector<uint8_t>(MEMORY_SIZE, 0);
    uint8_t ymfm_external_read(ymfm::access_class type, uint32_t address) override {
        if (type == ymfm::ACCESS_ADPCM_A) return address < rom.size() ? rom[address] : 0;
        if (type == ymfm::ACCESS_ADPCM_B) return address < memory.size() ? memory[address] : 0;
        return 0;
    }
    void ymfm_external_write(ymfm::access_class type, uint32_t address, uint8_t value) override {
        if (type == ymfm::ACCESS_ADPCM_B && address < memory.size()) memory[address] = value;
    }
};
struct Chip : ymfm::ym2608 {
    explicit Chip(Interface &intf) : ym2608(intf) {}
    uint8_t peek_status() {
        // The primary status read is observational; unlike read_status_hi it
        // does not propagate the ADPCM flags back into the interrupt engine.
        return read_status();
    }
    uint8_t peek_status_hi() {
        uint8_t value = peek_status() & (fm_engine::STATUS_TIMERA | fm_engine::STATUS_TIMERB);
        const uint8_t adpcm = m_adpcm_b.status();
        if (adpcm & ymfm::adpcm_b_channel::STATUS_EOS) value |= 0x04;
        if (adpcm & ymfm::adpcm_b_channel::STATUS_BRDY) value |= 0x08;
        if (adpcm & ymfm::adpcm_b_channel::STATUS_PLAYING) value |= 0x20;
        return value & ~(m_flag_control & 0x1f);
    }
    void clear_output_history() { m_last_fm.clear(); }
};
struct NativeYm2608 {
    Interface intf;
    Chip chip;
    uint32_t output_rate, clock = 8000000, source_mute_mask = 0;
    uint64_t remainder = 0;
    double last_left = 0, last_right = 0;
    std::array<uint8_t, 512> registers{};
    std::array<bool, 512> written{};
    std::array<uint8_t, 2> frequency_latch{};

    explicit NativeYm2608(uint32_t rate) : chip(intf), output_rate(rate) { reset(); }
    void reset() {
        chip.reset();
        remainder = 0; last_left = last_right = 0;
        registers.fill(0); written.fill(false); frequency_latch.fill(0);
        for (auto reg : {0xb4, 0xb5, 0xb6, 0x1b4, 0x1b5, 0x1b6}) registers[reg] = 0xc0;
        for (uint32_t reg = 0x18; reg <= 0x1d; ++reg) registers[reg] = 0xdf;
        registers[0x10c] = registers[0x10d] = 0xff;
        registers[0x29] = 0x1f; registers[0x110] = 0x1c;
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
        } else if (!(port == 1 && reg == 0x10 && (value & 0x80))) {
            // IRQ flag-clear is a command; it does not replace the applied
            // mask. Keeping the mask avoids changing IRQ settings on Stop.
            registers[base + reg] = value;
            written[base + reg] = true;
        }
        chip.write(port * 2, reg); chip.write(port * 2 + 1, value);
    }
    void key_off(uint8_t voice) {
        if (voice < 6) write(0, 0x28, voice < 3 ? voice : voice + 1);
        else if (voice < 9) write(0, 8 + voice - 6, 0);
        else if (voice < 15) write(0, 0x10, 0x80 | (1 << (voice - 9)));
        else if (voice == 15) write(1, 0, 1);
    }
    void stop() {
        const auto saved = registers;
        const auto valid = written;
        const auto saved_latch = frequency_latch;
        reset(); chip.clear_output_history();
        for (uint32_t index = 0; index < 512; ++index) {
            if (!valid[index]) continue;
            const uint8_t port = index >> 8, reg = index & 255;
            if ((reg >= 0xa0 && reg <= 0xaf) || (index >= 0x2d && index <= 0x2f) || index == 0x10 || index == 0x28 ||
                index == 0x100 || index == 0x108 || (index >= 8 && index <= 10)) continue;
            uint8_t value = saved[index];
            if (index == 0x27) value &= 0xc0; // Stop timers/CSM triggers.
            if (index == 0x110) value &= 0x7f;
            write(port, reg, value);
        }
        for (uint8_t port = 0; port < 2; ++port)
            for (uint8_t base : {0xa0, 0xa8}) for (uint8_t ch = 0; ch < 3; ++ch) {
                const uint8_t low = base + ch, high = low + 4;
                if (valid[port * 256 + high]) write(port, high, saved[port * 256 + high]);
                if (valid[port * 256 + low]) write(port, low, saved[port * 256 + low]);
            }
        write(0, 0xa4, saved_latch[0]); write(0, 0xac, saved_latch[1]);
        // Explicitly stop all non-FM sections; their RAM/ROM and pitch remain.
        for (uint8_t voice = 6; voice < 16; ++voice) key_off(voice);
    }
    static float normalize(int32_t value) {
        return static_cast<float>(std::max(-32768, std::min(32767, value))) / 32768.0f;
    }
    void sample(float *left, float *right) {
        // Match Ym2608AudioEngine exactly: normalize/clamp each native sample,
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
NativeYm2608 *core(void *ptr) { return static_cast<NativeYm2608 *>(ptr); }
int transfer(std::vector<uint8_t> &target, const uint8_t *bytes, uint32_t offset, uint32_t size) {
    if (offset > target.size() || size > target.size() - offset || (size && !bytes)) return 0;
    if (size) std::memcpy(target.data() + offset, bytes, size);
    return 1;
}
}
extern "C" {
void *native_ym2608_create(uint32_t rate) {
    if (rate < 8000 || rate > 384000) return nullptr;
    try { return new NativeYm2608(rate); } catch (...) { return nullptr; }
}
void native_ym2608_destroy(void *ptr) { delete core(ptr); }
void native_ym2608_reset(void *ptr) { core(ptr)->reset(); }
void native_ym2608_stop(void *ptr) { core(ptr)->stop(); }
void native_ym2608_key_off(void *ptr, uint8_t voice) { core(ptr)->key_off(voice); }
void native_ym2608_write(void *ptr, uint8_t port, uint8_t reg, uint8_t value) { core(ptr)->write(port, reg, value); }
void native_ym2608_sample(void *ptr, float *left, float *right) { core(ptr)->sample(left, right); }
int native_ym2608_set_clock(void *ptr, uint32_t clock) {
    if (clock < 100000 || clock > 100000000) return 0;
    core(ptr)->clock = clock; return 1;
}
void native_ym2608_set_source_mute_mask(void *ptr, uint32_t mask) {
    auto *h = core(ptr); h->source_mute_mask = mask & 7;
    h->chip.set_adpcm_mute((mask & 2) != 0, (mask & 4) != 0);
}
int native_ym2608_load_rom(void *ptr, const uint8_t *bytes, uint32_t offset, uint32_t size) { return transfer(core(ptr)->intf.rom, bytes, offset, size); }
int native_ym2608_load_memory(void *ptr, const uint8_t *bytes, uint32_t offset, uint32_t size) { return transfer(core(ptr)->intf.memory, bytes, offset, size); }
void native_ym2608_clear_memory(void *ptr) { auto &memory = core(ptr)->intf.memory; std::fill(memory.begin(), memory.end(), 0); }
int native_ym2608_copy_memory(void *ptr, uint8_t *bytes, uint32_t offset, uint32_t size) {
    const auto &memory = core(ptr)->intf.memory;
    if (offset > memory.size() || size > memory.size() - offset || (size && !bytes)) return 0;
    if (size) std::memcpy(bytes, memory.data() + offset, size);
    return 1;
}
uint8_t native_ym2608_read(void *ptr, uint8_t offset) { return core(ptr)->chip.read(offset); }
uint8_t native_ym2608_read_status(void *ptr) { return core(ptr)->chip.read_status(); }
uint8_t native_ym2608_read_status_hi(void *ptr) { return core(ptr)->chip.read_status_hi(); }
uint8_t native_ym2608_get_irq(void *ptr) { return core(ptr)->intf.irq_asserted ? 1 : 0; }
void native_ym2608_get_state(void *ptr, NativeYm2608State *state) {
    auto *h = core(ptr);
    std::memcpy(state->registers, h->registers.data(), h->registers.size());
    state->clock = h->clock; state->source_mute_mask = h->source_mute_mask;
    state->status = h->chip.peek_status(); state->status_hi = h->chip.peek_status_hi();
    state->irq = h->intf.irq_asserted ? 1 : 0;
}
}
