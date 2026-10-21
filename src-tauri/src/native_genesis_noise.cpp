#include "native_genesis_noise.h"
#include <cmath>
#include <cstdint>
#include <cstring>
#include <new>

namespace {
struct Noise {
    float input[2]{};
    uint32_t sample_rate;
    float *gain_input() {return input;}
    int gain_capacity() {return 1;}
#include "../native-noise/noise.inc"
    explicit Noise(uint32_t rate) : sample_rate(rate) {noise_reset(rate);}
};
}
extern "C" {
void *native_genesis_noise_create(uint32_t rate) {
    if(rate<8000||rate>384000)return nullptr;
    return new(std::nothrow) Noise(rate);
}
void native_genesis_noise_destroy(void *ptr){delete static_cast<Noise *>(ptr);}
void native_genesis_noise_reset(void *ptr){if(ptr){auto *noise=static_cast<Noise *>(ptr);noise->noise_reset(noise->sample_rate);}}
int32_t native_genesis_noise_voice_create(void *ptr,int32_t slot,int32_t type,uint32_t seed){return ptr?static_cast<Noise *>(ptr)->noise_create(slot,type,seed):0;}
int32_t native_genesis_noise_set(void *ptr,int32_t slot,int32_t parameter,double value,int32_t frames){return ptr?static_cast<Noise *>(ptr)->noise_set(slot,parameter,value,frames):0;}
void native_genesis_noise_filter(void *ptr,int32_t slot,int32_t mode){if(ptr)static_cast<Noise *>(ptr)->noise_filter(slot,mode);}
void native_genesis_noise_action(void *ptr,int32_t slot,int32_t action){if(ptr)static_cast<Noise *>(ptr)->noise_action(slot,action);}
int32_t native_genesis_noise_active(void *ptr,int32_t slot){
    if(!ptr||slot<0||slot>=32)return 0;const auto &voice=static_cast<Noise *>(ptr)->voices[slot];return voice.used&&voice.active;
}
void native_genesis_noise_mix(void *ptr,float *stereo){
    if(!ptr||!stereo)return;auto *noise=static_cast<Noise *>(ptr);
    noise->input[0]=stereo[0];noise->input[1]=stereo[1];noise->noise_mix(1);stereo[0]=noise->input[0];stereo[1]=noise->input[1];
}
}
