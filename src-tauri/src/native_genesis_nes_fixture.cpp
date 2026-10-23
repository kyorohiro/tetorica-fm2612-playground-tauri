// Offline golden comparison and callback-allocation audit. No device is opened.
// argv: sample-rate base-clock frames fds0/1
// stdin: frame write HEX_ADDRESS HEX_VALUE | load HEX_OFFSET HEX_BYTES |
// enable CHANNEL 0/1 | mute MASK | clock HZ | reset | stop | stop-channel CHANNEL
// stdout: interleaved float32 stereo.
#include "native_genesis_nes.h"
#include <algorithm>
#include <array>
#include <atomic>
#include <chrono>
#include <cstdint>
#include <cmath>
#include <cstdlib>
#include <iostream>
#include <new>
#include <sstream>
#include <stdexcept>
#include <string>
#include <vector>
namespace {
  std::atomic<bool> audit {
    false
  };
  std::atomic<uint64_t> allocations {
    0
  },deallocations {
    0
  };
  struct Event {
    uint32_t frame=0;
    std::string op;
    uint32_t a=0,b=0;
    std::vector<uint8_t>bytes;
  };
  uint32_t hex(const std::string&s) {
    size_t end=0;
    auto value=std::stoul(s,&end,16);
    if(end!=s.size()||value>0xffffffffUL)throw std::runtime_error("Invalid hex value");
    return static_cast<uint32_t>(value);
  }
}
void*operator new(std::size_t size) {
  if(audit.load(std::memory_order_relaxed))allocations.fetch_add(1,std::memory_order_relaxed);
  if(void*p=std::malloc(size?size:1))return p;
  throw std::bad_alloc();
}
void*operator new[](std::size_t size) {
  return ::operator new(size);
}
void operator delete(void*p)noexcept {
  if(p&&audit.load(std::memory_order_relaxed))deallocations.fetch_add(1,std::memory_order_relaxed);
  std::free(p);
}
void operator delete[](void*p)noexcept {
  ::operator delete(p);
}
void operator delete(void*p,std::size_t)noexcept {
  ::operator delete(p);
}
void operator delete[](void*p,std::size_t)noexcept {
  ::operator delete(p);
}
int selfTest() {
  auto require=[](bool condition,const char*message) {
    if(!condition)throw std::runtime_error(message);
  };
  require(!native_nes_create(0,1789773,true),"Invalid sample rate accepted");
  require(!native_nes_create(48000,1500000,true),"Invalid base clock accepted");
  void*core=native_nes_create(48000,1789773,true);
  require(core!=nullptr,"Creation failed");
  std::array<uint8_t,65536>ram {
  };
  for(size_t i=0;i<ram.size();++i)ram[i]=static_cast<uint8_t>(i*37+11);
  require(native_nes_load(core,ram.data(),0,ram.size())==1,"Full RAM upload failed");
  require(native_nes_load(core,ram.data(),65535,2)==0,"RAM overflow accepted");
  require(native_nes_load(core,nullptr,65536,0)==1,"Empty end-of-RAM upload failed");
  require(native_nes_read_memory(core,0xffff)==ram[0xffff],"RAM upload changed byte");
  native_nes_write(core,0x4023,2);
  native_nes_write(core,0x4089,128);
  for(int i=0;i<64;++i)native_nes_write(core,0x4040+i,i);
  native_nes_write(core,0x4087,128);
  for(int i=0;i<32;++i)native_nes_write(core,0x4088,i&7);
  native_nes_write(core,0x4089,2);
  native_nes_write(core,0x4080,128|31);
  native_nes_write(core,0x4082,123);
  native_nes_write(core,0x4083,3);
  NativeNesState state {
  };
  native_nes_get_state(core,&state);
  require(state.wave[63]==63&&state.modulation[31]==7,"FDS memory upload mismatch");
  require(state.fds_frequency==891&&state.fds_gain==31&&state.fds_master==2,"FDS applied controls mismatch");
  native_nes_write(core,0x4023,0);
  native_nes_write(core,0x4082,1);
  native_nes_get_state(core,&state);
  require(state.fds_frequency==891,"FDS IO disable ignored");
  native_nes_write(core,0x4023,2);
  native_nes_write(core,0x4010,79);
  native_nes_write(core,0x4012,0);
  native_nes_write(core,0x4013,2);
  native_nes_enable(core,4,true);
  native_nes_set_mute(core,37);
  require(native_nes_set_clock(core,2237216)==1,"Clock setter failed");
  audit.store(true);
  for(int i=0;i<5000;++i) {
    float l,r;
    native_nes_sample(core,&l,&r);
    require(std::isfinite(l)&&std::isfinite(r),"Nonfinite sample");
  }
  native_nes_stop(core);
  native_nes_get_state(core,&state);
  require(state.enable_mask==0&&state.dmc_remaining==0&&!state.fds_active&&state.fds_gain==0,"Stop left voices active");
  require(state.wave[63]==63&&state.modulation[31]==7&&state.fds_frequency==891,"Stop lost FDS patch");
  require(native_nes_read_memory(core,0xffff)==ram[0xffff],"Stop lost DMC RAM");
  native_nes_reset(core);
  native_nes_get_state(core,&state);
  require(state.wave[63]==0&&state.modulation[31]==0&&native_nes_read_memory(core,0xffff)==0,"Reset retained RAM");
  require(state.mute_mask==37&&state.clock==2237216,"Reset lost output settings");
  audit.store(false);
  void *second=native_nes_create(48000,1662607,false),*twin=native_nes_create(48000,1789773,true);
  require(second&&twin,"Independent core creation failed");
  auto program=[](void *chip) {
    native_nes_write(chip,0x4015,3);
    for(uint16_t base:{0x4000,0x4004}) {
      native_nes_write(chip,base,0xbf);
      native_nes_write(chip,base+2,0xfe);
      native_nes_write(chip,base+3,0x01);
    }
  };
  audit.store(true);
  native_nes_set_clock(core,1789773);native_nes_set_mute(core,0);native_nes_reset(core);
  native_nes_load(core,ram.data(),0,ram.size());native_nes_load(twin,ram.data(),0,ram.size());
  native_nes_load(second,ram.data()+1,0,ram.size()-1);
  program(core);program(twin);program(second);native_nes_set_clock(second,3325214);
  native_nes_write(core,0x4089,128);native_nes_write(core,0x4040,27);native_nes_write(core,0x4089,2);
  native_nes_write(twin,0x4089,128);native_nes_write(twin,0x4040,27);native_nes_write(twin,0x4089,2);
  native_nes_write(second,0x4089,128);native_nes_write(second,0x4040,49);
  NativeNesState before{},after{},other{};native_nes_get_state(core,&before);native_nes_get_state(second,&other);
  require(before.wave[0]==27&&other.wave[0]==0&&before.clock==1789773&&other.clock==3325214&&other.base_clock==1662607,"Independent wave/clock/FDS state changed");
  require(native_nes_read_memory(core,0)==ram[0]&&native_nes_read_memory(second,0)==ram[1],"DMC RAM isolation failed");
  for(unsigned i=0;i<300;++i){native_nes_get_state(core,&after);native_nes_active_mask(core);}
  require(!std::memcmp(&before,&after,sizeof before),"Observation mutated applied state");
  for(unsigned i=0;i<8192;++i){float l,r,tl,tr;native_nes_sample(core,&l,&r);native_nes_sample(twin,&tl,&tr);require(l==tl&&r==tr,"Observation changed phase or PCM");}
  native_nes_get_state(core,&before);native_nes_stop_channel(core,0);native_nes_get_state(core,&after);
  require((native_nes_active_mask(core)&3)==2&&after.enable_mask==2,"Individual APU stop changed other channel");
  require(!std::memcmp(before.registers,after.registers,0x15)&&after.wave[0]==27,"Individual APU stop lost patch or wave");
  native_nes_write(core,0x4080,151);native_nes_write(core,0x4083,1);
  native_nes_write(core,0x4087,128);native_nes_write(core,0x4088,7);native_nes_get_state(core,&before);
  require(native_nes_active_mask(core)&32,"FDS active mask missing");
  native_nes_write(core,0x4023,0);native_nes_stop_channel(core,5);native_nes_get_state(core,&after);
  require(!(native_nes_active_mask(core)&32)&&after.fds_gain==23&&after.wave[0]==27&&after.modulation[0]==7&&after.enable_mask==2&&after.fds_io==0&&after.registers[0x23]==0,"Individual FDS stop lost patch or sibling/IO");
  native_nes_write(core,0x4011,97);native_nes_stop_channel(core,4);native_nes_get_state(core,&after);
  require(after.dmc_output==0&&after.registers[0x11]==97&&after.enable_mask==2,"DMC stop retained DAC output or changed authored level/sibling");
  native_nes_stop_channel(core,255);native_nes_get_state(core,&state);require(!std::memcmp(&state,&after,sizeof state),"Invalid stop channel mutated core");
  native_nes_stop(core);native_nes_write(core,0x4010,15);native_nes_write(core,0x4012,0);native_nes_write(core,0x4013,0);native_nes_enable(core,4,true);
  require(native_nes_active_mask(core)&16,"DMC active lifetime missing last prefetched byte");
  for(unsigned i=0;i<4096;++i){float l,r;native_nes_sample(core,&l,&r);}
  require(!(native_nes_active_mask(core)&16),"Completed DMC remained held");
  require(!native_nes_set_clock(core,99999)&&!native_nes_set_clock(core,100000001),"Clock bounds accepted");
  require(native_nes_core_size()>=65536&&native_nes_core_size()<131072,"Core reservation must include one64KiB RAM and bounded fixed state");
  audit.store(false);
  void *dmcCore=native_nes_create(48000,1789773,false),*dmcTwin=native_nes_create(48000,1789773,false);
  require(dmcCore&&dmcTwin,"DMC gate-isolation core allocation failed");
  const uint8_t finalByte=0xff;
  audit.store(true);
  for(void *chip:{dmcCore,dmcTwin}) {
    native_nes_load(chip,&finalByte,0xc000,1);native_nes_write(chip,0x4010,15);native_nes_write(chip,0x4011,64);native_nes_write(chip,0x4012,0);native_nes_write(chip,0x4013,0);native_nes_enable(chip,4,true);
  }
  native_nes_get_state(dmcCore,&before);
  require(before.dmc_remaining==0&&(native_nes_active_mask(dmcCore)&16),"DMC final-byte fixture was not prefetched");
  native_nes_write_enable(dmcCore,0,16);native_nes_write_enable(dmcCore,16,16);
  native_nes_enable_channel(dmcCore,0,true);native_nes_stop_channel(dmcCore,0);native_nes_enable_channel(dmcCore,1,false);
  native_nes_get_state(dmcCore,&after);
  require(after.dmc_remaining==0&&after.dmc_address==before.dmc_address&&after.dmc_output==before.dmc_output&&(native_nes_active_mask(dmcCore)&16),"Protected enable mask refilled or truncated the prefetched DMC byte");
  for(unsigned i=0;i<2048;++i) {
    float l,r,tl,tr;native_nes_sample(dmcCore,&l,&r);native_nes_sample(dmcTwin,&tl,&tr);
    require(l==tl&&r==tr&&native_nes_active_mask(dmcCore)==native_nes_active_mask(dmcTwin),"Other gates changed DMC final-byte PCM or lifetime");
  }
  require(!(native_nes_active_mask(dmcCore)&16),"Protected mask restarted a completed DMC sample");
  audit.store(false);
  native_nes_destroy(dmcCore);native_nes_destroy(dmcTwin);
  require(allocations.load()==0&&deallocations.load()==0,"Audio callback allocated or freed memory");
  native_nes_destroy(core);native_nes_destroy(second);native_nes_destroy(twin);
  std::cerr<<"{\"independentInstances\":true,\"dmcRamIsolation\":true,\"fdsOffRejectsExpansionWrites\":true,\"readOnlyObservation\":true,\"stopPreservesRamWaveClock\":true,\"resetClearsMemoryPreservesClockMute\":true,\"individualChannelStop\":true,\"fdsIoDisabledStop\":true,\"dmcStopSilencesDacRetainsAuthoredLevel\":true,\"finiteDmcLifetime\":true,\"protectedDmcFinalBytePreservesPcmAndLifetime\":true,\"clockBounds\":true,\"coreHeapBytes\":"<<native_nes_core_size()<<",\"renderAllocations\":0,\"renderFrees\":0}\n";
  return 0;
}
int memoryCopyBenchmark() {
  constexpr unsigned trials=10000,maximum=16;
  std::array<void*,maximum> cores{};
  std::array<uint8_t,65536> bytes{};
  for(size_t i=0;i<bytes.size();++i)bytes[i]=uint8_t(i*37+19);
  for(auto &core:cores) {
    core=native_nes_create(48000,1789773,true);
    if(!core)throw std::runtime_error("Benchmark core allocation failed");
  }
  // Preallocate every sample and core before measuring. Timings include the
  // validated C ABI memcpy call, but exclude coverage/command/rack bookkeeping.
  std::vector<double> timings(trials);
  std::cout<<"{\"deviceOpened\":false,\"trials\":"<<trials<<",\"warmCache\":true,\"cases\":[";
  for(unsigned count:{1U,16U}) {
    for(unsigned warmup=0;warmup<200;++warmup)for(unsigned i=0;i<count;++i)native_nes_load(cores[i],bytes.data(),0,bytes.size());
    audit.store(true);
    for(unsigned trial=0;trial<trials;++trial) {
      const auto begin=std::chrono::steady_clock::now();
      for(unsigned i=0;i<count;++i)native_nes_load(cores[i],bytes.data(),0,bytes.size());
      timings[trial]=std::chrono::duration<double,std::micro>(std::chrono::steady_clock::now()-begin).count();
    }
    audit.store(false);
    std::sort(timings.begin(),timings.end());
    if(count!=1)std::cout<<',';
    std::cout<<"{\"instances\":"<<count<<",\"bytes\":"<<count*bytes.size()<<",\"medianMicroseconds\":"<<timings[trials/2]<<",\"p99Microseconds\":"<<timings[trials*99/100]<<",\"maximumMicroseconds\":"<<timings.back()<<'}';
  }
  std::cout<<"],\"renderAllocations\":"<<allocations.load()<<",\"renderFrees\":"<<deallocations.load()<<"}\n";
  for(auto core:cores)native_nes_destroy(core);
  if(allocations.load()||deallocations.load())throw std::runtime_error("Memory copy benchmark allocated or freed during application");
  return 0;
}
int main(int argc,char**argv) {
  void*core=nullptr;
  try {
    if(argc==2&&std::string(argv[1])=="--self-test")return selfTest();
    if(argc==2&&std::string(argv[1])=="--memory-copy-benchmark")return memoryCopyBenchmark();
    if(argc!=5)throw std::runtime_error("Usage: fixture sample-rate base-clock frames fds0/1");
    uint32_t rate=std::stoul(argv[1]),frames=std::stoul(argv[3]);
    double clock=std::stod(argv[2]);
    bool fds=std::stoul(argv[4])!=0;
    if(!frames||frames>16*1024*1024)throw std::runtime_error("Invalid frames");
    core=native_nes_create(rate,clock,fds);
    if(!core)throw std::runtime_error("Native NES creation failed");
    std::vector<Event>events;
    for(std::string line;std::getline(std::cin,line);) {
      if(line.empty())continue;
      std::istringstream in(line);
      Event event;
      if(!(in>>event.frame>>event.op)||event.frame>=frames)throw std::runtime_error("Invalid event");
      if(event.op=="write") {
        std::string a,b;
        if(!(in>>a>>b))throw std::runtime_error("Invalid write");
        event.a=hex(a);
        event.b=hex(b);
        if(event.a>65535||event.b>255)throw std::runtime_error("Write outside range");
      } else if(event.op=="load") {
        std::string a,data;
        if(!(in>>a>>data)||data.size()%2)throw std::runtime_error("Invalid RAM upload");
        event.a=hex(a);
        for(size_t i=0;i<data.size();i+=2) {
          auto byte=hex(data.substr(i,2));
          if(byte>255)throw std::runtime_error("Invalid RAM byte");
          event.bytes.push_back(byte);
        }
        if(event.a>65536||event.bytes.size()>65536-event.a)throw std::runtime_error("RAM outside range");
      } else if(event.op=="enable") {
        if(!(in>>event.a>>event.b)||event.a>4||event.b>1)throw std::runtime_error("Invalid enable");
      } else if(event.op=="stop-channel") {
        if(!(in>>event.a)||event.a>5)throw std::runtime_error("Invalid stop channel");
      } else if(event.op=="mute") {
        if(!(in>>event.a)||event.a>63)throw std::runtime_error("Invalid mask");
      } else if(event.op=="clock") {
        if(!(in>>event.a)||event.a<100000||event.a>100000000)throw std::runtime_error("Invalid clock");
      } else if(event.op!="reset"&&event.op!="stop")throw std::runtime_error("Unknown event");
      events.push_back(std::move(event));
    }
    std::stable_sort(events.begin(),events.end(),[](const Event&a,const Event&b) {
      return a.frame<b.frame;
    });
    std::vector<float>pcm(static_cast<size_t>(frames)*2);
    size_t next=0;
    audit.store(true);
    for(uint32_t frame=0;frame<frames;++frame) {
      while(next<events.size()&&events[next].frame==frame) {
        const auto&e=events[next++];
        if(e.op=="write")native_nes_write(core,e.a,e.b);
        else if(e.op=="load") {
          if(!native_nes_load(core,e.bytes.data(),e.a,e.bytes.size()))throw std::runtime_error("RAM upload failed");
        } else if(e.op=="enable")native_nes_enable(core,e.a,e.b!=0);
        else if(e.op=="mute")native_nes_set_mute(core,e.a);
        else if(e.op=="clock")native_nes_set_clock(core,e.a);
        else if(e.op=="reset")native_nes_reset(core);
        else if(e.op=="stop")native_nes_stop(core);
        else if(e.op=="stop-channel")native_nes_stop_channel(core,e.a);
      }
      native_nes_sample(core,&pcm[frame*2],&pcm[frame*2+1]);
    }
    audit.store(false);
    if(allocations.load()||deallocations.load())throw std::runtime_error("Audio callback allocated or freed memory");
    native_nes_destroy(core);
    core=nullptr;
    std::cout.write(reinterpret_cast<const char*>(pcm.data()),pcm.size()*sizeof(float));
    if(!std::cout)throw std::runtime_error("PCM output failed");
    return 0;
  } catch(const std::exception&error) {
    audit.store(false);
    if(core)native_nes_destroy(core);
    std::cerr<<error.what()<<'\n';
    return 1;
  }
}
