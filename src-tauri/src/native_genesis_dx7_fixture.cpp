// Offline PCM comparison fixture. No audio device is opened.
// stdin: <frame> voice <id> <live0or1> <155-byte-hex>, on <midi> <velocity>,
// off <midi>, alloff, panic. stdout: interleaved float32 stereo PCM.
#include "native_genesis_dx7.h"
#include <array>
#include <algorithm>
#include <cstdint>
#include <cstdlib>
#include <iostream>
#include <sstream>
#include <stdexcept>
#include <string>
#include <vector>

struct Event {
  uint32_t frame = 0, id = 0;
  int live = 0, note = 0, velocity = 0;
  std::string op;
  std::array<uint8_t, 155> voice{};
};
int main(int argc, char **argv) {
  void *core = nullptr;
  try {
    if (argc != 3) throw std::runtime_error("Usage: fixture sample-rate frames");
    const auto rate = static_cast<uint32_t>(std::stoul(argv[1]));
    const auto frames = static_cast<uint32_t>(std::stoul(argv[2]));
    if (!frames || frames > 16 * 1024 * 1024) throw std::runtime_error("Invalid frames");
    core = native_dx7_create(rate);
    if (!core) throw std::runtime_error("Could not create Native DX7");
    std::vector<Event> events;
    for (std::string line; std::getline(std::cin, line);) {
      if (line.empty()) continue;
      std::istringstream input(line); Event event;
      if (!(input >> event.frame >> event.op) || event.frame >= frames) throw std::runtime_error("Invalid event");
      if (event.op == "voice") {
        std::string hex;
        if (!(input >> event.id >> event.live >> hex) || !event.id || (event.live != 0 && event.live != 1) || hex.size() != 310) throw std::runtime_error("Invalid voice");
        for (size_t index = 0; index < event.voice.size(); ++index) {
          const auto value = std::stoul(hex.substr(index * 2, 2), nullptr, 16);
          if (value > 127) throw std::runtime_error("Invalid voice byte");
          event.voice[index] = static_cast<uint8_t>(value);
        }
      } else if (event.op == "on") {
        if (!(input >> event.note >> event.velocity) || event.note < 0 || event.note > 127 || event.velocity < 0 || event.velocity > 127) throw std::runtime_error("Invalid note-on");
      } else if (event.op == "off") {
        if (!(input >> event.note) || event.note < 0 || event.note > 127) throw std::runtime_error("Invalid note-off");
      } else if (event.op != "alloff" && event.op != "panic") throw std::runtime_error("Unknown event");
      events.push_back(event);
    }
    std::stable_sort(events.begin(), events.end(), [](const Event &left, const Event &right) { return left.frame < right.frame; });
    std::array<int, 16> noteForSlot{}; noteForSlot.fill(-1);
    const auto release = [&](int midi) {
      for (size_t slot = 0; slot < noteForSlot.size(); ++slot) if (midi < 0 || noteForSlot[slot] == midi) {
        native_dx7_release_slot(core, static_cast<uint8_t>(slot)); noteForSlot[slot] = -1;
      }
    };
    std::vector<float> pcm(static_cast<size_t>(frames) * 2);
    size_t next = 0;
    for (uint32_t frame = 0; frame < frames; ++frame) {
      while (next < events.size() && events[next].frame == frame) {
        const auto &event = events[next++];
        if (event.op == "voice") native_dx7_voice(core, event.voice.data(), event.id, event.live != 0);
        else if (event.op == "on" && event.velocity) {
          const int slot = native_dx7_note_on(core, static_cast<uint8_t>(event.note), static_cast<uint8_t>(event.velocity));
          // The shipped MSFA allocator drops an extra held key at capacity.
          if (slot >= 16) throw std::runtime_error("Invalid allocated slot");
          if (slot >= 0) noteForSlot[slot] = event.note;
        } else if (event.op == "off" || event.op == "on") release(event.note);
        else if (event.op == "alloff") release(-1);
        else if (event.op == "panic") { native_dx7_panic(core); noteForSlot.fill(-1); }
      }
      const auto sample = native_dx7_sample(core);
      pcm[frame * 2] = pcm[frame * 2 + 1] = sample;
    }
    native_dx7_destroy(core); core = nullptr;
    std::cout.write(reinterpret_cast<const char *>(pcm.data()), static_cast<std::streamsize>(pcm.size() * sizeof(float)));
    if (!std::cout) throw std::runtime_error("Could not write PCM");
    return 0;
  } catch (const std::exception &error) {
    if (core) native_dx7_destroy(core);
    std::cerr << error.what() << '\n'; return 1;
  }
}
