// Offline DSP count-growth benchmark. No devices, threads, IPC or Worklets.
#include "native_genesis_ym2612.h"
#include "native_genesis_ym2203.h"
#include "native_genesis_ym2608.h"
#include "native_genesis_ym2610b.h"
#include "native_genesis_ym2151.h"
#include "native_genesis_ym2413.h"
#include "native_genesis_ay8910.h"
#include "native_genesis_extra_opl.h"
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
static void program_ym2203(void *chip, uint32_t transpose) {
    auto write = [=](uint8_t reg, uint8_t value) { native_ym2203_write(chip, 0, reg, value); };
    for (uint8_t ch = 0; ch < 3; ++ch) {
        for (uint8_t op = 0; op < 4; ++op) {
            const uint8_t reg = ch + op * 4;
            write(0x30 + reg, op + 1); write(0x40 + reg, 8 + op * 9);
            write(0x50 + reg, 31); write(0x60 + reg, 0); write(0x70 + reg, 0); write(0x80 + reg, 15);
        }
        write(0xb0 + ch, ch | (5 << 3)); write(0xa4 + ch, 0x22); write(0xa0 + ch, 67 + ch * 23 + transpose);
        write(0x28, 0xf0 | ch);
        const uint16_t period = 127 + ch * 103 + transpose;
        write(ch * 2, period & 255); write(ch * 2 + 1, period >> 8); write(8 + ch, ch == 0 ? 16 : 14 - ch * 3);
    }
    write(6, 7); write(7, 0); write(11, 31); write(12, 0); write(13, 10);
}
static void program_ym2608(void *chip, uint32_t transpose) {
    native_ym2608_write(chip, 0, 0x29, 0x9f);
    for (uint8_t ch = 0; ch < 6; ++ch) {
        const uint8_t port = ch / 3, offset = ch % 3;
        auto write = [=](uint8_t reg, uint8_t value) { native_ym2608_write(chip, port, reg, value); };
        for (uint8_t op = 0; op < 4; ++op) {
            const uint8_t reg = offset + op * 4;
            write(0x30 + reg, op + 1); write(0x40 + reg, 8 + op * 9);
            write(0x50 + reg, 31); write(0x60 + reg, 0); write(0x70 + reg, 0); write(0x80 + reg, 15);
        }
        write(0xb0 + offset, ch | (5 << 3)); write(0xb4 + offset, 0xc0);
        write(0xa4 + offset, 0x22); write(0xa0 + offset, 67 + ch * 23 + transpose);
        native_ym2608_write(chip, 0, 0x28, 0xf0 | (ch < 3 ? ch : ch + 1));
    }
    for (uint8_t ch = 0; ch < 3; ++ch) {
        const uint16_t period = 127 + ch * 103 + transpose;
        native_ym2608_write(chip, 0, ch * 2, period & 255); native_ym2608_write(chip, 0, ch * 2 + 1, period >> 8);
        native_ym2608_write(chip, 0, 8 + ch, ch == 0 ? 16 : 14 - ch * 3);
    }
    native_ym2608_write(chip, 0, 6, 7); native_ym2608_write(chip, 0, 7, 0);
    native_ym2608_write(chip, 0, 11, 31); native_ym2608_write(chip, 0, 13, 10);
    // Deterministic ROM/RAM exercise all decoders; memory is reserved once.
    uint8_t bytes[8192]; for (uint32_t i = 0; i < sizeof bytes; ++i) bytes[i] = (i * 13 + 7) & 255;
    assert(native_ym2608_load_rom(chip, bytes, 0, sizeof bytes));
    assert(native_ym2608_load_memory(chip, bytes, 0, sizeof bytes));
    native_ym2608_write(chip, 0, 0x11, 52);
    for (uint8_t ch = 0; ch < 6; ++ch) native_ym2608_write(chip, 0, 0x18 + ch, 0xdf - ch);
    native_ym2608_write(chip, 0, 0x10, 63);
    native_ym2608_write(chip, 1, 1, 0xc2); native_ym2608_write(chip, 1, 2, 0); native_ym2608_write(chip, 1, 3, 0);
    native_ym2608_write(chip, 1, 4, 0xff); native_ym2608_write(chip, 1, 5, 0);
    native_ym2608_write(chip, 1, 9, 0); native_ym2608_write(chip, 1, 10, 0x40); native_ym2608_write(chip, 1, 11, 180);
    native_ym2608_write(chip, 1, 0, 0xb0);
}
static void program_ym2610(void *chip, uint32_t transpose, std::vector<void *> &pages) {
    for (uint8_t type = 0; type < 2; ++type) {
        auto *ptr = static_cast<uint8_t *>(native_ym2610b_alloc_rom_page()); assert(ptr); pages.push_back(ptr);
        for (uint32_t i = 0; i < 65536; ++i) ptr[i] = uint8_t(i * 13 + 7);
        assert(native_ym2610b_install_rom_page(chip, type, 0, ptr));
    }
    for (uint8_t ch : {1, 2, 4, 5}) {
        const uint8_t port = ch / 3, offset = ch % 3;
        auto write = [=](uint8_t reg, uint8_t value) { native_ym2610b_write(chip, port, reg, value); };
        for (uint8_t op = 0; op < 4; ++op) {
            const uint8_t reg = offset + op * 4;
            write(0x30 + reg, op + 1); write(0x40 + reg, 8 + op * 9); write(0x50 + reg, 31);
            write(0x60 + reg, 0); write(0x70 + reg, 0); write(0x80 + reg, 15);
        }
        write(0xb0 + offset, ch | (5 << 3)); write(0xb4 + offset, 0xc0); write(0xa4 + offset, 0x22); write(0xa0 + offset, 67 + ch * 23 + transpose);
        native_ym2610b_write(chip, 0, 0x28, 0xf0 | (ch < 3 ? ch : ch + 1));
    }
    for (uint8_t ch = 0; ch < 3; ++ch) {
        const uint16_t period = 127 + ch * 103 + transpose;
        native_ym2610b_write(chip, 0, ch * 2, period & 255); native_ym2610b_write(chip, 0, ch * 2 + 1, period >> 8); native_ym2610b_write(chip, 0, 8 + ch, ch == 0 ? 16 : 14 - ch * 3);
    }
    for (const auto reg : {6, 7, 11, 13}) native_ym2610b_write(chip, 0, reg, reg == 6 ? 7 : reg == 7 ? 0 : reg == 11 ? 31 : 10);
    native_ym2610b_write(chip, 1, 1, 52);
    for (uint8_t ch = 0; ch < 6; ++ch) { native_ym2610b_write(chip, 1, 8 + ch, 0xdf - ch); native_ym2610b_write(chip, 1, 0x10 + ch, ch); native_ym2610b_write(chip, 1, 0x20 + ch, ch); }
    native_ym2610b_write(chip, 1, 0, 63);
    native_ym2610b_write(chip, 0, 0x11, 0xc0); native_ym2610b_write(chip, 0, 0x14, 31); native_ym2610b_write(chip, 0, 0x19, 0); native_ym2610b_write(chip, 0, 0x1a, 0x40); native_ym2610b_write(chip, 0, 0x1b, 180); native_ym2610b_write(chip, 0, 0x10, 0xb0);
}
static void program_ym2151(void *chip, uint32_t transpose) {
    for (uint8_t ch = 0; ch < 8; ++ch) {
        for (uint8_t op = 0; op < 4; ++op) {
            const uint8_t n = ch + op * 8;
            native_ym2151_write(chip, 0x40 + n, op + 1); native_ym2151_write(chip, 0x60 + n, 12 + op * 7);
            native_ym2151_write(chip, 0x80 + n, 31); native_ym2151_write(chip, 0xa0 + n, 0x80); native_ym2151_write(chip, 0xc0 + n, 0); native_ym2151_write(chip, 0xe0 + n, 15);
        }
        native_ym2151_write(chip, 0x20 + ch, 0xc0 | (5 << 3) | ch); native_ym2151_write(chip, 0x28 + ch, 0x30 + ch * 3 + transpose);
        native_ym2151_write(chip, 0x30 + ch, ch * 16); native_ym2151_write(chip, 0x38 + ch, 0x73); native_ym2151_write(chip, 8, ch | 0x78);
    }
    native_ym2151_write(chip, 0x0f, 0x91); native_ym2151_write(chip, 0x18, 237); native_ym2151_write(chip, 0x19, 67); native_ym2151_write(chip, 0x19, 221); native_ym2151_write(chip, 0x1b, 3);
}
static void program_ym2413(void *chip, uint32_t transpose) {
    for (uint8_t ch = 0; ch < 9; ++ch) {
        native_ym2413_write(chip, 0x30 + ch, ((ch % 15 + 1) << 4) | 3);
        native_ym2413_write(chip, 0x10 + ch, 127 + ch * 13 + transpose);
        native_ym2413_write(chip, 0x20 + ch, 0x18);
    }
    native_ym2413_write(chip, 0x36, 2); native_ym2413_write(chip, 0x37, 0x43); native_ym2413_write(chip, 0x38, 0x65); native_ym2413_write(chip, 0x0e, 63);
}
static void program_ay8910(void *chip, uint32_t transpose) {
    for (uint8_t ch = 0; ch < 3; ++ch) {
        const uint16_t period = 127 + ch * 103 + transpose;
        native_ay8910_write(chip, ch * 2, period & 255); native_ay8910_write(chip, ch * 2 + 1, period >> 8);
        native_ay8910_write(chip, 8 + ch, ch == 0 ? 16 : 14 - ch * 3);
    }
    native_ay8910_write(chip, 6, 7); native_ay8910_write(chip, 7, 0); native_ay8910_write(chip, 11, 31); native_ay8910_write(chip, 13, 10);
}
static void program_extra_opl(void *chip, uint32_t transpose, uint8_t kind) {
    const uint8_t slots[9] = {0,1,2,8,9,10,16,17,18};
    native_extra_opl_write(chip,1,32);
    for (uint8_t ch=0;ch<9;++ch) {
        for(uint8_t slot:{slots[ch],uint8_t(slots[ch]+3)}) {
            native_extra_opl_write(chip,0x20+slot,0xe1);native_extra_opl_write(chip,0x40+slot,12);native_extra_opl_write(chip,0x60+slot,0xf5);native_extra_opl_write(chip,0x80+slot,0x58);native_extra_opl_write(chip,0xe0+slot,kind==1?ch%4:0);
        }
        native_extra_opl_write(chip,0xc0+ch,(ch&1)|10);native_extra_opl_write(chip,0xa0+ch,127+ch*13+transpose);native_extra_opl_write(chip,0xb0+ch,0x31);
    }
    native_extra_opl_write(chip,0xbd,255);
    if(kind==3) {native_extra_opl_write(chip,8,15);native_extra_opl_write(chip,11,31);native_extra_opl_write(chip,16,255);native_extra_opl_write(chip,17,255);native_extra_opl_write(chip,18,220);native_extra_opl_write(chip,7,0xb0);}
}
int main(int argc, char **argv) {
    if (argc < 4 || argc > 13) return 2;
    const uint32_t fm_count = std::strtoul(argv[1], nullptr, 10), psg_count = std::strtoul(argv[2], nullptr, 10);
    const uint32_t frames = std::strtoul(argv[3], nullptr, 10), rate = 48000;
    const uint32_t opn_count = argc >= 5 ? std::strtoul(argv[4], nullptr, 10) : 0;
    const uint32_t opna_count = argc >= 6 ? std::strtoul(argv[5], nullptr, 10) : 0;
    const uint32_t opnb_count = argc >= 7 ? std::strtoul(argv[6], nullptr, 10) : 0;
    const uint32_t opm_count = argc >= 8 ? std::strtoul(argv[7], nullptr, 10) : 0;
    const uint32_t opll_count = argc >= 9 ? std::strtoul(argv[8], nullptr, 10) : 0;
    const uint32_t ay_count = argc >= 10 ? std::strtoul(argv[9], nullptr, 10) : 0;
    const uint32_t opl1_count = argc >= 11 ? std::strtoul(argv[10], nullptr, 10) : 0;
    const uint32_t opl2_count = argc >= 12 ? std::strtoul(argv[11], nullptr, 10) : 0;
    const uint32_t y8950_count = argc >= 13 ? std::strtoul(argv[12], nullptr, 10) : 0;
    if (fm_count + psg_count + opn_count + opna_count + opnb_count + opm_count + opll_count + ay_count + opl1_count + opl2_count + y8950_count == 0 || fm_count + psg_count + opn_count + opna_count + opnb_count + opm_count + opll_count + ay_count + opl1_count + opl2_count + y8950_count > 16 || !frames || frames > rate * 60) return 2;
    std::vector<void *> fm, psg, opn, opna, opnb, opm, opll, ay, opl1, opl2, y8950, pages; fm.reserve(fm_count); psg.reserve(psg_count); opn.reserve(opn_count); opna.reserve(opna_count); opnb.reserve(opnb_count); pages.reserve(opnb_count * 2); opm.reserve(opm_count); opll.reserve(opll_count); ay.reserve(ay_count); opl1.reserve(opl1_count); opl2.reserve(opl2_count); y8950.reserve(y8950_count);
    const auto baseline_heap = live_bytes;
    const auto baseline_rss = peak_rss();
    for (uint32_t i = 0; i < fm_count; ++i) { auto *chip = native_ym2612_create(rate); assert(chip); fm.push_back(chip); program_fm(chip, i); }
    for (uint32_t i = 0; i < psg_count; ++i) { auto *chip = native_game_gear_create(rate); assert(chip); psg.push_back(chip); program_psg(chip, i); }
    for (uint32_t i = 0; i < opn_count; ++i) { auto *chip = native_ym2203_create_extra(rate); assert(chip); opn.push_back(chip); program_ym2203(chip, i); }
    for (uint32_t i = 0; i < opna_count; ++i) { auto *chip = native_ym2608_create_extra(rate); assert(chip); opna.push_back(chip); program_ym2608(chip, i); }
    for (uint32_t i = 0; i < opnb_count; ++i) { auto *chip = native_ym2610b_create_extra(rate); assert(chip); opnb.push_back(chip); program_ym2610(chip, i, pages); }
    for (uint32_t i = 0; i < opm_count; ++i) { auto *chip = native_ym2151_create(rate); assert(chip); opm.push_back(chip); program_ym2151(chip, i); }
    for (uint32_t i = 0; i < opll_count; ++i) { auto *chip = native_ym2413_create(rate); assert(chip); opll.push_back(chip); program_ym2413(chip, i); }
    for (uint32_t i = 0; i < ay_count; ++i) { auto *chip = native_ay8910_create(rate); assert(chip); ay.push_back(chip); program_ay8910(chip, i); }
    for(uint8_t kind:{0,1,3}) { auto &list=kind==0?opl1:kind==1?opl2:y8950; const uint32_t count=kind==0?opl1_count:kind==1?opl2_count:y8950_count;
        for(uint32_t i=0;i<count;++i) {void *chip=native_extra_opl_create(rate,kind);assert(chip);list.push_back(chip);program_extra_opl(chip,i,kind);} }
    const auto core_heap_bytes = live_bytes - baseline_heap;
    volatile double checksum = 0;
    auto render = [&](uint32_t n) {
        for (uint32_t frame = 0; frame < n; ++frame) {
            float mixed_left = 0, mixed_right = 0, l, r;
            for (void *chip : fm) { native_ym2612_sample(chip, &l, &r); mixed_left += l; mixed_right += r; }
            for (void *chip : psg) { native_game_gear_sample(chip, &l, &r); mixed_left += l; mixed_right += r; }
            for (void *chip : opn) { native_ym2203_sample(chip, &l, &r); mixed_left += l; mixed_right += r; }
            for (void *chip : opna) { native_ym2608_sample(chip, &l, &r); mixed_left += l; mixed_right += r; }
            for (void *chip : opnb) { native_ym2610b_sample(chip, &l, &r); mixed_left += l; mixed_right += r; }
            for (void *chip : opm) { native_ym2151_sample(chip, &l, &r); mixed_left += l; mixed_right += r; }
            for (void *chip : opll) { native_ym2413_sample(chip, &l, &r); mixed_left += l; mixed_right += r; }
            for (void *chip : ay) { native_ay8910_sample(chip, &l, &r); mixed_left += l; mixed_right += r; }
            for(const auto *list:{&opl1,&opl2,&y8950})for(void *chip:*list){native_extra_opl_sample(chip,&l,&r);mixed_left+=l;mixed_right+=r;}
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
    for (void *chip : opn) native_ym2203_destroy(chip);
    for (void *chip : opna) native_ym2608_destroy(chip);
    for (void *chip : opnb) native_ym2610b_destroy(chip);
    for (void *chip : opm) native_ym2151_destroy(chip);
    for (void *chip : opll) native_ym2413_destroy(chip);
    for (void *chip : ay) native_ay8910_destroy(chip);
    for(const auto *list:{&opl1,&opl2,&y8950})for(void *chip:*list)native_extra_opl_destroy(chip);
    for (void *page : pages) native_ym2610b_free_rom_page(page);
    assert(live_bytes == baseline_heap && callback_allocations == 0 && callback_frees == 0);
    std::printf("{\"ym2612\":%u,\"segapsg\":%u,\"ym2203\":%u,\"ym2608\":%u,\"ym2610\":%u,\"ym2151\":%u,\"ym2413\":%u,\"ay8910\":%u,\"ym3526\":%u,\"ym3812\":%u,\"y8950\":%u,\"sampleRate\":%u,\"frames\":%u,\"cpuSeconds\":%.9f,\"wallSeconds\":%.9f,\"coreHeapBytes\":%zu,\"romBytes\":%u,\"ramBytes\":%u,\"ym2610RomPageBytes\":%u,\"coreHeapExcludingMemoryBytes\":%zu,\"peakRssBytes\":%llu,\"peakRssGrowthBytes\":%llu,\"renderAllocations\":%llu,\"renderFrees\":%llu,\"checksum\":%.9f}\n",
        fm_count, psg_count, opn_count, opna_count, opnb_count, opm_count, opll_count, ay_count, opl1_count, opl2_count, y8950_count, rate, frames, cpu_seconds, wall_seconds, core_heap_bytes,
        opna_count * 8192, opna_count * 2097152, opnb_count * 2 * 65536, core_heap_bytes - opna_count * (8192 + 2097152) - opnb_count * 2 * 65536,
        static_cast<unsigned long long>(peak), static_cast<unsigned long long>(peak > baseline_rss ? peak - baseline_rss : 0),
        static_cast<unsigned long long>(callback_allocations), static_cast<unsigned long long>(callback_frees), double(checksum));
    return 0;
}
