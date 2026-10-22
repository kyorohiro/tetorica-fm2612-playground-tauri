// Offline synthesis/state/allocation fixture. Never opens an audio device.
#include "native_genesis_master_system.h"
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
        native_master_system_sample(chip, &pl, &pr, &fl, &fr); assert(pl == pr && fl == fr);
        energy += (which != 1 ? std::abs(pl) + std::abs(pr) : 0) + (which != 0 ? std::abs(fl) + std::abs(fr) : 0);
    }
    return energy;
}
static void voice(void *chip, uint8_t ch) {
    native_master_system_write(chip, 1, 0x30 + ch, 0x13);
    native_master_system_write(chip, 1, 0x10 + ch, 0xc0);
    native_master_system_write(chip, 1, 0x20 + ch, 0x18);
}
static int self_test() {
    assert(!native_master_system_create(0)); assert(!native_master_system_create(384001));
    void *chip = native_master_system_create(48000); assert(chip); NativeMasterSystemState s{};
    audit = true;
    native_master_system_get_state(chip, &s);
    assert(s.psg_clock == 3579545 && s.opll_clock == 3579545 && s.noise_mode == 0);
    for (uint8_t ch = 0; ch < 4; ++ch) assert(s.attenuation[ch] == 15 && s.levels[ch] == 0);
    assert(render(chip, 200) == 0);
    native_master_system_write(chip, 0, 0, 0x85); native_master_system_write(chip, 0, 0, 0x12);
    native_master_system_write(chip, 0, 0, 0x93); native_master_system_write(chip, 0, 0, 0xe7);
    native_master_system_write(chip, 0, 0, 0xf5); voice(chip, 0); voice(chip, 8);
    assert(render(chip, 1000, 0) > 1); assert(render(chip, 1000, 1) > 1);
    assert(native_master_system_set_clock(chip, 0, 100000)); assert(native_master_system_set_clock(chip, 1, 100000));
    assert(!native_master_system_set_clock(chip, 0, 99999)); assert(!native_master_system_set_clock(chip, 2, 3579545));
    native_master_system_stop(chip, 2); native_master_system_get_state(chip, &s);
    assert(s.psg_clock == 100000 && s.opll_clock == 100000 && s.periods[0] == 0x125 && s.noise_mode == 7);
    assert(s.attenuation[0] == 15 && s.levels[0] == 3 && s.attenuation[3] == 15 && s.levels[3] == 5);
    assert(s.registers[0x30] == 0x13 && s.registers[0x10] == 0xc0 && !(s.registers[0x20] & 16));
    assert(render(chip, 2000) == 0); // Stop also clears held downsampled PCM immediately.
    native_master_system_audition_key_on(chip, 0); assert(render(chip, 400, 0) > 1);
    native_master_system_set_clock(chip, 1, 3579545); native_master_system_audition_key_on(chip, 4);
    assert(render(chip, 1000, 1) > .1);
    native_master_system_key_off(chip, 4); render(chip, 10); assert(render(chip, 500, 1) == 0);
    native_master_system_audition_key_on(chip, 4); native_master_system_key_off(chip, 4); assert(render(chip, 1000, 1) == 0);
    native_master_system_audition_key_on(chip, 4); native_master_system_stop(chip, 1); assert(render(chip, 1000, 1) == 0);
    native_master_system_set_mute_mask(chip, 0, 15); native_master_system_set_mute_mask(chip, 1, 511);
    voice(chip, 8); assert(render(chip, 500) == 0);
    native_master_system_set_mute_mask(chip, 0, 0); native_master_system_set_mute_mask(chip, 1, 0);
    assert(render(chip, 500) > .1);
    native_master_system_reset(chip, 1); assert(render(chip, 100, 0) > .1); assert(render(chip, 100, 1) == 0);
    native_master_system_reset(chip, 0); voice(chip, 8); assert(render(chip, 1000, 1) > .1); assert(render(chip, 100, 0) == 0);
    native_master_system_write(chip, 1, 0x36, 3); native_master_system_write(chip, 1, 0x37, 0x44);
    native_master_system_write(chip, 1, 0x38, 0x55); native_master_system_write(chip, 1, 0x0e, 63);
    assert(render(chip, 2000, 1) > 1);
    native_master_system_key_off(chip, 14); native_master_system_get_state(chip, &s);
    assert(!(s.registers[0x0e] & 8) && (s.registers[0x0e] & 1)); assert(s.registers[0x37] == 0x44);
    native_master_system_audition_key_on(chip, 14); assert(render(chip, 1000, 1) > .01);
    native_master_system_audition_key_on(chip, 13); native_master_system_reset(chip, 1); assert(render(chip, 1000, 1) == 0);
    for (int i = 0; i < 100; ++i) { native_master_system_get_state(chip, &s); assert(s.status == native_master_system_read_status(chip)); assert(s.irq == native_master_system_get_irq(chip)); native_master_system_read(chip, 0); }
    native_master_system_reset(chip, 2); native_master_system_get_state(chip, &s);
    assert(s.psg_clock == 100000 && s.opll_clock == 3579545 && s.periods[0] == 0 && s.registers[0x37] == 0);
    audit = false; assert(allocations == 0 && frees == 0); native_master_system_destroy(chip);
    std::puts("Master System mono PSG/OPLL state, independent reset, source clocks/mutes, shared rhythm, deferred retrigger and retained Stop levels passed; callback allocations 0, frees 0"); return 0;
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
            else if (e.op == "clock" || e.op == "mute") in >> e.a >> e.b;
            else if (e.op == "reset" || e.op == "stop" || e.op == "off" || e.op == "audition") in >> e.a;
            else return 2;
            events.push_back(std::move(e));
        }
        std::stable_sort(events.begin(), events.end(), [](const Event &a, const Event &b) { return a.frame < b.frame; });
        void *chip = native_master_system_create(rate); if (!chip) return 3;
        std::vector<float> pcm(frames * 4); size_t next = 0; audit = true;
        for (uint32_t frame = 0; frame < frames; ++frame) {
            while (next < events.size() && events[next].frame == frame) {
                const auto &e = events[next++];
                if (e.op == "write") native_master_system_write(chip, e.a, e.b, e.c);
                else if (e.op == "clock") assert(native_master_system_set_clock(chip, e.a, e.b));
                else if (e.op == "mute") native_master_system_set_mute_mask(chip, e.a, e.b);
                else if (e.op == "audition") native_master_system_audition_key_on(chip, e.a);
                else if (e.op == "reset") native_master_system_reset(chip, e.a);
                else if (e.op == "stop") native_master_system_stop(chip, e.a);
                else if (e.op == "off") native_master_system_key_off(chip, e.a);
            }
            native_master_system_sample(chip, &pcm[frame*4], &pcm[frame*4+1], &pcm[frame*4+2], &pcm[frame*4+3]);
        }
        audit = false; assert(allocations == 0 && frees == 0); native_master_system_destroy(chip);
        return std::fwrite(pcm.data(), sizeof(float), pcm.size(), stdout) == pcm.size() ? 0 : 4;
    } catch (const std::exception &e) { std::cerr << e.what() << '\n'; return 1; }
}
