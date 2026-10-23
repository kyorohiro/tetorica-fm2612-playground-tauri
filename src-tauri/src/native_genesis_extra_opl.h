#pragma once
#include <cstdint>
struct NativeExtraOplState {
    uint8_t registers[256];
    uint32_t clock;
    uint8_t kind, channels, status, irq;
};
extern "C" {
// Same selected kind IDs as primary: 0 YM3526, 1 YM3812, 3 Y8950; OPL3 rejected.
void *native_extra_opl_create(uint32_t output_rate, uint8_t kind);
void native_extra_opl_destroy(void *);
void native_extra_opl_reset(void *);
void native_extra_opl_all_notes_off(void *);
void native_extra_opl_host_all_notes_off(void *);
void native_extra_opl_stop(void *);
void native_extra_opl_write(void *, uint16_t reg, uint8_t value);
// FM0..8 and BD/SD/TOM/CYM/HH18..22.
void native_extra_opl_key_off(void *, uint8_t voice);
void native_extra_opl_key_off_fm(void *, uint8_t channel);
void native_extra_opl_preview_key_off(void *, uint8_t voice);
void native_extra_opl_audition_key_on(void *, uint8_t voice);
void native_extra_opl_cancel_csm(void *);
uint8_t native_extra_opl_csm_active(void *);
void native_extra_opl_cancel_adpcm(void *);
void native_extra_opl_sample(void *, float *left, float *right);
int native_extra_opl_set_clock(void *, uint32_t clock);
uint8_t native_extra_opl_read(void *, uint8_t offset);
uint8_t native_extra_opl_read_status(void *);
uint8_t native_extra_opl_get_irq(void *);
void native_extra_opl_get_state(void *, NativeExtraOplState *);
}
