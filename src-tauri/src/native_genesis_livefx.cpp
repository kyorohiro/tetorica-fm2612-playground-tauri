// QuickJS-NG rack for the dedicated generation worker and offline probe.
// Never instantiate, execute or destroy this host in the audio callback.
#include "native_genesis_livefx.h"
#include "../native-livefx/quickjs-ng/quickjs.h"
#include "../native-livefx/custom_fx.inc"
#include "../native-livefx/context.inc"
#include <algorithm>
#include <chrono>
#include <cstdio>
#include <cstring>
#include <memory>
#include <new>
#include <string>
#include <vector>

namespace {
using Clock = std::chrono::steady_clock;
constexpr size_t max_command_bytes = 256 * 1024;
constexpr size_t max_report_bytes = 16 * 1024;
struct Channels {
    JSValue array = JS_UNDEFINED;
    JSValue buffers[2]{JS_UNDEFINED, JS_UNDEFINED};
    float *pcm[2]{};
};
struct Host {
    uint32_t rate, max_frames, active = 0;
    size_t memory_limit;
    uint64_t budget_us, calls = 0, initializations = 0;
    bool running = false, interrupted = false, faulted = false, has_processed = false;
    Clock::time_point deadline;
    JSRuntime *runtime = nullptr;
    JSContext *context = nullptr;
    JSValue rack = JS_UNDEFINED, command = JS_UNDEFINED, process = JS_UNDEFINED;
    JSValue reporter = JS_UNDEFINED, decode_context = JS_UNDEFINED;
    std::vector<std::unique_ptr<Channels>> channels;
    std::string reports;
    Host(uint32_t r, uint32_t f, size_t m, uint64_t b)
        : rate(r), max_frames(f), memory_limit(m), budget_us(b), channels(f + 1) {
        reports.reserve(max_report_bytes);
    }
    ~Host() { clear(); }
    void clear(bool reset_rendering = true) {
        if (context) {
            for (auto &block : channels) if (block) {
                JS_FreeValue(context, block->array);
                for (auto buffer : block->buffers) JS_FreeValue(context, buffer);
                block.reset();
            }
            JS_FreeValue(context, rack); JS_FreeValue(context, command);
            JS_FreeValue(context, process); JS_FreeValue(context, reporter);
            JS_FreeValue(context, decode_context);
            JS_FreeContext(context);
        }
        if (runtime) JS_FreeRuntime(runtime);
        runtime = nullptr; context = nullptr;
        rack = command = process = reporter = decode_context = JS_UNDEFINED;
        active = 0; running = interrupted = faulted = false;
        if (reset_rendering) { has_processed = false; reports.clear(); }
    }
    void begin() {
        interrupted = false; running = true;
        deadline = Clock::now() + std::chrono::microseconds(budget_us);
    }
    void end() { running = false; }
    bool exceeded() {
        if (running && Clock::now() >= deadline) interrupted = true;
        return interrupted;
    }
};
struct Budget {
    Host *host;
    explicit Budget(Host *value) : host(value) { host->begin(); }
    ~Budget() { host->end(); }
};
struct EmptyCleanup {
    Host *host;
    explicit EmptyCleanup(Host *value) : host(value) {}
    ~EmptyCleanup() {
        if (!host->active && host->context) {
            const bool failed = host->faulted;
            host->clear(false); host->faulted = failed;
        }
    }
};
void copy_message(char *output, size_t capacity, const std::string &message) {
    if (!output || !capacity) return;
    const size_t n = std::min(capacity - 1, message.size());
    std::memcpy(output, message.data(), n); output[n] = 0;
}
int interrupt(JSRuntime *, void *opaque) {
    auto *host = static_cast<Host *>(opaque);
    if (host->running && Clock::now() >= host->deadline) {
        host->interrupted = true; return 1;
    }
    return 0;
}
std::string exception(Host *host) {
    const JSValue value = JS_GetException(host->context);
    const bool already_interrupted = host->interrupted;
    // Thrown values can implement hostile toString/getters. Error formatting
    // must remain under the same bounded execution policy as the callback.
    host->begin();
    size_t length = 0;
    const char *text = JS_ToCStringLen(host->context, &length, value);
    host->end();
    std::string result = text ? std::string(text, std::min(length, max_report_bytes)) : "QuickJS exception";
    if (text) JS_FreeCString(host->context, text);
    else {
        const JSValue formatting_error = JS_GetException(host->context);
        JS_FreeValue(host->context, formatting_error);
    }
    JS_FreeValue(host->context, value);
    host->interrupted = already_interrupted || host->interrupted;
    if (host->interrupted) result = "QuickJS liveFx interrupt budget exceeded";
    return result;
}
JSValue report(JSContext *ctx, JSValueConst, int argc, JSValueConst *argv) {
    auto *host = static_cast<Host *>(JS_GetContextOpaque(ctx));
    if (!argc || host->reports.size() >= max_report_bytes) return JS_UNDEFINED;
    size_t n = 0; const char *text = JS_ToCStringLen(ctx, &n, argv[0]);
    if (!text) return JS_EXCEPTION;
    const size_t remaining = max_report_bytes - host->reports.size();
    host->reports.append(text, std::min(n, remaining));
    if (host->reports.size() < max_report_bytes) host->reports += '\x1e';
    JS_FreeCString(ctx, text); return JS_UNDEFINED;
}
bool initialize(Host *host, std::string &error) {
    if (host->context) return true;
    host->runtime = JS_NewRuntime();
    if (!host->runtime) { error = "Cannot create QuickJS runtime"; return false; }
    JS_SetMemoryLimit(host->runtime, host->memory_limit);
    JS_SetCanBlock(host->runtime, false);
    JS_SetMaxStackSize(host->runtime, 512 * 1024);
    JS_SetInterruptHandler(host->runtime, interrupt, host);
    host->context = JS_NewContext(host->runtime);
    if (!host->context) { error = "Cannot create QuickJS context"; host->clear(false); return false; }
    JS_SetContextOpaque(host->context, host);
    // Existing liveFx programs use the AudioWorklet's read-only rate for DSP
    // coefficients. Supply the real native device rate, never a guessed 48 kHz.
    JSValue global = JS_GetGlobalObject(host->context);
    const int rate_defined = JS_DefinePropertyValueStr(host->context, global, "sampleRate", JS_NewUint32(host->context, host->rate), 0);
    JS_FreeValue(host->context, global);
    if (rate_defined < 0) { error = exception(host); host->clear(false); return false; }
    // No quickjs-libc, module loader, filesystem, network, timers or host eval API.
    const std::string source = std::string(native_livefx_source) + "\nnew LiveFX(" + std::to_string(host->rate) + ");";
    host->begin();
    host->rack = JS_Eval(host->context, source.c_str(), source.size(), "custom_fx.js", JS_EVAL_TYPE_GLOBAL);
    const bool rack_expired = host->exceeded();
    host->end();
    if (JS_IsException(host->rack)) { error = exception(host); host->rack = JS_UNDEFINED; host->clear(false); return false; }
    if (rack_expired) { error = "QuickJS liveFx interrupt budget exceeded"; host->clear(false); return false; }
    host->command = JS_GetPropertyStr(host->context, host->rack, "command");
    host->process = JS_GetPropertyStr(host->context, host->rack, "process");
    host->reporter = JS_NewCFunction(host->context, report, "report", 1);
    const std::string decoder = std::string(native_livefx_context_source) + "\ndecodeNativeLiveFxContext;";
    host->begin();
    host->decode_context = JS_Eval(host->context, decoder.c_str(), decoder.size(), "native_livefx_context.js", JS_EVAL_TYPE_GLOBAL);
    const bool decoder_expired = host->exceeded();
    host->end();
    if (JS_IsException(host->decode_context)) { error = exception(host); host->decode_context = JS_UNDEFINED; host->clear(false); return false; }
    if (decoder_expired) { error = "QuickJS liveFx interrupt budget exceeded"; host->clear(false); return false; }
    if (!JS_IsFunction(host->context, host->command) || !JS_IsFunction(host->context, host->process) || JS_IsException(host->reporter)) {
        error = "Cannot initialize shared LiveFX methods"; host->clear(false); return false;
    }
    JS_SetPropertyStr(host->context, host->rack, "hasProcessed", JS_NewBool(host->context, host->has_processed));
    ++host->initializations; return true;
}
bool update_active(Host *host) {
    JSValue effects = JS_GetPropertyStr(host->context, host->rack, "effects");
    JSValue size = JS_GetPropertyStr(host->context, effects, "size");
    uint32_t count = 0;
    const bool ok = JS_ToUint32(host->context, &count, size) == 0 && count <= 8;
    JS_FreeValue(host->context, size); JS_FreeValue(host->context, effects);
    if (ok) host->active = count;
    return ok;
}
Channels *block(Host *host, uint32_t frames, std::string &error) {
    if (host->channels[frames]) return host->channels[frames].get();
    std::unique_ptr<Channels> candidate(new Channels());
    candidate->array = JS_NewArray(host->context);
    bool ok = !JS_IsException(candidate->array);
    for (uint32_t ch = 0; ch < 2 && ok; ++ch) {
        std::vector<float> zero(frames, 0);
        candidate->buffers[ch] = JS_NewArrayBufferCopy(host->context, reinterpret_cast<uint8_t *>(zero.data()), frames * sizeof(float));
        if (JS_IsException(candidate->buffers[ch])) { ok = false; break; }
        JSValue args[]{candidate->buffers[ch]};
        JSValue typed = JS_NewTypedArray(host->context, 1, args, JS_TYPED_ARRAY_FLOAT32);
        size_t size = 0;
        candidate->pcm[ch] = reinterpret_cast<float *>(JS_GetArrayBuffer(host->context, &size, candidate->buffers[ch]));
        if (JS_IsException(typed) || !candidate->pcm[ch] || size != frames * sizeof(float)) {
            JS_FreeValue(host->context, typed); ok = false; break;
        }
        ok = JS_SetPropertyUint32(host->context, candidate->array, ch, typed) >= 0;
    }
    if (!ok) {
        error = exception(host); JS_FreeValue(host->context, candidate->array);
        for (auto buffer : candidate->buffers) JS_FreeValue(host->context, buffer);
        return nullptr;
    }
    auto *result = candidate.get(); host->channels[frames] = std::move(candidate); return result;
}
bool validate_buffers(Host *host, Channels *channels, uint32_t frames) {
    // User code can replace a TypedArray builtin and obtain one of the rack's
    // source buffers. A transfer/detach during JS must never leave C++ using a
    // cached pointer. Reacquire immediately before each native access.
    for (uint32_t ch = 0; ch < 2; ++ch) {
        size_t bytes = 0;
        auto *pcm = reinterpret_cast<float *>(JS_GetArrayBuffer(host->context, &bytes, channels->buffers[ch]));
        if (!pcm || bytes != frames * sizeof(float)) return false;
        channels->pcm[ch] = pcm;
    }
    return true;
}
}

