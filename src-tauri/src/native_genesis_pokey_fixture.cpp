// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 kyorohiro
// Offline Browser/Native speaker parity fixture. No OS audio device is opened.
#include "native_genesis_pokey.h"
#include <cstdio>
#include <cstdlib>
int main(int argc,char**argv){
 if(argc!=5)return 2;const auto rate=uint32_t(std::strtoul(argv[1],nullptr,10)),clock=uint32_t(std::strtoul(argv[2],nullptr,10)),mode=unsigned(std::strtoul(argv[3],nullptr,10));
 auto*out=std::fopen(argv[4],"wb");if(!out)return 3;void*p=native_pokey_create(rate,clock);if(!p)return 4;
 auto write=[&](unsigned r,unsigned v){native_pokey_write(p,r,uint8_t(v));};
 write(15,3);write(8,mode==3?0x50:mode==4?0x04:0);for(unsigned ch=0;ch<4;ch++){write(ch*2,72+ch*29);write(ch*2+1,mode==1?(0x80|(ch+1)*3):mode==2?0xaf:0xa8);}write(9,0);
 for(unsigned i=0;i<8192;i++){
  if(i==1024)native_pokey_set_mute_mask(p,5);if(i==2048)native_pokey_set_mute_mask(p,0);
  if(i==3072){for(unsigned ch=0;ch<4;ch++)native_pokey_key_off(p,ch);}
  if(i==4096){native_pokey_reset(p);write(15,3);write(0,90);write(1,mode==5?0x1f:0xaa);write(9,0);}
  if(i==5120&&mode==5)write(1,0x10);if(i==6144&&mode==5)write(1,0x1f);
  if(i==7168)native_pokey_stop(p);float left=0,right=0;native_pokey_sample(p,&left,&right);std::fwrite(&left,sizeof(left),1,out);std::fwrite(&right,sizeof(right),1,out);
 }
 native_pokey_destroy(p);std::fclose(out);return 0;
}
