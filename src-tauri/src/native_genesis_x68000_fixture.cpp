// Offline PCM/state/allocation fixture. Never opens an audio device.
#include "native_genesis_x68000.h"
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
static double render(void *chip, uint32_t frames, bool fm_only = false) {
    double energy = 0; float fl, fr, ol, orr;
    for (uint32_t i = 0; i < frames; ++i) { native_x68000_sample(chip, &fl, &fr, &ol, &orr); energy += std::abs(fl) + std::abs(fr) + (fm_only ? 0 : std::abs(ol) + std::abs(orr)); }
    return energy;
}
static void fm(void *chip, uint8_t ch) {
    native_x68000_write(chip, 0, 0x20 + ch, 0xc7);
    for (uint8_t op = 0; op < 4; ++op) {
        native_x68000_write(chip, 0, 0x40 + op * 8 + ch, 1);
        native_x68000_write(chip, 0, 0x60 + op * 8 + ch, op == 3 ? 24 : 127);
        native_x68000_write(chip, 0, 0x80 + op * 8 + ch, 31);
        native_x68000_write(chip, 0, 0xe0 + op * 8 + ch, 1);
    }
    native_x68000_write(chip, 0, 0x28 + ch, 0x4a); native_x68000_write(chip, 0, 0x30 + ch, 0x54);
    native_x68000_write(chip, 0, 8, ch | 0x78);
}
static void upload(void *chip, const std::vector<uint8_t> &bytes) {
    assert(native_x68000_sample_begin(chip, bytes.size()));
    for (uint32_t offset = 0; offset < bytes.size(); offset += 65536)
        assert(native_x68000_sample_chunk(chip, bytes.data() + offset, offset, std::min(uint32_t(bytes.size()) - offset, 65536u)));
    assert(native_x68000_sample_commit(chip));
}
static int self_test() {
    assert(!native_x68000_create(0)); void *chip = native_x68000_create(48000); assert(chip);
    NativeX68000State s{};
    std::vector<uint8_t> data(0x1000000); for (uint32_t i = 0; i < data.size(); ++i) data[i] = uint8_t(i * 7 + 31);
    audit = true;
    native_x68000_get_state(chip, &s); assert(s.fm_clock == 4000000 && s.oki_clock == 8000000 && s.divider == 512 && s.sample_bytes == 0);
    assert(!native_x68000_oki_play(chip, 0));
    assert(!native_x68000_sample_begin(chip, 0)); assert(!native_x68000_sample_begin(chip, 0x1000001));
    upload(chip, data); assert(native_x68000_oki_play(chip, 1)); assert(render(chip, 500) > 1);
    native_x68000_get_state(chip, &s); assert(s.sample_bytes == 0x1000000 && s.streaming && s.playing && s.position > 0);
    assert(native_x68000_sample_begin(chip, 17)); assert(!native_x68000_sample_chunk(chip, nullptr, 0, 1));
    assert(!native_x68000_sample_chunk(chip, data.data(), 1, 16)); assert(native_x68000_sample_chunk(chip, data.data(), 0, 16));
    assert(!native_x68000_sample_commit(chip)); native_x68000_get_state(chip, &s); assert(s.playing && s.sample_bytes == 0x1000000);
    native_x68000_sample_abort(chip); assert(!native_x68000_sample_commit(chip));
    assert(native_x68000_set_clock(chip, 0, 3579545)); assert(native_x68000_set_clock(chip, 1, 7680000));
    assert(!native_x68000_set_clock(chip, 1, 1)); assert(!native_x68000_set_clock(chip, 2, 4000000));
    native_x68000_write(chip, 1, 12, 1); native_x68000_write(chip, 1, 2, 2);
    native_x68000_write(chip, 0, 0x19, 37); native_x68000_write(chip, 0, 0x19, 0x80 | 83);
    native_x68000_write(chip, 0, 0x18, 177); native_x68000_write(chip, 0, 0x1b, 2);
    for (uint8_t ch = 0; ch < 8; ++ch) fm(chip, ch);
    assert(render(chip, 3000, true) > 1);
    native_x68000_stop(chip, 2); native_x68000_get_state(chip, &s);
    assert(s.fm_clock == 3579545 && s.oki_clock == 7680000 && s.divider == 768 && s.pan == 2 && s.sample_bytes == 0x1000000);
    assert(s.amd == 37 && s.pmd == 83 && s.registers[0x18] == 177 && s.registers[0x1b] == 2);
    assert(s.registers[0x28] == 0x4a && s.registers[0x30] == 0x54 && s.registers[0x78] == 24);
    assert(render(chip, 256) == 0);
    native_x68000_write(chip, 0, 8, 0x78); assert(render(chip, 1000, true) > .1);
    native_x68000_key_off(chip, 0); render(chip, 10); assert(render(chip, 100) == 0);
    assert(native_x68000_oki_play(chip, 0)); native_x68000_reset(chip, 0); native_x68000_get_state(chip, &s); assert(s.playing && s.sample_bytes == 0x1000000);
    native_x68000_reset(chip, 1); native_x68000_get_state(chip, &s); assert(!s.playing && s.divider == 512 && s.pan == 0 && s.oki_clock == 7680000);
    native_x68000_write(chip, 0, 0x10, 255); native_x68000_write(chip, 0, 0x11, 3); native_x68000_write(chip, 0, 0x14, 5);
    render(chip, 100); native_x68000_get_state(chip, &s); assert(s.status & 1); assert(s.irq);
    for (int i = 0; i < 100; ++i) { assert(native_x68000_read_status(chip) == s.status); assert(native_x68000_read(chip, 1) == s.status); native_x68000_get_state(chip, &s); }
    native_x68000_write(chip, 0, 0x14, 0x10); assert(!(native_x68000_read_status(chip) & 1) && !native_x68000_get_irq(chip));
    native_x68000_set_fm_mute_mask(chip, 0xff); fm(chip, 7); assert(render(chip, 500, true) == 0);
    native_x68000_set_fm_mute_mask(chip, 0); assert(render(chip, 500, true) > .1);
    native_x68000_reset(chip, 2); fm(chip, 0);
    native_x68000_write(chip, 0, 0xb8, 31); native_x68000_write(chip, 0, 0xd8, 31); native_x68000_write(chip, 0, 0xf8, 0xff);
    render(chip, 10000); assert(render(chip, 1000, true) == 0);
    native_x68000_audition_key_on(chip, 0, 15); assert(render(chip, 1000, true) > .01);
    native_x68000_audition_key_on(chip, 0, 15); native_x68000_key_off(chip, 0); render(chip, 10); assert(render(chip, 1000, true) == 0);
    native_x68000_reset(chip, 2); native_x68000_get_state(chip, &s); assert(s.sample_bytes == 0x1000000 && s.fm_clock == 3579545 && s.registers[0x28] == 0 && s.amd == 0 && s.pmd == 0);
    assert(native_x68000_oki_play(chip, 1)); assert(render(chip, 500) > .1);
    audit = false; assert(allocations == 0 && frees == 0); native_x68000_destroy(chip);
    std::puts("X68000 FM/OKI state, independent reset, IRQ, atomic 16 MiB sample replacement and Stop self-test passed; callback allocations 0, frees 0"); return 0;
}
struct Event { uint32_t frame = 0, a = 0, b = 0, c = 0; std::string op; std::vector<uint8_t> bytes; };
int main(int argc, char **argv) {
    try {
        if (argc == 2 && std::string(argv[1]) == "--self-test") return self_test();
        if (argc != 3) return 2;
        const uint32_t rate = std::stoul(argv[1]), frames = std::stoul(argv[2]); if (frames > 10000000) return 2;
        std::vector<Event> events; std::string line;
        while (std::getline(std::cin, line)) {
            std::istringstream in(line); Event e; if (!(in >> e.frame >> e.op)) continue;
            if (e.op == "write") in >> e.a >> e.b >> e.c;
            else if (e.op == "clock" || e.op == "audition") in >> e.a >> e.b;
            else if (e.op == "reset" || e.op == "stop" || e.op == "off" || e.op == "mute" || e.op == "play" || e.op == "repeat") in >> e.a;
            else if (e.op == "memory") { std::string hex; in >> hex; assert(hex.size() % 2 == 0); for (size_t i = 0; i < hex.size(); i += 2) e.bytes.push_back(std::stoul(hex.substr(i, 2), nullptr, 16)); }
            else return 2;
            events.push_back(std::move(e));
        }
        std::stable_sort(events.begin(), events.end(), [](const Event &a, const Event &b) { return a.frame < b.frame; });
        void *chip = native_x68000_create(rate); if (!chip) return 3;
        std::vector<float> pcm(frames * 4); size_t next = 0; audit = true;
        for (uint32_t frame = 0; frame < frames; ++frame) {
            while (next < events.size() && events[next].frame == frame) {
                const auto &e = events[next++];
                if (e.op == "write") native_x68000_write(chip, e.a, e.b, e.c);
                else if (e.op == "clock") assert(native_x68000_set_clock(chip, e.a, e.b));
                else if (e.op == "audition") native_x68000_audition_key_on(chip, e.a, e.b);
                else if (e.op == "reset") native_x68000_reset(chip, e.a);
                else if (e.op == "stop") native_x68000_stop(chip, e.a);
                else if (e.op == "off") native_x68000_key_off(chip, e.a);
                else if (e.op == "mute") native_x68000_set_fm_mute_mask(chip, e.a);
                else if (e.op == "memory") upload(chip, e.bytes);
                else if (e.op == "play") assert(native_x68000_oki_play(chip, e.a));
                else if (e.op == "repeat") native_x68000_oki_set_repeat(chip, e.a);
            }
            native_x68000_sample(chip, &pcm[frame*4], &pcm[frame*4+1], &pcm[frame*4+2], &pcm[frame*4+3]);
        }
        audit = false; assert(allocations == 0 && frees == 0); native_x68000_destroy(chip);
        return std::fwrite(pcm.data(), sizeof(float), pcm.size(), stdout) == pcm.size() ? 0 : 4;
    } catch (const std::exception &e) { std::cerr << e.what() << '\n'; return 1; }
}