extern "C" {
void *native_genesis_livefx_create(uint32_t rate, uint32_t max_frames, size_t memory_limit, uint64_t budget_us) {
    if (rate < 8000 || rate > 384000 || !max_frames || max_frames > 4096 || memory_limit < 1024 * 1024 || memory_limit > size_t(1024) * 1024 * 1024 || !budget_us || budget_us > 1000000) return nullptr;
    try { return new Host(rate, max_frames, memory_limit, budget_us); } catch (...) { return nullptr; }
}
void native_genesis_livefx_destroy(void *opaque) { delete static_cast<Host *>(opaque); }
void native_genesis_livefx_clear(void *opaque) { if (opaque) static_cast<Host *>(opaque)->clear(); }
uint32_t native_genesis_livefx_active(void *opaque) { return opaque ? static_cast<Host *>(opaque)->active : 0; }
uint32_t native_genesis_livefx_initialized(void *opaque) { return opaque && static_cast<Host *>(opaque)->context; }
uint32_t native_genesis_livefx_faulted(void *opaque) { return opaque && static_cast<Host *>(opaque)->faulted; }
uint64_t native_genesis_livefx_process_calls(void *opaque) { return opaque ? static_cast<Host *>(opaque)->calls : 0; }
uint64_t native_genesis_livefx_initializations(void *opaque) { return opaque ? static_cast<Host *>(opaque)->initializations : 0; }
size_t native_genesis_livefx_reports(void *opaque, char *output, size_t capacity) {
    if (!opaque) { copy_message(output, capacity, ""); return 0; }
    auto *host = static_cast<Host *>(opaque); const size_t n = host->reports.size();
    copy_message(output, capacity, host->reports); host->reports.clear(); return n;
}
int32_t native_genesis_livefx_command(void *opaque, int32_t action, const char *command, size_t length, char *error, size_t capacity) {
    copy_message(error, capacity, ""); auto *host = static_cast<Host *>(opaque);
    if (!host || action < 0 || action > 2 || !command || !length || length > max_command_bytes) { copy_message(error, capacity, "Invalid LiveFX command bounds"); return 0; }
    if (!host->context && action == 2) return 1;
    if (!host->context && action == 1) { copy_message(error, capacity, "Unknown liveFx"); return 0; }
    // An interrupted or fatal rack remains dry until the caller clears it.
    if (host->faulted) { copy_message(error, capacity, "QuickJS LiveFX rack faulted; Stop and Run to retry"); return 0; }
    EmptyCleanup cleanup(host);
    std::string message;
    try {
        if (!initialize(host, message)) { copy_message(error, capacity, message); return 0; }
        // Property lookups and conversions may reach prototypes altered by a
        // previous effect. Keep the complete JS-facing control path guarded.
        Budget budget(host);
        // QuickJS requires a NUL at buf[buf_len], even for this length-based API.
        // Rust callers supply a byte slice, so own and terminate exactly its bytes.
        const std::string json(command, length);
        JSValue value = JS_ParseJSON(host->context, json.c_str(), json.size(), "liveFx command");
        if (JS_IsException(value)) { copy_message(error, capacity, exception(host)); return 0; }
        JSValue operation = JS_GetPropertyStr(host->context, value, "action");
        if (JS_IsException(operation)) {
            JS_FreeValue(host->context, value); copy_message(error, capacity, exception(host));
            if (host->interrupted) host->faulted = true; return 0;
        }
        const char *name = JS_ToCString(host->context, operation);
        static const char *expected[]{"register", "context", "remove"};
        const bool match = name && std::strcmp(name, expected[action]) == 0;
        if (name) JS_FreeCString(host->context, name);
        JS_FreeValue(host->context, operation);
        if (!name) {
            JS_FreeValue(host->context, value); copy_message(error, capacity, exception(host));
            if (host->interrupted) host->faulted = true; return 0;
        }
        if (!match) { JS_FreeValue(host->context, value); copy_message(error, capacity, "LiveFX action mismatch"); return 0; }
        JSValue graph = JS_GetPropertyStr(host->context, value, "contextGraph");
        if (JS_IsException(graph)) {
            JS_FreeValue(host->context, value); copy_message(error, capacity, exception(host));
            if (host->interrupted) host->faulted = true; return 0;
        }
        if (!JS_IsUndefined(graph)) {
            JSValue decoded = JS_Call(host->context, host->decode_context, JS_UNDEFINED, 1, &graph);
            if (JS_IsException(decoded)) {
                JS_FreeValue(host->context, graph); JS_FreeValue(host->context, value);
                copy_message(error, capacity, exception(host)); if (host->interrupted) host->faulted = true; return 0;
            }
            if (host->exceeded()) {
                JS_FreeValue(host->context, decoded); JS_FreeValue(host->context, graph); JS_FreeValue(host->context, value);
                host->faulted = true; copy_message(error, capacity, "QuickJS liveFx interrupt budget exceeded"); return 0;
            }
            // Define an own data property: a previous callback may have added
            // an inherited Object.prototype.context setter.
            const int defined = JS_DefinePropertyValueStr(host->context, value, "context", decoded, JS_PROP_C_W_E);
            if (defined < 0) {
                JS_FreeValue(host->context, graph); JS_FreeValue(host->context, value);
                copy_message(error, capacity, exception(host)); if (host->interrupted) host->faulted = true; return 0;
            }
        }
        JS_FreeValue(host->context, graph);
        JSValue result = JS_Call(host->context, host->command, host->rack, 1, &value);
        JS_FreeValue(host->context, value);
        if (JS_IsException(result)) {
            message = exception(host); if (host->interrupted) host->faulted = true;
            copy_message(error, capacity, message); return 0;
        }
        JS_FreeValue(host->context, result);
        if (host->exceeded()) { host->faulted = true; copy_message(error, capacity, "QuickJS liveFx interrupt budget exceeded"); return 0; }
        const bool valid = update_active(host);
        if (!valid) { copy_message(error, capacity, exception(host)); host->faulted = true; return 0; }
        if (host->exceeded()) { host->faulted = true; copy_message(error, capacity, "QuickJS liveFx interrupt budget exceeded"); return 0; }
        // Releasing the last effect also releases the JS heap. Keep the small
        // host-side rendering flag so a later first effect still gets the
        // shared 5 ms hot-add fade without invoking JS during dry rendering.
        if (!host->active) host->clear(false);
        return 1;
    } catch (...) { host->end(); host->faulted = true; copy_message(error, capacity, "Native LiveFX host allocation failed"); return 0; }
}
int32_t native_genesis_livefx_process(void *opaque, float *stereo, uint32_t frames, char *error, size_t capacity) {
    copy_message(error, capacity, ""); auto *host = static_cast<Host *>(opaque);
    if (!host || !stereo || !frames || frames > host->max_frames) { copy_message(error, capacity, "Invalid LiveFX PCM bounds"); return 0; }
    host->has_processed = true;
    // This branch does not touch QuickJS: the normal native path remains direct.
    if (!host->context || !host->active || host->faulted) return 1;
    try {
        Budget budget(host);
        std::string message; Channels *channels = block(host, frames, message);
        if (!channels) { host->faulted = true; copy_message(error, capacity, message); return 0; }
        if (host->exceeded()) { host->faulted = true; copy_message(error, capacity, "QuickJS liveFx interrupt budget exceeded"); return 0; }
        if (!validate_buffers(host, channels, frames)) { host->faulted = true; copy_message(error, capacity, "Detached LiveFX PCM buffer"); return 0; }
        for (uint32_t i = 0; i < frames; ++i) for (uint32_t ch = 0; ch < 2; ++ch) channels->pcm[ch][i] = stereo[i * 2 + ch];
        JSValue args[]{channels->array, host->reporter};
        ++host->calls;
        JSValue result = JS_Call(host->context, host->process, host->rack, 2, args);
        if (JS_IsException(result)) {
            host->faulted = true; copy_message(error, capacity, exception(host)); return 0;
        }
        JS_FreeValue(host->context, result);
        // Polling cannot preempt every builtin/GC or OS scheduling pause. Check
        // elapsed time at the boundary as well and keep an over-budget block
        // entirely dry. This remains a soft limit, not a real-time guarantee.
        if (host->exceeded()) { host->faulted = true; copy_message(error, capacity, "QuickJS liveFx interrupt budget exceeded"); return 0; }
        if (!validate_buffers(host, channels, frames)) { host->faulted = true; copy_message(error, capacity, "Detached LiveFX PCM buffer"); return 0; }
        for (uint32_t i = 0; i < frames; ++i) for (uint32_t ch = 0; ch < 2; ++ch) stereo[i * 2 + ch] = channels->pcm[ch][i];
        return 1;
    } catch (...) { host->end(); host->faulted = true; copy_message(error, capacity, "Native LiveFX host allocation failed"); return 0; }
}
}
