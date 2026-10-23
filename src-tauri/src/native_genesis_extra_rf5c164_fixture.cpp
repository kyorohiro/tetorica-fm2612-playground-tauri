// Offline additional RF5C164 DSP/source-isolation fixture. No device.
#include "native_genesis_extra_rf5c164.h"
#include <array>
#include <cassert>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <iostream>
#include <new>
#include <sstream>
#include <string>
#include <vector>
static bool audit=false;static uint64_t allocations=0,frees=0;
void*operator new(std::size_t n){if(audit)++allocations;if(void*p=std::malloc(n))return p;throw std::bad_alloc();}void*operator new[](std::size_t n){return ::operator new(n);}
void operator delete(void*p)noexcept{if(audit&&p)++frees;std::free(p);}void operator delete[](void*p)noexcept{::operator delete(p);}void operator delete(void*p,std::size_t)noexcept{::operator delete(p);}void operator delete[](void*p,std::size_t)noexcept{::operator delete(p);}
static uint8_t pattern(uint32_t i,uint32_t seed){return uint8_t((i*seed+17)%254);}
static void voice(void*p,uint8_t ch){native_extra_rf5c164_write(p,7,0xc0|ch);for(uint8_t reg=0;reg<7;++reg)native_extra_rf5c164_write(p,reg,reg==0?255:reg==1?255:reg==2?uint8_t(17+ch*29):reg==3?8:0);NativeExtraRf5c164State s{};native_extra_rf5c164_get_state(p,&s);native_extra_rf5c164_write(p,8,s.mask&~(1u<<ch));}
static void next(void*p,float&l,float&r){native_extra_rf5c164_sample(p,&l,&r);assert(std::isfinite(l)&&std::isfinite(r));}
static int self_test(){
    static_assert(sizeof(NativeExtraRf5c164State)==100,"Rust ABI");assert(!native_extra_rf5c164_create(0));
    for(uint32_t hz:{100000u,12500000u,100000000u})for(uint32_t rate:{44100u,48000u,96000u}){
        void*a=native_extra_rf5c164_create(rate),*b=native_extra_rf5c164_create(rate);assert(a&&b);std::array<uint8_t,65536> ram{};for(uint32_t i=0;i<ram.size();++i)ram[i]=pattern(i,71);
        audit=true;for(void*p:{a,b}){assert(native_extra_rf5c164_load(p,ram.data(),0,65536));assert(native_extra_rf5c164_load(p,nullptr,65536,0));assert(!native_extra_rf5c164_load(p,ram.data(),65536,1));assert(!native_extra_rf5c164_load(p,nullptr,0,1));for(uint8_t ch=1;ch<8;++ch)voice(p,ch);}voice(a,0);
        float al,ar,bl,br;for(uint32_t i=0;i<1703;++i){next(a,al,ar);next(b,bl,br);}for(void*p:{a,b})assert(native_extra_rf5c164_set_clock(p,hz));assert(!native_extra_rf5c164_set_clock(a,99999));
        for(uint32_t i=0;i<37;++i){next(a,al,ar);next(b,bl,br);}native_extra_rf5c164_write(a,7,0x85);NativeExtraRf5c164State before{},after{};native_extra_rf5c164_get_state(a,&before);native_extra_rf5c164_key_off(a,0);native_extra_rf5c164_get_state(a,&after);assert(after.selected==before.selected&&after.bank==before.bank&&after.control==before.control&&after.mask==(before.mask|1));
        double energy=0;std::array<uint8_t,65536> copy{};
        for(uint32_t i=0;i<1301;++i){native_extra_rf5c164_get_state(a,&after);if(!(i%71))assert(native_extra_rf5c164_read_memory_range(a,copy.data(),0,65536));next(a,al,ar);next(b,bl,br);assert(al==bl&&ar==br);energy+=std::abs(ar);}assert(energy>1);
        assert(copy==ram);assert(!native_extra_rf5c164_read_memory_range(a,copy.data(),65536,1));native_extra_rf5c164_reset(a);native_extra_rf5c164_get_state(a,&after);assert(after.clock==hz&&after.mask==255&&!after.control&&!after.selected&&!after.bank&&native_extra_rf5c164_read_memory(a,65535)==ram[65535]);
        for(uint32_t i=0;i<200;++i){next(a,al,ar);assert(al==0&&ar==0);}voice(a,7);next(a,al,ar);native_extra_rf5c164_stop(a);for(uint32_t i=0;i<200;++i){next(a,al,ar);assert(al==0&&ar==0);}
        native_extra_rf5c164_clear_memory(a);assert(native_extra_rf5c164_read_memory(a,65535)==0);audit=false;assert(!allocations&&!frees);native_extra_rf5c164_destroy(a);native_extra_rf5c164_destroy(b);
    }
    std::puts("RF5C164: 9 rate/clock isolation cases, clipped8voice/lowclock held PCM, RAM retain/range/snapshot, source cancel, Stop/reset; callback allocations/frees 0; ABI 100 bytes");return 0;
}
struct Event{uint32_t frame;std::string op;std::vector<uint32_t> args;};
constexpr uint16_t probes[16]={0,1,255,256,4095,4096,8191,8192,12287,16384,32767,32768,49152,65279,65534,65535};
int main(int argc,char**argv){if(argc==2&&std::string(argv[1])=="--self-test")return self_test();if(argc!=3)return 2;uint32_t rate=std::strtoul(argv[1],nullptr,10),frames=std::strtoul(argv[2],nullptr,10);void*p=native_extra_rf5c164_create(rate);if(!p||frames>1048576)return 2;
    std::vector<Event> events;for(std::string line;std::getline(std::cin,line);){std::istringstream in(line);Event e;if(!(in>>e.frame>>e.op))continue;for(uint32_t arg;in>>arg;)e.args.push_back(arg);events.push_back(e);}std::vector<float> output;output.reserve(size_t(frames)*95);size_t index=0;
    std::array<uint8_t,65536> memory{};audit=true;
    for(uint32_t frame=0;frame<frames;++frame){while(index<events.size()&&events[index].frame==frame){const auto&e=events[index++];const auto&a=e.args;
        if(e.op=="write")native_extra_rf5c164_write(p,a[0],a[1]);else if(e.op=="clock")assert(native_extra_rf5c164_set_clock(p,a[0]));else if(e.op=="reset")native_extra_rf5c164_reset(p);else if(e.op=="clear")native_extra_rf5c164_clear_memory(p);else if(e.op=="memory")native_extra_rf5c164_write_memory(p,a[0],a[1]);else if(e.op=="load"){for(uint32_t i=0;i<a[1];++i)memory[i]=pattern(i,a[2]);assert(native_extra_rf5c164_load(p,memory.data(),a[0],a[1]));}else if(e.op=="marker"){const uint8_t marker=255;assert(native_extra_rf5c164_load(p,&marker,a[0],1));}else return 3;
    }float l,r;next(p,l,r);output.push_back(l);output.push_back(r);NativeExtraRf5c164State s{};native_extra_rf5c164_get_state(p,&s);for(auto&ch:s.registers)for(uint8_t v:ch)output.push_back(v);for(uint32_t value:{uint32_t(s.control),uint32_t(s.selected),uint32_t(s.mask),s.clock,uint32_t(s.bank)})output.push_back(float(value));for(uint8_t reg=0;reg<16;++reg)output.push_back(native_extra_rf5c164_read(p,reg));for(uint16_t address:probes)output.push_back(native_extra_rf5c164_read_memory(p,address));}
    audit=false;assert(!allocations&&!frees);native_extra_rf5c164_destroy(p);assert(output.size()==size_t(frames)*95);return std::fwrite(output.data(),sizeof(float),output.size(),stdout)==output.size()?0:4;
}
