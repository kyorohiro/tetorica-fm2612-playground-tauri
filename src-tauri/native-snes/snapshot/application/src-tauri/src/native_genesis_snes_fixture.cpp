// Offline PCM fixture and callback new/delete audit. No output device is opened.
#include "native_genesis_snes.h"
#include <algorithm>
#include <atomic>
#include <cmath>
#include <cstdlib>
#include <cstring>
#include <iostream>
#include <new>
#include <sstream>
#include <stdexcept>
#include <string>
#include <vector>
namespace {
std::atomic<bool> audit{false};
std::atomic<unsigned> allocations{0},deallocations{0};
struct Event {uint32_t frame=0,a=0,b=0;std::string op;std::vector<uint8_t> bytes,registers;std::vector<uint32_t> words;};
uint32_t hex(const std::string& text){return static_cast<uint32_t>(std::stoul(text,nullptr,16));}
std::vector<uint8_t> bytes(const std::string& text){if(text.size()%2)throw std::runtime_error("Invalid hex bytes");std::vector<uint8_t> out(text.size()/2);for(size_t i=0;i<out.size();++i)out[i]=hex(text.substr(i*2,2));return out;}
void require(bool ok,const char*message){if(!ok)throw std::runtime_error(message);}
}
void* operator new(std::size_t n){if(audit.load())++allocations;if(void* p=std::malloc(n?n:1))return p;throw std::bad_alloc();}
void* operator new[](std::size_t n){return ::operator new(n);}
void operator delete(void* p)noexcept{if(p&&audit.load())++deallocations;std::free(p);}
void operator delete[](void* p)noexcept{::operator delete(p);}
void operator delete(void* p,std::size_t)noexcept{::operator delete(p);}
void operator delete[](void* p,std::size_t)noexcept{::operator delete(p);}
int self_test(){
  require(!native_snes_create(0),"Invalid output rate accepted");
  require(!native_snes_create(384001),"Output rate above Native limit accepted");
  auto* high_rate=native_snes_create(384000);require(high_rate,"384 kHz output rate rejected");native_snes_destroy(high_rate);
  auto* c=native_snes_create(48000);require(c,"Creation failed");
  const uint8_t wave[9]={0xb3,0x12,0x34,0x56,0x78,0x9a,0xbc,0xde,0xf0};
  uint8_t directory[4]={0,4,0,4};require(native_snes_load(c,wave,0x400,9),"Upload failed");require(native_snes_load(c,directory,0,4),"Directory failed");require(!native_snes_load(c,wave,65535,9),"Overflow accepted");
  const uint8_t* ram_view=native_snes_ram_data(c);require(ram_view&&ram_view[0x400]==0xb3,"Borrowed RAM view differs from loaded memory");
  native_snes_write(c,0x6c,0x20);native_snes_write(c,0x0c,127);native_snes_write(c,0x1c,127);native_snes_write(c,0x5d,0);native_snes_write(c,0,100);native_snes_write(c,1,100);native_snes_write(c,3,0x10);native_snes_write(c,7,127);
  audit=true;native_snes_key_on(c,0);
  for(int i=0;i<4096;++i){float l,r;native_snes_sample(c,&l,&r);require(std::isfinite(l)&&std::isfinite(r),"Nonfinite output");}
  NativeSnesState state{};native_snes_get_state(c,&state);require(state.native_frames>2000,"DSP did not advance");native_snes_stop(c);native_snes_get_state(c,&state);require(state.registers[0x5c]==255&&state.registers[0x4c]==0,"Stop left gates open");require(native_snes_read_memory(c,0x400)==0xb3&&state.registers[7]==127,"Stop lost BRR/patch");require(native_snes_ram_data(c)==ram_view&&ram_view[0x400]==0xb3,"Stop invalidated the borrowed RAM allocation");native_snes_reset(c);require(native_snes_read_memory(c,0x400)==0,"Reset retained RAM");require(native_snes_ram_data(c)==ram_view&&ram_view[0x400]==0,"Reset invalidated the borrowed RAM allocation");audit=false;
  require(!allocations&&!deallocations,"Callback allocated or freed memory");native_snes_destroy(c);std::cerr<<"Native SNES validation/Stop/reset/callback allocation checks passed\n";return 0;
}
int extra_self_test(){
  static_assert(sizeof(NativeSnesState)==152,"Native S-DSP state ABI changed");
  auto* a=native_snes_create(48000);auto* b=native_snes_create(48000);auto* reference=native_snes_create(48000);require(a&&b&&reference,"Independent creation failed");
  const uint8_t wave[18]={0xb0,0x12,0x34,0x56,0x78,0x76,0x54,0x32,0x10,0xb3,0xfe,0xdc,0xba,0x98,0x9a,0xbc,0xde,0xf0};
  uint8_t directory[4]={0,4,0,4};
  for(void* p:{a,reference}){
    native_snes_load(p,wave,0x400,sizeof wave);native_snes_load(p,directory,0,4);
    native_snes_write(p,0x6c,0x20);native_snes_write(p,0x0c,127);native_snes_write(p,0x1c,127);
    native_snes_write(p,0,100);native_snes_write(p,1,73);native_snes_write(p,3,0x10);native_snes_write(p,7,127);native_snes_key_on(p,0);
  }
  std::vector<uint8_t> timeline_ram(65536),timeline_dsp(128),other_ram(65536),copy(65536);
  std::vector<uint32_t> events{0,0,0x5c,0,0,0,0x4c,1,1701,0,2,0x80,3333,1,0x402,0xff};
  std::copy(wave,wave+sizeof wave,timeline_ram.begin()+0x400);std::copy(directory,directory+4,timeline_ram.begin());
  timeline_dsp[0x6c]=0x20;timeline_dsp[0x0c]=timeline_dsp[0x1c]=127;timeline_dsp[0]=timeline_dsp[1]=100;timeline_dsp[3]=16;timeline_dsp[7]=127;
  other_ram=timeline_ram;
  const uint8_t* a_ram=native_snes_ram_data(a);const uint8_t* b_ram=native_snes_ram_data(b);
  NativeSnesState before{},after{};native_snes_get_state(b,&before);
  bool exact=true,isolated=true,read_pure=true,protected_gates=true;
  audit=true;
  for(uint32_t frame=0;frame<8192;++frame){
    NativeSnesState state{};native_snes_get_state(a,&state);native_snes_read_memory(a,0x400+(frame%18));
    if(frame%127==0)native_snes_get_ram(a,copy.data());
    float l,r,x,y;native_snes_sample(a,&l,&r);native_snes_sample(reference,&x,&y);exact=exact&&l==x&&r==y;
  }
  native_snes_get_state(b,&after);isolated=isolated&&std::memcmp(&before,&after,sizeof before)==0&&b_ram[0x400]==0;
  native_snes_get_state(a,&before);for(int i=0;i<1000;++i)native_snes_get_state(a,&after);read_pure=std::memcmp(&before,&after,sizeof before)==0;
  native_snes_stop(a);native_snes_get_state(a,&after);const bool stopped=a_ram==native_snes_ram_data(a)&&a_ram[0x400]==wave[0]&&after.registers[7]==127&&after.registers[0x4c]==0&&after.registers[0x5c]==255;
  native_snes_prepare_timeline(b,timeline_ram.data(),timeline_dsp.data(),events.data(),events.size()/4,4096*32);native_snes_play_timeline(b);
  for(int i=0;i<128;++i){float l,r;native_snes_sample(b,&l,&r);}
  native_snes_get_state(b,&before);native_snes_reset(a);native_snes_get_state(b,&after);
  const bool timeline_isolated=std::memcmp(&before,&after,sizeof before)==0&&after.timeline_loaded&&after.timeline_playing&&after.timeline_position>4&&timeline_ram==other_ram;
  const bool reset=a_ram==native_snes_ram_data(a)&&a_ram[0x400]==0;
  native_snes_key_on(a,0);native_snes_write_protected(a,0x4c,2,1);native_snes_write_protected(a,0x5c,255,1);native_snes_get_state(a,&after);protected_gates=after.registers[0x4c]==3&&after.registers[0x5c]==254;
  native_snes_stop_timeline(b);native_snes_get_state(b,&before);for(int i=0;i<64;++i){float l,r;native_snes_sample(b,&l,&r);require(l==0&&r==0,"Paused timeline sounded");}native_snes_get_state(b,&after);
  const bool pause_pure=before.timeline_loaded&&!after.timeline_playing&&before.timeline_position==after.timeline_position;
  audit=false;native_snes_destroy(a);native_snes_destroy(b);native_snes_destroy(reference);
  std::cout<<"{\"stateBytes\":"<<sizeof(NativeSnesState)<<",\"coreHeapBytes\":"<<native_snes_core_size()<<",\"independentInstances\":"<<(isolated?"true":"false")<<",\"readPure\":"<<(read_pure&&exact?"true":"false")<<",\"timelineIndependent\":"<<(timeline_isolated?"true":"false")<<",\"pausedTimelineDoesNotAdvance\":"<<(pause_pure?"true":"false")<<",\"protectedGates\":"<<(protected_gates?"true":"false")<<",\"stopRetainsRamAndPatch\":"<<(stopped?"true":"false")<<",\"resetClearsRam\":"<<(reset?"true":"false")<<",\"renderAllocations\":"<<allocations<<",\"renderFrees\":"<<deallocations<<"}\n";
  return isolated&&read_pure&&exact&&timeline_isolated&&pause_pure&&protected_gates&&stopped&&reset&&!allocations&&!deallocations?0:2;
}
int main(int argc,char**argv){void* c=nullptr;try{
  if(argc==2&&std::string(argv[1])=="--self-test")return self_test();
  if(argc==2&&std::string(argv[1])=="--self-test-extra")return extra_self_test();
  require(argc==3,"Usage: fixture output-rate frames");const uint32_t rate=std::stoul(argv[1]),frames=std::stoul(argv[2]);require(frames&&frames<16000000,"Invalid frame count");
  std::vector<Event> events;for(std::string line;std::getline(std::cin,line);){if(line.empty())continue;Event e;std::istringstream in(line);require(bool(in>>e.frame>>e.op)&&e.frame<frames,"Invalid event");std::string a,b;
    if(e.op=="write"){require(bool(in>>a>>b),"Invalid write");e.a=hex(a);e.b=hex(b);require(e.a<128&&e.b<256,"Register/value overflow");}
    else if(e.op=="load"){require(bool(in>>a>>b),"Invalid upload");e.a=hex(a);e.bytes=bytes(b);require(e.a<=65536&&e.bytes.size()<=65536-e.a,"RAM overflow");}
    else if(e.op=="key-on"||e.op=="key-off"){require(bool(in>>e.a)&&e.a<8,"Invalid channel");}
    else if(e.op=="timeline"){std::string ram,dsp;require(bool(in>>e.a>>ram>>dsp),"Invalid timeline");e.bytes=bytes(ram);e.registers=bytes(dsp);require(e.bytes.size()==65536&&e.registers.size()==128&&e.a>=32&&e.a%32==0,"Invalid timeline state");while(in>>a)e.words.push_back(hex(a));require(e.words.size()%4==0,"Invalid timeline events");}
    else require(e.op=="reset"||e.op=="stop"||e.op=="play-timeline"||e.op=="stop-timeline"||e.op=="read","Unknown event");events.push_back(std::move(e));}
  std::stable_sort(events.begin(),events.end(),[](const Event&a,const Event&b){return a.frame<b.frame;});std::vector<float> pcm(frames*2);c=native_snes_create(rate);require(c,"Creation failed");size_t index=0;audit=true;
  for(uint32_t frame=0;frame<frames;++frame){while(index<events.size()&&events[index].frame==frame){const auto& e=events[index++];if(e.op=="write")native_snes_write(c,e.a,e.b);else if(e.op=="load")native_snes_load(c,e.bytes.data(),e.a,e.bytes.size());else if(e.op=="key-on")native_snes_key_on(c,e.a);else if(e.op=="key-off")native_snes_key_off(c,e.a);else if(e.op=="reset")native_snes_reset(c);else if(e.op=="stop")native_snes_stop(c);else if(e.op=="timeline")native_snes_prepare_timeline(c,e.bytes.data(),e.registers.data(),e.words.data(),e.words.size()/4,e.a);else if(e.op=="play-timeline")native_snes_play_timeline(c);else if(e.op=="stop-timeline")native_snes_stop_timeline(c);else if(e.op=="read"){NativeSnesState state{};native_snes_get_state(c,&state);native_snes_read_memory(c,0x400);}}native_snes_sample(c,&pcm[frame*2],&pcm[frame*2+1]);}
  audit=false;require(!allocations&&!deallocations,"Callback allocated or freed memory");native_snes_destroy(c);c=nullptr;std::cout.write(reinterpret_cast<const char*>(pcm.data()),pcm.size()*sizeof(float));return std::cout?0:1;
}catch(const std::exception& e){audit=false;if(c)native_snes_destroy(c);std::cerr<<e.what()<<'\n';return 1;}}
