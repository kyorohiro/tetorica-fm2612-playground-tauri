// Offline synthesis/state/allocation fixture. Never opens an audio device.
#include "native_genesis_msx.h"
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
static double render(void *chip, uint32_t frames, uint8_t which = 2) {
    double energy = 0; float pl, pr, fl, fr;
    for (uint32_t i = 0; i < frames; ++i) {
        native_msx_sample(chip, &pl, &pr, &fl, &fr); assert(pl == pr && fl == fr);
        energy += (which != 1 ? std::abs(pl) + std::abs(pr) : 0) + (which != 0 ? std::abs(fl) + std::abs(fr) : 0);
    }
    return energy;
}
static void voice(void *chip, uint8_t ch) {
    native_msx_write(chip, 1, 0x30 + ch, 0x13);
    native_msx_write(chip, 1, 0x10 + ch, 0xc0);
    native_msx_write(chip, 1, 0x20 + ch, 0x18);
}
static int self_test() {
    assert(!native_msx_create(0)); assert(!native_msx_create(384001));
    void *chip = native_msx_create(48000); assert(chip); NativeMsxState s{};
    static_assert(sizeof(NativeMsxState) == 104, "Rust FFI layout");
    audit = true;
    native_msx_get_state(chip, &s);
    assert(s.ay_clock == 1789773 && s.opll_clock == 3579545);
    for (uint8_t ch = 0; ch < 3; ++ch) assert(s.ay_registers[8 + ch] == 0 && s.levels[ch] == 9);
    assert(render(chip, 200) == 0);
    native_msx_write(chip, 0, 0, 127); native_msx_write(chip, 0, 1, 1);
    native_msx_write(chip, 0, 6, 7); native_msx_write(chip, 0, 7, 0x38);
    native_msx_write(chip, 0, 8, 11); native_msx_write(chip, 0, 9, 16);
    native_msx_write(chip, 0, 11, 37); native_msx_write(chip, 0, 12, 1); native_msx_write(chip, 0, 13, 10);
    native_msx_write(chip, 0, 14, 0xaa); native_msx_write(chip, 0, 15, 0x55);
    voice(chip, 0); voice(chip, 8);
    assert(render(chip, 1000, 0) > 1); assert(render(chip, 1000, 1) > 1);
    assert(native_msx_set_clock(chip, 0, 100000)); assert(native_msx_set_clock(chip, 1, 100000));
    assert(!native_msx_set_clock(chip, 0, 99999)); assert(!native_msx_set_clock(chip, 2, 3579545));
    native_msx_stop(chip, 2); native_msx_get_state(chip, &s);
    assert(s.ay_clock == 100000 && s.opll_clock == 100000 && s.ay_registers[0] == 127 && s.ay_registers[1] == 1);
    assert(s.ay_registers[6] == 7 && s.ay_registers[7] == 0x38 && s.ay_registers[11] == 37 && s.ay_registers[12] == 1 && s.ay_registers[13] == 10);
    assert(s.ay_registers[14] == 0xaa && s.ay_registers[15] == 0x55 && s.levels[0] == 11 && s.levels[1] == 16);
    assert(s.ay_registers[8] == 0 && s.ay_registers[9] == 0 && s.registers[0x30] == 0x13 && !(s.registers[0x20] & 16));
    assert(render(chip, 2000) == 0);
    native_msx_audition_key_on(chip, 0); assert(render(chip, 400, 0) > .1);
    native_msx_set_retained_level(chip, 0, 0); native_msx_key_off(chip, 0); native_msx_audition_key_on(chip, 0);
    native_msx_get_state(chip, &s); assert(s.levels[0] == 0 && s.ay_registers[8] == 0);
    render(chip, 30); assert(render(chip, 400, 0) == 0);
    native_msx_set_retained_level(chip, 0, 17); native_msx_audition_key_on(chip, 0); assert(render(chip, 2000, 0) > .01);
    native_msx_set_clock(chip, 1, 3579545); native_msx_audition_key_on(chip, 3); assert(render(chip, 1000, 1) > .1);
    native_msx_key_off(chip, 3); render(chip, 10); assert(render(chip, 500, 1) == 0);
    native_msx_audition_key_on(chip, 3); native_msx_key_off(chip, 3); assert(render(chip, 1000, 1) == 0);
    native_msx_audition_key_on(chip, 3); native_msx_stop(chip, 1); assert(render(chip, 1000, 1) == 0);
    native_msx_set_mute_mask(chip, 0, 7); native_msx_set_mute_mask(chip, 1, 511); voice(chip, 8); assert(render(chip, 500) == 0);
    native_msx_set_mute_mask(chip, 0, 0); native_msx_set_mute_mask(chip, 1, 0); assert(render(chip, 500) > .1);
    native_msx_reset(chip, 1); assert(render(chip, 100, 0) > .01); assert(render(chip, 100, 1) == 0);
    native_msx_reset(chip, 0); voice(chip, 8); assert(render(chip, 1000, 1) > .1); assert(render(chip, 100, 0) == 0);
    native_msx_write(chip, 1, 0x36, 3); native_msx_write(chip, 1, 0x37, 0x44);
    native_msx_write(chip, 1, 0x38, 0x55); native_msx_write(chip, 1, 0x0e, 63); assert(render(chip, 2000, 1) > 1);
    native_msx_key_off(chip, 13); native_msx_get_state(chip, &s);
    assert(!(s.registers[0x0e] & 8) && (s.registers[0x0e] & 1)); assert(s.registers[0x37] == 0x44);
    native_msx_audition_key_on(chip, 13); assert(render(chip, 1000, 1) > .01);
    native_msx_audition_key_on(chip, 12); native_msx_reset(chip, 1); assert(render(chip, 1000, 1) == 0);
    for (int i = 0; i < 100; ++i) { native_msx_get_state(chip, &s); assert(s.status == native_msx_read_status(chip)); assert(s.irq == native_msx_get_irq(chip)); native_msx_read(chip, 0, 14); }
    native_msx_reset(chip, 2); native_msx_get_state(chip, &s);
    assert(s.ay_clock == 100000 && s.opll_clock == 3579545 && s.ay_registers[0] == 0 && s.levels[0] == 9 && s.registers[0x37] == 0);
    audit = false; assert(allocations == 0 && frees == 0); native_msx_destroy(chip);
    std::puts("MSX mono AY/OPLL, independent resets/mutes, source clocks, AY I/O and retained envelope levels, shared rhythm, deferred retrigger and cancellation passed; callback allocations 0, frees 0"); return 0;
}
struct Event { uint32_t frame = 0, a = 0, b = 0, c = 0; std::string op; };
int main(int argc, char **argv) {
    try {
        if (argc == 2 && std::string(argv[1]) == "--self-test") return self_test();
        if (argc != 3) return 2;
        const uint32_t rate = std::stoul(argv[1]), frames = std::stoul(argv[2]); if (frames > 10000000) return 2;
        std::vector<Event> events; std::string line;
        while (std::getline(std::cin, line)) {
            std::istringstream in(line); Event e; if (!(in >> e.frame >> e.op)) continue;
            if (e.op == "write") in >> e.a >> e.b >> e.c;
            else if (e.op == "clock" || e.op == "mute" || e.op == "retained") in >> e.a >> e.b;
            else if (e.op == "reset" || e.op == "stop" || e.op == "off" || e.op == "audition") in >> e.a;
            else return 2;
            events.push_back(std::move(e));
        }
        std::stable_sort(events.begin(), events.end(), [](const Event &a, const Event &b) { return a.frame < b.frame; });
        void *chip = native_msx_create(rate); if (!chip) return 3;
        std::vector<float> pcm(frames * 4); size_t next = 0; audit = true;
        for (uint32_t frame = 0; frame < frames; ++frame) {
            while (next < events.size() && events[next].frame == frame) {
                const auto &e = events[next++];
                if (e.op == "write") native_msx_write(chip, e.a, e.b, e.c);
                else if (e.op == "clock") assert(native_msx_set_clock(chip, e.a, e.b));
                else if (e.op == "mute") native_msx_set_mute_mask(chip, e.a, e.b);
                else if (e.op == "audition") native_msx_audition_key_on(chip, e.a);
                else if (e.op == "retained") native_msx_set_retained_level(chip, e.a, e.b);
                else if (e.op == "reset") native_msx_reset(chip, e.a);
                else if (e.op == "stop") native_msx_stop(chip, e.a);
                else if (e.op == "off") native_msx_key_off(chip, e.a);
            }
            native_msx_sample(chip, &pcm[frame*4], &pcm[frame*4+1], &pcm[frame*4+2], &pcm[frame*4+3]);
        }
        audit = false; assert(allocations == 0 && frees == 0); native_msx_destroy(chip);
        return std::fwrite(pcm.data(), sizeof(float), pcm.size(), stdout) == pcm.size() ? 0 : 4;
    } catch (const std::exception &e) { std::cerr << e.what() << '\n'; return 1; }
}
