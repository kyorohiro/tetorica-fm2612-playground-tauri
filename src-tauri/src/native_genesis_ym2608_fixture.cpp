// Offline PCM/state/allocation fixture; never opens an audio device.
#include "native_genesis_ym2608.h"
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

static void write(void *chip, uint8_t port, uint8_t reg, uint8_t value) { native_ym2608_write(chip, port, reg, value); }
static int self_test() {
    assert(!native_ym2608_create(0));
    void *chip = native_ym2608_create(48000); assert(chip);
    uint8_t bytes[32]; for (uint8_t i = 0; i < 32; ++i) bytes[i] = i * 7;
    uint8_t readback[32] = {};
    NativeYm2608State state{};
    audit = true;
    assert(native_ym2608_load_rom(chip, bytes, 0x1fe0, 32));
    assert(!native_ym2608_load_rom(chip, bytes, 0x1ff0, 32));
    assert(native_ym2608_load_memory(chip, bytes, 0x1fffe0, 32));
    assert(!native_ym2608_load_memory(chip, bytes, 0x1ffff0, 32));
    assert(!native_ym2608_load_memory(chip, nullptr, 0, 1));
    assert(native_ym2608_set_clock(chip, 100000));
    assert(native_ym2608_set_clock(chip, 100000000));
    assert(!native_ym2608_set_clock(chip, 1));
    assert(native_ym2608_set_clock(chip, 7987200));
    native_ym2608_set_source_mute_mask(chip, 5);
    write(chip, 0, 0x29, 0x9f); write(chip, 0, 0xb0, 7);
    write(chip, 1, 0x10, 5); write(chip, 1, 0x10, 0x80);
    for (uint8_t slot : {0, 4, 8, 12}) {
        write(chip, 0, 0x30 + slot, 1);
        write(chip, 0, 0x40 + slot, slot == 12 ? 24 : 127);
        write(chip, 0, 0x50 + slot, 31);
        write(chip, 0, 0x80 + slot, 1);
    }
    write(chip, 0, 0xa4, 0x22); write(chip, 0, 0xa0, 0x69); write(chip, 0, 0x28, 0xf0);
    write(chip, 0, 0xa4, 0x22); write(chip, 1, 0xa1, 0x77);
    native_ym2608_get_state(chip, &state);
    assert(state.registers[0x1a5] == 0x22 && state.registers[0x1a1] == 0x77);
    write(chip, 0, 0, 0x34); write(chip, 0, 1, 2); write(chip, 0, 7, 0x3e); write(chip, 0, 8, 12);
    write(chip, 1, 2, 0x12); write(chip, 1, 9, 0x34); write(chip, 0, 0x18, 0xde);
    float left = 0, right = 0; double sounding_energy = 0;
    for (uint32_t i = 0; i < 4000; ++i) { native_ym2608_sample(chip, &left, &right); sounding_energy += std::abs(left) + std::abs(right); }
    assert(sounding_energy > 1);
    native_ym2608_stop(chip);
    native_ym2608_get_state(chip, &state);
    assert(state.clock == 7987200 && state.source_mute_mask == 5);
    assert(state.registers[0xb0] == 7 && state.registers[0x29] == 0x9f);
    assert(state.registers[0x110] == 5); // Flag-clear retained the IRQ mask.
    assert(state.registers[0x1a5] == 0x22 && state.registers[0x1a1] == 0x77);
    assert(state.registers[0] == 0x34 && state.registers[1] == 2 && state.registers[8] == 0);
    assert(state.registers[0x102] == 0x12 && state.registers[0x109] == 0x34 && state.registers[0x18] == 0xde);
    for (uint32_t i = 0; i < 256; ++i) native_ym2608_sample(chip, &left, &right);
    assert(left == 0 && right == 0);
    write(chip, 0, 0x28, 0xf0); sounding_energy = 0;
    for (uint32_t i = 0; i < 256; ++i) { native_ym2608_sample(chip, &left, &right); sounding_energy += std::abs(left) + std::abs(right); }
    assert(sounding_energy > 0.1); // Stop kept the authored FM instrument/pitch.
    native_ym2608_reset(chip);
    native_ym2608_get_state(chip, &state);
    assert(state.clock == 7987200 && state.source_mute_mask == 5 && state.registers[0xb0] == 0);
    assert(native_ym2608_copy_memory(chip, readback, 0x1fffe0, 32));
    assert(std::memcmp(bytes, readback, 32) == 0);
    write(chip, 0, 0, 0x55);
    assert(native_ym2608_read(chip, 1) == 0x55);
    const uint8_t irq = native_ym2608_get_irq(chip);
    for (int i = 0; i < 100; ++i) native_ym2608_get_state(chip, &state);
    assert(native_ym2608_get_irq(chip) == irq);
    write(chip, 0, 0x29, 0x9f); write(chip, 1, 0x10, 0x1c);
    write(chip, 0, 0x24, 255); write(chip, 0, 0x25, 3); write(chip, 0, 0x27, 5);
    for (uint32_t i = 0; i < 128; ++i) native_ym2608_sample(chip, &left, &right);
    native_ym2608_get_state(chip, &state);
    assert((state.status & 1) && (state.status_hi & 1) && state.irq);
    assert(native_ym2608_read_status(chip) == state.status);
    assert(native_ym2608_read_status_hi(chip) == state.status_hi);
    write(chip, 0, 0x27, 0x10);
    native_ym2608_get_state(chip, &state);
    assert(!(state.status & 1) && !state.irq);
    // Hardware-style CPU memory upload/download uses the very same reserved
    // RAM as load_memory; inspection must not consume these two dummy reads.
    write(chip, 1, 1, 2); write(chip, 1, 2, 0x10); write(chip, 1, 3, 0);
    write(chip, 1, 4, 0x11); write(chip, 1, 5, 0); write(chip, 1, 0, 0x60);
    for (auto value : bytes) write(chip, 1, 8, value);
    assert(native_ym2608_copy_memory(chip, readback, 0x200, 32));
    assert(std::memcmp(bytes, readback, 32) == 0);
    write(chip, 1, 0, 0x20); write(chip, 1, 8, 0);
    native_ym2608_get_state(chip, &state);
    assert(native_ym2608_read(chip, 3) == 0);
    native_ym2608_get_state(chip, &state);
    assert(native_ym2608_read(chip, 3) == 0);
    for (auto value : bytes) assert(native_ym2608_read(chip, 3) == value);
    for (uint8_t voice = 0; voice < 16; ++voice) native_ym2608_key_off(chip, voice);
    native_ym2608_clear_memory(chip);
    assert(native_ym2608_copy_memory(chip, readback, 0x1fffe0, 32));
    for (auto value : readback) assert(value == 0);
    audit = false;
    assert(allocations == 0 && frees == 0);
    native_ym2608_destroy(chip);
    std::puts("YM2608 state/Stop/reset/memory/clock self-test passed; callback allocations 0, frees 0");
    return 0;
}
struct Event { uint32_t frame = 0, a = 0, b = 0, c = 0; std::string op; std::vector<uint8_t> bytes; };
int main(int argc, char **argv) {
    try {
        if (argc == 2 && std::string(argv[1]) == "--self-test") return self_test();
        if (argc != 3) return 2;
        const uint32_t rate = std::stoul(argv[1]), frames = std::stoul(argv[2]);
        if (frames > 10000000) return 2;
        std::vector<Event> events;
        std::string line;
        while (std::getline(std::cin, line)) {
            std::istringstream in(line); Event event;
            if (!(in >> event.frame >> event.op)) continue;
            if (event.op == "write") in >> event.a >> event.b >> event.c;
            else if (event.op == "clock" || event.op == "mute" || event.op == "off") in >> event.a;
            else if (event.op == "rom" || event.op == "memory") {
                std::string hex; in >> event.a >> hex; assert(hex.size() % 2 == 0);
                for (size_t i = 0; i < hex.size(); i += 2) event.bytes.push_back(std::stoul(hex.substr(i, 2), nullptr, 16));
            }
            else if (event.op != "clear" && event.op != "reset" && event.op != "stop") return 2;
            events.push_back(std::move(event));
        }
        std::stable_sort(events.begin(), events.end(), [](const Event &a, const Event &b) { return a.frame < b.frame; });
        void *chip = native_ym2608_create(rate); if (!chip) return 3;
        std::vector<float> pcm(frames * 2);
        size_t next = 0;
        audit = true;
        for (uint32_t frame = 0; frame < frames; ++frame) {
            while (next < events.size() && events[next].frame == frame) {
                const auto &e = events[next++];
                if (e.op == "write") write(chip, e.a, e.b, e.c);
                else if (e.op == "clock") assert(native_ym2608_set_clock(chip, e.a));
                else if (e.op == "mute") native_ym2608_set_source_mute_mask(chip, e.a);
                else if (e.op == "off") native_ym2608_key_off(chip, e.a);
                else if (e.op == "rom") assert(native_ym2608_load_rom(chip, e.bytes.data(), e.a, e.bytes.size()));
                else if (e.op == "memory") assert(native_ym2608_load_memory(chip, e.bytes.data(), e.a, e.bytes.size()));
                else if (e.op == "clear") native_ym2608_clear_memory(chip);
                else if (e.op == "reset") native_ym2608_reset(chip);
                else if (e.op == "stop") native_ym2608_stop(chip);
            }
            native_ym2608_sample(chip, &pcm[frame * 2], &pcm[frame * 2 + 1]);
        }
        audit = false;
        assert(allocations == 0 && frees == 0);
        native_ym2608_destroy(chip);
        return std::fwrite(pcm.data(), sizeof(float), pcm.size(), stdout) == pcm.size() ? 0 : 4;
    } catch (const std::exception &error) { std::cerr << error.what() << '\n'; return 1; }
}
