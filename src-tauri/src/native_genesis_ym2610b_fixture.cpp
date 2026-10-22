// Offline PCM/state/allocation fixture; never opens an audio device.
#include "native_genesis_ym2610b.h"
#include <algorithm>
#include <cassert>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <iostream>
#include <new>
#include <sstream>
#include <string>
#include <vector>

static bool audit = false;
static uint64_t allocations = 0, frees = 0;
void *operator new(std::size_t n) { if (audit) ++allocations; if (void *p = std::malloc(n)) return p; throw std::bad_alloc(); }
void *operator new[](std::size_t n) { return ::operator new(n); }
void operator delete(void *p) noexcept { if (audit && p) ++frees; std::free(p); }
void operator delete[](void *p) noexcept { ::operator delete(p); }
void operator delete(void *p, std::size_t) noexcept { ::operator delete(p); }
void operator delete[](void *p, std::size_t) noexcept { ::operator delete(p); }

static void write(void *chip, uint8_t port, uint8_t reg, uint8_t value) { native_ym2610b_write(chip, port, reg, value); }
static double render(void *chip, uint32_t frames) {
    double energy = 0; float l = 0, r = 0;
    for (uint32_t i = 0; i < frames; ++i) { native_ym2610b_sample(chip, &l, &r); energy += std::abs(l) + std::abs(r); }
    return energy;
}
static void fm(void *chip, uint8_t channel) {
    const uint8_t port = channel / 3, ch = channel % 3;
    write(chip, port, 0xb0 + ch, 7);
    for (uint8_t slot : {0, 4, 8, 12}) {
        write(chip, port, 0x30 + slot + ch, 1);
        write(chip, port, 0x40 + slot + ch, slot == 12 ? 24 : 127);
        write(chip, port, 0x50 + slot + ch, 31);
        write(chip, port, 0x80 + slot + ch, 1);
    }
    write(chip, port, 0xa4 + ch, 0x22); write(chip, port, 0xa0 + ch, 0x69);
    write(chip, 0, 0x28, 0xf0 | (channel < 3 ? channel : channel + 1));
}
static int self_test() {
    assert(!native_ym2610b_create(0, 1)); assert(!native_ym2610b_create(48000, 2));
    for (uint8_t variant = 0; variant < 2; ++variant) {
        void *chip = native_ym2610b_create(48000, variant); assert(chip);
        uint8_t bytes[256], readback[256];
        for (uint32_t i = 0; i < 256; ++i) bytes[i] = static_cast<uint8_t>(i * 7);
        NativeYm2610BState state{};
        audit = true;
        for (uint8_t type = 0; type < 2; ++type) {
            assert(native_ym2610b_load_rom(chip, type, bytes, 0xffff00, 256));
            assert(native_ym2610b_load_rom(chip, type, bytes, 0x1000000, 0));
            assert(!native_ym2610b_load_rom(chip, type, bytes, 0xffff01, 256));
            assert(!native_ym2610b_load_rom(chip, type, nullptr, 0, 1));
            assert(native_ym2610b_copy_rom(chip, type, readback, 0xffff00, 256));
            assert(std::memcmp(bytes, readback, 256) == 0);
        }
        assert(!native_ym2610b_load_rom(chip, 2, bytes, 0, 1));
        assert(!native_ym2610b_copy_rom(chip, 2, readback, 0, 1));
        assert(native_ym2610b_set_clock(chip, 100000)); assert(native_ym2610b_set_clock(chip, 100000000));
        assert(!native_ym2610b_set_clock(chip, 1)); assert(native_ym2610b_set_clock(chip, 7987200));
        native_ym2610b_set_source_mute_mask(chip, 5);
        fm(chip, 1);
        write(chip, 0, 0xa4, 0x22); write(chip, 1, 0xa1, 0x77);
        native_ym2610b_get_state(chip, &state);
        assert(state.registers[0x1a5] == 0x22 && state.registers[0x1a1] == 0x77);
        write(chip, 0, 0, 0x34); write(chip, 0, 1, 2); write(chip, 0, 7, 0x3e); write(chip, 0, 8, 12);
        write(chip, 0, 0x12, 0x12); write(chip, 0, 0x19, 0x34); write(chip, 1, 8, 0xde);
        assert(render(chip, 4000) > 1);
        native_ym2610b_stop(chip); native_ym2610b_get_state(chip, &state);
        assert(state.variant == variant && state.clock == 7987200 && state.source_mute_mask == 5);
        assert(state.registers[0xb1] == 7 && state.registers[0x108] == 0xde);
        assert(state.registers[0x1a5] == 0x22 && state.registers[0x1a1] == 0x77);
        assert(state.registers[0] == 0x34 && state.registers[1] == 2 && state.registers[8] == 0);
        assert(state.registers[0x12] == 0x12 && state.registers[0x19] == 0x34);
        assert(render(chip, 256) == 0);
        write(chip, 0, 0x28, 0xf1); assert(render(chip, 256) > 0.1);
        native_ym2610b_reset(chip); native_ym2610b_get_state(chip, &state);
        assert(state.clock == 7987200 && state.source_mute_mask == 5 && state.registers[0xb1] == 0);
        for (uint8_t type = 0; type < 2; ++type) {
            assert(native_ym2610b_copy_rom(chip, type, readback, 0xffff00, 256));
            assert(std::memcmp(bytes, readback, 256) == 0);
        }
        // Independent bank clear does not touch the other bank.
        native_ym2610b_clear_rom(chip, 0);
        assert(native_ym2610b_copy_rom(chip, 0, readback, 0xffff00, 256));
        for (auto value : readback) assert(value == 0);
        assert(native_ym2610b_copy_rom(chip, 1, readback, 0xffff00, 256)); assert(std::memcmp(bytes, readback, 256) == 0);
        native_ym2610b_load_rom(chip, 0, bytes, 0xffff00, 256);
        write(chip, 0, 0, 0x55); assert(native_ym2610b_read(chip, 1) == 0x55);
        // YM2610's extended status/EOS reads are observational; ID and I/O
        // reads follow the real hardware bus implementation.
        write(chip, 0, 0xff, 0); assert(native_ym2610b_read(chip, 1) == 1);
        write(chip, 0, 0x0e, 0); assert(native_ym2610b_read(chip, 1) == 255);
        write(chip, 0, 0x24, 255); write(chip, 0, 0x25, 3); write(chip, 0, 0x27, 5);
        render(chip, 128); native_ym2610b_get_state(chip, &state);
        assert((state.status & 1) && state.irq);
        for (int i = 0; i < 100; ++i) native_ym2610b_get_state(chip, &state);
        assert((state.status & 1) && state.irq);
        assert(native_ym2610b_read_status(chip) == state.status);
        write(chip, 0, 0x27, 0x10); native_ym2610b_get_state(chip, &state);
        assert(!(state.status & 1) && !state.irq);
        native_ym2610b_set_source_mute_mask(chip, 0);
        // Both decoders reach their own EOS flag through the final 256-byte
        // ROM block. Reads do not acknowledge flags; register 1C masks/clears.
        for (uint8_t ch = 0; ch < 6; ++ch) {
            write(chip, 1, 0x10 + ch, 255); write(chip, 1, 0x18 + ch, 255);
            write(chip, 1, 0x20 + ch, 255); write(chip, 1, 0x28 + ch, 255);
            write(chip, 1, 8 + ch, 0xdf);
        }
        write(chip, 1, 1, 63); write(chip, 1, 0, 63);
        for (uint8_t reg : {0x12, 0x13, 0x14, 0x15, 0x19, 0x1a, 0x1b}) write(chip, 0, reg, 255);
        write(chip, 0, 0x11, 0xc0); write(chip, 0, 0x10, 0xa0);
        assert(render(chip, 6000) > 1);
        native_ym2610b_get_state(chip, &state); assert((state.status_hi & 0xbf) == 0xbf);
        for (int i = 0; i < 100; ++i) assert(native_ym2610b_read_status_hi(chip) == 0xbf);
        write(chip, 0, 0x1c, 0x81); assert(native_ym2610b_read_status_hi(chip) == 0x3e);
        native_ym2610b_get_state(chip, &state); assert(state.status_hi == 0x3e);
        write(chip, 0, 0x1c, 0); assert(native_ym2610b_read_status_hi(chip) == 0x3e);
        for (uint8_t voice = 0; voice < 16; ++voice) native_ym2610b_key_off(chip, voice);
        // Missing YM2610 FM channels remain silent; the B variant enables them.
        native_ym2610b_reset(chip); fm(chip, 0);
        const double extra = render(chip, 1000); assert(variant ? extra > 1 : extra == 0);
        native_ym2610b_clear_rom(chip, 0); native_ym2610b_clear_rom(chip, 1);
        audit = false;
        assert(allocations == 0 && frees == 0);
        native_ym2610b_destroy(chip);
    }
    std::puts("YM2610/YM2610B variant/status/EOS/Stop/reset/independent ROM self-test passed; callback allocations 0, frees 0");
    return 0;
}
struct Event { uint32_t frame = 0, a = 0, b = 0, c = 0; std::string op; std::vector<uint8_t> bytes; };
int main(int argc, char **argv) {
    try {
        if (argc == 2 && std::string(argv[1]) == "--self-test") return self_test();
        if (argc != 4) return 2;
        const uint32_t rate = std::stoul(argv[1]), frames = std::stoul(argv[2]), variant = std::stoul(argv[3]);
        if (frames > 10000000) return 2;
        std::vector<Event> events;
        std::string line;
        while (std::getline(std::cin, line)) {
            std::istringstream in(line); Event event;
            if (!(in >> event.frame >> event.op)) continue;
            if (event.op == "write") in >> event.a >> event.b >> event.c;
            else if (event.op == "clock" || event.op == "mute" || event.op == "off") in >> event.a;
            else if (event.op == "rom") {
                std::string hex; in >> event.a >> event.b >> hex; assert(hex.size() % 2 == 0);
                for (size_t i = 0; i < hex.size(); i += 2) event.bytes.push_back(std::stoul(hex.substr(i, 2), nullptr, 16));
            }
            else if (event.op == "clear") in >> event.a;
            else if (event.op != "reset" && event.op != "stop") return 2;
            events.push_back(std::move(event));
        }
        std::stable_sort(events.begin(), events.end(), [](const Event &a, const Event &b) { return a.frame < b.frame; });
        void *chip = native_ym2610b_create(rate, variant); if (!chip) return 3;
        std::vector<float> pcm(frames * 2);
        size_t next = 0;
        audit = true;
        for (uint32_t frame = 0; frame < frames; ++frame) {
            while (next < events.size() && events[next].frame == frame) {
                const auto &e = events[next++];
                if (e.op == "write") write(chip, e.a, e.b, e.c);
                else if (e.op == "clock") assert(native_ym2610b_set_clock(chip, e.a));
                else if (e.op == "mute") native_ym2610b_set_source_mute_mask(chip, e.a);
                else if (e.op == "off") native_ym2610b_key_off(chip, e.a);
                else if (e.op == "rom") assert(native_ym2610b_load_rom(chip, e.a, e.bytes.data(), e.b, e.bytes.size()));
                else if (e.op == "clear") native_ym2610b_clear_rom(chip, e.a);
                else if (e.op == "reset") native_ym2610b_reset(chip);
                else if (e.op == "stop") native_ym2610b_stop(chip);
            }
            native_ym2610b_sample(chip, &pcm[frame * 2], &pcm[frame * 2 + 1]);
        }
        audit = false;
        assert(allocations == 0 && frees == 0);
        native_ym2610b_destroy(chip);
        return std::fwrite(pcm.data(), sizeof(float), pcm.size(), stdout) == pcm.size() ? 0 : 4;
    } catch (const std::exception &error) { std::cerr << error.what() << '\n'; return 1; }
}
