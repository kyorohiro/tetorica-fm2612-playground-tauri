// Offline PCM/state fixture, never opens a playback device.
#include "native_genesis_ym2203.h"
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
static void write(void *p, uint8_t r, uint8_t v) { native_ym2203_write(p, 0, r, v); }
static void voice(void *p, uint8_t ch) {
    for (uint8_t op = 0; op < 4; ++op) {
        const uint8_t off = ch + op * 4;
        write(p, 0x30 + off, 1); write(p, 0x40 + off, 12 + op * 5);
        write(p, 0x50 + off, 31); write(p, 0x60 + off, 3); write(p, 0x70 + off, 0); write(p, 0x80 + off, 0x2f);
    }
    write(p, 0xb0 + ch, 7); write(p, 0xa4 + ch, 0x24); write(p, 0xa0 + ch, 0x81); write(p, 0x28, 0xf0 | ch);
}
static double render(void *p, uint32_t n) {
    double energy = 0; float l, r;
    for (uint32_t i = 0; i < n; ++i) { native_ym2203_sample(p, &l, &r); assert(std::isfinite(l) && l == r); energy += std::abs(l); }
    return energy;
}
static int self_test() {
    static_assert(sizeof(NativeYm2203State) == 280, "Rust FFI layout");
    assert(!native_ym2203_create(0) && !native_ym2203_create(384001));
    void *p = native_ym2203_create(48000); assert(p); NativeYm2203State s{}; audit = true;
    native_ym2203_get_state(p, &s); assert(s.clock == 4000000 && s.prescaler == 6 && render(p, 200) == 0);
    for (uint8_t ch = 0; ch < 3; ++ch) voice(p, ch);
    assert(render(p, 4000) > 1);
    write(p, 0, 127); write(p, 1, 1); write(p, 6, 13); write(p, 7, 0x38); write(p, 8, 11);
    write(p, 9, 16); write(p, 11, 37); write(p, 12, 1); write(p, 13, 10); write(p, 14, 0xaa); write(p, 15, 0x55);
    assert(render(p, 1000) > .1);
    write(p, 0x2e, 0); native_ym2203_get_state(p, &s); assert(s.prescaler == 3);
    write(p, 0x2f, 0); write(p, 0x2e, 0); native_ym2203_get_state(p, &s); assert(s.prescaler == 2);
    write(p, 0x2d, 0); native_ym2203_get_state(p, &s); assert(s.prescaler == 6);
    write(p, 0xa4, 0x21); write(p, 0xa0, 0x11); write(p, 0xa4, 0x2a); write(p, 0xa1, 0x22);
    native_ym2203_get_state(p, &s); assert(s.registers[0xa4] == 0x21 && s.registers[0xa5] == 0x2a);
    assert(native_ym2203_set_clock(p, 100000)); assert(!native_ym2203_set_clock(p, 99999));
    write(p, 0x27, 0x40); native_ym2203_stop(p); native_ym2203_get_state(p, &s);
    assert(s.clock == 100000 && s.registers[0x27] == 0x40 && s.registers[0xa4] == 0x21 && s.registers[0xa5] == 0x2a);
    assert(s.registers[0x40] == 12 && s.ssg_levels[0] == 11 && s.ssg_levels[1] == 16);
    assert(s.registers[6] == 13 && s.registers[7] == 0x38 && s.registers[13] == 10 && s.registers[14] == 0xaa && s.registers[15] == 0x55);
    assert(render(p, 2000) == 0);
    native_ym2203_audition_key_on(p, 3); assert(render(p, 2000) > .01);
    native_ym2203_key_off(p, 3); native_ym2203_set_retained_ssg_level(p, 0, 0); native_ym2203_audition_key_on(p, 3);
    render(p, 200); assert(render(p, 1000) == 0); native_ym2203_get_state(p, &s); assert(s.ssg_levels[0] == 0 && s.registers[8] == 0);
    native_ym2203_set_clock(p, 4000000); native_ym2203_audition_key_on(p, 0); assert(render(p, 1000) > .01);
    native_ym2203_key_off(p, 0); render(p, 40); assert(render(p, 200) == 0);
    native_ym2203_audition_key_on(p, 0); native_ym2203_key_off(p, 0); assert(render(p, 2000) == 0);
    native_ym2203_audition_key_on(p, 0); native_ym2203_stop(p); assert(render(p, 1000) == 0);
    voice(p, 0); native_ym2203_set_source_mute_mask(p, 3); assert(render(p, 1000) == 0);
    native_ym2203_set_source_mute_mask(p, 0); assert(render(p, 1000) > .1);
    native_ym2203_set_mute_mask(p, 7); render(p, 40); assert(render(p, 200) == 0); native_ym2203_set_mute_mask(p, 0);
    write(p, 0x24, 255); write(p, 0x25, 3); write(p, 0x26, 255); write(p, 0x27, 15); render(p, 1000);
    assert((native_ym2203_read_status(p) & 3) == 3 && native_ym2203_get_irq(p));
    write(p, 0x27, 0x30); assert(!native_ym2203_get_irq(p));
    for (int i = 0; i < 100; ++i) { native_ym2203_get_state(p, &s); assert(s.status == native_ym2203_read_status(p)); for (uint8_t port = 0; port < 4; ++port) native_ym2203_read(p, port); }
    native_ym2203_stop(p); native_ym2203_reset(p); native_ym2203_get_state(p, &s); assert(s.clock == 4000000 && !s.irq && s.registers[8] == 0 && s.ssg_levels[0] == 0);
    render(p, 40); assert(render(p, 200) == 0);
    audit = false; assert(allocations == 0 && frees == 0); native_ym2203_destroy(p);
    std::puts("Selected mono YM2203 only; FM/SSG six voices, source clocks/mutes, prescalers, applied frequency pairs, retained TL/SSG levels and Stop, deferred retrigger/cancellation, timers/status/IRQ passed; callback allocations 0, frees 0"); return 0;
}
struct Event { uint32_t frame = 0, a = 0, b = 0; std::string op; };
int main(int argc, char **argv) {
    try {
        if (argc == 2 && std::string(argv[1]) == "--self-test") return self_test();
        if (argc != 3) return 2;
        const uint32_t rate = std::stoul(argv[1]), frames = std::stoul(argv[2]); if (frames > 10000000) return 2;
        std::vector<Event> events; std::string line;
        while (std::getline(std::cin, line)) {
            std::istringstream in(line); Event e; if (!(in >> e.frame >> e.op)) continue;
            if (e.op == "write" || e.op == "level" || e.op == "audition-mask") in >> e.a >> e.b;
            else if (e.op == "clock" || e.op == "source" || e.op == "mute" || e.op == "off" || e.op == "audition") in >> e.a;
            else if (e.op != "reset" && e.op != "stop") return 2;
            events.push_back(std::move(e));
        }
        std::stable_sort(events.begin(), events.end(), [](const Event &a, const Event &b) { return a.frame < b.frame; });
        void *p = native_ym2203_create(rate); if (!p) return 3;
        constexpr uint32_t PLANES = 8; std::vector<float> pcm(frames * PLANES); size_t next = 0; audit = true;
        for (uint32_t frame = 0; frame < frames; ++frame) {
            while (next < events.size() && events[next].frame == frame) {
                const auto &e = events[next++];
                if (e.op == "write") write(p, e.a, e.b);
                else if (e.op == "level") native_ym2203_set_retained_ssg_level(p, e.a, e.b);
                else if (e.op == "clock") assert(native_ym2203_set_clock(p, e.a));
                else if (e.op == "source") native_ym2203_set_source_mute_mask(p, e.a);
                else if (e.op == "mute") native_ym2203_set_mute_mask(p, e.a);
                else if (e.op == "off") native_ym2203_key_off(p, e.a);
                else if (e.op == "audition") native_ym2203_audition_key_on(p, e.a);
                else if (e.op == "audition-mask") native_ym2203_audition_fm_key_on(p, e.a, e.b);
                else if (e.op == "reset") native_ym2203_reset(p);
                else if (e.op == "stop") native_ym2203_stop(p);
            }
            native_ym2203_sample(p, &pcm[frame*PLANES], &pcm[frame*PLANES+1]); NativeYm2203State s{}; native_ym2203_get_state(p, &s);
            pcm[frame*PLANES+2] = s.status; pcm[frame*PLANES+3] = s.irq;
            for (uint8_t port = 0; port < 4; ++port) pcm[frame*PLANES+4+port] = native_ym2203_read(p, port);
        }
        audit = false; assert(allocations == 0 && frees == 0); native_ym2203_destroy(p);
        return std::fwrite(pcm.data(), sizeof(float), pcm.size(), stdout) == pcm.size() ? 0 : 4;
    } catch (const std::exception &e) { std::cerr << e.what() << '\n'; return 1; }
}
