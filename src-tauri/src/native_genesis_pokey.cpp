// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 kyorohiro
// Application speaker boundary. The pinned SDK raw POKEY DSP stays unchanged.
#include "native_genesis_pokey.h"
#include "pokey.h"
#include <cmath>
#include <cstring>
#include <new>
namespace {
struct NativePokey {
 Pokey core;
 NativePokeyState state{};
 const uint32_t rate;
 const double coefficient;
 uint8_t mute=0;
 double input=0,output=0;
  NativePokey(uint32_t r,uint32_t c):core(c,r),rate(r),coefficient(std::exp(-2*3.14159265358979323846*10/r)){state.clock=c;for(auto&level:state.levels)level=8;}
 void write(uint32_t reg,uint8_t value){if(reg>15)return;core.writeRegister(reg,value);state.registers[reg]=value;if(reg<8&&(reg&1)&&(value&15))state.levels[reg/2]=value&15;}
 void reset(){const uint32_t clock=state.clock;core.reset();std::memset(&state,0,sizeof(state));state.clock=clock;mute=0;for(auto&level:state.levels)level=8;input=output=0;}
 void key_off(uint32_t ch){if(ch<4)write(ch*2+1,state.registers[ch*2+1]&0xf0);}
 bool active()const{for(unsigned ch=0;ch<4;ch++)if(!(mute&(1<<ch))&&(state.registers[ch*2+1]&15))return true;return false;}
 float internal_sample(){float left=0,right=0;core.generate(&left,&right,1);if(!active()){input=output=0;return 0;}output=double(left)-input+coefficient*output;input=left;return float(output);}
 void sample(float&left,float&right){left=right=internal_sample();}
};
static_assert(sizeof(NativePokeyState)==40,"Rust/C++ POKEY state layout");
}
extern "C" {
size_t native_pokey_core_size(){return sizeof(NativePokey);}
void *native_pokey_create(uint32_t rate,uint32_t clock){if(!Pokey::validConfiguration(clock,rate))return nullptr;return new(std::nothrow) NativePokey(rate,clock);}
void native_pokey_destroy(void*p){delete static_cast<NativePokey*>(p);}
void native_pokey_reset(void*p){if(p)static_cast<NativePokey*>(p)->reset();}
void native_pokey_write(void*p,uint32_t r,uint8_t v){if(p)static_cast<NativePokey*>(p)->write(r,v);}
void native_pokey_key_off(void*p,uint32_t c){if(p)static_cast<NativePokey*>(p)->key_off(c);}
void native_pokey_stop(void*p){if(p){auto&v=*static_cast<NativePokey*>(p);for(unsigned c=0;c<4;c++)v.key_off(c);v.input=v.output=0;}}
void native_pokey_set_retained_level(void*p,uint32_t ch,uint8_t level){if(p&&ch<4&&level<16)static_cast<NativePokey*>(p)->state.levels[ch]=level;}
void native_pokey_set_mute_mask(void*p,uint32_t mask){if(p&&mask<16){auto&v=*static_cast<NativePokey*>(p);v.mute=uint8_t(mask);for(unsigned c=0;c<4;c++)v.core.mute(c,(mask&(1<<c))!=0);if(!v.active())v.input=v.output=0;}}
int32_t native_pokey_set_clock(void*p,uint32_t hz){return p&&static_cast<NativePokey*>(p)->state.clock==hz;}
void native_pokey_sample(void*p,float*l,float*r){if(!l||!r)return;if(p)static_cast<NativePokey*>(p)->sample(*l,*r);else*l=*r=0;}
void native_pokey_raw_sample(void*p,float*l,float*r){if(!l||!r)return;if(p)static_cast<NativePokey*>(p)->core.generate(l,r,1);else*l=*r=0;}
void native_pokey_get_state(void*p,NativePokeyState*out){if(!out)return;if(!p){std::memset(out,0,sizeof(*out));return;}auto&v=*static_cast<NativePokey*>(p);*out=v.state;for(unsigned r=0;r<16;r++)out->reads[r]=v.core.readRegister(r);}
}
