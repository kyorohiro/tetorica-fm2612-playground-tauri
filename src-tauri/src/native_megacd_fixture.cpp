// Offline RF5C164 parity fixture. No device is opened. All allocations and
// text parsing finish before the sample loop, matching native core ownership.
#include "native_genesis_rf5c164.h"
#include <algorithm>
#include <array>
#include <cstdio>
#include <cstdlib>
#include <iostream>
#include <sstream>
#include <stdexcept>
#include <string>
#include <vector>
struct Event {
    uint64_t frame;
    std::string op;
    uint32_t a=0,b=0;
    std::array<float,2> gain{{1,1}};
    std::vector<uint8_t> bytes;
};
int main(int argc, char **argv) {
    try {
        if (argc != 3) throw std::runtime_error("usage: native-megacd-fixture rate frames");
        uint32_t rate=std::stoul(argv[1]); uint64_t frames=std::stoull(argv[2]);
        if (rate<8000 || rate>384000 || frames>10000000) throw std::runtime_error("invalid size");
        std::vector<Event> events;
        std::string line;
        while (std::getline(std::cin,line)) {
            if (line.empty() || line[0]=='#') continue;
            std::istringstream in(line); Event event;
            if (!(in >> event.frame >> event.op)) throw std::runtime_error("invalid event");
            if (event.op=="write" || event.op=="memory") { if (!(in>>event.a>>event.b)) throw std::runtime_error("invalid write"); }
            else if (event.op=="clock") { if (!(in>>event.a)) throw std::runtime_error("invalid clock"); }
            else if (event.op=="load") {
                std::string hex; if (!(in>>event.a>>hex) || hex.size()%2) throw std::runtime_error("invalid load");
                for (size_t i=0;i<hex.size();i+=2) event.bytes.push_back(std::stoul(hex.substr(i,2),nullptr,16));
                if (event.a>65536 || event.bytes.size()>65536-event.a) throw std::runtime_error("load range");
            } else if (event.op=="mix") { if (!(in>>event.gain[0]>>event.gain[1])) throw std::runtime_error("invalid gain"); }
            else if (event.op!="clear" && event.op!="reset" && event.op!="stop") throw std::runtime_error("unknown event");
            events.push_back(std::move(event));
        }
        std::stable_sort(events.begin(),events.end(),[](const Event&a,const Event&b){return a.frame<b.frame;});
        void *chip=native_rf5c164_create(rate);
        if(!chip)throw std::runtime_error("allocation failed");
        std::array<float,2> gain{{1,1}},target{{1,1}};
        uint32_t ramp=0; size_t next=0;
        for (uint64_t frame=0;frame<frames;++frame) {
            while (next<events.size() && events[next].frame<=frame) {
                const auto&e=events[next++];
                if(e.op=="write")native_rf5c164_write(chip,e.a,e.b);
                else if(e.op=="memory")native_rf5c164_write_memory(chip,e.a,e.b);
                else if(e.op=="load")native_rf5c164_load(chip,e.bytes.data(),e.a,e.bytes.size());
                else if(e.op=="clock")native_rf5c164_set_clock(chip,e.a);
                else if(e.op=="reset")native_rf5c164_reset(chip);
                else if(e.op=="clear")native_rf5c164_clear_memory(chip);
                else if(e.op=="stop")native_rf5c164_write(chip,8,255);
                else if(e.op=="mix"){target=e.gain;ramp=std::max(1u,rate/200);}
            }
            if(ramp){for(size_t c=0;c<2;++c)gain[c]+=(target[c]-gain[c])/ramp;--ramp;}
            float left=0,right=0;native_rf5c164_sample(chip,&left,&right);
            float pcm[2]={left*gain[0],right*gain[1]};
            if(std::fwrite(pcm,sizeof(float),2,stdout)!=2)throw std::runtime_error("write failed");
        }
        native_rf5c164_destroy(chip);
        return 0;
    } catch (const std::exception&e) { std::cerr<<e.what()<<"\n";return 1; }
}
