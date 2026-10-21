// SPDX-License-Identifier: BSD-3-Clause
// Offline only: no output device is constructed or opened.
#include "native_genesis_pwm.h"
#include <atomic>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <iostream>
#include <new>
#include <string>
#include <vector>

static bool measured = false;
static uint64_t allocations = 0, deallocations = 0;
void *operator new(std::size_t size) { if (measured) ++allocations; if (void *p=std::malloc(size)) return p; throw std::bad_alloc(); }
void *operator new[](std::size_t size) { return ::operator new(size); }
void operator delete(void *p) noexcept { if (measured && p) ++deallocations;std::free(p); }
void operator delete[](void *p) noexcept { ::operator delete(p); }
void operator delete(void *p,std::size_t) noexcept { ::operator delete(p); }
void operator delete[](void *p,std::size_t) noexcept { ::operator delete(p); }
struct Event { uint32_t frame; std::string op; uint32_t address,value; };
int main(int argc,char **argv) {
    if (argc == 2 && !std::strcmp(argv[1],"--self-test")) {
        void *chip=native_pwm_create(48000);if(!chip)return 2;
        NativePwmState state{};
        measured=true;
        native_pwm_write(chip,0,5);native_pwm_write(chip,1,1000);native_pwm_write(chip,4,300);
        native_pwm_set_clock(chip,12000000);
        float l=0,r=0;for(unsigned i=0;i<10000;++i)native_pwm_sample(chip,&l,&r);
        native_pwm_stop(chip);native_pwm_get_state(chip,&state);
        const bool stopped=state.left_valid==0&&state.right_valid==0&&state.left_size==0&&state.right_size==0&&state.registers[0]==5&&state.registers[1]==1000&&state.clock==12000000;
        native_pwm_reset(chip);native_pwm_get_state(chip,&state);
        const bool reset=state.registers[0]==0&&state.registers[1]==0&&state.clock==12000000;
        const bool bounds=!native_pwm_set_clock(chip,99999)&&!native_pwm_set_clock(chip,100000001);
        measured=false;native_pwm_destroy(chip);
        std::fprintf(stderr,"{\"stopRetainsRoutingCycleClock\":%s,\"resetClearsRegisters\":%s,\"bounds\":%s,\"allocationsDuringRender\":%llu,\"freesDuringRender\":%llu}\n",stopped?"true":"false",reset?"true":"false",bounds?"true":"false",(unsigned long long)allocations,(unsigned long long)deallocations);
        return stopped&&reset&&bounds&&!allocations&&!deallocations?0:3;
    }
    if(argc!=3)return 1;
    const uint32_t rate=std::strtoul(argv[1],nullptr,10),frames=std::strtoul(argv[2],nullptr,10);
    std::vector<Event> events;Event event{};
    while(std::cin>>event.frame>>event.op>>event.address>>event.value)events.push_back(event);
    std::vector<float> output(size_t(frames)*2);
    void *chip=native_pwm_create(rate);if(!chip)return 2;
    size_t index=0;measured=true;
    for(uint32_t frame=0;frame<frames;++frame){
        while(index<events.size()&&events[index].frame==frame){const auto&e=events[index++];
            if(e.op=="write")native_pwm_write(chip,e.address,e.value);
            else if(e.op=="clock")native_pwm_set_clock(chip,e.value);
            else if(e.op=="reset")native_pwm_reset(chip);
            else if(e.op=="stop")native_pwm_stop(chip);
        }
        native_pwm_sample(chip,&output[frame*2],&output[frame*2+1]);
    }
    measured=false;native_pwm_destroy(chip);
    if(allocations||deallocations){std::fprintf(stderr,"Callback allocation audit failed: %llu/%llu\n",(unsigned long long)allocations,(unsigned long long)deallocations);return 4;}
    if(std::fwrite(output.data(),sizeof(float),output.size(),stdout)!=output.size())return 5;
    return 0;
}
