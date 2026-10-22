// Offline state/PCM fixture: never opens a playback device.
#include "native_genesis_opl.h"
#include <algorithm>
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
static double render(void *chip, uint32_t frames, bool mono = false) {
    double energy = 0; float l, r;
    for (uint32_t i = 0; i < frames; ++i) { native_opl_sample(chip, &l, &r); assert(std::isfinite(l) && std::isfinite(r)); if (mono) assert(l == r); energy += std::abs(l) + std::abs(r); }
    return energy;
}
static void voice(void *chip, uint8_t ch) {
    const uint8_t slots[9] = {0,1,2,8,9,10,16,17,18};
    const uint16_t bank = ch >= 9 ? 0x100 : 0;
    for (uint8_t s : {slots[ch % 9], uint8_t(slots[ch % 9] + 3)}) {
        native_opl_write(chip, bank + 0x20 + s, 0x21); native_opl_write(chip, bank + 0x40 + s, 9);
        native_opl_write(chip, bank + 0x60 + s, 0xf4); native_opl_write(chip, bank + 0x80 + s, 0x2c);
    }
    native_opl_write(chip, bank + 0xc0 + ch % 9, 0x31);
    native_opl_write(chip, bank + 0xa0 + ch % 9, 0x81);
    native_opl_write(chip, bank + 0xb0 + ch % 9, 0x31);
}
static int self_test() {
    static_assert(sizeof(NativeOplState) == 524, "Rust FFI layout");
    assert(!native_opl_create(0, 0) && !native_opl_create(384001, 0) && !native_opl_create(48000, 3));
    for (uint8_t kind = 0; kind < 3; ++kind) {
        void *chip = native_opl_create(48000, kind); assert(chip); NativeOplState s{};
        audit = true; native_opl_get_state(chip, &s);
        assert(s.kind == kind && s.channels == (kind == 2 ? 18 : 9) && s.clock == (kind == 2 ? 14318180u : 3579545u));
        assert(render(chip, 300, kind != 2) == 0);
        if (kind == 1) native_opl_write(chip, 1, 32);
        if (kind == 2) native_opl_write(chip, 0x105, 1);
        voice(chip, 0); voice(chip, s.channels - 1); assert(render(chip, 4000, kind != 2) > .1);
        assert(native_opl_set_clock(chip, 100000)); assert(!native_opl_set_clock(chip, 99999)); assert(!native_opl_set_clock(chip, 100000001));
        native_opl_stop(chip); native_opl_get_state(chip, &s);
        assert(s.clock == 100000 && s.registers[0x40] == 9 && s.registers[0xc0] == 0x31 && !(s.registers[0xb0] & 32));
        assert(render(chip, 1000, kind != 2) == 0);
        native_opl_set_clock(chip, kind == 2 ? 14318180 : 3579545);
        native_opl_audition_key_on(chip, 0); assert(render(chip, 4000, kind != 2) > .1);
        native_opl_key_off(chip, 0); render(chip, 20, kind != 2); assert(render(chip, 300, kind != 2) == 0);
        native_opl_audition_key_on(chip, 0); native_opl_key_off(chip, 0); assert(render(chip, 2000, kind != 2) == 0);
        native_opl_audition_key_on(chip, 0); native_opl_stop(chip); assert(render(chip, 2000, kind != 2) == 0);
        voice(chip, 0); native_opl_set_mute_mask(chip, (1u << s.channels) - 1); assert(render(chip, 1000, kind != 2) == 0);
        native_opl_set_mute_mask(chip, 0); assert(render(chip, 1000, kind != 2) > .01);
        if (kind == 2) {
            native_opl_write(chip, 0x104, 63); voice(chip, 3); native_opl_get_state(chip, &s);
            native_opl_key_off(chip, 0); native_opl_get_state(chip, &s); assert(!(s.registers[0xb3] & 32));
            native_opl_audition_key_on(chip, 0); render(chip, 100); native_opl_stop(chip); native_opl_get_state(chip, &s);
            assert(s.registers[0x104] == 63 && s.registers[0x105] == 1 && s.registers[0xc0] == 0x31);
        }
        for (uint8_t ch = 6; ch < 9; ++ch) voice(chip, ch);
        native_opl_write(chip, 0xbd, 255); render(chip, 500); native_opl_key_off(chip, 19); native_opl_get_state(chip, &s);
        assert((s.registers[0xbd] & 1) && !(s.registers[0xbd] & 8));
        native_opl_audition_key_on(chip, 19); render(chip, 40); native_opl_get_state(chip, &s); assert(s.registers[0xbd] & 8);
        native_opl_stop(chip); native_opl_get_state(chip, &s); assert(s.registers[0xbd] == 224 && render(chip, 500) == 0);
        native_opl_write(chip, 2, 255); native_opl_write(chip, 4, 1); render(chip, 1000);
        assert((native_opl_read_status(chip) & 0xc0) == 0xc0 && native_opl_get_irq(chip));
        for (int i = 0; i < 100; ++i) { native_opl_get_state(chip, &s); assert(s.status == native_opl_read_status(chip)); assert(s.irq == native_opl_get_irq(chip)); assert(s.status == native_opl_read(chip, 0)); }
        native_opl_write(chip, 4, 128); assert(!native_opl_get_irq(chip));
        native_opl_audition_key_on(chip, 0); native_opl_reset(chip); native_opl_get_state(chip, &s);
        assert(s.clock == (kind == 2 ? 14318180u : 3579545u) && s.registers[0xc0] == 0 && !s.irq && render(chip, 500) == 0);
        audit = false; assert(allocations == 0 && frees == 0); native_opl_destroy(chip);
    }
    std::puts("Selected OPL/OPL2/OPL3 only; mono/stereo planes, retained clocks/TL/pairs/rhythm/routing, deferred retrigger and cancellation, timers/status/IRQ passed; callback allocations 0, frees 0"); return 0;
}
struct Event { uint32_t frame = 0, a = 0, b = 0; std::string op; };
int main(int argc, char **argv) {
    try {
        if (argc == 2 && std::string(argv[1]) == "--self-test") return self_test();
        if (argc != 4) return 2;
        const uint32_t kind = std::stoul(argv[1]), rate = std::stoul(argv[2]), frames = std::stoul(argv[3]); if (frames > 10000000 || kind > 2) return 2;
        std::vector<Event> events; std::string line;
        while (std::getline(std::cin, line)) {
            std::istringstream in(line); Event e; if (!(in >> e.frame >> e.op)) continue;
            if (e.op == "write") in >> e.a >> e.b;
            else if (e.op == "clock" || e.op == "mute" || e.op == "off" || e.op == "audition") in >> e.a;
            else if (e.op != "reset" && e.op != "stop") return 2;
            events.push_back(std::move(e));
        }
        std::stable_sort(events.begin(), events.end(), [](const Event &a, const Event &b) { return a.frame < b.frame; });
        void *chip = native_opl_create(rate, kind); if (!chip) return 3;
        std::vector<float> pcm(frames * 2); size_t next = 0; audit = true;
        for (uint32_t frame = 0; frame < frames; ++frame) {
            while (next < events.size() && events[next].frame == frame) {
                const auto &e = events[next++];
                if (e.op == "write") native_opl_write(chip, e.a, e.b);
                else if (e.op == "clock") assert(native_opl_set_clock(chip, e.a));
                else if (e.op == "mute") native_opl_set_mute_mask(chip, e.a);
                else if (e.op == "off") native_opl_key_off(chip, e.a);
                else if (e.op == "audition") native_opl_audition_key_on(chip, e.a);
                else if (e.op == "reset") native_opl_reset(chip);
                else if (e.op == "stop") native_opl_stop(chip);
            }
            native_opl_sample(chip, &pcm[frame * 2], &pcm[frame * 2 + 1]);
        }
        audit = false; assert(allocations == 0 && frees == 0); native_opl_destroy(chip);
        return std::fwrite(pcm.data(), sizeof(float), pcm.size(), stdout) == pcm.size() ? 0 : 4;
    } catch (const std::exception &e) { std::cerr << e.what() << '\n'; return 1; }
}
