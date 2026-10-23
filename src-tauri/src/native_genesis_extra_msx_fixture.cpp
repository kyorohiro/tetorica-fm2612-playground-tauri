// Offline standalone extra OPLL/AY checks; no machine-level DSP or devices.
#include "native_genesis_ym2413.h"
#include "native_genesis_ay8910.h"
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
static void opll_voice(void *chip, uint8_t ch) {
    native_ym2413_write(chip, 0x30 + ch, 0x13); native_ym2413_write(chip, 0x10 + ch, 0xc0); native_ym2413_write(chip, 0x20 + ch, 0x18);
}
static void ay_voice(void *chip, uint8_t ch) {
    native_ay8910_write(chip, ch * 2, 127 + ch * 43); native_ay8910_write(chip, ch * 2 + 1, 1); native_ay8910_write(chip, 7, 0x38); native_ay8910_write(chip, 8 + ch, 12 - ch);
}
static void sample(bool ay, void *chip, float &l, float &r) { if (ay) native_ay8910_sample(chip, &l, &r); else native_ym2413_sample(chip, &l, &r); assert(l == r && std::isfinite(l)); }
static double render(bool ay, void *chip, uint32_t frames) { double sum = 0; float l, r; for (uint32_t i = 0; i < frames; ++i) { sample(ay, chip, l, r); sum += std::abs(l); } return sum; }
static int self_test() {
    static_assert(sizeof(NativeYm2413State) == 72, "OPLL ABI"); static_assert(sizeof(NativeAy8910State) == 24, "AY ABI");
    assert(!native_ym2413_create(0) && !native_ay8910_create(384001));
    void *opll = native_ym2413_create(48000), *ay = native_ay8910_create(48000); assert(opll && ay);
    audit = true; NativeYm2413State f{}; NativeAy8910State p{};
    native_ym2413_get_state(opll, &f); native_ay8910_get_state(ay, &p);
    assert(f.clock == 3579545 && p.clock == 1789773 && p.levels[0] == 9 && p.levels[2] == 9);
    assert(render(false, opll, 100) == 0 && render(true, ay, 100) == 0);
    opll_voice(opll, 0); opll_voice(opll, 8); ay_voice(ay, 0); ay_voice(ay, 2);
    assert(render(false, opll, 1000) > .01 && render(true, ay, 1000) > .01);
    assert(!native_ym2413_set_clock(opll, 99999) && !native_ay8910_set_clock(ay, 100000001));
    assert(native_ym2413_set_clock(opll, 100000) && native_ay8910_set_clock(ay, 100000));
    native_ym2413_stop(opll); native_ay8910_stop(ay); assert(render(false, opll, 100) == 0 && render(true, ay, 100) == 0);
    native_ay8910_get_state(ay, &p); assert(p.levels[0] == 12 && p.levels[2] == 10 && !p.registers[8] && !p.registers[10]);
    native_ay8910_set_retained_level(ay, 0, 0); native_ay8910_audition_key_on(ay, 0); native_ay8910_get_state(ay, &p); assert(!p.levels[0] && !p.registers[8]);
    native_ay8910_set_retained_level(ay, 0, 17); native_ay8910_audition_key_on(ay, 0); assert(render(true, ay, 500) > .01);
    native_ym2413_audition_key_on(opll, 0); native_ym2413_get_state(opll, &f); assert(!(f.registers[0x20] & 16));
    assert(render(false, opll, 2000) > .01); native_ym2413_audition_key_on(opll, 0); native_ym2413_key_off(opll, 0); native_ym2413_get_state(opll, &f); assert(!(f.registers[0x20] & 16));
    native_ym2413_stop(opll); native_ym2413_audition_key_on(opll, 0); native_ym2413_reset(opll); assert(render(false, opll, 300) == 0);
    native_ay8910_reset(ay); native_ay8910_get_state(ay, &p); assert(p.levels[0] == 9 && p.clock == 100000);
    for (uint8_t reg = 0; reg < 16; ++reg) assert(!p.registers[reg]);
    audit = false; assert(!allocations && !frees); native_ym2413_destroy(opll); native_ay8910_destroy(ay);
    // Cancel one voice while a foreign voice is cached at the minimum clock.
    for (bool is_ay : {false, true}) {
        void *a = is_ay ? native_ay8910_create(48000) : native_ym2413_create(48000), *b = is_ay ? native_ay8910_create(48000) : native_ym2413_create(48000);
        audit = true;
        if (is_ay) { ay_voice(a, 0); ay_voice(a, 2); ay_voice(b, 2); }
        else { opll_voice(a, 0); opll_voice(a, 5); opll_voice(b, 5); }
        render(is_ay, a, 701); render(is_ay, b, 701);
        if (is_ay) { native_ay8910_set_clock(a, 100000); native_ay8910_set_clock(b, 100000); }
        else { native_ym2413_set_clock(a, 100000); native_ym2413_set_clock(b, 100000); }
        render(is_ay, a, 39); render(is_ay, b, 39);
        if (is_ay) native_ay8910_key_off(a, 0); else native_ym2413_key_off(a, 0);
        double energy = 0;
        for (uint32_t i = 0; i < 1000; ++i) {
            float al, ar, bl, br; sample(is_ay, a, al, ar); sample(is_ay, b, bl, br);
            assert(std::abs(al - bl) <= 1e-7); energy += std::abs(al);
            if (is_ay) { NativeAy8910State state{}; native_ay8910_get_state(a, &state); native_ay8910_read(a, 14); }
            else { NativeYm2413State state{}; native_ym2413_get_state(a, &state); native_ym2413_read(a, 1); }
        }
        assert(energy > .01); audit = false; assert(!allocations && !frees);
        if (is_ay) { native_ay8910_destroy(a); native_ay8910_destroy(b); } else { native_ym2413_destroy(a); native_ym2413_destroy(b); }
    }
    // Shared rhythm channels retain the independently owned operator, including
    // the held mixed sample and each channel's nonlinear DAC clipping.
    {
        void *a = native_ym2413_create(96000), *b = native_ym2413_create(96000);
        audit = true;
        for (void *chip : {a, b}) {
            opll_voice(chip, 4);
            for (uint8_t ch = 6; ch < 9; ++ch) { native_ym2413_write(chip, 0x10 + ch, 127 + ch * 13); native_ym2413_write(chip, 0x20 + ch, 8); }
            native_ym2413_write(chip, 0x36, 2); native_ym2413_write(chip, 0x37, 0x43); native_ym2413_write(chip, 0x38, 0x65);
        }
        native_ym2413_write(a, 0x0e, 63); native_ym2413_write(b, 0x0e, 35); // foreign HH/CYM
        render(false, a, 1703); render(false, b, 1703);
        native_ym2413_set_clock(a, 100000); native_ym2413_set_clock(b, 100000);
        render(false, a, 37); render(false, b, 37);
        native_ym2413_key_off(a, 9); native_ym2413_key_off(a, 10); native_ym2413_key_off(a, 11);
        NativeYm2413State state{}; native_ym2413_get_state(a, &state);
        assert(state.registers[0x0e] == 35 && state.registers[0x37] == 0x43 && state.registers[0x38] == 0x65);
        // A stale melodic release on an aliased physical channel cannot mute
        // its replacement rhythm owner.
        native_ym2413_key_off(a, 7); native_ym2413_key_off(a, 8);
        for (uint32_t i = 0; i < 1000; ++i) { float al, ar, bl, br; sample(false, a, al, ar); sample(false, b, bl, br); assert(al == bl); }
        native_ym2413_audition_key_on(a, 10); native_ym2413_audition_key_on(a, 13); native_ym2413_key_off(a, 10);
        render(false, a, 200); native_ym2413_get_state(a, &state); assert(!(state.registers[0x0e] & 8) && (state.registers[0x0e] & 1));
        audit = false; assert(!allocations && !frees); native_ym2413_destroy(a); native_ym2413_destroy(b);
    }
    // Inverse alias: cancelling inactive authored drum flags cannot mute their
    // physical replacement FM6..8 when rhythm mode is disabled.
    {
        void *a = native_ym2413_create(96000), *b = native_ym2413_create(96000);
        audit = true;
        for (void *chip : {a, b}) { for (uint8_t ch = 6; ch < 9; ++ch) opll_voice(chip, ch); native_ym2413_write(chip, 0x0e, 31); }
        render(false, a, 1703); render(false, b, 1703);
        native_ym2413_set_clock(a, 100000); native_ym2413_set_clock(b, 100000);
        render(false, a, 37); render(false, b, 37);
        for (uint8_t voice = 9; voice < 14; ++voice) native_ym2413_key_off(a, voice);
        for (uint32_t i = 0; i < 1000; ++i) { float al, ar, bl, br; sample(false, a, al, ar); sample(false, b, bl, br); assert(al == bl); }
        audit = false; assert(!allocations && !frees); native_ym2413_destroy(a); native_ym2413_destroy(b);
    }
    std::puts("Standalone OPLL/AY independent defaults, reset, retained zero, deferred OPLL preview, foreign cached PCM, readonly state; callback allocations 0, frees 0"); return 0;
}
struct Event { uint32_t frame = 0, a = 0, b = 0; std::string op; };
int main(int argc, char **argv) {
    try {
        if (argc == 2 && std::string(argv[1]) == "--self-test") return self_test();
        if (argc != 4) return 2; const bool ay = std::string(argv[1]) == "ay8910";
        const uint32_t rate = std::stoul(argv[2]), frames = std::stoul(argv[3]), planes = ay ? 22 : 71; if (frames > 10000000) return 2;
        std::vector<Event> events; std::string line;
        while (std::getline(std::cin, line)) { std::istringstream in(line); Event e; if (!(in >> e.frame >> e.op)) continue; in >> e.a >> e.b; events.push_back(std::move(e)); }
        std::stable_sort(events.begin(), events.end(), [](const Event &a, const Event &b) { return a.frame < b.frame; });
        void *chip = ay ? native_ay8910_create(rate) : native_ym2413_create(rate); if (!chip) return 3;
        std::vector<float> pcm(frames * planes); size_t next = 0; audit = true;
        for (uint32_t frame = 0; frame < frames; ++frame) {
            while (next < events.size() && events[next].frame == frame) {
                const auto &e = events[next++];
                if (e.op == "write") { if (ay) native_ay8910_write(chip, e.a, e.b); else native_ym2413_write(chip, e.a, e.b); }
                else if (e.op == "clock") assert(ay ? native_ay8910_set_clock(chip, e.a) : native_ym2413_set_clock(chip, e.a));
                else if (e.op == "reset") { if (ay) native_ay8910_reset(chip); else native_ym2413_reset(chip); }
                else if (e.op == "stop") { if (ay) native_ay8910_stop(chip); else native_ym2413_stop(chip); }
                else if (e.op == "all-off") { if (ay) native_ay8910_all_notes_off(chip); else native_ym2413_stop(chip); }
                else if (e.op == "off") { if (ay) native_ay8910_preview_key_off(chip, e.a); else native_ym2413_preview_key_off(chip, e.a); }
                else if (e.op == "audition") { if (ay) native_ay8910_audition_key_on(chip, e.a); else native_ym2413_audition_key_on(chip, e.a); }
                else if (e.op == "retained" && ay) native_ay8910_set_retained_level(chip, e.a, e.b); else return 2;
            }
            float *out = &pcm[frame * planes]; sample(ay, chip, out[0], out[1]);
            if (ay) {
                NativeAy8910State state{}; native_ay8910_get_state(chip, &state); for (uint8_t i = 0; i < 16; ++i) out[2 + i] = state.registers[i];
                for (uint8_t i = 0; i < 3; ++i) out[18 + i] = state.levels[i]; out[21] = state.clock;
            } else {
                NativeYm2413State state{}; native_ym2413_get_state(chip, &state); for (uint8_t i = 0; i < 64; ++i) out[2 + i] = state.registers[i];
                out[66] = state.clock; out[67] = state.status; out[68] = state.irq; out[69] = native_ym2413_read(chip, 0); out[70] = native_ym2413_read(chip, 1);
            }
        }
        audit = false; assert(!allocations && !frees); if (ay) native_ay8910_destroy(chip); else native_ym2413_destroy(chip);
        return std::fwrite(pcm.data(), sizeof(float), pcm.size(), stdout) == pcm.size() ? 0 : 4;
    } catch (const std::exception &e) { std::cerr << e.what() << '\n'; return 1; }
}
