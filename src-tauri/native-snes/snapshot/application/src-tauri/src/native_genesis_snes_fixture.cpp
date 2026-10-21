// Offline PCM fixture and callback new/delete audit. No output device is opened.
#include "native_genesis_snes.h"
#include <algorithm>
#include <atomic>
#include <cmath>
#include <cstdlib>
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
int main(int argc,char**argv){void* c=nullptr;try{
  if(argc==2&&std::string(argv[1])=="--self-test")return self_test();
  require(argc==3,"Usage: fixture output-rate frames");const uint32_t rate=std::stoul(argv[1]),frames=std::stoul(argv[2]);require(frames&&frames<16000000,"Invalid frame count");
  std::vector<Event> events;for(std::string line;std::getline(std::cin,line);){if(line.empty())continue;Event e;std::istringstream in(line);require(bool(in>>e.frame>>e.op)&&e.frame<frames,"Invalid event");std::string a,b;
    if(e.op=="write"){require(bool(in>>a>>b),"Invalid write");e.a=hex(a);e.b=hex(b);require(e.a<128&&e.b<256,"Register/value overflow");}
    else if(e.op=="load"){require(bool(in>>a>>b),"Invalid upload");e.a=hex(a);e.bytes=bytes(b);require(e.a<=65536&&e.bytes.size()<=65536-e.a,"RAM overflow");}
    else if(e.op=="key-on"||e.op=="key-off"){require(bool(in>>e.a)&&e.a<8,"Invalid channel");}
    else if(e.op=="timeline"){std::string ram,dsp;require(bool(in>>e.a>>ram>>dsp),"Invalid timeline");e.bytes=bytes(ram);e.registers=bytes(dsp);require(e.bytes.size()==65536&&e.registers.size()==128&&e.a>=32&&e.a%32==0,"Invalid timeline state");while(in>>a)e.words.push_back(hex(a));require(e.words.size()%4==0,"Invalid timeline events");}
    else require(e.op=="reset"||e.op=="stop"||e.op=="play-timeline"||e.op=="stop-timeline","Unknown event");events.push_back(std::move(e));}
  std::stable_sort(events.begin(),events.end(),[](const Event&a,const Event&b){return a.frame<b.frame;});std::vector<float> pcm(frames*2);c=native_snes_create(rate);require(c,"Creation failed");size_t index=0;audit=true;
  for(uint32_t frame=0;frame<frames;++frame){while(index<events.size()&&events[index].frame==frame){const auto& e=events[index++];if(e.op=="write")native_snes_write(c,e.a,e.b);else if(e.op=="load")native_snes_load(c,e.bytes.data(),e.a,e.bytes.size());else if(e.op=="key-on")native_snes_key_on(c,e.a);else if(e.op=="key-off")native_snes_key_off(c,e.a);else if(e.op=="reset")native_snes_reset(c);else if(e.op=="stop")native_snes_stop(c);else if(e.op=="timeline")native_snes_prepare_timeline(c,e.bytes.data(),e.registers.data(),e.words.data(),e.words.size()/4,e.a);else if(e.op=="play-timeline")native_snes_play_timeline(c);else if(e.op=="stop-timeline")native_snes_stop_timeline(c);}native_snes_sample(c,&pcm[frame*2],&pcm[frame*2+1]);}
  audit=false;require(!allocations&&!deallocations,"Callback allocated or freed memory");native_snes_destroy(c);c=nullptr;std::cout.write(reinterpret_cast<const char*>(pcm.data()),pcm.size()*sizeof(float));return std::cout?0:1;
}catch(const std::exception& e){audit=false;if(c)native_snes_destroy(c);std::cerr<<e.what()<<'\n';return 1;}}
