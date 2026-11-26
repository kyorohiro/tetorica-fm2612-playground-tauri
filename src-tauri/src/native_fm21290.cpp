// SPDX-License-Identifier: BSD-3-Clause
#include "native_genesis_dx7.h"
// Embed the pinned SDK core so slot retirement and panic can reset DSP state
// in place on the audio callback, without allocating a replacement chip.
#include "../vendor/tetorica-soundchip-native/native/src/fm21290.cpp"
namespace {
#include "native_fm21290_tables.h"
int attenuation(int level) { return level < 20 ? lowLevelAttenuations[level] : 99 - level; }
int keycode(int note) { return std::max(0, (note - 1) * 4 / 3); }
int scaled(int value) { return value * 165 >> 6; }
struct Key {
    int note = -1, velocity = 0, pitch = 0, last = 0, level = 0, stage = 5;
};
struct Instrument {
    fm21290_chip chip;
    std::array<uint8_t, 155> patch{};
    std::array<Key, 16> keys{};
    // Preserve output-level edits through release, even while muted. The SDK
    // otherwise retires a slot as soon as its attenuated sample is inaudible.
    std::array<std::array<double, 6>, 16> releaseLevel{};
    unsigned releasing = 0, cursor = 0;
    uint64_t timer = 0;
    int group = 0, phase = 0, random = 0, wave = 0, fade = 255, delayPhase = 65535;
    explicit Instrument(unsigned rate) : chip(rate) {}
    void word(unsigned address, int value) {
        value = std::clamp(value, 0, 65535);
        chip.write(address, value >> 8);
        chip.write(address + 1, value & 255);
    }
    int code(int ch) const {
        return keycode(std::min(127, (keys[ch].note + patch[144] - 24) & 255));
    }
    void output(int ch, int op) {
        const auto *p = patch.data() + op * 21;
        const int distance = (code(ch) >> 2) + 1 - (keycode(p[8] + 20) >> 2);
        const int curve = p[distance > 0 ? 12 : 11], depth = p[distance > 0 ? 10 : 9];
        const int weight = keyCurveBytes[(curve == 0 || curve == 3 ? 36 : 0) + std::abs(distance)];
        const int adjustment = std::min(127, ((depth * 660 >> 8) * weight) >> 8);
        const int base =
            std::clamp(attenuation(p[16]) + (curve < 2 ? adjustment : -adjustment), 0, 127);
        const int velocity = velocityScale[keyTravel[keys[ch].velocity >> 2] >> 2];
        chip.write(0x3080 + op * 16 + ch,
                   std::clamp(2 * base + 15 - 2 * p[15] + ((velocity * p[15]) >> 3), 4, 255));
    }
    void voice(const uint8_t *bytes) {
        const bool oldAm = patch[140], oldPm = patch[139] && patch[143];
        std::copy_n(bytes, 155, patch.begin());
        chip.write(0x2804, patch[136] ? 0x30 : 0x50);
        chip.write(0x2805, patch[134] * 8 + patch[135]);
        for (int op = 0; op < 6; ++op) {
            const auto *p = bytes + op * 21;
            const double freq = p[17] ? std::pow(10.0, (p[18] & 3) + p[19] / 100.0)
                                      : (p[18] ? p[18] : 0.5) * (1 + p[19] / 100.0);
            const double pitch = (p[17] ? 0x16ac : 0x232c) + 4096 * std::log2(freq);
            word(0x3020 + 2 * op, static_cast<int>(std::floor(pitch / 4 + 0.5)) * 4 + p[17]);
            chip.write(0x3030 + op, p[20] < 7 ? 15 - p[20] : p[20] - 7);
            chip.write(0x30e0 + op, p[14] * 8 + p[13]);
            for (int i = 0; i < 4; ++i) {
                chip.write(0x3040 + 4 * op + i, p[i] * 164 >> 8);
                chip.write(0x3060 + 4 * op + i, attenuation(p[4 + i]) >> 1);
            }
            for (int ch = 0; ch < 16; ++ch)
                if (keys[ch].note >= 0)
                    output(ch, op);
        }
        if (!patch[138]) {
            fade = 255;
            delayPhase = 65535;
        }
        if (oldAm && !patch[140])
            chip.write(0x30f0, 0);
        if (oldPm && !(patch[139] && patch[143]))
            word(0x30f2, 0);
    }
    void publishPitch(int ch) {
        auto &k = keys[ch];
        if (k.note < 0)
            return;
        const int pitch = std::clamp(k.pitch + k.level - 16384, 0, 65535);
        if (pitch != k.last) {
            word(0x3000 + 2 * ch, pitch);
            k.last = pitch;
        }
    }
    int on(int note, int velocity, unsigned preserve) {
        for (int i = 0; i < 16; ++i) {
            const int ch = (cursor + i) % 16;
            if (chip.voices[ch].held || (preserve & (1u << ch)))
                continue;
            releasing &= ~(1u << ch);
            cursor = (ch + 1) % 16;
            auto &k = keys[ch];
            k.note = note;
            k.velocity = velocity;
            const int c = code(ch);
            k.pitch = k.last = c * 256 + (c & 3) * 84;
            word(0x3000 + 2 * ch, k.pitch);
            for (int op = 0; op < 6; ++op)
                output(ch, op);
            k.stage = 0;
            k.level = levelTable[patch[133]] << 7;
            publishPitch(ch);
            if (patch[141])
                phase = 0x7fff;
            if (patch[138]) {
                fade = 0;
                delayPhase = 0;
            }
            chip.write(0x30f1, ch * 4 + 1);
            return ch;
        }
        return -1;
    }
    void off(int ch) {
        if (chip.voices[ch].held) {
            releaseLevel[ch] = chip.voices[ch].level;
            releasing |= 1u << ch;
        }
        chip.write(0x30f1, ch * 4 + 2);
        if (keys[ch].stage < 4)
            keys[ch].stage = 4;
    }
    void tick() {
        const int speed = std::max(1, scaled(patch[137])), old = phase;
        phase = (phase + speed * (11 + std::max(0, (speed - 160) >> 2))) & 65535;
        const int hi = phase >> 8;
        switch (patch[142]) {
        case 0:
            wave = (((phase >> 7) ^ (hi & 128 ? 255 : 0)) + 128) & 255;
            break;
        case 1:
            wave = 255 - hi;
            break;
        case 2:
            wave = hi;
            break;
        case 3:
            wave = hi & 128 ? 128 : 127;
            break;
        case 4:
            wave = sine[(hi & 63) ^ (hi & 64 ? 63 : 0)] ^ (hi & 128 ? 255 : 0);
            break;
        case 5:
            if (old < 32768 && phase >= 32768)
                wave = random = (random * 179 + 17) & 255;
            break;
        }
        const int delay = 99 - patch[138], inc = (16 + (delay & 15)) << (2 + (delay >> 4));
        delayPhase += inc;
        if (delayPhase > 65535) {
            delayPhase = 65535;
            fade = std::min(255, fade + std::max(1, inc >> 8));
        }
        if (patch[140]) {
            const int depth = std::min(255, scaled(patch[140]) * fade >> 8);
            chip.write(0x30f0, ampTable[255 - ((depth * (wave ^ 127)) >> 8)]);
        }
        for (int ch = group; ch < group + 8; ++ch)
            publishPitch(ch);
        group ^= 8;
        if (group)
            for (auto &k : keys) {
                if (k.note < 0 || k.stage == 3 || k.stage == 5)
                    continue;
                const int i = std::min(k.stage, 3), target = levelTable[patch[130 + i]] << 7;
                const bool rising = k.level < target;
                const int next = k.level + (rising ? 1 : -1) * rateTable[patch[126 + i]];
                if (rising ? (next >> 8) < (target >> 8) : (next >> 8) > (target >> 8))
                    k.level = next;
                else {
                    k.level = target;
                    ++k.stage;
                }
            }
        if (patch[139] && patch[143]) {
            const int pitch = ((wave ^ 128) - 128) * sensitivities[patch[143]] / 256;
            word(0x30f2, ((pitch * std::min(255, scaled(patch[139]) * fade >> 8)) >> 1) & 65535);
        }
    }
    float sample() {
        float pcm;
        chip.render(&pcm, 1);
        for (unsigned ch = 0; releasing && ch < 16; ++ch) {
            const unsigned bit = 1u << ch;
            if (!(releasing & bit))
                continue;
            auto &v = chip.voices[ch];
            if (v.active)
                continue;
            for (int op = 0; op < 6; ++op)
                if (v.gain[op] * releaseLevel[ch][op] > 1e-5) {
                    v.active = true;
                    break;
                }
            if (!v.active)
                releasing &= ~bit;
        }
        timer += 2356625;
        const uint64_t period = uint64_t(chip.rate) * 6280;
        if (timer >= period) {
            timer -= period;
            tick();
        }
        return pcm;
    }
};
} // namespace
extern "C" {
void *native_dx7_create(uint32_t rate) {
    return rate >= 8000 && rate <= 384000 ? new (std::nothrow) Instrument(rate) : nullptr;
}
void native_dx7_destroy(void *p) { delete static_cast<Instrument *>(p); }
void native_dx7_voice(void *p, const uint8_t *v, uint32_t, bool) {
    static_cast<Instrument *>(p)->voice(v);
}
int native_dx7_note_on_mask(void *p, uint8_t note, uint8_t velocity, uint16_t mask) {
    return static_cast<Instrument *>(p)->on(note, velocity, mask);
}
int native_dx7_note_on(void *p, uint8_t n, uint8_t v) {
    return native_dx7_note_on_mask(p, n, v, 0);
}
void native_dx7_release_slot(void *p, uint8_t ch) { static_cast<Instrument *>(p)->off(ch); }
void native_dx7_kill_slot(void *p, uint8_t ch) {
    auto &s = *static_cast<Instrument *>(p);
    s.off(ch);
    s.releasing &= ~(1u << ch);
    s.keys[ch] = Key{};
    auto &v = s.chip.voices[ch];
    v.active = false;
    v.gain.fill(0);
    v.idle_since = s.chip.frame;
}
void native_dx7_write(void *p, uint16_t address, uint8_t value) {
    auto &s = *static_cast<Instrument *>(p);
    if (address == 0x30f1) {
        const int ch = (value >> 2) & 15;
        if ((value & 3) == 1) {
            s.keys[ch] = Key{};
            s.releasing &= ~(1u << ch);
        }
        if ((value & 3) == 2) {
            s.off(ch);
            return;
        }
    }
    s.chip.write(address, value);
}
void native_dx7_panic(void *p) {
    auto *s = static_cast<Instrument *>(p);
    const unsigned rate = s->chip.rate;
    s->~Instrument();
    new (p) Instrument(rate);
}
float native_dx7_sample(void *p) { return static_cast<Instrument *>(p)->sample(); }
size_t native_dx7_core_size() { return sizeof(Instrument); }
}
