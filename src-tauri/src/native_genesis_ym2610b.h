#pragma once
#include <cstdint>

// Applied register/status view. Inspection does not read ADPCM data or change IRQ.
struct NativeYm2610BState {
    uint8_t registers[512];
    uint32_t clock;
    uint32_t source_mute_mask;
    uint8_t status;
    uint8_t status_hi;
    uint8_t irq;
    uint8_t variant;
};

extern "C" {
void *native_ym2610b_create(uint32_t output_rate, uint8_t variant);
void native_ym2610b_destroy(void *chip);
// Variant: 0 = YM2610 (physical FM channels 1,2,4,5), 1 = YM2610B (all six).
// Reset retains both independently uploaded ADPCM ROM banks, clock and source mute.
void native_ym2610b_reset(void *chip);
// Stop clears active voices while retaining their authored controls and memory.
void native_ym2610b_stop(void *chip);
void native_ym2610b_key_off(void *chip, uint8_t voice);
void native_ym2610b_write(void *chip, uint8_t port, uint8_t reg, uint8_t value);
void native_ym2610b_sample(void *chip, float *left, float *right);
int native_ym2610b_set_clock(void *chip, uint32_t clock);
void native_ym2610b_set_source_mute_mask(void *chip, uint32_t mask);
// type: 0 = ADPCM-A, 1 = ADPCM-B. Each bank reserves 16 MiB at create.
int native_ym2610b_load_rom(void *chip, uint8_t type, const uint8_t *bytes, uint32_t offset, uint32_t size);
void native_ym2610b_clear_rom(void *chip, uint8_t type);
int native_ym2610b_copy_rom(void *chip, uint8_t type, uint8_t *bytes, uint32_t offset, uint32_t size);
uint8_t native_ym2610b_read(void *chip, uint8_t offset);
uint8_t native_ym2610b_read_status(void *chip);
uint8_t native_ym2610b_read_status_hi(void *chip);
uint8_t native_ym2610b_get_irq(void *chip);
void native_ym2610b_get_state(void *chip, NativeYm2610BState *state);
}
