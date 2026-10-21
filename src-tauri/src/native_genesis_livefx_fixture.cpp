// Line-JSON offline fixture. Its separate parser realm is test infrastructure;
// the host's initialized/calls counters refer only to the lazy effect runtime.
// No device, CPAL, WebView, filesystem or audio output is opened.
#include "native_genesis_livefx.h"
#include "../native-livefx/quickjs-ng/quickjs.h"
#include <algorithm>
#include <chrono>
#include <cmath>
#include <cstring>
#include <iostream>
#include <string>
#include <vector>

static double number(JSContext *ctx, JSValueConst row, const char *name, double fallback) {
    JSValue value = JS_GetPropertyStr(ctx, row, name); double result = fallback;
    if (!JS_IsUndefined(value) && JS_ToFloat64(ctx, &result, value) < 0) result = fallback;
    JS_FreeValue(ctx, value); return result;
}
static std::string text(JSContext *ctx, JSValueConst value) {
    const char *str = JS_ToCString(ctx, value); std::string result = str ? str : "";
    if (str) JS_FreeCString(ctx, str); return result;
}
static std::string property(JSContext *ctx, JSValueConst row, const char *name) {
    JSValue value = JS_GetPropertyStr(ctx, row, name); auto result = text(ctx, value);
    JS_FreeValue(ctx, value); return result;
}
static void set_number(JSContext *ctx, JSValue result, const char *name, double value) {
    JS_SetPropertyStr(ctx, result, name, JS_NewFloat64(ctx, value));
}
int main() {
    JSRuntime *parser = JS_NewRuntime(); if (!parser) return 2;
    JS_SetMemoryLimit(parser, 32 * 1024 * 1024);
    JSContext *ctx = JS_NewContext(parser); if (!ctx) { JS_FreeRuntime(parser); return 3; }
    void *host = nullptr; uint32_t max_frames = 128, rate = 48000;
    std::string line;
    while (std::getline(std::cin, line)) {
        if (line.size() > 1024 * 1024) return 4;
        JSValue row = JS_ParseJSON(ctx, line.data(), line.size(), "fixture line");
        JSValue result = JS_NewObject(ctx); bool ok = true; char error[4096]{};
        const std::string op = JS_IsException(row) ? "invalid" : property(ctx, row, "op");
        if (op == "init") {
            native_genesis_livefx_destroy(host);
            max_frames = uint32_t(number(ctx, row, "maxBlock", 128));
            rate = uint32_t(number(ctx, row, "rate", 48000));
            host = native_genesis_livefx_create(rate, max_frames,
                size_t(number(ctx, row, "memoryLimit", 32 * 1024 * 1024)), uint64_t(number(ctx, row, "budgetUs", 100000)));
            ok = host != nullptr; if (!ok) std::strcpy(error, "Invalid host configuration");
        } else if (!host) {
            ok = false; std::strcpy(error, "Initialize fixture first");
        } else if (op == "command") {
            JSValue command = JS_GetPropertyStr(ctx, row, "command");
            const auto action = property(ctx, command, "action");
            const int action_id = action == "register" ? 0 : action == "context" ? 1 : action == "remove" ? 2 : -1;
            JSValue json = JS_JSONStringify(ctx, command, JS_UNDEFINED, JS_UNDEFINED);
            const auto source = text(ctx, json);
            ok = native_genesis_livefx_command(host, action_id, source.data(), source.size(), error, sizeof(error));
            JS_FreeValue(ctx, json); JS_FreeValue(ctx, command);
        } else if (op == "clear") {
            native_genesis_livefx_clear(host);
        } else if (op == "process") {
            const uint32_t frames = uint32_t(number(ctx, row, "frames", 128));
            JSValue samples = JS_GetPropertyStr(ctx, row, "samples");
            JSValue length = JS_GetPropertyStr(ctx, samples, "length"); uint32_t count = 0;
            JS_ToUint32(ctx, &count, length); JS_FreeValue(ctx, length);
            if (!frames || frames > max_frames || count != frames * 2) { ok = false; std::strcpy(error, "Invalid stereo samples"); }
            else {
                std::vector<float> pcm(count);
                for (uint32_t i = 0; i < count; ++i) {
                    JSValue value = JS_GetPropertyUint32(ctx, samples, i); double sample = 0;
                    JS_ToFloat64(ctx, &sample, value); JS_FreeValue(ctx, value); pcm[i] = float(sample);
                }
                ok = native_genesis_livefx_process(host, pcm.data(), frames, error, sizeof(error));
                JSValue output = JS_NewArray(ctx);
                for (uint32_t i = 0; i < count; ++i) JS_SetPropertyUint32(ctx, output, i, JS_NewFloat64(ctx, pcm[i]));
                JS_SetPropertyStr(ctx, result, "samples", output);
            }
            JS_FreeValue(ctx, samples);
        } else if (op == "benchmark") {
            const uint32_t frames = uint32_t(number(ctx, row, "frames", 128));
            const uint32_t blocks = uint32_t(number(ctx, row, "blocks", 4000));
            const uint32_t warmup = uint32_t(number(ctx, row, "warmup", 100));
            if (!frames || frames > max_frames || !blocks || blocks > 100000 || warmup > 10000) { ok = false; std::strcpy(error, "Invalid benchmark bounds"); }
            else {
                std::vector<float> input(frames * 2), pcm(frames * 2);
                for (uint32_t i = 0; i < frames; ++i) { input[i * 2] = std::sin(i * .171) * .4; input[i * 2 + 1] = std::cos(i * .117) * .3; }
                for (uint32_t i = 0; i < warmup && ok; ++i) { pcm = input; ok = native_genesis_livefx_process(host, pcm.data(), frames, error, sizeof(error)); }
                double total = 0, maximum = 0, checksum = 0;
                std::vector<double> durations; durations.reserve(blocks);
                uint32_t over_deadline = 0;
                for (uint32_t i = 0; i < blocks && ok; ++i) {
                    pcm = input; const auto before = std::chrono::steady_clock::now();
                    ok = native_genesis_livefx_process(host, pcm.data(), frames, error, sizeof(error));
                    const double us = std::chrono::duration<double, std::micro>(std::chrono::steady_clock::now() - before).count();
                    total += us; maximum = std::max(maximum, us); checksum += pcm[(i % frames) * 2];
                    durations.push_back(us);
                    if (us > double(frames) / rate * 1000000) ++over_deadline;
                }
                set_number(ctx, result, "totalUs", total); set_number(ctx, result, "maxUs", maximum);
                set_number(ctx, result, "checksum", checksum); set_number(ctx, result, "blocks", blocks);
                std::sort(durations.begin(), durations.end());
                const auto percentile = [&](double p) { return durations.empty() ? 0.0 : durations[size_t((durations.size() - 1) * p)]; };
                set_number(ctx, result, "p50Us", percentile(.50)); set_number(ctx, result, "p95Us", percentile(.95));
                set_number(ctx, result, "p99Us", percentile(.99)); set_number(ctx, result, "overDeadlineBlocks", over_deadline);
            }
        } else if (op != "stats") {
            ok = false; std::strcpy(error, "Unknown fixture operation");
        }
        JS_SetPropertyStr(ctx, result, "ok", JS_NewBool(ctx, ok));
        JS_SetPropertyStr(ctx, result, "initialized", JS_NewBool(ctx, native_genesis_livefx_initialized(host)));
        JS_SetPropertyStr(ctx, result, "faulted", JS_NewBool(ctx, native_genesis_livefx_faulted(host)));
        set_number(ctx, result, "active", native_genesis_livefx_active(host));
        set_number(ctx, result, "processCalls", native_genesis_livefx_process_calls(host));
        set_number(ctx, result, "initializations", native_genesis_livefx_initializations(host));
        if (error[0]) JS_SetPropertyStr(ctx, result, "error", JS_NewString(ctx, error));
        char reports[16385]{}; native_genesis_livefx_reports(host, reports, sizeof(reports));
        JSValue messages = JS_NewArray(ctx); uint32_t index = 0; const char *start = reports;
        for (const char *cursor = reports; *cursor; ++cursor) if (*cursor == '\x1e') {
            JS_SetPropertyUint32(ctx, messages, index++, JS_NewStringLen(ctx, start, cursor - start)); start = cursor + 1;
        }
        if (*start) JS_SetPropertyUint32(ctx, messages, index, JS_NewString(ctx, start));
        JS_SetPropertyStr(ctx, result, "reports", messages);
        JSValue json = JS_JSONStringify(ctx, result, JS_UNDEFINED, JS_UNDEFINED);
        std::cout << text(ctx, json) << std::endl;
        JS_FreeValue(ctx, json); JS_FreeValue(ctx, result); JS_FreeValue(ctx, row);
    }
    native_genesis_livefx_destroy(host); JS_FreeContext(ctx); JS_FreeRuntime(parser); return 0;
}
