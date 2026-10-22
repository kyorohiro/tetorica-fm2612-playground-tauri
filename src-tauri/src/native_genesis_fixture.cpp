// Hardware-free comparison tool. This file is not linked into the app.
#include <algorithm>
#include <chrono>
#include <cstdint>
#include <cstring>
#include <cstdio>
#include <cstdlib>
#include <iostream>
#include <vector>
#ifdef _WIN32
#include <fcntl.h>
#include <io.h>
#endif
extern "C" {
void *native_genesis_create(uint32_t);
void native_genesis_destroy(void *);
void native_genesis_reset(void *);
void native_genesis_write(void *, uint8_t, uint8_t, uint8_t);
void native_genesis_psg_write(void *, uint8_t);
void native_genesis_psg_stereo(void *, uint8_t);
void native_genesis_sample(void *, float *);
}
struct Command { char kind; uint32_t frame; uint32_t port = 0, reg = 0, value = 0; };
int main(int argc, char **argv) {
    const bool timing = argc == 4 && std::strcmp(argv[3], "--timing") == 0;
    if (argc != 3 && !timing) return 2;
    const auto setup_start = std::chrono::steady_clock::now();
#ifdef _WIN32
    if (_setmode(_fileno(stdout), _O_BINARY) == -1) return 1;
#endif
    const auto rate = static_cast<uint32_t>(std::strtoul(argv[1], nullptr, 10));
    const auto frames = static_cast<uint32_t>(std::strtoul(argv[2], nullptr, 10));
    if (rate < 8000 || rate > 384000 || static_cast<uint64_t>(frames) > static_cast<uint64_t>(rate) * 120) return 2;
    std::vector<Command> commands;
    Command command;
    while (std::cin >> command.kind >> command.frame) {
        if (command.kind == 'w') std::cin >> command.port >> command.reg >> command.value;
        else if (command.kind == 'p' || command.kind == 's') std::cin >> command.value;
        else if (command.kind != 'r') return 2;
        if (!std::cin || command.frame > frames || command.port > 1 || command.reg > 255 || command.value > 255) return 2;
        commands.push_back(command);
    }
    std::stable_sort(commands.begin(), commands.end(), [](const Command &a, const Command &b) { return a.frame < b.frame; });
    void *core = native_genesis_create(rate);
    if (!core) return 1;
    const auto render_start = std::chrono::steady_clock::now();
    size_t offset = 0;
    double render_ms = 0, write_ms = 0;
    float block[128][2];
    for (uint32_t start = 0; start < frames;) {
        const uint32_t count = std::min<uint32_t>(128, frames - start);
        const auto block_start = timing ? std::chrono::steady_clock::now() : render_start;
        for (uint32_t i = 0; i < count; ++i) {
            const uint32_t frame = start + i;
            while (offset < commands.size() && commands[offset].frame == frame) {
                const auto &event = commands[offset++];
                if (event.kind == 'w') native_genesis_write(core, event.port, event.reg, event.value);
                else if (event.kind == 'p') native_genesis_psg_write(core, event.value);
                else if (event.kind == 's') native_genesis_psg_stereo(core, event.value);
                else native_genesis_reset(core);
            }
            native_genesis_sample(core, block[i]);
        }
        const auto write_start = timing ? std::chrono::steady_clock::now() : render_start;
        if (std::fwrite(block, sizeof(float) * 2, count, stdout) != count) return 1;
        if (timing) {
            const auto write_end = std::chrono::steady_clock::now();
            render_ms += std::chrono::duration<double, std::milli>(write_start - block_start).count();
            write_ms += std::chrono::duration<double, std::milli>(write_end - write_start).count();
        }
        start += count;
    }
    const auto flush_start = timing ? std::chrono::steady_clock::now() : render_start;
    if (std::fflush(stdout) != 0) return 1;
    if (timing) write_ms += std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - flush_start).count();
    if (timing) {
        const auto end = std::chrono::steady_clock::now();
        std::fprintf(stderr, "{\"frames\":%u,\"nativeRenderMilliseconds\":%.6f,\"nativeWriteMilliseconds\":%.6f,\"nativeSetupMilliseconds\":%.6f,\"nativeStreamMilliseconds\":%.6f}\n",
            frames, render_ms, write_ms,
            std::chrono::duration<double, std::milli>(render_start - setup_start).count(),
            std::chrono::duration<double, std::milli>(end - render_start).count());
    }
    native_genesis_destroy(core);
    return 0;
}
