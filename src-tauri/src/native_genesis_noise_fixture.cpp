// Offline C/WASM parity fixture. Never opens an audio device.
#include "native_genesis_noise.h"
#include <cstdio>
#include <cstdint>
#include <vector>

struct Event {uint32_t frame;int32_t operation,slot,argument;double value;int32_t frames;uint32_t seed;};
static bool isolation(uint32_t rate) {
    void *a=native_genesis_noise_create(rate),*b=native_genesis_noise_create(rate),*reference=native_genesis_noise_create(rate);
    if(!a||!b||!reference)return false;
    for(void *noise:{a,b,reference}) {native_genesis_noise_voice_create(noise,0,1,123);native_genesis_noise_action(noise,0,0);}
    bool valid=true;
    for(int frame=0;frame<300;frame++) {
        if(frame==80)native_genesis_noise_reset(a);
        float aa[2]{},bb[2]{},rr[2]{};native_genesis_noise_mix(a,aa);native_genesis_noise_mix(b,bb);native_genesis_noise_mix(reference,rr);
        valid=valid&&bb[0]==rr[0]&&bb[1]==rr[1];if(frame<80)valid=valid&&aa[0]==rr[0]&&aa[1]==rr[1];
    }
    native_genesis_noise_destroy(a);native_genesis_noise_destroy(b);native_genesis_noise_destroy(reference);return valid;
}
static bool active_release(uint32_t rate) {
    void *noise=native_genesis_noise_create(rate);if(!noise)return false;
    bool valid=native_genesis_noise_voice_create(noise,31,0,0)&&!native_genesis_noise_active(noise,31);
    valid=valid&&!native_genesis_noise_active(noise,-1)&&!native_genesis_noise_active(noise,32);
    native_genesis_noise_set(noise,31,5,.001,0);native_genesis_noise_action(noise,31,0);
    valid=valid&&native_genesis_noise_active(noise,31);native_genesis_noise_action(noise,31,1);
    valid=valid&&native_genesis_noise_active(noise,31);
    for(uint32_t i=0;i<rate/1000+2;i++){float stereo[2]{};native_genesis_noise_mix(noise,stereo);}
    valid=valid&&!native_genesis_noise_active(noise,31);native_genesis_noise_action(noise,31,2);
    valid=valid&&!native_genesis_noise_active(noise,31);native_genesis_noise_destroy(noise);return valid;
}
int main(int argc,char **argv) {
    if(argc!=3)return 2;
    FILE *input=std::fopen(argv[1],"rb");if(!input)return 3;
    uint32_t rate,frames,count;
    if(std::fread(&rate,4,1,input)!=1||std::fread(&frames,4,1,input)!=1||std::fread(&count,4,1,input)!=1||frames>1000000||count>10000)return 4;
    std::vector<Event> events(count);std::vector<float> pcm(frames*2);
    if(std::fread(events.data(),sizeof(Event),count,input)!=count||std::fread(pcm.data(),4,frames*2,input)!=frames*2)return 5;
    std::fclose(input);if(!isolation(rate)||!active_release(rate))return 6;
    void *noise=native_genesis_noise_create(rate);if(!noise)return 7;
    size_t cursor=0;
    for(uint32_t frame=0;frame<frames;frame++) {
        while(cursor<events.size()&&events[cursor].frame==frame) {
            const auto &e=events[cursor++];
            switch(e.operation) {
            case 0: if(!native_genesis_noise_voice_create(noise,e.slot,e.argument,e.seed))return 8;break;
            case 1: if(!native_genesis_noise_set(noise,e.slot,e.argument,e.value,e.frames))return 9;break;
            case 2: native_genesis_noise_filter(noise,e.slot,e.argument);break;
            case 3: native_genesis_noise_action(noise,e.slot,e.argument);break;
            case 4: native_genesis_noise_reset(noise);break;
            default:return 10;
            }
        }
        native_genesis_noise_mix(noise,&pcm[frame*2]);
    }
    native_genesis_noise_destroy(noise);if(cursor!=events.size())return 11;
    FILE *output=std::fopen(argv[2],"wb");if(!output)return 12;
    const auto written=std::fwrite(pcm.data(),4,frames*2,output);std::fclose(output);return written==frames*2?0:13;
}
