// Hardware-free Native Nuked regression fixture; never linked into the app.
#include <algorithm>
#include <atomic>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <iostream>
#include <new>
#include <string>
#include <vector>
extern "C" {
void *native_genesis_create_nuked(uint32_t);
void native_genesis_destroy(void *);
void native_genesis_reset(void *);
void native_genesis_stop_voices(void *);
void native_genesis_write(void *,uint8_t,uint8_t,uint8_t);
void native_genesis_psg_write(void *,uint8_t);
void native_genesis_psg_reset(void *);
void native_genesis_psg_mute(void *,uint8_t);
void native_genesis_psg_stereo(void *,uint8_t);
void native_genesis_mix(void *,const float *);
void native_genesis_sample(void *,float *);
float native_genesis_master(void *);
uint8_t native_genesis_read(void *,uint32_t);
uint8_t native_genesis_read_status(void *);
uint8_t native_genesis_get_irq(void *);
}
static bool auditing=false;
static uint64_t allocations=0,deallocations=0,c_allocations=0,c_deallocations=0;
extern "C" void *tetorica_nuked_fixture_malloc(std::size_t n){if(auditing)++c_allocations;return std::malloc(n);}
extern "C" void tetorica_nuked_fixture_free(void *p){if(auditing&&p)++c_deallocations;std::free(p);}
#ifdef TETORICA_NUKED_ISOLATED_FIXTURE
// Selected Nuked must never construct or call the alternate ymfm provider.
// These link-time fail-fast stubs make that invariant executable.
extern "C" {
void *ym2612_create(){std::abort();}
void ym2612_destroy(void *){std::abort();}
void ym2612_reset(void *){std::abort();}
void ym2612_write(void *,uint32_t,uint8_t){std::abort();}
uint32_t ym2612_sample_rate(void *,uint32_t){std::abort();}
void ym2612_generate(void *,float *,float *,uint32_t){std::abort();}
uint8_t ym2612_read(void *,uint32_t){std::abort();}
uint8_t ym2612_read_status(void *){std::abort();}
uint8_t ym2612_get_irq(void *){std::abort();}
}
#endif
void *operator new(std::size_t n){if(auditing)++allocations;if(void *p=std::malloc(n?n:1))return p;throw std::bad_alloc();}
void *operator new[](std::size_t n){return ::operator new(n);}
void operator delete(void *p)noexcept{if(auditing&&p)++deallocations;std::free(p);}
void operator delete[](void *p)noexcept{::operator delete(p);}
void operator delete(void *p,std::size_t)noexcept{::operator delete(p);}
void operator delete[](void *p,std::size_t)noexcept{::operator delete(p);}
static void patch(void *p,uint8_t port,uint8_t ch){
 for(uint8_t op:{0,4,8,12}){native_genesis_write(p,port,0x30+ch+op,1);native_genesis_write(p,port,0x40+ch+op,20);native_genesis_write(p,port,0x50+ch+op,31);native_genesis_write(p,port,0x80+ch+op,15);}
 native_genesis_write(p,port,0xb0+ch,7);native_genesis_write(p,port,0xb4+ch,0xc0);native_genesis_write(p,port,0xa4+ch,0x22);native_genesis_write(p,port,0xa0+ch,0x84);native_genesis_write(p,0,0x28,0xf0|ch|(port?4:0));
}
static int self_test(){
 if(native_genesis_create_nuked(0)||native_genesis_create_nuked(7999)||native_genesis_create_nuked(384001))return 1;
 void *p=native_genesis_create_nuked(96000);if(!p)return 1;
 float pcm[2],gains[5]={.7f,.8f,.6f,.9f,.5f};
 for(int i=0;i<500;i++)native_genesis_sample(p,pcm);
 if(std::fabs(pcm[0]-float(.024169921875*.9))>1e-7f)return 2;
 bool sounded=false;for(uint8_t ch=0;ch<6;ch++)patch(p,ch/3,ch%3);
 auditing=true;
 for(int i=0;i<20000;i++){
  if(i==101)native_genesis_mix(p,gains);
  if(i==200)native_genesis_psg_write(p,0x94);
  if(i==701)native_genesis_psg_stereo(p,0x91);
  if(i==1001)native_genesis_write(p,0,0x27,0x85);
  if(i==1703)native_genesis_stop_voices(p);
  if(i==2801)native_genesis_write(p,0,0x28,0xf0);
  if(i==3901)native_genesis_reset(p);
  if(i==5101)patch(p,1,2);
  if(i==6101)native_genesis_psg_mute(p,0);
  if(i==7101)native_genesis_psg_reset(p);
  native_genesis_sample(p,pcm);
  if(!std::isfinite(pcm[0])||!std::isfinite(pcm[1]))return 3;
  if(i>=1703&&i<2801&&(pcm[0]!=0||pcm[1]!=0))return 4;
  if(i>100&&i<1703&&std::fabs(pcm[0])>.05f)sounded=true;
  native_genesis_read_status(p);native_genesis_get_irq(p);for(int port=0;port<4;port++)native_genesis_read(p,port);
 }
 auditing=false;
 if(!sounded||allocations||deallocations||c_allocations||c_deallocations)return 5;
 if(native_genesis_master(p)!=gains[4])return 8;
 // Every path that can restart voices must reopen a stopped core, while
 // register edits and mutes alone must retain an exactly silent Stop.
 for(int trigger=0;trigger<5;trigger++){
  native_genesis_reset(p);patch(p,0,0);native_genesis_stop_voices(p);
  native_genesis_write(p,0,0xb0,7);native_genesis_psg_stereo(p,255);native_genesis_write(p,0,0x28,0xf3);native_genesis_write(p,0,0x28,0xf7);native_genesis_write(p,0,0x27,0xc5);
  for(int i=0;i<300;i++){native_genesis_sample(p,pcm);if(pcm[0]||pcm[1])return 6;}
  if(trigger==0)native_genesis_write(p,0,0x28,0xf0);
  if(trigger==1){native_genesis_write(p,0,0x2b,128);native_genesis_write(p,0,0x2a,220);}
  if(trigger==2)native_genesis_psg_write(p,0x94);
  if(trigger==3){native_genesis_write(p,0,0x24,255);native_genesis_write(p,0,0x25,3);native_genesis_write(p,0,0x27,0x85);}
  if(trigger==4){native_genesis_psg_write(p,0xe4);native_genesis_psg_write(p,0xf4);}
  bool changed=false;for(int i=0;i<400;i++){native_genesis_sample(p,pcm);changed|=pcm[0]!=0||pcm[1]!=0;}if(!changed)return 7;
 }
 native_genesis_destroy(p);
 std::puts("Native Nuked self-test passed: raw reset DC, exact Stop silence, FM/CSM/DAC/PSG retrigger, finite PCM and no alternate ymfm calls and zero C/C++ callback allocations/deallocations");return 0;
}
struct Event{uint32_t frame=0;std::string op;float args[5]{};};
int main(int argc,char **argv){
 if(argc==2&&std::strcmp(argv[1],"--self-test")==0)return self_test();
 if(argc!=3)return 2;const uint32_t rate=std::strtoul(argv[1],nullptr,10),frames=std::strtoul(argv[2],nullptr,10);if(rate<8000||rate>384000||frames>3840000)return 2;
 std::vector<Event> events;Event e;
 while(std::cin>>e.frame>>e.op){int n=e.op=="write"?3:e.op=="mix"?5:e.op=="reset"||e.op=="stop"||e.op=="psg-reset"?0:1;for(int i=0;i<n;i++)std::cin>>e.args[i];if(!std::cin||e.frame>frames)return 2;events.push_back(e);}
 std::stable_sort(events.begin(),events.end(),[](const Event&a,const Event&b){return a.frame<b.frame;});
 void *p=native_genesis_create_nuked(rate);if(!p)return 1;std::size_t next=0;
 for(uint32_t frame=0;frame<frames;frame++){
  while(next<events.size()&&events[next].frame==frame){const auto &v=events[next++];
   if(v.op=="write")native_genesis_write(p,v.args[0],v.args[1],v.args[2]);else if(v.op=="psg")native_genesis_psg_write(p,v.args[0]);else if(v.op=="stereo")native_genesis_psg_stereo(p,v.args[0]);else if(v.op=="psg-off")native_genesis_psg_mute(p,v.args[0]);else if(v.op=="psg-reset")native_genesis_psg_reset(p);else if(v.op=="mix")native_genesis_mix(p,v.args);else if(v.op=="stop")native_genesis_stop_voices(p);else if(v.op=="reset")native_genesis_reset(p);else return 2;
  }
  float values[8];native_genesis_sample(p,values);values[2]=native_genesis_read_status(p);values[3]=native_genesis_get_irq(p);for(int port=0;port<4;port++)values[4+port]=native_genesis_read(p,port);
  if(std::fwrite(values,sizeof(float),8,stdout)!=8)return 1;
 }
 native_genesis_destroy(p);return 0;
}
