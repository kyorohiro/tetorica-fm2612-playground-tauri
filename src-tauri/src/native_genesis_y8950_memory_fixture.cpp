// Real primary/independent C++ Y8950 versus WASM. Rust COW is tested separately.
#include "native_genesis_opl.h"
#include "native_genesis_extra_opl.h"
#include <algorithm>
#include <cassert>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <iostream>
#include <new>
#include <sstream>
#include <string>
#include <vector>
static bool audit=false;static size_t allocations=0,frees=0;
void*operator new(std::size_t n){if(audit)++allocations;if(void*p=std::malloc(n?n:1))return p;throw std::bad_alloc();}
void*operator new[](std::size_t n){return ::operator new(n);}void operator delete(void*p)noexcept{if(audit&&p)++frees;std::free(p);}void operator delete[](void*p)noexcept{::operator delete(p);}void operator delete(void*p,std::size_t)noexcept{::operator delete(p);}void operator delete[](void*p,std::size_t)noexcept{::operator delete(p);}
struct Event{uint32_t frame=0,a=0,b=0;std::string op;std::vector<uint8_t>data;};
struct Chip{
 bool extra;void*p;uint32_t size=0,clock=3579545;std::vector<uint8_t>ram;
 explicit Chip(bool additional,uint32_t rate):extra(additional),p(extra?native_extra_opl_create(rate,3):native_opl_create(rate,3)),ram(2097152,0){assert(p);for(uint32_t i=0;i<512;++i)page(i,ram.data()+i*4096);}
 ~Chip(){if(extra)native_extra_opl_destroy(p);else native_opl_destroy(p);}
 void page(uint32_t i,uint8_t*data){if(extra)native_extra_opl_memory_page(p,i,data);else native_opl_memory_page(p,i,data);}
 void length(){if(extra)native_extra_opl_memory_length(p,size);else native_opl_memory_length(p,size);}
 void write(uint16_t r,uint8_t v){if(extra)native_extra_opl_write(p,r,v);else native_opl_write(p,r,v);}
 void cancel(){if(extra)native_extra_opl_cancel_adpcm(p);else native_opl_cancel_adpcm(p);}
 void apply(const Event&e){
  if(e.op=="write")write(e.a,e.b);
  else if(e.op=="load"){assert(e.a<=ram.size()&&e.data.size()<=ram.size()-e.a);write(7,1);std::memcpy(ram.data()+e.a,e.data.data(),e.data.size());size=std::max(size,uint32_t(e.a+e.data.size()));length();}
  else if(e.op=="clear"){write(7,1);std::fill(ram.begin(),ram.end(),0);size=0;length();}
  else if(e.op=="clock"){assert(extra?native_extra_opl_set_clock(p,e.a):native_opl_set_clock(p,e.a));clock=e.a;}
  else if(e.op=="reset"){if(extra)native_extra_opl_reset(p);else native_opl_reset(p);}
  else if(e.op=="stop"){if(extra)native_extra_opl_stop(p);else native_opl_stop(p);}
  else if(e.op=="audition-adpcm"){if(extra)native_extra_opl_source_audition_key_on(p,23);else native_opl_audition_key_on(p,23);}
  else if(e.op=="off-adpcm"){if(extra)native_extra_opl_key_off_fm(p,23);else native_opl_key_off(p,23);}
  else std::abort();
 }
 void sample(float*o){if(extra)native_extra_opl_sample(p,o,o+1);else native_opl_sample(p,o,o+1);o[2]=extra?native_extra_opl_read_status(p):native_opl_read_status(p);o[3]=extra?native_extra_opl_get_irq(p):native_opl_get_irq(p);for(int i=0;i<4;++i)o[4+i]=extra?native_extra_opl_read(p,i):native_opl_read(p,i);o[8]=float(size);o[9]=float(clock);}
};
int main(int argc,char**argv){if(argc!=4)return 2;const bool extra=std::string(argv[1])=="extra";const auto rate=std::stoul(argv[2]),frames=std::stoul(argv[3]);if(frames>1000000)return 2;std::vector<Event>events;std::string line;
 while(std::getline(std::cin,line)){Event e;std::istringstream in(line);if(!(in>>e.frame>>e.op))continue;if(e.op=="write")in>>e.a>>e.b;else if(e.op=="clock")in>>e.a;else if(e.op=="load"){std::string hex;in>>e.a>>hex;assert(hex.size()%2==0);for(size_t i=0;i<hex.size();i+=2)e.data.push_back(uint8_t(std::stoul(hex.substr(i,2),nullptr,16)));}events.push_back(std::move(e));}
 std::stable_sort(events.begin(),events.end(),[](const Event&a,const Event&b){return a.frame<b.frame;});Chip chip(extra,rate);std::vector<float>out(frames*10);size_t next=0;audit=true;for(uint32_t frame=0;frame<frames;++frame){while(next<events.size()&&events[next].frame==frame)chip.apply(events[next++]);chip.sample(out.data()+frame*10);}audit=false;assert(allocations==0&&frees==0);return std::fwrite(out.data(),sizeof(float),out.size(),stdout)==out.size()?0:4;}
