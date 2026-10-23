// Independent additional 9-channel OPL/OPL2/Y8950; no machine or PCM memory bank.
#include "native_genesis_extra_opl.h"
#include "ymfm_opl.h"
#include "ymfm_wasm_interface.h"
#include "ymfm_fm.ipp"
#include <algorithm>
#include <array>
#include <cstring>
#include <vector>
extern template class ymfm::fm_engine_base<ymfm::opl_registers>;
extern template class ymfm::fm_engine_base<ymfm::opl2_registers>;
extern template class ymfm::fm_channel<ymfm::opl_registers>;
extern template class ymfm::fm_channel<ymfm::opl2_registers>;
extern template class ymfm::fm_operator<ymfm::opl_registers>;
extern template class ymfm::fm_operator<ymfm::opl2_registers>;
template void ymfm::fm_operator<ymfm::opl_registers>::keyonoff(uint32_t, ymfm::keyon_type);
template void ymfm::fm_operator<ymfm::opl2_registers>::keyonoff(uint32_t, ymfm::keyon_type);
template int32_t ymfm::fm_operator<ymfm::opl_registers>::compute_volume(uint32_t, uint32_t) const;
template int32_t ymfm::fm_operator<ymfm::opl2_registers>::compute_volume(uint32_t, uint32_t) const;
template uint32_t ymfm::fm_operator<ymfm::opl_registers>::envelope_attenuation(uint32_t) const;
template uint32_t ymfm::fm_operator<ymfm::opl2_registers>::envelope_attenuation(uint32_t) const;
namespace {
constexpr uint8_t SLOTS[9] = {0,1,2,8,9,10,16,17,18}, PHYSICAL[5] = {6,7,8,8,7};
float normalize(int32_t value) { return float(std::max(-32768, std::min(32767,value))) / 32768.0f; }
template<class T> struct Cached : T {
    using Output = typename T::output_data;
    std::array<int32_t,9> held{};
    std::array<int32_t,5> drums{};
    int32_t adpcm = 0, combined = 0;
    explicit Cached(ymfm_wasm_interface &intf) : T(intf) {}
    void remix() { Output output; output.clear(); for (auto value : held) output.data[0] += value; output.data[0] += adpcm; output.roundtrip_fp(); combined = output.data[0]; }
    void clear_history() { held.fill(0); drums.fill(0); adpcm = combined = 0; }
    void silence(uint8_t ch) { held[ch] = 0; remix(); }
    void silence_drum(uint8_t index) {
        drums[index] = 0;
        if (index == 0) held[6] = 0;
        else { const uint8_t ch = PHYSICAL[index]; const int32_t raw = ch == 7 ? drums[1]+drums[4] : drums[2]+drums[3]; held[ch] = std::max(-32768,std::min(32767,raw)) * 2; }
        remix();
    }
    void extras() { adpcm = 0; }
    void clear_csm() { for (uint8_t op = 0; op < 18; ++op) this->m_fm.debug_operator(op)->keyonoff(0,ymfm::KEYON_CSM); this->m_fm.invalidate_caches(); }
    void generate_cached(uint32_t muted,uint8_t cancelled) {
        this->m_fm.clock(T::fm_engine::ALL_CHANNELS); extras();
        for (uint8_t ch=0;ch<9;++ch) { Output output; this->m_fm.output(output.clear(),1,32767,1u<<ch); held[ch] = muted&(1u<<ch) ? 0 : output.data[0]; }
        if (this->m_fm.regs().rhythm_enable()) {
            const auto *hh=this->m_fm.debug_channel(7)->debug_operator(0), *sd=this->m_fm.debug_channel(7)->debug_operator(1), *tom=this->m_fm.debug_channel(8)->debug_operator(0), *cym=this->m_fm.debug_channel(8)->debug_operator(1);
            const uint32_t a=hh->phase(), b=cym->phase();
            const uint32_t select=((a>>2^a>>7)&1)|((a>>3)&1)|((b>>5^b>>3)&1), noise=this->m_fm.regs().noise_state()&1, am=this->m_fm.regs().lfo_am_offset(7);
            drums[0]=held[6]; drums[1]=sd->compute_volume((0x100<<((a>>8)&1))^(noise<<8),am)>>1;
            drums[4]=hh->compute_volume((select<<9)|(0xd0>>(2*(noise^select))),am)>>1;
            drums[2]=tom->compute_volume(tom->phase(),am)>>1; drums[3]=cym->compute_volume(0x100|(select<<9),am)>>1;
            if (muted&(1u<<7)) drums[1]=drums[4]=0; if (muted&(1u<<8)) drums[2]=drums[3]=0;
            for(uint8_t index=0;index<5;++index) if(cancelled&(1u<<index)) silence_drum(index);
        } else drums.fill(0);
        remix();
    }
};
template<> void Cached<ymfm::y8950>::extras() { this->m_adpcm_b.clock(); Output output; this->m_adpcm_b.output(output.clear(),3); adpcm=output.data[0]; }
struct Base {
    virtual ~Base()=default;
    virtual void reset()=0; virtual void stop(bool)=0; virtual void all_off()=0; virtual void write(uint16_t,uint8_t)=0;
    virtual void off(uint8_t,bool)=0; virtual void preview_off(uint8_t)=0; virtual void audition(uint8_t)=0;
    virtual void cancel_csm()=0; virtual void cancel_adpcm()=0; virtual void sample(float*,float*)=0;
    virtual bool csm() const=0;
    virtual int set_clock(uint32_t)=0; virtual uint8_t read(uint8_t)=0; virtual uint8_t status()=0; virtual uint8_t irq()const=0; virtual void state(NativeExtraOplState*)=0;
};
template<class T> struct Native final : Base {
    ymfm_wasm_interface intf;
    Cached<T> chip;
    const uint32_t rate;
    const uint8_t kind;
    uint32_t clock=3579545, forced=0x1ff, pending=0;
    uint8_t cancelled_drums=0, timer_loads=0;
    uint64_t remainder=0;
    double held=0;
    bool silent=true;
    std::array<uint8_t,256> registers{};
    std::vector<uint8_t> initial;
    Native(uint32_t sr,uint8_t type):chip(intf),rate(sr),kind(type) { chip.reset(); initial.reserve(4096); ymfm::ymfm_saved_state state(initial,true); chip.save_restore(state); }
    void raw(uint16_t reg,uint8_t value) { chip.write(0,uint8_t(reg)); chip.write(1,value); }
    bool csm() const override { return (registers[8]&128) && (timer_loads&1); }
    bool rhythm() const { return registers[0xbd]&32; }
    bool drum_active(uint8_t ch) const { if(!rhythm()) return false; for(uint8_t i=0;i<5;++i) if(PHYSICAL[i]==ch && (registers[0xbd]&(16>>i))) return true; return false; }
    void restore(uint8_t ch) { if(forced&(1u<<ch)) { forced&=~(1u<<ch); for(uint8_t slot:{SLOTS[ch],uint8_t(SLOTS[ch]+3)}) raw(0x40+slot,registers[0x40+slot]); } }
    int operator_channel(uint16_t reg) const { const uint8_t slot=reg&31; for(uint8_t ch=0;ch<9;++ch) if(slot==SLOTS[ch] || slot==SLOTS[ch]+3) return ch; return -1; }
    void write(uint16_t reg,uint8_t value) override {
        if(reg>=256) return; const uint8_t old=registers[reg]; registers[reg]=value;
        // IRQ reset ignores all timer-load controls, although the provider and
        // authoring shadow still retain the written mode byte.
        if(reg==4 && !(value&128)) timer_loads=value&3;
        if(csm()) cancelled_drums=0;
        if(reg>=0xb0&&reg<=0xb8) { const uint8_t ch=reg-0xb0; pending&=~(1u<<ch); if(value&32) { restore(ch); silent=false; } }
        if(reg==0xbd) { for(uint8_t i=0;i<5;++i) if((old^value)&(16>>i)) pending&=~(1u<<(18+i));
            if(value&31) { for(uint8_t ch=6;ch<9;++ch) restore(ch); silent=false; }
            for(uint8_t i=0;i<5;++i) if(value&(16>>i)) cancelled_drums&=~(1u<<i);
        }
        const int ch=reg>=0x40&&reg<=0x55?operator_channel(reg):-1;
        raw(reg,ch>=0&&(forced&(1u<<ch))?uint8_t((value&192)|63):value);
    }
    void update_hold() { if(chip.sample_rate(clock)<rate) held=normalize(chip.combined); }
    void off(uint8_t voice,bool normal_only) override {
        if(voice<9) {
            pending&=~(1u<<voice); registers[0xb0+voice]&=~32; raw(0xb0+voice,registers[0xb0+voice]);
            if((normal_only&&csm()) || (rhythm()&&voice>=6)) return;
            forced|=1u<<voice; for(uint8_t slot:{SLOTS[voice],uint8_t(SLOTS[voice]+3)}) raw(0x40+slot,(registers[0x40+slot]&192)|63);
            chip.silence(voice); update_hold();
        } else if(voice>=18&&voice<23) {
            const uint8_t index=voice-18; pending&=~(1u<<voice); registers[0xbd]&=~(16>>index); raw(0xbd,registers[0xbd]);
            if(!rhythm() || csm()) return;
            cancelled_drums|=1u<<index; chip.silence_drum(index); update_hold();
        }
    }
    void preview_off(uint8_t voice) override {
        // Public Worklet release keeps the old source frame until ordinary rendering.
        if(voice<9) { pending&=~(1u<<voice); registers[0xb0+voice]&=~32; raw(0xb0+voice,registers[0xb0+voice]); forced|=1u<<voice;
            for(uint8_t slot:{SLOTS[voice],uint8_t(SLOTS[voice]+3)}) raw(0x40+slot,(registers[0x40+slot]&192)|63);
        } else if(voice>=18&&voice<23) { pending&=~(1u<<voice); write(0xbd,registers[0xbd]&~(16>>(voice-18))); }
    }
    void audition(uint8_t voice) override {
        if(voice<9 || (voice>=18&&voice<23&&rhythm())) { preview_off(voice); pending|=1u<<voice; }
    }
    void cancel_csm() override {
        write(8,registers[8]&~128); write(4,(registers[4]&~0x83)|(timer_loads&2)); chip.clear_csm();
        for(uint8_t ch=0;ch<9;++ch) if(!(registers[0xb0+ch]&32) && !drum_active(ch) && !(pending&(1u<<ch))) off(ch,false);
    }
    void cancel_adpcm() override { if(kind==3) { write(7,1); chip.adpcm=0; chip.remix(); update_hold(); } }
    void all_off() override {
        for(uint8_t ch=0;ch<9;++ch) write(0xb0+ch,registers[0xb0+ch]&~32);
        write(0xbd,registers[0xbd]&224);
    }
    void stop(bool strong) override {
        pending=0;
        for(uint8_t ch=0;ch<9;++ch) preview_off(ch);
        write(0xbd,registers[0xbd]&224); silent=true; held=0;
        if(strong) { cancel_csm(); cancel_adpcm(); }
    }
    void reset() override {
        ymfm::ymfm_saved_state state(initial,false); chip.save_restore(state);
        intf.irq_asserted=false; intf.timer_remaining[0]=intf.timer_remaining[1]=-1;
        registers.fill(0); forced=0x1ff; pending=cancelled_drums=timer_loads=0; remainder=0; held=0; silent=true; chip.clear_history();
    }
    void sample(float*l,float*r) override {
        remainder+=chip.sample_rate(clock); const uint32_t count=uint32_t(remainder/rate); remainder-=uint64_t(count)*rate;
        if(count) { double sum=0; for(uint32_t i=0;i<count;++i) { chip.generate_cached(forced,cancelled_drums); intf.advance(72); sum+=normalize(chip.combined); } held=sum/count; }
        *l=*r=silent?0:float(held);
        if(count&&pending) {
            const uint32_t starts=pending; pending=0;
            for(uint8_t ch=0;ch<9;++ch) if(starts&(1u<<ch)) write(0xb0+ch,registers[0xb0+ch]|32);
            uint8_t drums=0;for(uint8_t i=0;i<5;++i)if(starts&(1u<<(18+i)))drums|=16>>i;
            if(drums&&rhythm())write(0xbd,registers[0xbd]|drums);
        }
    }
    int set_clock(uint32_t hz) override { if(hz<100000||hz>100000000) return 0; clock=hz; return 1; }
    uint8_t read(uint8_t offset) override { return chip.read(offset); }
    uint8_t status() override { return chip.read_status(); }
    uint8_t irq() const override { return intf.irq_asserted?1:0; }
    void state(NativeExtraOplState*s) override { std::memcpy(s->registers,registers.data(),256); s->clock=clock; s->kind=kind; s->channels=9; s->status=status(); s->irq=irq(); }
};
Base*core(void*p){return static_cast<Base*>(p);}
}
extern "C" {
void*native_extra_opl_create(uint32_t rate,uint8_t kind) { if(rate<8000||rate>384000) return nullptr; try { if(kind==0)return new Native<ymfm::ym3526>(rate,kind); if(kind==1)return new Native<ymfm::ym3812>(rate,kind); if(kind==3)return new Native<ymfm::y8950>(rate,kind); } catch(...){} return nullptr; }
void native_extra_opl_destroy(void*p){delete core(p);} void native_extra_opl_reset(void*p){core(p)->reset();}
void native_extra_opl_all_notes_off(void*p){core(p)->all_off();}
void native_extra_opl_host_all_notes_off(void*p){core(p)->stop(false);} void native_extra_opl_stop(void*p){core(p)->stop(true);}
void native_extra_opl_write(void*p,uint16_t reg,uint8_t value){core(p)->write(reg,value);} void native_extra_opl_key_off(void*p,uint8_t voice){core(p)->off(voice,false);}
void native_extra_opl_key_off_fm(void*p,uint8_t ch){core(p)->off(ch,true);} void native_extra_opl_preview_key_off(void*p,uint8_t voice){core(p)->preview_off(voice);}
void native_extra_opl_audition_key_on(void*p,uint8_t voice){core(p)->audition(voice);} void native_extra_opl_cancel_csm(void*p){core(p)->cancel_csm();}
void native_extra_opl_cancel_adpcm(void*p){core(p)->cancel_adpcm();} void native_extra_opl_sample(void*p,float*l,float*r){core(p)->sample(l,r);}
uint8_t native_extra_opl_csm_active(void*p){return core(p)->csm()?1:0;}
int native_extra_opl_set_clock(void*p,uint32_t hz){return core(p)->set_clock(hz);} uint8_t native_extra_opl_read(void*p,uint8_t offset){return core(p)->read(offset);}
uint8_t native_extra_opl_read_status(void*p){return core(p)->status();} uint8_t native_extra_opl_get_irq(void*p){return core(p)->irq();}
void native_extra_opl_get_state(void*p,NativeExtraOplState*s){core(p)->state(s);}
}
