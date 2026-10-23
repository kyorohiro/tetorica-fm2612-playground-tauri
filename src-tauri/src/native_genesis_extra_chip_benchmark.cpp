// Offline DSP count-growth benchmark. No devices, threads, IPC or Worklets.
#include "native_genesis_ym2612.h"
#include "native_genesis_game_gear.h"
#include <cassert>
#include <chrono>
#include <cstddef>
#include <cstdio>
#include <cstdlib>
#include <ctime>
#include <new>
#include <vector>
#if defined(__APPLE__) || defined(__linux__)
#include <sys/resource.h>
#endif

struct alignas(std::max_align_t) Allocation { std::size_t size; };
static std::size_t live_bytes = 0;
static uint64_t allocations = 0, frees = 0;
void *operator new(std::size_t n) {
    auto *block = static_cast<Allocation *>(std::malloc(sizeof(Allocation) + n));
    if (!block) throw std::bad_alloc();
    block->size = n; live_bytes += n; ++allocations; return block + 1;
}
void *operator new[](std::size_t n) { return ::operator new(n); }
void operator delete(void *ptr) noexcept {
    if (!ptr) return;
    auto *block = static_cast<Allocation *>(ptr) - 1; live_bytes -= block->size; ++frees; std::free(block);
}
void operator delete[](void *ptr) noexcept { ::operator delete(ptr); }
void operator delete(void *ptr, std::size_t) noexcept { ::operator delete(ptr); }
void operator delete[](void *ptr, std::size_t) noexcept { ::operator delete(ptr); }
static uint64_t peak_rss() {
#if defined(__APPLE__) || defined(__linux__)
    rusage usage{}; if (getrusage(RUSAGE_SELF, &usage)) return 0;
#if defined(__APPLE__)
    return static_cast<uint64_t>(usage.ru_maxrss);
#else
    return static_cast<uint64_t>(usage.ru_maxrss) * 1024;
#endif
#else
    return 0;
#endif
}
static void program_fm(void *chip, uint32_t transpose) {
    for (uint8_t ch = 0; ch < 6; ++ch) {
        const uint8_t port = ch / 3, offset = ch % 3;
        auto write = [=](uint8_t reg, uint8_t value) { native_ym2612_write(chip, port, reg, value); };
        for (uint8_t op = 0; op < 4; ++op) {
            const uint8_t reg = offset + op * 4;
            write(0x30 + reg, op + 1); write(0x40 + reg, 8 + op * 9);
            write(0x50 + reg, 31); write(0x60 + reg, 0x80); write(0x70 + reg, 0); write(0x80 + reg, 15);
        }
        write(0xb0 + offset, (ch % 8) | (5 << 3)); write(0xb4 + offset, 0xc0 | 0x20 | 3);
        write(0xa4 + offset, 0x22); write(0xa0 + offset, 67 + ch * 23 + transpose);
        native_ym2612_write(chip, 0, 0x28, 0xf0 | (ch < 3 ? ch : ch + 1));
    }
    native_ym2612_write(chip, 0, 0x22, 15);
}
static void program_psg(void *chip, uint32_t transpose) {
    for (uint8_t ch = 0; ch < 3; ++ch) {
        const uint16_t period = 127 + ch * 103 + transpose;
        native_game_gear_write(chip, 0x80 | (ch << 5) | (period & 15)); native_game_gear_write(chip, period >> 4);
        native_game_gear_write(chip, 0x90 | (ch << 5) | (3 + ch));
    }
    native_game_gear_write(chip, 0xe7); native_game_gear_write(chip, 0xf6);
}
int main(int argc, char **argv) {
    if (argc != 4) return 2;
    const uint32_t fm_count = std::strtoul(argv[1], nullptr, 10), psg_count = std::strtoul(argv[2], nullptr, 10);
    const uint32_t frames = std::strtoul(argv[3], nullptr, 10), rate = 48000;
    if (fm_count + psg_count == 0 || fm_count + psg_count > 16 || !frames || frames > rate * 60) return 2;
    std::vector<void *> fm, psg; fm.reserve(fm_count); psg.reserve(psg_count);
    const auto baseline_heap = live_bytes;
    const auto baseline_rss = peak_rss();
    for (uint32_t i = 0; i < fm_count; ++i) { auto *chip = native_ym2612_create(rate); assert(chip); fm.push_back(chip); program_fm(chip, i); }
    for (uint32_t i = 0; i < psg_count; ++i) { auto *chip = native_game_gear_create(rate); assert(chip); psg.push_back(chip); program_psg(chip, i); }
    const auto core_heap_bytes = live_bytes - baseline_heap;
    volatile double checksum = 0;
    auto render = [&](uint32_t n) {
        for (uint32_t frame = 0; frame < n; ++frame) {
            float mixed_left = 0, mixed_right = 0, l, r;
            for (void *chip : fm) { native_ym2612_sample(chip, &l, &r); mixed_left += l; mixed_right += r; }
            for (void *chip : psg) { native_game_gear_sample(chip, &l, &r); mixed_left += l; mixed_right += r; }
            checksum += mixed_left + mixed_right;
        }
    };
    render(4096); // Warm the DSP/code pages before the measured render interval.
    const auto begin_alloc = allocations, begin_free = frees;
    const auto cpu_begin = std::clock(); const auto wall_begin = std::chrono::steady_clock::now();
    render(frames);
    const double wall_seconds = std::chrono::duration<double>(std::chrono::steady_clock::now() - wall_begin).count();
    const double cpu_seconds = double(std::clock() - cpu_begin) / CLOCKS_PER_SEC;
    const auto callback_allocations = allocations - begin_alloc, callback_frees = frees - begin_free;
    const auto peak = peak_rss();
    for (void *chip : fm) native_ym2612_destroy(chip);
    for (void *chip : psg) native_game_gear_destroy(chip);
    assert(live_bytes == baseline_heap && callback_allocations == 0 && callback_frees == 0);
    std::printf("{\"ym2612\":%u,\"segapsg\":%u,\"sampleRate\":%u,\"frames\":%u,\"cpuSeconds\":%.9f,\"wallSeconds\":%.9f,\"coreHeapBytes\":%zu,\"peakRssBytes\":%llu,\"peakRssGrowthBytes\":%llu,\"renderAllocations\":%llu,\"renderFrees\":%llu,\"checksum\":%.9f}\n",
        fm_count, psg_count, rate, frames, cpu_seconds, wall_seconds, core_heap_bytes,
        static_cast<unsigned long long>(peak), static_cast<unsigned long long>(peak > baseline_rss ? peak - baseline_rss : 0),
        static_cast<unsigned long long>(callback_allocations), static_cast<unsigned long long>(callback_frees), double(checksum));
    return 0;
}
