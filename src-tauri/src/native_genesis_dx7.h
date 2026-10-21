#pragma once
#include <stdint.h>
extern "C" {
void *native_dx7_create(uint32_t rate);
void native_dx7_destroy(void *ptr);
void native_dx7_voice(void *ptr, const uint8_t *voice, uint32_t id, bool live);
int native_dx7_note_on(void *ptr, uint8_t note, uint8_t velocity);
void native_dx7_release_slot(void *ptr, uint8_t slot);
void native_dx7_panic(void *ptr);
float native_dx7_sample(void *ptr);
}
