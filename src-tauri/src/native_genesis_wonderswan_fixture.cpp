// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 kyorohiro
// Offline ABI, allocation and PCM fixture; never opens an output device.
#include "native_genesis_wonderswan.h"
#include "wonderswan.h"
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <iostream>
#include <new>
#include <string>
#include <utility>
#include <vector>
#ifdef _WIN32
#include <fcntl.h>
#include <io.h>
#endif

static bool measured = false;
static size_t allocations = 0, frees = 0, checks = 0;
void *operator new(size_t n) { if (measured) ++allocations; if (void *p = std::malloc(n ? n : 1)) return p; throw std::bad_alloc(); }
void *operator new[](size_t n) { return ::operator new(n); }
void operator delete(void *p) noexcept { if (measured && p) ++frees; std::free(p); }
void operator delete[](void *p) noexcept { ::operator delete(p); }
void operator delete(void *p, size_t) noexcept { ::operator delete(p); }
void operator delete[](void *p, size_t) noexcept { ::operator delete(p); }
static bool check(bool value) { ++checks; return value; }
static void write(void *chip, unsigned address, unsigned value, unsigned mask = 255) {
    native_wonderswan_write(chip, address, uint8_t(value), uint8_t(mask));
}
static void program(void *chip, unsigned ch = 0, unsigned byte = 0x87) {
    for (unsigned i = 0; i < 16; ++i) native_wonderswan_write_memory(chip, ch * 16 + i, uint8_t(byte + i));
    write(chip, 0x80 + ch * 2, 0x50); write(chip, 0x81 + ch * 2, 7);
    write(chip, 0x88 + ch, 0xf9); write(chip, 0x91, 8);
    write(chip, 0x90, 1u << ch, 1u << ch);
}
static int self_test() {
    bool ok = true;
    ok &= check(!native_wonderswan_create(7999, 3072000));
    ok &= check(!native_wonderswan_create(384001, 3072000));
    ok &= check(!native_wonderswan_create(48000, 99999));
    ok &= check(!native_wonderswan_create(48000, 100000001));
    void *p = native_wonderswan_create(48000, 3072000);
    void *twin = native_wonderswan_create(48000, 3072000);
    if (!p || !twin) return 2;
    NativeWonderSwanState before{}, after{}, empty{};
    float l = 0, r = 0, tl = 0, tr = 0;
    measured = true;
    native_wonderswan_destroy(nullptr); native_wonderswan_reset(nullptr);
    native_wonderswan_stop(nullptr); native_wonderswan_key_off(nullptr, 0);
    native_wonderswan_write(nullptr, 0x90, 255, 255);
    native_wonderswan_write_memory(nullptr, 0, 255);
    native_wonderswan_set_mute_mask(nullptr, 15);
    native_wonderswan_sample(nullptr, &l, &r);
    native_wonderswan_raw_sample(nullptr, &l, &r);
    native_wonderswan_get_state(nullptr, &empty);
    native_wonderswan_get_state(p, nullptr);
    native_wonderswan_sample(p, nullptr, &r);
    ok &= check(!l && !r && !native_wonderswan_read_register(nullptr, 0x80) && !native_wonderswan_read_memory(nullptr, 0));
    ok &= check(!native_wonderswan_set_clock(nullptr, 3072000));
    program(p); program(p, 1); program(twin); program(twin, 1);
    write(p, 0x90, 0xe0, 0xf0); write(twin, 0x90, 0xe0, 0xf0);
    native_wonderswan_get_state(p, &before);
    ok &= check(before.registers[16] == 0xe3);
    write(p, 0x90, 0, 1); native_wonderswan_get_state(p, &after);
    ok &= check(after.registers[16] == 0xe2 && before.memory[0] == after.memory[0]);
    write(p, 0x90, 1, 1);
    write(p, 0x8e, 0x1f); write(twin, 0x8e, 0x1f);
    ok &= check(native_wonderswan_read_register(p, 0x8e) == 0x17);
    ok &= check(!native_wonderswan_set_clock(p, 99999) && !native_wonderswan_set_clock(p, 100000001));
    native_wonderswan_get_state(p, &before);
    for (unsigned i = 0; i < 100; ++i) native_wonderswan_get_state(p, &after);
    ok &= check(std::memcmp(&before, &after, sizeof(before)) == 0);
    write(p, 0x7f, 99); write(p, 0xa0, 99); native_wonderswan_write_memory(p, 0x4000, 99);
    native_wonderswan_get_state(p, &after);
    ok &= check(std::memcmp(&before, &after, sizeof(before)) == 0);
    ok &= check(!native_wonderswan_read_register(p, 0x7f) && !native_wonderswan_read_register(p, 0xa0) && !native_wonderswan_read_memory(p, 0x4000));
    for (unsigned i = 0; i < 8192; ++i) {
        native_wonderswan_sample(p, &l, &r); native_wonderswan_sample(twin, &tl, &tr);
        ok &= check(l == tl && r == tr && std::isfinite(l) && std::isfinite(r));
    }
    native_wonderswan_set_mute_mask(p, 15);
    for (unsigned i = 0; i < 8192; ++i) {
        native_wonderswan_sample(p, &l, &r); native_wonderswan_sample(twin, &tl, &tr);
        ok &= check(l == 0 && r == 0);
    }
    native_wonderswan_set_mute_mask(p, 0);
    for (unsigned i = 0; i < 4096; ++i) {
        native_wonderswan_sample(p, &l, &r); native_wonderswan_sample(twin, &tl, &tr);
        ok &= check(l == tl && r == tr);
    }
    native_wonderswan_set_clock(p, 1536000); native_wonderswan_get_state(p, &before);
    native_wonderswan_stop(p); native_wonderswan_get_state(p, &after);
    ok &= check(after.clock == 1536000 && after.registers[16] == 0xe0 && std::memcmp(before.memory, after.memory, 16384) == 0);
    for (unsigned i = 0; i < 32; ++i) {
        native_wonderswan_sample(p, &l, &r); ok &= check(l == 0 && r == 0);
    }
    program(p, 0); program(p, 1); native_wonderswan_key_off(p, 0);
    ok &= check((native_wonderswan_read_register(p, 0x90) & 15) == 2);
    native_wonderswan_set_mute_mask(p, 5); native_wonderswan_reset(p); native_wonderswan_get_state(p, &after);
    ok &= check(after.clock == 1536000 && after.sample_rate == 48000 && after.mute_mask == 5);
    for (auto byte : after.registers) ok &= check(byte == 0);
    for (auto byte : after.memory) ok &= check(byte == 0);
    // A constant unsigned wavetable must never fabricate a cold-start -DC pulse.
    native_wonderswan_set_clock(p, 3072000); native_wonderswan_set_mute_mask(p, 0);
    program(p, 0, 0x88); for (unsigned i = 0; i < 16; ++i) native_wonderswan_write_memory(p, i, 0x88);
    for (unsigned i = 0; i < 512; ++i) { native_wonderswan_sample(p, &l, &r); ok &= check(l == 0 && r == 0); }
    // Every partial-mute combination centers only its audible static DACs.
    for (unsigned ch = 1; ch < 4; ++ch) {
        program(p, ch, 0x88);
        for (unsigned i = 0; i < 16; ++i) native_wonderswan_write_memory(p, ch * 16 + i, 0x88);
    }
    for (unsigned mask = 0; mask < 16; ++mask) {
        native_wonderswan_set_mute_mask(p, mask);
        for (unsigned i = 0; i < 256; ++i) { native_wonderswan_sample(p, &l, &r); ok &= check(l == 0 && r == 0); }
    }
    native_wonderswan_set_mute_mask(p, 0);
    for (unsigned block = 0; block < 2048; ++block) {
        for (unsigned i = 0; i < 128; ++i) {
            native_wonderswan_sample(p, &l, &r); ok &= check(std::isfinite(l) && std::isfinite(r));
        }
    }
    measured = false;
    native_wonderswan_destroy(p); native_wonderswan_destroy(twin);
    // Diagnostic raw output independently equals the public SDK class.
    for (auto rate : {44100u, 48000u, 96000u}) for (auto clock : {100001u, 3072000u, 3579545u}) {
        p = native_wonderswan_create(rate, clock); WonderSwan reference(clock, rate);
        measured = true;
        for (unsigned i = 0; i < 64; ++i) { const uint8_t v = uint8_t(i * 31 + 5); native_wonderswan_write_memory(p, i, v); reference.writeMemory(i, v); }
        for (unsigned ch = 0; ch < 4; ++ch) {
            for (auto pair : {std::pair<unsigned, unsigned>{0x80 + ch * 2, 0x61 + ch * 11}, {0x81 + ch * 2, 7}, {0x88 + ch, 0xf1 + ch * 3}}) {
                write(p, pair.first, pair.second); reference.writeRegister(pair.first, uint8_t(pair.second));
            }
        }
        write(p, 0x91, 8); reference.writeRegister(0x91, 8); write(p, 0x90, 15); reference.writeRegister(0x90, 15);
        for (unsigned i = 0; i < 8192; ++i) {
            native_wonderswan_raw_sample(p, &l, &r); reference.generate(&tl, &tr, 1);
            ok &= check(l == tl && r == tr);
        }
        measured = false; native_wonderswan_destroy(p);
    }
    ok &= !allocations && !frees;
    std::fprintf(stderr, "{\"passed\":%s,\"checks\":%zu,\"allocationsDuringRender\":%zu,\"freesDuringRender\":%zu,\"coreBytes\":%zu,\"stateBytes\":%zu,\"rawSdkParityFrames\":73728,\"hotBlocks\":2048,\"hotBlockFrames\":128,\"allocationScope\":\"C++ new/new[] and delete/delete[] only; direct malloc is not intercepted\"}\n", ok ? "true" : "false", checks, allocations, frees, native_wonderswan_core_size(), sizeof(NativeWonderSwanState));
    return ok ? 0 : 3;
}
struct Event { uint32_t frame = 0; std::string op; uint32_t address = 0, value = 0, mask = 255; };
int main(int argc, char **argv) {
    if (argc == 2 && !std::strcmp(argv[1], "--self-test")) return self_test();
    if (argc != 5) return 1;
#ifdef _WIN32
    _setmode(_fileno(stdout), _O_BINARY);
#endif
    const auto rate = uint32_t(std::strtoul(argv[1], nullptr, 10));
    const auto frames = uint32_t(std::strtoul(argv[2], nullptr, 10));
    const auto clock = uint32_t(std::strtoul(argv[3], nullptr, 10));
    const bool raw = !std::strcmp(argv[4], "raw");
    if (frames > 4000000) return 2;
    std::vector<Event> events; Event event;
    while (std::cin >> event.frame >> event.op >> event.address >> event.value >> event.mask) events.push_back(event);
    std::vector<float> output(size_t(frames) * 2);
    NativeWonderSwanState state{};
    void *p = native_wonderswan_create(rate, clock); if (!p) return 2;
    size_t next = 0;
    measured = true;
    for (uint32_t frame = 0; frame < frames; ++frame) {
        while (next < events.size() && events[next].frame == frame) {
            const auto &e = events[next++];
            if (e.op == "write") write(p, e.address, e.value, e.mask);
            else if (e.op == "memory") native_wonderswan_write_memory(p, e.address, uint8_t(e.value));
            else if (e.op == "clock") native_wonderswan_set_clock(p, e.value);
            else if (e.op == "reset") native_wonderswan_reset(p);
            else if (e.op == "stop") native_wonderswan_stop(p);
            else if (e.op == "off") native_wonderswan_key_off(p, e.address);
            else if (e.op == "mute") native_wonderswan_set_mute_mask(p, e.value);
            else return 6;
        }
        if (raw) native_wonderswan_raw_sample(p, &output[frame * 2], &output[frame * 2 + 1]);
        else native_wonderswan_sample(p, &output[frame * 2], &output[frame * 2 + 1]);
    }
    native_wonderswan_get_state(p, &state);
    measured = false; native_wonderswan_destroy(p);
    if (allocations || frees || next != events.size()) return 4;
    if (std::fwrite(output.data(), sizeof(float), output.size(), stdout) != output.size()) return 5;
    return std::fwrite(&state, sizeof(state), 1, stdout) == 1 ? 0 : 5;
}
