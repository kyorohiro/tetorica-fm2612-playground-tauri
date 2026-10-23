// Standalone additional OPL fixture, no device or primary machine instance.
#include "native_genesis_extra_opl.h"
#include <algorithm>
#include <cassert>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <iostream>
#include <new>
#include <sstream>
#include <string>
#include <vector>
static bool audit=false; static uint64_t allocations=0,frees=0;
void*operator new(std::size_t n){if(audit)++allocations;if(void*p=std::malloc(n))return p;throw std::bad_alloc();}void*operator new[](std::size_t n){return ::operator new(n);}
void operator delete(void*p)noexcept{if(audit&&p)++frees;std::free(p);}void operator delete[](void*p)noexcept{::operator delete(p);}void operator delete(void*p,std::size_t)noexcept{::operator delete(p);}void operator delete[](void*p,std::size_t)noexcept{::operator delete(p);}
constexpr uint8_t SLOTS[9]={0,1,2,8,9,10,16,17,18};
static void voice(void*p,uint8_t ch){for(uint8_t slot:{SLOTS[ch],uint8_t(SLOTS[ch]+3)}){native_extra_opl_write(p,0x20+slot,0x21);native_extra_opl_write(p,0x40+slot,12);native_extra_opl_write(p,0x60+slot,0xf0);native_extra_opl_write(p,0x80+slot,15);}native_extra_opl_write(p,0xc0+ch,1);native_extra_opl_write(p,0xa0+ch,127+ch*11);native_extra_opl_write(p,0xb0+ch,0x31);}
static float sample(void*p){float l,r;native_extra_opl_sample(p,&l,&r);assert(l==r&&std::isfinite(l));return l;}
static double render(void*p,uint32_t n){double e=0;while(n--)e+=std::abs(sample(p));return e;}
static int self_test(){
    static_assert(sizeof(NativeExtraOplState)==264,"RustABI");assert(!native_extra_opl_create(0,0)&&!native_extra_opl_create(48000,2));
    for(uint8_t kind:{0,1,3}){
        void*a=native_extra_opl_create(96000,kind),*b=native_extra_opl_create(96000,kind);assert(a&&b);audit=true;
        NativeExtraOplState s{};native_extra_opl_get_state(a,&s);assert(s.clock==3579545&&s.channels==9&&s.kind==kind&&render(a,37)==0);render(b,37);
        voice(a,0);voice(a,5);voice(b,5);render(a,1703);render(b,1703);
        assert(native_extra_opl_set_clock(a,100000)&&native_extra_opl_set_clock(b,100000));assert(!native_extra_opl_set_clock(a,99999));
        render(a,37);render(b,37);native_extra_opl_key_off(a,0);
        for(uint32_t i=0;i<1000;++i){assert(sample(a)==sample(b));native_extra_opl_get_state(a,&s);}
        native_extra_opl_reset(a);native_extra_opl_audition_key_on(a,0);native_extra_opl_key_off(a,0);assert(render(a,200)==0);
        native_extra_opl_stop(a);assert(render(a,100)==0);
        audit=false;assert(!allocations&&!frees);native_extra_opl_destroy(a);native_extra_opl_destroy(b);
        // Forward/inverse rhythm aliases and the independent shared operators.
        a=native_extra_opl_create(96000,kind);b=native_extra_opl_create(96000,kind);audit=true;
        for(void*p:{a,b}){for(uint8_t ch=0;ch<9;++ch)voice(p,ch);for(uint8_t ch=6;ch<9;++ch)native_extra_opl_write(p,0xb0+ch,0x11);}
        native_extra_opl_write(a,0xbd,63);native_extra_opl_write(b,0xbd,35);render(a,1703);render(b,1703);
        native_extra_opl_set_clock(a,100000);native_extra_opl_set_clock(b,100000);render(a,37);render(b,37);
        for(uint8_t v:{18,19,20})native_extra_opl_key_off(a,v);native_extra_opl_key_off(a,7);native_extra_opl_key_off(a,8);
        for(uint32_t i=0;i<1000;++i)assert(sample(a)==sample(b));
        native_extra_opl_audition_key_on(a,19);native_extra_opl_audition_key_on(a,22);native_extra_opl_key_off_fm(a,19);
        render(a,200);native_extra_opl_get_state(a,&s);assert(!(s.registers[0xbd]&8)&&(s.registers[0xbd]&1));
        native_extra_opl_audition_key_on(a,20);native_extra_opl_reset(a);assert(render(a,200)==0);
        native_extra_opl_reset(a);native_extra_opl_reset(b);for(void*p:{a,b}){for(uint8_t ch=6;ch<9;++ch)voice(p,ch);native_extra_opl_write(p,0xbd,31);}render(a,1703);render(b,1703);for(uint8_t v=18;v<23;++v)native_extra_opl_key_off(a,v);
        for(uint32_t i=0;i<1000;++i)assert(sample(a)==sample(b));
        audit=false;assert(!allocations&&!frees);native_extra_opl_destroy(a);native_extra_opl_destroy(b);
        // Shared CSM cancellation preserves explicit ordinary FM and rhythms.
        a=native_extra_opl_create(48000,kind);audit=true;voice(a,3);voice(a,7);native_extra_opl_write(a,2,255);native_extra_opl_write(a,3,249);native_extra_opl_write(a,4,3);native_extra_opl_write(a,8,128);
        render(a,1000);native_extra_opl_write(a,4,128);assert(native_extra_opl_csm_active(a));native_extra_opl_key_off_fm(a,7);native_extra_opl_get_state(a,&s);assert(s.registers[8]&128);assert(s.registers[4]==128);assert(!(s.registers[0xb7]&32));
        native_extra_opl_cancel_csm(a);native_extra_opl_get_state(a,&s);assert(!(s.registers[8]&128)&&!(s.registers[4]&1)&&(s.registers[4]&2)&&(s.registers[0xb3]&32));assert(render(a,1000)>.1);
        native_extra_opl_host_all_notes_off(a);assert(render(a,300)==0);voice(a,3);assert(render(a,300)>.01);native_extra_opl_stop(a);assert(render(a,300)==0);
        audit=false;assert(!allocations&&!frees);native_extra_opl_destroy(a);
        // Snapshot polling never advances the selected chip or decoders.
        a=native_extra_opl_create(48000,kind);b=native_extra_opl_create(48000,kind);audit=true;
        for(void*p:{a,b}){voice(p,0);native_extra_opl_write(p,2,255);native_extra_opl_write(p,4,1);if(kind==3){native_extra_opl_write(p,9,31);native_extra_opl_write(p,16,255);native_extra_opl_write(p,17,255);native_extra_opl_write(p,18,220);native_extra_opl_write(p,7,0x90);}}
        for(uint32_t i=0;i<3000;++i){native_extra_opl_get_state(a,&s);assert(sample(a)==sample(b));}
        audit=false;assert(!allocations&&!frees);native_extra_opl_destroy(a);native_extra_opl_destroy(b);
        if(kind==3){
            // Execute-off leaves the decoder accumulator audible as DC. Its
            // independently owned source must survive foreign FM/CSM release.
            a=native_extra_opl_create(96000,kind);b=native_extra_opl_create(96000,kind);audit=true;
            for(void*p:{a,b}){voice(p,0);native_extra_opl_key_off(p,0);native_extra_opl_write(p,8,15);native_extra_opl_write(p,16,255);native_extra_opl_write(p,17,255);native_extra_opl_write(p,18,220);native_extra_opl_write(p,7,0x80);native_extra_opl_write(p,15,0x77);}
            voice(a,3);render(a,2000);render(b,2000);for(void*p:{a,b}){native_extra_opl_write(p,7,0);native_extra_opl_set_clock(p,100000);}render(a,37);render(b,37);native_extra_opl_key_off_fm(a,3);
            double energy=0;for(uint32_t i=0;i<1000;++i){const float x=sample(a),y=sample(b);assert(x==y);energy+=std::abs(y);}assert(energy>.1);
            native_extra_opl_write(a,2,255);native_extra_opl_write(a,4,1);native_extra_opl_write(a,8,0x8f);native_extra_opl_cancel_csm(a);
            for(uint32_t i=0;i<1000;++i)assert(sample(a)==sample(b));
            // Cancelling that held decoder source keeps foreign FM exactly.
            for(void*p:{a,b}){voice(p,5);native_extra_opl_set_clock(p,3579545);}render(a,1000);render(b,1000);native_extra_opl_cancel_adpcm(b);native_extra_opl_cancel_adpcm(a);
            for(uint32_t i=0;i<1000;++i)assert(sample(a)==sample(b));
            audit=false;assert(!allocations&&!frees);native_extra_opl_destroy(a);native_extra_opl_destroy(b);
            a=native_extra_opl_create(96000,kind);b=native_extra_opl_create(96000,kind);audit=true;
            for(void*p:{a,b})voice(p,5);
            native_extra_opl_write(a,8,15);native_extra_opl_write(a,16,255);native_extra_opl_write(a,17,255);native_extra_opl_write(a,18,220);native_extra_opl_write(a,7,0x80);native_extra_opl_write(a,15,0x77);
            render(a,1703);render(b,1703);native_extra_opl_write(a,7,0);
            for(void*p:{a,b})native_extra_opl_set_clock(p,100000);render(a,37);render(b,37);native_extra_opl_cancel_adpcm(a);
            energy=0;for(uint32_t i=0;i<1000;++i){const float x=sample(a),y=sample(b);assert(x==y);energy+=std::abs(y);}assert(energy>.01);
            audit=false;assert(!allocations&&!frees);native_extra_opl_destroy(a);native_extra_opl_destroy(b);
        }
    }
    std::puts("Additional OPL264ABI, selected core only, source clocks, independent resets/preview/cancel, CSM normal ownership, aliased drums, readonly state; callback allocations0/frees0");return 0;
}
struct Event{uint32_t frame=0,a=0,b=0;std::string op;};
int main(int argc,char**argv){try{
    if(argc==2&&std::string(argv[1])=="--self-test")return self_test();if(argc!=4)return 2;
    const uint8_t kind=std::stoul(argv[1]);const uint32_t rate=std::stoul(argv[2]),frames=std::stoul(argv[3]),planes=263;if(frames>10000000)return 2;
    std::vector<Event>events;std::string line;while(std::getline(std::cin,line)){std::istringstream in(line);Event e;if(!(in>>e.frame>>e.op))continue;in>>e.a>>e.b;events.push_back(std::move(e));}std::stable_sort(events.begin(),events.end(),[](const Event&a,const Event&b){return a.frame<b.frame;});
    void*p=native_extra_opl_create(rate,kind);if(!p)return 3;std::vector<float>pcm(frames*planes);size_t next=0;audit=true;
    for(uint32_t frame=0;frame<frames;++frame){while(next<events.size()&&events[next].frame==frame){const auto&e=events[next++];
        if(e.op=="write")native_extra_opl_write(p,e.a,e.b);else if(e.op=="clock")assert(native_extra_opl_set_clock(p,e.a));else if(e.op=="off")native_extra_opl_preview_key_off(p,e.a);else if(e.op=="cancel")native_extra_opl_key_off_fm(p,e.a);else if(e.op=="audition")native_extra_opl_audition_key_on(p,e.a);else if(e.op=="reset")native_extra_opl_reset(p);else if(e.op=="public-off")native_extra_opl_all_notes_off(p);else if(e.op=="stop"||e.op=="all-off")native_extra_opl_host_all_notes_off(p);else return 2;
    }float*out=&pcm[frame*planes];native_extra_opl_sample(p,out,out+1);NativeExtraOplState s{};native_extra_opl_get_state(p,&s);for(uint32_t reg=0;reg<256;++reg)out[2+reg]=s.registers[reg];out[258]=s.clock;out[259]=s.status;out[260]=s.irq;out[261]=native_extra_opl_read_status(p);out[262]=native_extra_opl_get_irq(p);}
    audit=false;assert(!allocations&&!frees);native_extra_opl_destroy(p);return std::fwrite(pcm.data(),sizeof(float),pcm.size(),stdout)==pcm.size()?0:4;
}catch(const std::exception&e){std::cerr<<e.what()<<'\n';return 1;}}
