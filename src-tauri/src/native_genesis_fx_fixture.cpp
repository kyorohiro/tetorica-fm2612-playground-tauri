// Offline shared-DSP comparison only. Does not open an audio device.
#include "native_genesis_fx.h"
#include <cstdio>
#include <cstdint>
#include <vector>

int main(int argc,char **argv) {
    if(argc!=3)return 2;
    FILE *input=std::fopen(argv[1],"rb");if(!input)return 3;
    uint32_t rate,count,units,frames;double beat;
    if(std::fread(&rate,4,1,input)!=1||std::fread(&count,4,1,input)!=1||std::fread(&units,4,1,input)!=1||std::fread(&frames,4,1,input)!=1||std::fread(&beat,8,1,input)!=1||count>32||units>104||frames>1000000)return 4;
    std::vector<NativeFxNode> nodes(count);std::vector<NativeFxUnit> settings(units);std::vector<float> pcm(frames*2);
    if(std::fread(nodes.data(),sizeof(NativeFxNode),count,input)!=count||std::fread(settings.data(),sizeof(NativeFxUnit),units,input)!=units||std::fread(pcm.data(),sizeof(float),frames*2,input)!=frames*2)return 5;
    std::fclose(input);
    void *graph=native_genesis_fx_create(rate,nodes.data(),count,settings.data(),units,beat);if(!graph)return 6;
    for(uint32_t frame=0;frame<frames;frame++)native_genesis_fx_sample(graph,&pcm[frame*2]);
    native_genesis_fx_destroy(graph);
    FILE *output=std::fopen(argv[2],"wb");if(!output)return 7;
    auto written=std::fwrite(pcm.data(),sizeof(float),frames*2,output);std::fclose(output);
    return written==frames*2?0:8;
}
