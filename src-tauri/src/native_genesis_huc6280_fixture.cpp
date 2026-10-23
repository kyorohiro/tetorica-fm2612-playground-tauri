// Offline PCM/state/allocation fixture; never opens an output device.
#include "native_genesis_huc6280.h"
#include <atomic>
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
static uint64_t allocations = 0, deallocations = 0;
void *operator new(std::size_t n) { if (audit) ++allocations; if (void *p = std::malloc(n)) return p; throw std::bad_alloc(); }
void *operator new[](std::size_t n) { return ::operator new(n); }
void operator delete(void *p) noexcept { if (audit && p) ++deallocations; std::free(p); }
void operator delete[](void *p) noexcept { ::operator delete(p); }
void operator delete(void *p, std::size_t) noexcept { ::operator delete(p); }
void operator delete[](void *p, std::size_t) noexcept { ::operator delete(p); }
struct Event { uint32_t frame, a = 0, b = 0; int channel = -1; std::string op; uint8_t wave[32] = {}; };
static void apply(void *ptr, const Event &e) {
    if (e.op == "write") native_huc6280_write(ptr, e.a, e.b, e.channel);
    else if (e.op == "waveform") native_huc6280_waveform(ptr, e.a, e.wave);
    else if (e.op == "off") native_huc6280_key_off(ptr, e.a);
    else if (e.op == "mute") native_huc6280_set_mute_mask(ptr, e.a);
    else if (e.op == "clock") assert(native_huc6280_set_clock(ptr, e.a));
    else if (e.op == "stop") native_huc6280_stop(ptr);
    else if (e.op == "reset") native_huc6280_reset(ptr);
    else std::abort();
}
static int self_test() {
    void *ptr = native_huc6280_create(48000);
    void *sibling = native_huc6280_create(48000);
    void *reference = native_huc6280_create(48000);
    assert(ptr && sibling && reference);
    assert(!native_huc6280_create(1) && !native_huc6280_create(384001));
    uint8_t wave[32]; for (int i = 0; i < 32; ++i) wave[i] = i;
    audit = true;
    native_huc6280_write(ptr, 0, 7, -1);
    native_huc6280_write(ptr, 1, 255, -1);
    native_huc6280_write(ptr, 4, 27, 2);
    native_huc6280_waveform(ptr, 2, wave);
    native_huc6280_write(ptr, 2, 177, 2);
    native_huc6280_write(ptr, 5, 0xf9, 2);
    native_huc6280_write(ptr, 4, 155, 2);
    assert(native_huc6280_set_clock(ptr, 7159090));
    float l, r; for (int i = 0; i < 2048; ++i) native_huc6280_sample(ptr, &l, &r);
    NativeHuc6280State state;
    native_huc6280_get_state(ptr, &state);
    assert(state.selected == 7 && state.globals[0] == 7);
    assert(state.registers[2][4] == 155 && state.registers[2][2] == 177);
    assert(std::memcmp(state.waveforms[2], wave, 32) == 0);
    native_huc6280_stop(ptr);
    native_huc6280_get_state(ptr, &state);
    assert(state.registers[2][4] == 27 && state.clock == 7159090 && state.selected == 7);
    assert(std::memcmp(state.waveforms[2], wave, 32) == 0);
    native_huc6280_write(ptr, 4, 0xdf, 3); native_huc6280_write(ptr, 6, 29, 3);
    native_huc6280_get_state(ptr, &state); assert(state.dda[3] == 29);
    native_huc6280_key_off(ptr, 3); native_huc6280_get_state(ptr, &state);
    assert(state.registers[3][4] == 0x5f && state.dda[3] == 29);
    native_huc6280_reset(ptr); native_huc6280_get_state(ptr, &state);
    assert(state.clock == 7159090 && state.selected == 0 && state.globals[1] == 0);
    for (auto &waveform : state.waveforms) for (auto value : waveform) assert(value == 0);
    assert(!native_huc6280_set_clock(ptr, 1));
    assert(!native_huc6280_set_clock(ptr, 100000001));
    // Read-only status and a muted source must leave every oscillator, shared
    // LFO and LFSR progressing identically to an unobserved reference.
    assert(native_huc6280_set_clock(ptr, 3579545));
    assert(native_huc6280_set_clock(reference, 3579545));
    for (void *chip : {ptr, reference}) {
        native_huc6280_write(chip, 1, 255, -1);
        for (uint32_t ch = 0; ch < 6; ++ch) {
            native_huc6280_waveform(chip, ch, wave);
            native_huc6280_write(chip, 2, 177 + ch * 19, ch);
            native_huc6280_write(chip, 3, 1, ch);
            native_huc6280_write(chip, 5, 255, ch);
            native_huc6280_write(chip, 4, 159, ch);
        }
        native_huc6280_write(chip, 7, 0x87, 4);
        native_huc6280_write(chip, 8, 7, -1);
        native_huc6280_write(chip, 9, 3, -1);
        native_huc6280_write(chip, 0, 7, -1);
    }
    native_huc6280_set_mute_mask(ptr, 63);
    for (int i = 0; i < 2048; ++i) {
        float rl, rr;
        native_huc6280_get_state(ptr, &state);
        native_huc6280_sample(ptr, &l, &r);
        native_huc6280_sample(reference, &rl, &rr);
        assert(l == 0 && r == 0);
    }
    native_huc6280_set_mute_mask(ptr, 0);
    for (int i = 0; i < 2048; ++i) {
        float rl, rr;
        native_huc6280_get_state(ptr, &state);
        native_huc6280_sample(ptr, &l, &r);
        native_huc6280_sample(reference, &rl, &rr);
        assert(l == rl && r == rr && std::isfinite(l) && std::isfinite(r));
    }
    native_huc6280_key_off(ptr, 0);
    native_huc6280_get_state(ptr, &state);
    assert(state.selected == 7 && state.registers[0][4] == 31);
    for (int ch = 1; ch < 6; ++ch) assert(state.registers[ch][4] == 159);
    assert(state.globals[8] == 7 && state.globals[9] == 3);
    NativeHuc6280State sibling_state;
    native_huc6280_get_state(sibling, &sibling_state);
    assert(sibling_state.clock == 3579545 && sibling_state.globals[1] == 0);
    for (const auto &values : sibling_state.waveforms) for (auto value : values) assert(value == 0);
    // Clearing key-on silences DDA without destroying the last authored DAC,
    // waveform, raw selection or shared LFO configuration.
    native_huc6280_write(sibling, 1, 255, -1);
    native_huc6280_write(sibling, 5, 255, 3);
    native_huc6280_write(sibling, 4, 223, 3);
    native_huc6280_write(sibling, 6, 29, 3);
    native_huc6280_sample(sibling, &l, &r); assert(l != 0 && r != 0);
    native_huc6280_key_off(sibling, 3);
    native_huc6280_sample(sibling, &l, &r); assert(l == 0 && r == 0);
    native_huc6280_get_state(sibling, &sibling_state);
    assert(sibling_state.registers[3][4] == 95 && sibling_state.dda[3] == 29);
    audit = false;
    assert(allocations == 0 && deallocations == 0);
    native_huc6280_destroy(ptr);
    native_huc6280_destroy(sibling);
    native_huc6280_destroy(reference);
    std::printf("{\"stateBytes\":%zu,\"coreHeapBytes\":%zu,\"independentInstances\":true,\"observationPreservesPhase\":true,\"mutePreservesLfoAndNoisePhase\":true,\"targetedDdaStop\":true,\"renderAllocations\":0,\"renderFrees\":0}\n", sizeof(NativeHuc6280State), native_huc6280_core_size());
    return 0;
}
int main(int argc, char **argv) {
    if (argc == 2 && std::string(argv[1]) == "--self-test") return self_test();
    if (argc != 3) return 2;
    const uint32_t rate = std::stoul(argv[1]), frames = std::stoul(argv[2]);
    std::vector<Event> events;
    std::string line;
    while (std::getline(std::cin, line)) {
        std::istringstream in(line); Event e; if (!(in >> e.frame >> e.op)) continue;
        if (e.op == "write") in >> e.a >> e.b >> e.channel;
        else if (e.op == "waveform") { std::string hex; in >> e.a >> hex; assert(hex.size() == 64); for (int i = 0; i < 32; ++i) e.wave[i] = std::stoul(hex.substr(i * 2, 2), nullptr, 16); }
        else if (e.op == "off" || e.op == "clock" || e.op == "mute") in >> e.a;
        events.push_back(e);
    }
    void *ptr = native_huc6280_create(rate); if (!ptr) return 3;
    std::vector<float> output(frames * 2);
    size_t next = 0;
    audit = true;
    for (uint32_t frame = 0; frame < frames; ++frame) {
        while (next < events.size() && events[next].frame == frame) apply(ptr, events[next++]);
        native_huc6280_sample(ptr, &output[frame * 2], &output[frame * 2 + 1]);
    }
    audit = false;
    assert(allocations == 0 && deallocations == 0);
    native_huc6280_destroy(ptr);
    return std::fwrite(output.data(), sizeof(float), output.size(), stdout) == output.size() ? 0 : 4;
}
