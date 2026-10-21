#pragma once
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif
// Offline prototype only: all operations must run on one dedicated owner
// thread. QuickJS can allocate, collect cycles and exceed its interrupt budget;
// this interface must never be called from the CPAL output callback.
void *native_genesis_livefx_create(uint32_t rate, uint32_t max_frames,
                                  size_t memory_limit, uint64_t budget_us);
void native_genesis_livefx_destroy(void *);
// action 0=register, 1=context, 2=remove. command is the existing LiveFX.command
// JSON object. An explicit contextGraph uses the shared bounded structured
// value codec; plain JSON context remains supported by the offline fixture.
int32_t native_genesis_livefx_command(void *, int32_t action, const char *command,
                                     size_t length, char *error, size_t capacity);
// Exact interleaved stereo Float32 PCM in/out. A failed outer JS call leaves
// the entire block unchanged; shared-rack callback errors bypass that effect.
int32_t native_genesis_livefx_process(void *, float *stereo, uint32_t frames,
                                     char *error, size_t capacity);
// clear releases the engine. An empty rack never evaluates/processes JavaScript.
void native_genesis_livefx_clear(void *);
uint32_t native_genesis_livefx_active(void *);
uint32_t native_genesis_livefx_initialized(void *);
uint32_t native_genesis_livefx_faulted(void *);
uint64_t native_genesis_livefx_process_calls(void *);
uint64_t native_genesis_livefx_initializations(void *);
// Reports are bounded and copied without invoking JavaScript. Messages are
// separated by ASCII record separator (0x1e). Reading clears the report buffer,
// not the rack's bypass state.
size_t native_genesis_livefx_reports(void *, char *output, size_t capacity);
#ifdef __cplusplus
}
#endif
