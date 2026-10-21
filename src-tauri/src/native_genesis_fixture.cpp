// Hardware-free comparison tool. This file is not linked into the app.
#include <algorithm>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <iostream>
#include <vector>
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
    if (argc != 3) return 2;
    const auto rate = static_cast<uint32_t>(std::strtoul(argv[1], nullptr, 10));
    const auto frames = static_cast<uint32_t>(std::strtoul(argv[2], nullptr, 10));
    if (rate < 8000 || rate > 384000 || frames > 3840000) return 2;
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
    size_t offset = 0;
    for (uint32_t frame = 0; frame < frames; ++frame) {
        while (offset < commands.size() && commands[offset].frame == frame) {
            const auto &event = commands[offset++];
            if (event.kind == 'w') native_genesis_write(core, event.port, event.reg, event.value);
            else if (event.kind == 'p') native_genesis_psg_write(core, event.value);
            else if (event.kind == 's') native_genesis_psg_stereo(core, event.value);
            else native_genesis_reset(core);
        }
        float pcm[2]; native_genesis_sample(core, pcm);
        if (std::fwrite(pcm, sizeof(float), 2, stdout) != 2) return 1;
    }
    native_genesis_destroy(core);
    return 0;
}
