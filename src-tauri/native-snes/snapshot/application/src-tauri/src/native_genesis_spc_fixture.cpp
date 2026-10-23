// SPDX-License-Identifier: LGPL-2.1-or-later
// Device-free fixture. All Core/file/event/capture storage is prepared before
// the simulated callback; superseded Cores are destroyed afterward on control.
#include "native_genesis_spc.h"
#include <algorithm>
#include <cassert>
#include <cstddef>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <iostream>
#include <new>
#include <sstream>
#include <stdexcept>
#include <string>
#include <vector>
static bool measured=false;static uint64_t allocations=0,frees=0;
void *operator new(std::size_t n){if(measured)++allocations;if(void*p=std::malloc(n?n:1))return p;throw std::bad_alloc();}
void *operator new[](std::size_t n){return ::operator new(n);}
void operator delete(void*p)noexcept{if(measured&&p)++frees;std::free(p);}
void operator delete[](void*p)noexcept{::operator delete(p);}
void operator delete(void*p,std::size_t)noexcept{::operator delete(p);}
void operator delete[](void*p,std::size_t)noexcept{::operator delete(p);}
static void require(bool ok,const char* text){if(!ok)throw std::runtime_error(text);}
static std::vector<uint8_t> file(const std::string& name){std::ifstream in(name,std::ios::binary);require(bool(in),"Cannot open SPC fixture");return std::vector<uint8_t>(std::istreambuf_iterator<char>(in),{});}
static void* prepared(uint32_t rate,const std::vector<uint8_t>& bytes,uint32_t frames=0,uint32_t mute=0){
  void*p=native_spc_create(rate);require(p,"SPC allocation failed");require(native_spc_set_mute_mask(p,mute),"Mute rejected");require(native_spc_load_control(p,bytes.data(),bytes.size()),"SPC load failed");
  while(frames){const uint32_t count=std::min(frames,16000u);require(native_spc_seek_control(p,count),"SPC seek failed");frames-=count;}return p;
}
static uint32_t hash(const uint8_t*p,size_t n){uint32_t h=2166136261u;for(size_t i=0;i<n;++i)h=(h^p[i])*16777619u;return h;}
static std::vector<uint8_t> synthetic(){
  std::vector<uint8_t>b(0x10200);const char signature[]="SNES-SPC700 Sound File Data v0.30";std::copy(signature,signature+sizeof(signature)-1,b.begin());
  b[0x21]=b[0x22]=0x1a;b[0x23]=0x1b;b[0x24]=30;b[0x26]=2;b[0x2b]=0xef;
  const uint8_t program[10]={0x8f,0,0xf2,0xe8,0x20,0xc4,0xf3,0xbc,0x2f,0xfb};std::copy(program,program+10,b.begin()+0x300);
  auto*ram=b.data()+0x100,*dsp=b.data()+0x10100;ram[0x1001]=ram[0x1003]=0x20;
  const uint8_t wave[9]={0xc3,0x13,0x57,0x75,0x31,0xfd,0xb9,0x9b,0xdf};std::copy(wave,wave+9,ram+0x2000);
  dsp[0]=64;dsp[1]=48;dsp[3]=16;dsp[7]=127;dsp[0x0c]=dsp[0x1c]=127;dsp[0x4c]=1;dsp[0x5d]=0x10;dsp[0x6c]=0x20;return b;
}
static int self_test(const std::vector<uint8_t>& bytes){
  static_assert(sizeof(NativeSpcState)==56,"Native SPC state ABI changed");
  void*a=prepared(48000,bytes),*b=prepared(48000,bytes),*reference=prepared(48000,bytes);
  std::vector<uint8_t> before(0x10200),after(0x10200),scratch(0x10200);
  native_spc_snapshot(b,before.data(),before.size());native_spc_set_playing(a,1);native_spc_set_playing(reference,1);native_spc_observe(a,1,16);
  bool exact=true,read_pure=true;NativeSpcState state{},paused{};
  measured=true;
  for(uint32_t i=0;i<8192;++i){
    native_spc_get_state(a,&state);if(i%127==0)native_spc_snapshot(a,scratch.data(),scratch.size());
    float l,r,x,y;native_spc_sample(a,&l,&r);native_spc_sample(reference,&x,&y);exact=exact&&l==x&&r==y;
  }
  native_spc_snapshot(b,after.data(),after.size());const bool isolated=before==after;
  native_spc_get_state(a,&state);native_spc_snapshot(a,before.data(),before.size());
  for(uint32_t i=0;i<1000;++i){native_spc_get_state(a,&paused);native_spc_snapshot(a,after.data(),after.size());}
  read_pure=before==after&&std::memcmp(&state,&paused,sizeof state)==0;
  native_spc_set_playing(a,0);native_spc_get_state(a,&state);
  for(uint32_t i=0;i<1024;++i){float l,r;native_spc_sample(a,&l,&r);require(l==0&&r==0,"Paused SPC sounded");}
  native_spc_get_state(a,&paused);const bool pause=state.native_frames==paused.native_frames&&state.output_frames==paused.output_frames;
  const uint32_t events=native_spc_event_count(a),lost=native_spc_overflow_take(a);native_spc_events_clear(a);const bool bounded=events<=16&&native_spc_event_count(a)==0&&native_spc_overflow_take(a)==0;
  native_spc_observe(a,0,2048);native_spc_set_mute_mask(a,255);native_spc_get_state(b,&state);const bool mute=state.mute_mask==0;
  measured=false;native_spc_destroy(a);native_spc_destroy(b);native_spc_destroy(reference);
  std::cout<<"{\"coreHeapBytes\":"<<native_spc_core_size()<<",\"stateBytes\":"<<sizeof(NativeSpcState)<<",\"independentInstances\":"<<(isolated?"true":"false")<<",\"snapshotPure\":"<<(read_pure&&exact?"true":"false")<<",\"pauseFreezesPosition\":"<<(pause?"true":"false")<<",\"independentMute\":"<<(mute?"true":"false")<<",\"boundedObserver\":"<<(bounded?"true":"false")<<",\"observerEvents\":"<<events<<",\"observerOverflow\":"<<lost<<",\"renderAllocations\":"<<allocations<<",\"renderFrees\":"<<frees<<"}\n";
  return isolated&&read_pure&&exact&&pause&&bounded&&mute&&!allocations&&!frees?0:3;
}
struct Event{uint32_t frame=0,a=0,b=0;std::string op;void*core=nullptr;std::vector<uint8_t> snapshot;NativeSpcState state{};};
int main(int argc,char**argv){std::vector<void*> cores;try{
  if(argc==2&&std::string(argv[1])=="--self-test")return self_test(synthetic());
  if(argc==3&&std::string(argv[1])=="--self-test")return self_test(file(argv[2]));
  require(argc==3,"Usage: fixture output-rate frames; timeline on stdin");
  const uint32_t rate=std::strtoul(argv[1],nullptr,10),frames=std::strtoul(argv[2],nullptr,10);require(frames&&frames<=10000000,"Bad frame count");
  std::vector<Event> events;
  for(std::string line;std::getline(std::cin,line);){if(line.empty())continue;Event e;std::istringstream in(line);require(bool(in>>e.frame>>e.op)&&e.frame<frames,"Bad event frame");
    if(e.op=="install"){std::string path;uint32_t skip,mute,playing;require(bool(in>>path>>skip>>mute>>playing),"Bad prepared Core");e.core=prepared(rate,file(path),skip,mute);native_spc_set_playing(e.core,playing);cores.push_back(e.core);}
    else if(e.op=="mute")require(bool(in>>e.a),"Missing mute");
    else if(e.op=="observe")require(bool(in>>e.a>>e.b),"Missing observer options");
    else if(e.op=="read")e.snapshot.resize(0x10200);
    else require(e.op=="play"||e.op=="pause","Unknown fixture event");events.push_back(std::move(e));
  }
  std::stable_sort(events.begin(),events.end(),[](const Event&a,const Event&b){return a.frame<b.frame;});
  void*current=native_spc_create(rate);require(current,"Core create failed");cores.push_back(current);
  std::vector<float> pcm(size_t(frames)*2);std::vector<NativeSpcEvent> observed(size_t(frames)*64+8192);size_t count=0,next=0;uint64_t lost=0;
  measured=true;
  for(uint32_t frame=0;frame<frames;++frame){
    while(next<events.size()&&events[next].frame==frame){auto&e=events[next++];
      if(e.op=="install")current=e.core;
      else if(e.op=="mute")require(native_spc_set_mute_mask(current,e.a),"Mute failed");
      else if(e.op=="play")require(native_spc_set_playing(current,1),"Play failed");
      else if(e.op=="pause")require(native_spc_set_playing(current,0),"Pause failed");
      else if(e.op=="observe")require(native_spc_observe(current,e.a,e.b),"Observe failed");
      else if(e.op=="read"){native_spc_get_state(current,&e.state);require(native_spc_snapshot(current,e.snapshot.data(),e.snapshot.size()),"Snapshot failed");}
    }
    native_spc_sample(current,&pcm[frame*2],&pcm[frame*2+1]);
    const auto n=native_spc_event_count(current);require(count+n<=observed.size(),"Fixture observer exhausted");
    std::copy_n(native_spc_event_data(current),n,observed.begin()+count);count+=n;lost+=native_spc_overflow_take(current);native_spc_events_clear(current);
  }
  measured=false;for(void*p:cores)native_spc_destroy(p);cores.clear();require(!allocations&&!frees,"Callback allocation/free");
  std::cerr<<"{\"renderAllocations\":"<<allocations<<",\"renderFrees\":"<<frees<<",\"overflow\":"<<lost<<",\"reads\":[";bool comma=false;
  for(const auto&e:events)if(e.op=="read"){
    if(comma)std::cerr<<",";comma=true;
    const auto&s=e.state;std::cerr<<"{\"frame\":"<<e.frame<<",\"nativeFrames\":"<<s.native_frames<<",\"baseNativeFrames\":"<<s.base_native_frames<<",\"outputFrames\":"<<s.output_frames<<",\"muteMask\":"<<s.mute_mask<<",\"playing\":"<<s.playing<<",\"ramHash\":"<<hash(e.snapshot.data()+0x100,65536)<<",\"dsp\":[";
    for(uint32_t i=0;i<128;++i){if(i)std::cerr<<",";std::cerr<<unsigned(e.snapshot[0x10100+i]);}std::cerr<<"]}";
  }
  std::cerr<<"],\"events\":[";for(size_t i=0;i<count;++i){if(i)std::cerr<<",";std::cerr<<observed[i].clock<<","<<observed[i].reg<<","<<observed[i].value;}std::cerr<<"]}\n";
  require(std::fwrite(pcm.data(),sizeof(float),pcm.size(),stdout)==pcm.size(),"PCM output failed");return 0;
}catch(const std::exception&e){measured=false;for(void*p:cores)native_spc_destroy(p);std::cerr<<e.what()<<"\n";return 1;}}
