// Offline PCM comparison fixture. No audio device is opened.
// stdin: <frame> voice <id> <live0or1> <155-byte-hex>, on <midi> <velocity>,
// off <midi>, alloff, panic. stdout: interleaved float32 stereo PCM.
#include "native_genesis_dx7.h"
#include <array>
#include <algorithm>
#include <cassert>
#include <cmath>
#include <cstddef>
#include <cstdint>
#include <cstdlib>
#include <iostream>
#include <new>
#include <sstream>
#include <stdexcept>
#include <string>
#include <vector>

// Request-size accounting only, excluding allocator headers/runtime globals.
struct alignas(std::max_align_t) Allocation { size_t size; };
static size_t heap_bytes = 0;
static uint64_t allocations = 0, frees = 0;
static int fail_after = -1;
void *operator new(size_t size) {
  if (fail_after == 0) throw std::bad_alloc();
  if (fail_after > 0) --fail_after;
  auto *block = static_cast<Allocation *>(std::malloc(sizeof(Allocation) + size));
  if (!block) throw std::bad_alloc();
  block->size = size; heap_bytes += size; ++allocations; return block + 1;
}
void *operator new[](size_t size) { return ::operator new(size); }
void operator delete(void *ptr) noexcept {
  if (!ptr) return;
  auto *block = static_cast<Allocation *>(ptr) - 1;
  heap_bytes -= block->size; ++frees; std::free(block);
}
void operator delete[](void *ptr) noexcept { ::operator delete(ptr); }
void operator delete(void *ptr, size_t) noexcept { ::operator delete(ptr); }
void operator delete[](void *ptr, size_t) noexcept { ::operator delete(ptr); }

static std::array<uint8_t, 155> sine_voice() {
  std::array<uint8_t, 155> voice{};
  for (int op = 0; op < 6; ++op) {
    auto *p = voice.data() + op * 21;
    p[0] = p[1] = p[2] = 99; p[3] = 75;
    p[4] = p[5] = p[6] = 99; p[7] = 0; p[8] = 39;
    p[16] = op == 5 ? 85 : 0; p[18] = 1; p[20] = 7;
  }
  for (int i = 126; i < 130; ++i) voice[i] = 99;
  for (int i = 130; i < 134; ++i) voice[i] = 50;
  voice[134] = 31; voice[136] = 1; voice[141] = 1;
  voice[144] = 24;
  for (int i = 145; i < 155; ++i) voice[i] = 32;
  return voice;
}

static int self_test() {
  auto *warm = native_dx7_create(48000); assert(warm); native_dx7_destroy(warm);
  const auto base = heap_bytes;
  for (int failure = 0; failure < 17; ++failure) {
    fail_after = failure;
    assert(!native_dx7_create(48000));
    fail_after = -1;
    assert(heap_bytes == base); // Including all partially allocated note arrays.
  }
  auto *a = native_dx7_create(48000), *b = native_dx7_create(48000), *other = native_dx7_create(96000);
  assert(a && b && other);
  const auto core_bytes = native_dx7_core_size();
  assert(heap_bytes - base == core_bytes * 3);
  auto voice = sine_voice(), distinct = voice;
  distinct[137] = 78; distinct[139] = 31; distinct[140] = 22;
  distinct[142] = 4; distinct[143] = 3; distinct[123] = 3;
  native_dx7_voice(a, voice.data(), 1, false); native_dx7_voice(b, voice.data(), 1, false);
  native_dx7_voice(other, distinct.data(), 71, false);
  assert(native_dx7_note_on(a, 69, 100) == 0 && native_dx7_note_on(b, 69, 100) == 0);
  assert(native_dx7_note_on(other, 33, 127) == 0);
  double energy = 0;
  const auto before_alloc = allocations, before_free = frees;
  for (int frame = 0; frame < 16000; ++frame) {
    if (frame == 701) { voice[123] = 2; native_dx7_voice(a, voice.data(), 1, true); native_dx7_voice(b, voice.data(), 1, true); }
    if (frame == 1403) { native_dx7_release_slot(a, 0); native_dx7_release_slot(b, 0); }
    if (frame == 2101) { native_dx7_panic(a); native_dx7_panic(b); native_dx7_note_on(a, 60, 80); native_dx7_note_on(b, 60, 80); }
    if (frame % 127 == 0) { distinct[135] = (frame / 127) & 7; native_dx7_voice(other, distinct.data(), 71, true); }
    const float left = native_dx7_sample(a), expected = native_dx7_sample(b);
    assert(std::isfinite(left) && left == expected); energy += left * left;
    // Rendering a different-rate instance advances its own LFO/filter/source
    // cache and cannot mutate the lookup tables or either reference instance.
    for (int i = 0; i < (frame % 3) + 1; ++i) assert(std::isfinite(native_dx7_sample(other)));
  }
  assert(energy > 0.01);
  native_dx7_panic(a);
  for (int slot = 0; slot < 16; ++slot) assert(native_dx7_note_on(a, 48 + slot, 100) == slot);
  assert(native_dx7_note_on(a, 76, 100) == -1);
  for (int slot = 0; slot < 16; ++slot) native_dx7_release_slot(a, slot);
  assert(native_dx7_note_on_mask(a, 76, 100, 0xffff) == -1);
  assert(native_dx7_note_on_mask(a, 76, 100, 0xfffe) == 0);
  native_dx7_kill_slot(a, 0);
  assert(native_dx7_note_on_mask(a, 77, 100, 0xfffe) == 0);
  for (int slot = 0; slot < 16; ++slot) native_dx7_kill_slot(a, slot);
  for (int frame = 0; frame < 4096; ++frame) assert(std::isfinite(native_dx7_sample(a)));
  native_dx7_panic(a);
  for (int frame = 0; frame < 257; ++frame) assert(native_dx7_sample(a) == 0);
  assert(allocations == before_alloc && frees == before_free);
  native_dx7_destroy(a); native_dx7_destroy(b); native_dx7_destroy(other);
  assert(heap_bytes == base);
  std::cout << "{\"status\":\"passed\",\"coreHeapBytes\":" << core_bytes
    << ",\"allocatedNoteObjects\":16,\"partialCreateFailures\":17,\"renderAllocations\":0,\"renderFrees\":0,\"independentLfoFilterResampler\":true,\"protectedSlotAllocation\":true,\"singleSlotKill\":true}\n";
  return 0;
}

struct Event {
  uint32_t frame = 0, id = 0;
  int live = 0, note = 0, velocity = 0;
  std::string op;
  std::array<uint8_t, 155> voice{};
};
int main(int argc, char **argv) {
  void *core = nullptr;
  try {
    if (argc == 2 && std::string(argv[1]) == "--self-test") return self_test();
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
