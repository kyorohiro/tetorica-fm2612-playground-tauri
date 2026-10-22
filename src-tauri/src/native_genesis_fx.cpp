#include "native_genesis_fx.h"
#include <algorithm>
#include <cmath>
#include <cstdlib>
#include <cstring>
#include <new>

namespace {
// The shared C DSP uses unqualified isfinite; <cmath> only guarantees std::isfinite.
using std::isfinite;
// Every DSP function and state member below comes from the shared C source.
// Includes are transformed only to turn file storage into instance storage.
struct DSP {
#include "../native-fx/gain.inc"
#include "../native-fx/eq.inc"
#include "../native-fx/noise_gate.inc"
#include "../native-fx/compressor.inc"
#include "../native-fx/reverb.inc"
#include "../native-fx/extra_fx.inc"
#include "../native-fx/graph.inc"
    ~DSP() { for (auto &type : extra_fx_states) for (auto &state : type) free(state.delay); }
};
struct Ramp { double start=0, target=0; uint32_t elapsed=0, total=0; };
struct Unit { bool used=false; double values[8]{}; Ramp ramps[8]{}; };
struct Rack {
    DSP dsp{};
    Unit units[15][8]{};
    uint32_t rate, quantum=0;
    double beat_seconds;
    explicit Rack(uint32_t sample_rate, double beat) : rate(sample_rate), beat_seconds(beat) { dsp.graph_reset(rate); }
    static double clamp(double value,double low,double high) { return std::max(low,std::min(high,value)); }
    void extra(int type,int slot,int key,double value) { dsp.extra_set(type,slot,key,value); }
    void apply(int type,int slot) {
        auto &unit=units[type][slot]; if (!unit.used) return; const double *v=unit.values;
        switch(type) {
        case 2: dsp.gain_select(slot); dsp.gain_set(v[0],std::max(1u,static_cast<uint32_t>(llround(rate*.005)))); return;
        case 3: dsp.eq_select(slot); for(int i=0;i<3;i++)dsp.eq_set(i,v[i]); return;
        case 4: dsp.gate_select(slot); dsp.gate_set(20*log10(v[0]),v[1],v[2]*1000,v[3]*1000,v[4]*1000,0); return;
        case 5: dsp.compressor_select(slot); dsp.compressor_set(v[0],v[1],v[2]*1000,v[3]*1000,v[4],0); return;
        case 6: dsp.reverb_select(slot); dsp.reverb_set(v[0],v[1],v[2]); return;
        case 7: extra(type,slot,0,v[0]);extra(type,slot,1,v[1]);extra(type,slot,2,v[2]);break;
        case 8: extra(type,slot,0,v[0]*1000);extra(type,slot,1,v[1]);break;
        case 9: extra(type,slot,0,v[0]);break;
        case 10: extra(type,slot,0,v[0]);extra(type,slot,1,clamp(rate/v[1],100,rate));break;
        case 11: extra(type,slot,0,v[0]);extra(type,slot,1,clamp(log2(1+v[1]/v[0]),0,4));extra(type,slot,2,clamp(1/(v[2]*beat_seconds),.05,20));extra(type,slot,3,v[3]);break;
        case 12: extra(type,slot,0,v[0]*1000);extra(type,slot,1,v[1]*1000);extra(type,slot,2,clamp(1/(v[2]*beat_seconds),.05,10));extra(type,slot,3,v[3]);break;
        case 13: extra(type,slot,0,clamp(1/(v[0]*beat_seconds),.05,30));extra(type,slot,1,v[1]);extra(type,slot,2,v[2]);break;
        case 14: extra(type,slot,0,v[0]*1000);extra(type,slot,1,v[1]*1000);extra(type,slot,2,clamp(1/(v[2]*beat_seconds),.05,10));break;
        }
        const int mix_key = type==7?3:type==8?2:type==9?1:type==10?2:type==11?4:type==12?4:3;
        extra(type,slot,6,v[mix_key]);extra(type,slot,7,0);
    }
    void parameter(int type,int slot,int key,double value,uint32_t frames) {
        if(type<2||type>14||slot<0||slot>=8||key<0||key>=8||!units[type][slot].used)return;
        auto &unit=units[type][slot];auto &ramp=unit.ramps[key];ramp.total=frames;ramp.elapsed=0;ramp.start=unit.values[key];ramp.target=value;
        if(!frames) {unit.values[key]=value;if(type==6&&key==3)unit.values[2]=1-(value-200)/19800;apply(type,slot);}
    }
    void tick(float *stereo) {
        // The shared JS engine advances ramps once per 128-frame quantum.
        // Keep the same bounded update cadence; never allocate on this path.
        if(!quantum) {
            for(int type=2;type<15;type++)for(int slot=0;slot<8;slot++) {
                auto &unit=units[type][slot];if(!unit.used)continue;bool changed=false;
                for(int key=0;key<8;key++) {auto &r=unit.ramps[key];if(!r.total)continue;
                    r.elapsed=std::min(r.total,r.elapsed+128);unit.values[key]=r.start+(r.target-r.start)*r.elapsed/r.total;
                    if(type==6&&key==3)unit.values[2]=1-(unit.values[3]-200)/19800;
                    if(r.elapsed==r.total)r.total=0;changed=true;
                }
                if(changed)apply(type,slot);
            }
        }
        quantum=(quantum+1)%128;
        if(dsp.graph_root>=0)dsp.tick(dsp.graph_root,&stereo[0],&stereo[1]);
    }
};
}
extern "C" {
void *native_genesis_fx_create(uint32_t rate,const NativeFxNode *nodes,uint32_t count,const NativeFxUnit *units,uint32_t unit_count,double beat) {
    if(rate<16000||rate>384000||!nodes||count<1||count>32||unit_count>104||!std::isfinite(beat)||beat<=0)return nullptr;
    auto *rack=new(std::nothrow)Rack(rate,beat);if(!rack)return nullptr;
    rack->dsp.graph_begin();
    for(uint32_t i=0;i<count;i++) {
        const auto &node=nodes[i];int id=rack->dsp.graph_add(node.type,node.slot);
        if(id<0||node.count<0||node.count>32) {delete rack;return nullptr;}
        for(int j=0;j<node.count;j++)if(!rack->dsp.graph_append(id,node.children[j])){delete rack;return nullptr;}
    }
    if(!rack->dsp.graph_commit(count-1)){delete rack;return nullptr;}
    for(uint32_t i=0;i<unit_count;i++) {
        const auto &u=units[i];if(u.type<2||u.type>14||u.slot<0||u.slot>=8){delete rack;return nullptr;}
        auto &target=rack->units[u.type][u.slot];target.used=true;std::copy(u.values,u.values+8,target.values);
        // Any delay allocation happens here, before the graph is submitted.
        if(u.type>=7&&!rack->dsp.extra_prepare(u.type,u.slot)){delete rack;return nullptr;}
        rack->apply(u.type,u.slot);
    }
    return rack;
}
void native_genesis_fx_destroy(void *ptr){delete static_cast<Rack *>(ptr);}
void native_genesis_fx_sample(void *ptr,float *stereo){if(ptr)static_cast<Rack *>(ptr)->tick(stereo);}
void native_genesis_fx_parameter(void *ptr,int32_t type,int32_t slot,int32_t key,double value,uint32_t frames){if(ptr)static_cast<Rack *>(ptr)->parameter(type,slot,key,value,frames);}
void native_genesis_fx_tempo(void *ptr,double beat){if(!ptr)return;auto *rack=static_cast<Rack *>(ptr);rack->beat_seconds=beat;for(int t=2;t<15;t++)for(int s=0;s<8;s++)rack->apply(t,s);}
}
