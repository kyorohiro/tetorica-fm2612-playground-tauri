// Test-only allocation probe, never linked into the application.
#include <atomic>
#include <cstdlib>
#include <new>
#include <cstdint>
#include "scsp.h"
// Match the extra-rack charge: 2 MiB raw core, plus the common queue and
// 64 KiB Rust/shared/renderer allowance. Never silently undercharge a new pin.
static_assert(sizeof(scsp_device) <= 2 * 1024 * 1024, "SCSP raw core exceeds its Native instance budget");
namespace { std::atomic<bool> hot{false}; std::atomic<uint64_t> allocations{0}, frees{0}; }
void *operator new(std::size_t n) {
    if(hot.load(std::memory_order_relaxed)) ++allocations;
    if(void *p=std::malloc(n ? n : 1))return p;
    throw std::bad_alloc();
}
void *operator new[](std::size_t n) { return ::operator new(n); }
void operator delete(void *p) noexcept { if(p && hot.load(std::memory_order_relaxed))++frees;std::free(p); }
void operator delete[](void *p) noexcept { ::operator delete(p); }
void operator delete(void *p,std::size_t) noexcept { ::operator delete(p); }
void operator delete[](void *p,std::size_t) noexcept { ::operator delete(p); }
extern "C" {
void scsp_probe_enable(int on) {hot.store(on!=0);}
uint64_t scsp_probe_new() {return allocations.load();}
uint64_t scsp_probe_delete() {return frees.load();}
}
