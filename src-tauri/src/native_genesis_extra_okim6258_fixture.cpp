// Offline standalone OKIM6258 reference and callback-allocation fixture.
#include "native_genesis_extra_okim6258.h"
#include <array>
#include <cassert>
#include <cmath>
#include <cstdio>
#include <cstdlib>
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
static void next(void *p, float &left, float &right) { native_extra_okim6258_sample(p, &left, &right); assert(std::isfinite(left) && std::isfinite(right)); }
static int self_test() {
    static_assert(sizeof(NativeExtraOkim6258State) == 28, "Rust ABI");
    assert(!native_extra_okim6258_create(0)); assert(!native_extra_okim6258_create(384001));
    const std::array<uint8_t, 63> bytes = {{0x17, 0x82, 0x03, 0xe7, 0x49, 0xbd, 0xfa}};
    for (uint32_t hz : {100000u, 8000000u, 100000000u}) for (uint32_t rate : {44100u, 48000u, 96000u}) {
        void *a = native_extra_okim6258_create(rate), *b = native_extra_okim6258_create(rate); assert(a && b);
        audit = true;
        for (void *p : {a, b}) {
            assert(!native_extra_okim6258_play(p, 0));
            assert(!native_extra_okim6258_load_sample(p, nullptr, 1));
            assert(!native_extra_okim6258_load_sample(p, bytes.data(), 0));
            assert(!native_extra_okim6258_load_sample(p, bytes.data(), 0x1000001));
            assert(native_extra_okim6258_load_sample(p, bytes.data(), bytes.size()));
            assert(native_extra_okim6258_set_clock(p, hz));
            assert(!native_extra_okim6258_set_clock(p, 99999));
            assert(!native_extra_okim6258_write(p, 13, 0));
            assert(!native_extra_okim6258_play(p, 2));
            assert(native_extra_okim6258_play(p, 1));
        }
        double energy = 0;
        for (uint32_t i = 0; i < 7001; ++i) { float al, ar, bl, br; next(a, al, ar); next(b, bl, br); assert(al == bl && ar == br); energy += std::abs(al); }
        assert(energy > 1);
        native_extra_okim6258_stop(a);
        for (uint32_t i = 0; i < 701; ++i) { float l, r; next(a, l, r); assert(l == 0 && r == 0); next(b, l, r); }
        native_extra_okim6258_reset(a);
        NativeExtraOkim6258State before{}, after{};
        native_extra_okim6258_get_state(a, &before);
        assert(before.sample_bytes == bytes.size() && before.clock == hz && before.divider == 512 && !before.pan && !before.playing);
        for (uint32_t i = 0; i < 71; ++i) native_extra_okim6258_get_state(a, &after);
        assert(before.position == after.position && before.fed_bytes == after.fed_bytes);
        assert(native_extra_okim6258_write(a, 2, 1)); assert(native_extra_okim6258_play(a, 1));
        energy = 0;
        for (uint32_t i = 0; i < 701; ++i) { float l, r; next(a, l, r); assert(r == 0); energy += std::abs(l); }
        assert(energy > 0);
        native_extra_okim6258_set_repeat(a, 0);
        for (uint32_t i = 0; i < 200000; ++i) { float l, r; next(a, l, r); }
        native_extra_okim6258_get_state(a, &after); assert(!after.playing && !after.streaming);
        for (uint32_t i = 0; i < 71; ++i) { float l, r; next(a, l, r); assert(l == 0 && r == 0); }
        audit = false; assert(!allocations && !frees);
        native_extra_okim6258_destroy(a); native_extra_okim6258_destroy(b);
    }
    // At low output rates clocks must also be safe after reset's divider512.
    void *p = native_extra_okim6258_create(8000); assert(p);
    assert(!native_extra_okim6258_set_clock(p, 100000000));
    NativeExtraOkim6258State state{}; native_extra_okim6258_get_state(p, &state); assert(state.clock == 8000000);
    assert(native_extra_okim6258_write(p, 12, 0));
    assert(native_extra_okim6258_set_clock(p, 57344000));
    assert(!native_extra_okim6258_set_clock(p, 57344001));
    const uint32_t invalid = 57344001;
    for (uint8_t i = 0; i < 3; ++i) assert(native_extra_okim6258_write(p, 8 + i, invalid >> (i * 8)));
    native_extra_okim6258_get_state(p, &state); const uint32_t staged_clock = state.clock_buffer;
    assert(!native_extra_okim6258_write(p, 11, invalid >> 24));
    native_extra_okim6258_get_state(p, &state); assert(state.clock == 57344000 && state.clock_buffer == staged_clock);
    assert(native_extra_okim6258_load_sample(p, bytes.data(), bytes.size()));
    native_extra_okim6258_reset(p); assert(native_extra_okim6258_play(p, 1));
    audit = true;
    for (uint32_t i = 0; i < 7001; ++i) { float l, r; next(p, l, r); }
    audit = false; assert(!allocations && !frees);
    native_extra_okim6258_get_state(p, &state); assert(state.clock == 57344000 && state.divider == 512 && state.playing);
    native_extra_okim6258_destroy(p);
    std::puts("OKIM6258: 9 rate/clock isolation cases, external retained sample, low-nibble FIFO/repeat/end silence/mono pan/reset/rate guard; callback allocations/frees 0; ABI 28 bytes");
    return 0;
}
struct Event { uint32_t frame; std::string op; std::vector<uint32_t> args; std::vector<uint8_t> bytes; };
int main(int argc, char **argv) {
    if (argc == 2 && std::string(argv[1]) == "--self-test") return self_test();
    if (argc != 3) return 2;
    const uint32_t rate = std::strtoul(argv[1], nullptr, 10), frames = std::strtoul(argv[2], nullptr, 10);
    void *p = native_extra_okim6258_create(rate); if (!p || frames > 1048576) return 2;
    std::vector<Event> events;
    for (std::string line; std::getline(std::cin, line);) {
        std::istringstream in(line); Event e; if (!(in >> e.frame >> e.op)) continue;
        if (e.op == "memory") { std::string hex; in >> hex; for (size_t i = 0; i < hex.size(); i += 2) e.bytes.push_back(uint8_t(std::strtoul(hex.substr(i, 2).c_str(), nullptr, 16))); }
        else for (uint32_t arg; in >> arg;) e.args.push_back(arg);
        events.push_back(e);
    }
    std::vector<float> output; output.reserve(size_t(frames) * 12); size_t index = 0;
    audit = true;
    for (uint32_t frame = 0; frame < frames; ++frame) {
        while (index < events.size() && events[index].frame == frame) {
            const auto &e = events[index++]; const auto &a = e.args;
            if (e.op == "memory") assert(native_extra_okim6258_load_sample(p, e.bytes.data(), e.bytes.size()));
            else if (e.op == "write") assert(native_extra_okim6258_write(p, a[0], a[1]));
            else if (e.op == "clock") assert(native_extra_okim6258_set_clock(p, a[0]));
            else if (e.op == "play") assert(native_extra_okim6258_play(p, a[0]));
            else if (e.op == "repeat") native_extra_okim6258_set_repeat(p, a[0]);
            else if (e.op == "stop") native_extra_okim6258_stop(p);
            else if (e.op == "reset") native_extra_okim6258_reset(p);
            else return 3;
        }
        float l, r; next(p, l, r); output.push_back(l); output.push_back(r);
        NativeExtraOkim6258State s{}; native_extra_okim6258_get_state(p, &s);
        for (uint32_t v : {s.clock, s.clock_buffer, s.divider, s.sample_bytes, s.position, s.fed_bytes,
            uint32_t(s.pan), uint32_t(s.playing), uint32_t(s.streaming), uint32_t(s.repeat)}) output.push_back(float(v));
    }
    audit = false; assert(!allocations && !frees); native_extra_okim6258_destroy(p);
    return std::fwrite(output.data(), sizeof(float), output.size(), stdout) == output.size() ? 0 : 4;
}
