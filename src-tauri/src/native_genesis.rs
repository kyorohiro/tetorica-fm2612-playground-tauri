//! Experimental Genesis output: the device callback owns both chips and PCM.
//! The control side only submits bounded timestamped register events.
use crate::native_genesis_fx::{FxAction, FxController, FxMessage, FxRenderer};
use crossbeam_queue::ArrayQueue;
use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    cmp::Ordering as Compare,
    collections::{BinaryHeap, HashSet},
    ffi::c_void,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};

#[cfg(test)]
#[path = "native_genesis_consoles_tests.rs"]
mod consoles_tests;
#[path = "native_genesis_dx7.rs"]
mod dx7;
#[cfg(test)]
#[path = "native_genesis_dx7_tests.rs"]
mod dx7_tests;
#[path = "native_genesis_extra_budget.rs"]
mod extra_budget;
#[path = "native_genesis_extra_dx7.rs"]
mod extra_dx7;
#[cfg(test)]
#[path = "native_genesis_extra_dx7_tests.rs"]
mod extra_dx7_tests;
#[path = "native_genesis_extra_gameboy.rs"]
mod extra_gameboy;
#[cfg(test)]
#[path = "native_genesis_extra_gameboy_tests.rs"]
mod extra_gameboy_tests;
#[path = "native_genesis_extra_huc6280.rs"]
mod extra_huc6280;
#[cfg(test)]
#[path = "native_genesis_extra_huc6280_tests.rs"]
mod extra_huc6280_tests;
#[path = "native_genesis_extra_msx.rs"]
mod extra_msx;
#[cfg(test)]
#[path = "native_genesis_extra_msx_tests.rs"]
mod extra_msx_tests;
#[path = "native_genesis_extra_nes.rs"]
mod extra_nes;
#[cfg(test)]
#[path = "native_genesis_extra_nes_tests.rs"]
mod extra_nes_tests;
#[path = "native_genesis_extra_okim6258.rs"]
mod extra_okim6258;
#[cfg(test)]
#[path = "native_genesis_extra_okim6258_tests.rs"]
mod extra_okim6258_tests;
#[path = "native_genesis_extra_opl.rs"]
mod extra_opl;
#[cfg(test)]
#[path = "native_genesis_extra_opl_tests.rs"]
mod extra_opl_tests;
#[path = "native_genesis_extra_psg.rs"]
mod extra_psg;
#[cfg(test)]
#[path = "native_genesis_extra_psg_tests.rs"]
mod extra_psg_tests;
#[path = "native_genesis_extra_pwm.rs"]
mod extra_pwm;
#[cfg(test)]
#[path = "native_genesis_extra_pwm_tests.rs"]
mod extra_pwm_tests;
#[path = "native_genesis_extra_rf5c164.rs"]
mod extra_rf5c164;
#[cfg(test)]
#[path = "native_genesis_extra_rf5c164_tests.rs"]
mod extra_rf5c164_tests;
#[path = "native_genesis_extra_ym2151.rs"]
mod extra_ym2151;
#[cfg(test)]
#[path = "native_genesis_extra_ym2151_tests.rs"]
mod extra_ym2151_tests;
#[cfg(test)]
#[path = "native_genesis_extra_ym2203_tests.rs"]
mod extra_ym2203_tests;
#[path = "native_genesis_extra_ym2608.rs"]
mod extra_ym2608;
#[cfg(test)]
#[path = "native_genesis_extra_ym2608_tests.rs"]
mod extra_ym2608_tests;
#[path = "native_genesis_extra_ym2610.rs"]
mod extra_ym2610;
#[cfg(test)]
#[path = "native_genesis_extra_ym2610_tests.rs"]
mod extra_ym2610_tests;
#[cfg(test)]
#[path = "native_genesis_extra_ym2612_tests.rs"]
mod extra_ym2612_tests;
#[cfg(test)]
#[path = "native_genesis_extra_ymf262_tests.rs"]
mod extra_ymf262_tests;
#[path = "native_genesis_gameboy.rs"]
mod gameboy;
#[path = "native_genesis_huc6280.rs"]
mod huc6280;
#[path = "native_genesis_livefx.rs"]
mod livefx;
#[path = "native_genesis_master_system.rs"]
mod master_system;
#[path = "native_genesis_media.rs"]
mod media;
#[path = "native_genesis_monitor.rs"]
mod monitor;
#[path = "native_genesis_msx.rs"]
mod msx;
#[path = "native_genesis_nes.rs"]
mod nes;
#[path = "native_genesis_opl.rs"]
mod opl;
#[path = "native_genesis_snes.rs"]
mod snes;
#[path = "native_genesis_x68000.rs"]
mod x68000;
#[path = "native_genesis_ym2203.rs"]
mod ym2203;
#[path = "native_genesis_ym2608.rs"]
mod ym2608;
#[path = "native_genesis_ym2610b.rs"]
mod ym2610b;
#[path = "native_genesis_ym2612.rs"]
mod ym2612;

#[cfg(test)]
#[path = "native_genesis_nes_tests.rs"]
mod nes_tests;
#[path = "native_genesis_noise.rs"]
mod noise;
#[cfg(test)]
#[path = "native_genesis_noise_tests.rs"]
mod noise_tests;
#[cfg(test)]
#[path = "native_genesis_nuked_tests.rs"]
mod nuked_tests;
#[path = "native_genesis_pwm.rs"]
mod pwm;
#[cfg(test)]
#[path = "native_genesis_pwm_tests.rs"]
mod pwm_tests;
#[path = "native_genesis_recording.rs"]
mod recording;
#[path = "native_genesis_rf5c164.rs"]
mod rf5c164;
#[cfg(test)]
#[path = "native_genesis_rf5c164_tests.rs"]
mod rf5c164_tests;
#[cfg(test)]
#[path = "native_genesis_stability_tests.rs"]
mod stability_tests;
const OWNERS: usize = 512;
const CAPACITY: usize = 65_536;
const MAX_BATCH: usize = 4096;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);

extern "C" {
    fn native_genesis_create(rate: u32) -> *mut c_void;
    fn native_genesis_create_nuked(rate: u32) -> *mut c_void;
    fn native_genesis_destroy(core: *mut c_void);
    fn native_genesis_reset(core: *mut c_void);
    fn native_genesis_stop_voices(core: *mut c_void);
    fn native_genesis_write(core: *mut c_void, port: u8, register: u8, value: u8);
    fn native_genesis_psg_write(core: *mut c_void, value: u8);
    fn native_genesis_psg_stereo(core: *mut c_void, value: u8);
    fn native_genesis_psg_mute(core: *mut c_void, channel: u8);
    fn native_genesis_psg_reset(core: *mut c_void);
    fn native_genesis_mix(core: *mut c_void, gains: *const f32);
    fn native_genesis_sample(core: *mut c_void, output: *mut f32);
    fn native_genesis_master(core: *mut c_void) -> f32;
}

struct Core(*mut c_void);
// The pointer is uniquely owned. It is moved into, and used only by, one device
// callback. C++ creates no threads and stores no callback into Rust.
unsafe impl Send for Core {}
impl Core {
    fn new(rate: u32) -> Result<Self, String> {
        Self::new_engine(rate, false)
    }
    fn new_nuked(rate: u32) -> Result<Self, String> {
        Self::new_engine(rate, true)
    }
    fn new_engine(rate: u32, nuked: bool) -> Result<Self, String> {
        if !(8000..=384000).contains(&rate) {
            return Err("Unsupported native sample rate".into());
        }
        let ptr = unsafe {
            if nuked {
                native_genesis_create_nuked(rate)
            } else {
                native_genesis_create(rate)
            }
        };
        if ptr.is_null() {
            Err("Could not allocate native Genesis sound cores".into())
        } else {
            Ok(Self(ptr))
        }
    }
    #[cfg(test)]
    fn sample(&mut self) -> [f32; 2] {
        let mut sample = self.sample_bus();
        let master = unsafe { native_genesis_master(self.0) };
        sample[0] *= master;
        sample[1] *= master;
        sample
    }
    fn empty() -> Self {
        Self(std::ptr::null_mut())
    }
    fn master(&self) -> f32 {
        if self.0.is_null() {
            1.0
        } else {
            unsafe { native_genesis_master(self.0) }
        }
    }
    fn sample_bus(&mut self) -> [f32; 2] {
        if self.0.is_null() {
            return [0.0; 2];
        }
        let mut sample = [0.0; 2];
        unsafe {
            native_genesis_sample(self.0, sample.as_mut_ptr());
        }
        sample
    }
    fn apply(&mut self, action: Action) {
        if self.0.is_null() {
            return;
        }
        unsafe {
            match action {
                Action::Write(port, register, value) => {
                    native_genesis_write(self.0, port, register, value)
                }
                Action::PsgWrite(value) => native_genesis_psg_write(self.0, value),
                Action::PsgStereo(value) => native_genesis_psg_stereo(self.0, value),
                Action::PsgReset => native_genesis_psg_reset(self.0),
                Action::Reset => native_genesis_reset(self.0),
                Action::Mix(gains) => native_genesis_mix(self.0, gains.as_ptr()),
                _ => unreachable!("Media actions are owned by Renderer"),
            }
        }
    }
}
impl Drop for Core {
    fn drop(&mut self) {
        unsafe {
            if !self.0.is_null() {
                native_genesis_destroy(self.0);
            }
        }
    }
}

#[derive(Clone, Debug)]
enum Action {
    ExtraPsgInstall(Arc<extra_psg::Instance>),
    ExtraPsg(Arc<extra_psg::Instance>, Arc<master_system::Prepared>),
    ExtraYm2612(Arc<extra_psg::Instance>, Arc<ym2612::Prepared>),
    ExtraYm2203(Arc<extra_psg::Instance>, Arc<ym2203::Prepared>),
    ExtraYm2608(Arc<extra_psg::Instance>, Arc<extra_ym2608::Prepared>),
    ExtraYm2610(Arc<extra_psg::Instance>, Arc<extra_ym2610::Prepared>),
    ExtraYm2151(Arc<extra_psg::Instance>, Arc<extra_ym2151::Prepared>),
    ExtraAy8910(Arc<extra_psg::Instance>, Arc<extra_msx::Prepared>),
    ExtraY8950(Arc<extra_psg::Instance>, Arc<extra_opl::Prepared>),
    ExtraYmf262(Arc<extra_psg::Instance>, Arc<extra_opl::Prepared>),
    ExtraRf5c164(Arc<extra_psg::Instance>, Arc<extra_rf5c164::Prepared>),
    ExtraOkim6258(Arc<extra_psg::Instance>, Arc<extra_okim6258::Prepared>),
    ExtraGameboy(Arc<extra_psg::Instance>, Arc<extra_gameboy::Prepared>),
    ExtraHuc6280(Arc<extra_psg::Instance>, Arc<extra_huc6280::Prepared>),
    ExtraDx7(Arc<extra_psg::Instance>, Arc<extra_dx7::Prepared>),
    ExtraPwm(Arc<extra_psg::Instance>, Arc<extra_pwm::Prepared>),
    ExtraNes(Arc<extra_psg::Instance>, Arc<extra_nes::Prepared>),
    ExtraYm3812(Arc<extra_psg::Instance>, Arc<extra_opl::Prepared>),
    ExtraYm3526(Arc<extra_psg::Instance>, Arc<extra_opl::Prepared>),
    ExtraYm2413(Arc<extra_psg::Instance>, Arc<extra_msx::Prepared>),
    Dx7(Arc<dx7::Prepared>),
    Nes(Arc<nes::Prepared>),
    Gameboy(Arc<gameboy::Prepared>),
    Snes(Arc<snes::Prepared>),
    Huc6280(Arc<huc6280::Prepared>),
    Ym2608(Arc<ym2608::Prepared>),
    Ym2203(Arc<ym2203::Prepared>),
    Ym2610b(Arc<ym2610b::Prepared>),
    X68000(Arc<x68000::Prepared>),
    MasterSystem(Arc<master_system::Prepared>),
    Msx(Arc<msx::Prepared>),
    Opl(Arc<opl::Prepared>),
    Pwm(Arc<pwm::Prepared>),

    RfWrite(u8, u8, bool),
    RfWriteMemory(u16, u8),
    RfLoad(Arc<rf5c164::Memory>),
    RfReset,
    RfClearMemory,
    RfClock(u32),
    RfMix([f32; 2]),
    RfKey(u8, bool, Option<u64>),
    Write(u8, u8, u8),
    PsgWrite(u8),
    PsgStereo(u8),
    PsgReset,
    Reset,
    Mix([f32; 5]),
    DacPlay {
        bank: Arc<media::Bank>,
        offset: usize,
        size: usize,
        pan: u8,
    },
    DacStream(Arc<media::Bank>),
    DacStop,
    SamplePlay(Arc<media::Reservation>),
    SampleStop {
        voice: Option<u64>,
        name: Option<String>,
    },
    SampleRelease,
    SampleTimedStop(Arc<media::StopTargets>),
    Record(Arc<recording::Capture>),
    Fx(FxAction),
    FxReset(FxAction),
    Noise(Option<noise::Prepared>),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Owner {
    performance: u64,
    owner: u32,
}

#[derive(Clone, Debug)]
struct Event {
    frame: u64,
    sequence: u64,
    epoch: u64,
    action: RefCell<Action>,
    owner: Option<Owner>,
}
impl PartialEq for Event {
    fn eq(&self, other: &Self) -> bool {
        (self.frame, self.sequence) == (other.frame, other.sequence)
    }
}
impl Eq for Event {}
impl PartialOrd for Event {
    fn partial_cmp(&self, other: &Self) -> Option<Compare> {
        Some(self.cmp(other))
    }
}
impl Ord for Event {
    fn cmp(&self, other: &Self) -> Compare {
        (other.frame, other.sequence).cmp(&(self.frame, self.sequence))
    }
}

#[derive(Deserialize, Debug)]
pub struct Command {
    frame: u64,
    #[serde(default)]
    performance: Option<u64>,
    #[serde(default)]
    owner: Option<u32>,
    #[serde(flatten)]
    action: CommandAction,
}
#[derive(Deserialize, Debug)]
#[serde(tag = "type", rename_all = "kebab-case")]
enum CommandAction {
    ExtraPsg {
        instance: u64,
        message: master_system::Message,
    },
    ExtraYm2612 {
        instance: u64,
        message: ym2612::Message,
    },
    ExtraYm2203 {
        instance: u64,
        message: ym2203::Message,
    },
    ExtraYm2608 {
        instance: u64,
        message: extra_ym2608::Message,
    },
    ExtraYm2610 {
        instance: u64,
        message: extra_ym2610::Message,
    },
    ExtraYm2151 {
        instance: u64,
        message: extra_ym2151::Message,
    },
    ExtraAy8910 {
        instance: u64,
        message: extra_msx::Message,
    },
    ExtraY8950 {
        instance: u64,
        message: extra_opl::Message,
    },
    ExtraYmf262 {
        instance: u64,
        message: extra_opl::Message,
    },
    ExtraRf5c164 {
        instance: u64,
        message: extra_rf5c164::Message,
    },
    ExtraOkim6258 {
        instance: u64,
        message: extra_okim6258::Message,
    },
    ExtraHuc6280 {
        instance: u64,
        message: extra_huc6280::Message,
    },
    ExtraDx7 {
        instance: u64,
        message: extra_dx7::Message,
    },
    ExtraPwm {
        instance: u64,
        message: extra_pwm::Message,
    },
    ExtraGameboy {
        instance: u64,
        message: extra_gameboy::Message,
    },
    ExtraNes {
        instance: u64,
        message: extra_nes::Message,
    },
    ExtraYm3812 {
        instance: u64,
        message: extra_opl::Message,
    },
    ExtraYm3526 {
        instance: u64,
        message: extra_opl::Message,
    },
    ExtraYm2413 {
        instance: u64,
        message: extra_msx::Message,
    },
    Dx7 {
        message: dx7::Message,
    },
    Nes {
        message: nes::Message,
    },
    Gameboy {
        message: gameboy::Message,
    },
    Snes {
        message: snes::Message,
    },
    Huc6280 {
        message: huc6280::Message,
    },
    Ym2608 {
        message: ym2608::Message,
    },
    Ym2203 {
        message: ym2203::Message,
    },
    Ym2610b {
        message: ym2610b::Message,
    },
    X68000 {
        message: x68000::Message,
    },
    MasterSystem {
        message: master_system::Message,
    },
    Msx {
        message: msx::Message,
    },
    Opl {
        message: opl::Message,
    },
    Pwm {
        message: pwm::Message,
    },

    Rf5c164Write {
        register: u8,
        value: u8,
        #[serde(default)]
        audition: bool,
    },
    Rf5c164WriteMemory {
        offset: u16,
        value: u8,
    },
    Rf5c164LoadMemory {
        offset: u32,
        data: Vec<u8>,
    },
    Rf5c164Reset,
    Rf5c164ClearMemory,
    Rf5c164Clock {
        hz: u32,
    },
    Rf5c164Mix {
        left: f32,
        right: f32,
    },
    Rf5c164KeyOn {
        channel: u8,
        voice: Option<u64>,
    },
    Rf5c164KeyOff {
        channel: u8,
        voice: Option<u64>,
    },
    Write {
        #[serde(default)]
        port: u8,
        register: u8,
        value: u8,
    },
    PsgWrite {
        value: u8,
    },
    PsgStereo {
        value: u8,
    },
    PsgReset,
    Reset,
    Noise {
        message: noise::Message,
    },
    DacPlay {
        name: String,
        #[serde(default)]
        offset: usize,
        size: Option<usize>,
        #[serde(default = "both")]
        pan: String,
    },
    DacStreamPlay {
        name: String,
    },
    DacStop,
    SamplePlay {
        name: String,
        voice: u64,
        #[serde(flatten)]
        options: media::Options,
    },
    SampleStop {
        voice: Option<u64>,
        name: Option<String>,
    },
    Mix {
        #[serde(rename = "fmLeft")]
        fm_left: f32,
        #[serde(rename = "fmRight")]
        fm_right: f32,
        #[serde(rename = "psgLeft")]
        psg_left: f32,
        #[serde(rename = "psgRight")]
        psg_right: f32,
        master: f32,
    },
}
fn both() -> String {
    "both".into()
}
impl Command {
    fn validate(&self) -> Result<Action, String> {
        if self.frame > MAX_SAFE_INTEGER {
            return Err("Invalid native command frame".into());
        }
        Ok(match self.action {
            CommandAction::ExtraPsg { .. }
            | CommandAction::ExtraYm2612 { .. }
            | CommandAction::ExtraYm2203 { .. }
            | CommandAction::ExtraYm2608 { .. }
            | CommandAction::ExtraYm2610 { .. }
            | CommandAction::ExtraYm2151 { .. }
            | CommandAction::ExtraAy8910 { .. }
            | CommandAction::ExtraY8950 { .. }
            | CommandAction::ExtraYmf262 { .. }
            | CommandAction::ExtraRf5c164 { .. }
            | CommandAction::ExtraOkim6258 { .. }
            | CommandAction::ExtraGameboy { .. }
            | CommandAction::ExtraHuc6280 { .. }
            | CommandAction::ExtraDx7 { .. }
            | CommandAction::ExtraPwm { .. }
            | CommandAction::ExtraNes { .. }
            | CommandAction::ExtraYm3812 { .. }
            | CommandAction::ExtraYm3526 { .. }
            | CommandAction::ExtraYm2413 { .. } => {
                return Err("Additional sound-chip commands require instance resolution".into())
            }
            CommandAction::Dx7 { ref message } => Action::Dx7(message.prepare()?),
            CommandAction::Nes { ref message } => Action::Nes(message.prepare()?),
            CommandAction::Gameboy { ref message } => Action::Gameboy(message.prepare()?),
            CommandAction::Snes { ref message } => Action::Snes(message.prepare()?),
            CommandAction::Huc6280 { ref message } => Action::Huc6280(message.prepare()?),
            CommandAction::Ym2608 { ref message } => Action::Ym2608(message.prepare()?),
            CommandAction::Ym2203 { ref message } => Action::Ym2203(message.prepare()?),
            CommandAction::Ym2610b { ref message } => Action::Ym2610b(message.prepare()?),
            CommandAction::X68000 { ref message } => Action::X68000(message.prepare()?),
            CommandAction::MasterSystem { ref message } => Action::MasterSystem(message.prepare()?),
            CommandAction::Msx { ref message } => Action::Msx(message.prepare()?),
            CommandAction::Opl { ref message } => Action::Opl(message.prepare()?),
            CommandAction::Pwm { ref message } => Action::Pwm(message.prepare()?),

            CommandAction::Rf5c164Write {
                register,
                value,
                audition,
            } => {
                if register > 8 {
                    return Err("RF5C164 register must be 0–8".into());
                }
                Action::RfWrite(register, value, audition)
            }
            CommandAction::Rf5c164WriteMemory { offset, value } => {
                Action::RfWriteMemory(offset, value)
            }
            CommandAction::Rf5c164LoadMemory { offset, ref data } => {
                Action::RfLoad(rf5c164::Memory::new(offset, data)?)
            }
            CommandAction::Rf5c164Reset => Action::RfReset,
            CommandAction::Rf5c164ClearMemory => Action::RfClearMemory,
            CommandAction::Rf5c164Clock { hz } => {
                if !(100_000..=100_000_000).contains(&hz) {
                    return Err("RF5C164 clock must be 100000–100000000 Hz".into());
                }
                Action::RfClock(hz)
            }
            CommandAction::Rf5c164Mix { left, right } => {
                if [left, right]
                    .iter()
                    .any(|v| !v.is_finite() || !(0.0..=2.0).contains(v))
                {
                    return Err("RF5C164 source gains must be 0–2".into());
                }
                Action::RfMix([left, right])
            }
            CommandAction::Rf5c164KeyOn { channel, voice }
            | CommandAction::Rf5c164KeyOff { channel, voice } => {
                if channel > 7 {
                    return Err("RF5C164 channel must be 0–7".into());
                }
                if voice.is_some_and(|voice| voice == 0 || voice > MAX_SAFE_INTEGER) {
                    return Err("Invalid RF5C164 voice lease".into());
                }
                Action::RfKey(
                    channel,
                    matches!(self.action, CommandAction::Rf5c164KeyOn { .. }),
                    voice,
                )
            }
            CommandAction::Write {
                port,
                register,
                value,
            } => {
                if port > 1 {
                    return Err("YM2612 port must be 0 or 1".into());
                }
                Action::Write(port, register, value)
            }
            CommandAction::PsgWrite { value } => Action::PsgWrite(value),
            CommandAction::PsgStereo { value } => Action::PsgStereo(value),
            CommandAction::PsgReset => Action::PsgReset,
            CommandAction::Reset => Action::Reset,
            CommandAction::DacStop => Action::DacStop,
            CommandAction::SampleStop { voice, ref name } => {
                if voice.is_some_and(|v| v == 0 || v > MAX_SAFE_INTEGER) {
                    return Err("Invalid native sample voice ID".into());
                }
                Action::SampleStop {
                    voice,
                    name: name.clone(),
                }
            }
            CommandAction::Noise { .. } => {
                return Err("Native noise command requires resource preparation".into())
            }
            CommandAction::DacPlay { .. }
            | CommandAction::DacStreamPlay { .. }
            | CommandAction::SamplePlay { .. } => {
                return Err("Native media command requires a loaded bank".into())
            }
            CommandAction::Mix {
                fm_left,
                fm_right,
                psg_left,
                psg_right,
                master,
            } => {
                let gains = [fm_left, fm_right, psg_left, psg_right, master];
                if gains[..4]
                    .iter()
                    .any(|v| !v.is_finite() || !(0.0..=2.0).contains(v))
                    || !master.is_finite()
                    || !(0.0..=3.8).contains(&master)
                {
                    return Err("Native source gains must be 0–2 and master gain 0–3.8".into());
                }
                Action::Mix(gains)
            }
        })
    }
}

impl Command {
    fn resolve(
        &self,
        store: &media::Store,
        rate: u32,
        owner: Option<Owner>,
    ) -> Result<Action, String> {
        if self.frame > MAX_SAFE_INTEGER {
            return Err("Invalid native command frame".into());
        }
        match &self.action {
            CommandAction::DacPlay {
                name,
                offset,
                size,
                pan,
            } => {
                let bank = store.get(media::Kind::Dac, name)?;
                let size = size.unwrap_or_else(|| bank.frames.saturating_sub(*offset));
                if size == 0 || offset.checked_add(size).is_none_or(|end| end > bank.frames) {
                    return Err("PCM range is outside sample".into());
                }
                let pan = match pan.as_str() {
                    "both" => 0xc0,
                    "left" => 0x80,
                    "right" => 0x40,
                    _ => return Err("DAC pan must be both, left or right".into()),
                };
                Ok(Action::DacPlay {
                    bank,
                    offset: *offset,
                    size,
                    pan,
                })
            }
            CommandAction::DacStreamPlay { name } => {
                Ok(Action::DacStream(store.get(media::Kind::DacStream, name)?))
            }
            CommandAction::SamplePlay {
                name,
                voice,
                options,
            } => {
                if *voice == 0
                    || *voice > MAX_SAFE_INTEGER
                    || store.reservations.contains_key(voice)
                {
                    return Err("Native sample voice ID must be unique and positive".into());
                }
                let bank = store.get(media::Kind::Sample, name)?;
                options.validate(&bank, rate)?;
                Ok(Action::SamplePlay(Arc::new(media::Reservation {
                    voice: *voice,
                    bank,
                    options: *options,
                    owner,
                    cancelled: AtomicBool::new(false),
                })))
            }
            _ => self.validate(),
        }
    }
    fn owner(&self, shared: &Shared) -> Result<Option<Owner>, String> {
        match (self.performance, self.owner) {
            (None, None) => Ok(None),
            (Some(performance), Some(owner))
                if performance > 0
                    && performance <= MAX_SAFE_INTEGER
                    && owner > 0
                    && owner <= OWNERS as u32 =>
            {
                let value = Owner { performance, owner };
                if shared.valid_owner(Some(value)) {
                    Ok(Some(value))
                } else {
                    Err("Native performance owner has expired".into())
                }
            }
            _ => {
                Err("Native performance commands require matching performance and owner IDs".into())
            }
        }
    }
}

struct Shared {
    extra_psg: extra_psg::Shared,
    mega_cd: bool,
    nuked: bool,
    genesis_engine: Option<&'static str>,
    dx7: Option<dx7::Shared>,
    nes: Option<nes::Shared>,
    gameboy: Option<gameboy::Shared>,
    snes: Option<snes::Shared>,
    huc6280: Option<huc6280::Shared>,
    ym2608: Option<ym2608::Shared>,
    ym2203: Option<ym2203::Shared>,
    ym2610b: Option<ym2610b::Shared>,
    x68000: Option<x68000::Shared>,
    master_system: Option<master_system::Shared>,
    msx: Option<msx::Shared>,
    opl: Option<opl::Shared>,
    pwm: Option<pwm::Shared>,

    rf5c164: rf5c164::Shared,
    rf_memory: Mutex<Vec<Arc<rf5c164::Memory>>>,
    retired_rf: ArrayQueue<Arc<rf5c164::Memory>>,
    queue: ArrayQueue<Event>,
    monitor: monitor::Monitor,
    livefx: livefx::Control,
    noise: Mutex<noise::Control>,
    noise_completed: [AtomicU64; noise::VOICES],
    media: Mutex<media::Store>,
    fx: Mutex<FxController>,
    retired: ArrayQueue<Arc<media::Reservation>>,
    retired_stops: ArrayQueue<Arc<media::StopTargets>>,
    recording: Mutex<Option<Arc<recording::Capture>>>,
    recorded: ArrayQueue<Arc<recording::Capture>>,
    active_performance: AtomicU64,
    invalidation: AtomicU64,
    cancelled_owners: [AtomicBool; OWNERS],
    dac_cancel_before: AtomicU64,
    owner_dac_cancel_before: [AtomicU64; OWNERS],
    nes_cancel_before: AtomicU64,
    owner_nes_cancel_before: [AtomicU64; OWNERS],
    dac_streams: AtomicUsize,
    producer: Mutex<u64>,
    pending: AtomicUsize,
    frame: AtomicU64,
    epoch: AtomicU64,
    stopped_epoch: AtomicU64,
    late: AtomicU64,
    overflows: AtomicU64,
    failed: AtomicBool,
    rate: u32,
    session: u64,
}
impl Shared {
    #[cfg(test)]
    fn new(rate: u32, session: u64) -> Self {
        Self::new_mode(rate, session, false)
    }
    #[cfg(test)]
    fn new_mode(rate: u32, session: u64, mega_cd: bool) -> Self {
        Self::new_primary(rate, session, mega_cd, false)
    }
    #[cfg(test)]
    fn new_primary(rate: u32, session: u64, mega_cd: bool, dx7: bool) -> Self {
        Self::new_config(rate, session, mega_cd, dx7, false)
    }
    #[cfg(test)]
    fn new_config(rate: u32, session: u64, mega_cd: bool, dx7: bool, nes: bool) -> Self {
        Self::new_modes(rate, session, mega_cd, dx7, nes, None)
    }
    fn new_modes(
        rate: u32,
        session: u64,
        mega_cd: bool,
        dx7: bool,
        nes: bool,
        console: Option<&str>,
    ) -> Self {
        let nuked = matches!(console, Some("32x-nuked" | "genesis-nuked"));
        let genesis_engine = if !dx7
            && !nes
            && console.is_none_or(|mode| matches!(mode, "32x" | "32x-nuked" | "genesis-nuked"))
        {
            Some(if nuked { "nuked" } else { "ymfm" })
        } else {
            None
        };
        Self {
            extra_psg: extra_psg::Shared::default(),
            mega_cd,
            nuked,
            genesis_engine,
            dx7: dx7.then(dx7::Shared::default),
            nes: nes.then(nes::Shared::default),
            gameboy: (console == Some("gameboy")).then(gameboy::Shared::default),
            snes: (console == Some("snes")).then(snes::Shared::default),
            huc6280: (console == Some("huc6280")).then(huc6280::Shared::default),
            ym2608: (console == Some("ym2608")).then(ym2608::Shared::default),
            ym2203: (console == Some("ym2203")).then(ym2203::Shared::default),
            ym2610b: matches!(console, Some("ym2610" | "ym2610b"))
                .then(|| ym2610b::Shared::new(console == Some("ym2610b"))),
            x68000: (console == Some("x68000")).then(x68000::Shared::default),
            master_system: match console {
                Some("master-system") => Some(master_system::Shared::default()),
                Some("game-gear") => Some(master_system::Shared::game_gear()),
                _ => None,
            },
            msx: matches!(console, Some("msx" | "msx-audio")).then(msx::Shared::default),
            opl: console.and_then(opl::Chip::from_mode).map(opl::Shared::new),
            pwm: matches!(console, Some("32x" | "32x-nuked")).then(pwm::Shared::default),

            rf5c164: rf5c164::Shared::default(),
            rf_memory: Mutex::new(Vec::new()),
            retired_rf: ArrayQueue::new(MAX_BATCH),
            queue: ArrayQueue::new(CAPACITY),
            monitor: monitor::Monitor::new(rate),
            livefx: livefx::Control::new(),
            noise: Mutex::new(noise::Control::default()),
            noise_completed: std::array::from_fn(|_| AtomicU64::new(0)),
            media: Mutex::new(media::Store::default()),
            fx: Mutex::new(FxController::default()),
            retired: ArrayQueue::new(CAPACITY),
            retired_stops: ArrayQueue::new(CAPACITY),
            recording: Mutex::new(None),
            recorded: ArrayQueue::new(2),
            active_performance: AtomicU64::new(0),
            invalidation: AtomicU64::new(0),
            cancelled_owners: std::array::from_fn(|_| AtomicBool::new(false)),
            dac_cancel_before: AtomicU64::new(0),
            owner_dac_cancel_before: std::array::from_fn(|_| AtomicU64::new(0)),
            nes_cancel_before: AtomicU64::new(0),
            owner_nes_cancel_before: std::array::from_fn(|_| AtomicU64::new(0)),
            dac_streams: AtomicUsize::new(0),
            producer: Mutex::new(0),
            pending: AtomicUsize::new(0),
            frame: AtomicU64::new(0),
            epoch: AtomicU64::new(0),
            stopped_epoch: AtomicU64::new(0),
            late: AtomicU64::new(0),
            overflows: AtomicU64::new(0),
            failed: AtomicBool::new(false),
            rate,
            session,
        }
    }
    fn enqueue(&self, session: u64, commands: &[Command]) -> Result<(), String> {
        self.check_session(session)?;
        if commands.len() > MAX_BATCH {
            return Err(format!("Native batch exceeds {MAX_BATCH} commands"));
        }
        let mut sequence = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        let mut store = self
            .media
            .lock()
            .map_err(|_| "Native PCM state is unavailable")?;
        self.reclaim(&mut store);
        let mut noise_control = self
            .noise
            .lock()
            .map_err(|_| "Native noise state is unavailable")?;
        let mut noise_plan = noise_control.clone();
        noise_plan.collect(&self.noise_completed);
        let mut rf_memory = self
            .rf_memory
            .lock()
            .map_err(|_| "Native RF5C164 RAM uploads are unavailable")?;
        let mut retired_rf = HashSet::new();
        while let Some(retired) = self.retired_rf.pop() {
            // The control registry remains a lifetime guard until retain below.
            retired_rf.insert(Arc::as_ptr(&retired) as usize);
        }
        if !retired_rf.is_empty() {
            rf_memory.retain(|memory| !retired_rf.contains(&(Arc::as_ptr(memory) as usize)));
        }
        let upload_bytes: usize = commands
            .iter()
            .filter_map(|command| {
                if let CommandAction::Rf5c164LoadMemory { data, .. } = &command.action {
                    Some(data.len())
                } else {
                    None
                }
            })
            .sum();
        let upload_count = commands
            .iter()
            .filter(|command| matches!(&command.action, CommandAction::Rf5c164LoadMemory { .. }))
            .count();
        if upload_count + rf_memory.len() > MAX_BATCH
            || upload_bytes > rf5c164::RAM
            || rf_memory
                .iter()
                .map(|memory| memory.bytes.len())
                .sum::<usize>()
                + upload_bytes
                > rf5c164::PENDING_BYTES
        {
            return Err(
                "Native RF5C164 upload limit reached (64 KiB per batch, 8 MiB pending)".into(),
            );
        }
        // Bound aggregate PC98 uploads before cloning into retained callback
        // commands. Individual uploads are additionally limited to 64 KiB.
        let ym2608_upload_bytes: usize = commands
            .iter()
            .filter_map(|command| {
                if let CommandAction::Ym2608 { message } = &command.action {
                    match &message.method {
                        ym2608::Method::Rom { data, .. } | ym2608::Method::Memory { data, .. } => {
                            Some(data.len())
                        }
                        _ => None,
                    }
                } else {
                    None
                }
            })
            .sum();
        if ym2608_upload_bytes > ym2608::MAX_BATCH_UPLOAD_BYTES {
            return Err("Native YM2608 upload batch exceeds 2 MiB RAM plus 8 KiB ROM".into());
        }
        let ym2610b_upload_bytes: usize = commands
            .iter()
            .filter_map(|command| {
                if let CommandAction::Ym2610b { message } = &command.action {
                    if let ym2610b::Method::Memory { data, .. } = &message.method {
                        return Some(data.len());
                    }
                }
                None
            })
            .sum();
        if ym2610b_upload_bytes > ym2610b::MAX_BATCH_UPLOAD_BYTES {
            return Err("Native YM2610B upload batch exceeds two 16 MiB ROM banks".into());
        }
        let x68000_upload_bytes: usize = commands
            .iter()
            .filter_map(|command| {
                if let CommandAction::X68000 { message } = &command.action {
                    if let x68000::Method::MemoryChunk { data, .. } = &message.method {
                        return Some(data.len());
                    }
                }
                None
            })
            .sum();
        if x68000_upload_bytes > x68000::MAX_BATCH_UPLOAD_BYTES {
            return Err("Native OKIM6258 upload batch exceeds 16 MiB".into());
        }
        let extra_nes_upload_bytes: usize = commands
            .iter()
            .filter_map(|c| match &c.action {
                CommandAction::ExtraNes { message, .. } => match &message.method {
                    extra_nes::Method::Memory { data, .. }
                    | extra_nes::Method::LoadMemory { data, .. } => Some(data.len()),
                    _ => None,
                },
                _ => None,
            })
            .sum();
        self.extra_psg.check_nes_uploads(extra_nes_upload_bytes)?;
        let extra_permits = self
            .extra_psg
            .reserve_payloads(commands.iter().filter_map(|cmd| match &cmd.action {
                CommandAction::ExtraPsg { message, .. } => Some(message.prepared_bytes()),
                CommandAction::ExtraYm2612 { message, .. } => Some(message.prepared_bytes()),
                CommandAction::ExtraYm2203 { message, .. } => Some(message.prepared_bytes()),
                CommandAction::ExtraYm2608 { message, .. } => Some(message.prepared_bytes()),
                CommandAction::ExtraYm2610 { message, .. } => Some(message.prepared_bytes()),
                CommandAction::ExtraYm2151 { message, .. } => Some(message.prepared_bytes()),
                CommandAction::ExtraAy8910 { message, .. } => Some(message.prepared_bytes()),
                CommandAction::ExtraY8950 { message, .. } => Some(message.prepared_bytes()),
                CommandAction::ExtraYmf262 { message, .. } => Some(message.prepared_bytes()),
                CommandAction::ExtraRf5c164 { message, .. } => Some(message.prepared_bytes()),
                CommandAction::ExtraOkim6258 { message, .. } => Some(message.prepared_bytes()),
                CommandAction::ExtraGameboy { message, .. } => Some(message.prepared_bytes()),
                CommandAction::ExtraHuc6280 { message, .. } => Some(message.prepared_bytes()),
                CommandAction::ExtraDx7 { message, .. } => Some(message.prepared_bytes()),
                CommandAction::ExtraPwm { message, .. } => Some(message.prepared_bytes()),
                CommandAction::ExtraNes { message, .. } => Some(message.prepared_bytes()),
                CommandAction::ExtraYm3812 { message, .. } => Some(message.prepared_bytes()),
                CommandAction::ExtraYm3526 { message, .. } => Some(message.prepared_bytes()),
                CommandAction::ExtraYm2413 { message, .. } => Some(message.prepared_bytes()),
                _ => None,
            }))?;
        let extra_rf_upload_bytes: usize = commands
            .iter()
            .filter_map(|command| match &command.action {
                CommandAction::ExtraRf5c164 { message, .. } => match &message.method {
                    extra_rf5c164::Method::LoadMemory { data, .. } => Some(data.len()),
                    extra_rf5c164::Method::WriteMemory { .. } => Some(1),
                    _ => None,
                },
                _ => None,
            })
            .sum();
        if extra_rf_upload_bytes > extra_rf5c164::RAM {
            return Err("Native additional RF5C164 upload batch exceeds64KiB".into());
        }
        let ym2610_upload_bytes: usize = commands
            .iter()
            .map(|command| {
                if let CommandAction::ExtraYm2610 { message, .. } = &command.action {
                    if let extra_ym2610::Method::Memory { data, .. } = &message.method {
                        data.len()
                    } else {
                        0
                    }
                } else {
                    0
                }
            })
            .sum();
        if ym2610_upload_bytes > extra_ym2610::MAX_BATCH_UPLOAD_BYTES {
            return Err("Native additional YM2610 batch upload exceeds 32 MiB".into());
        }
        // Resolve every bank, tag and range before admission or state mutation.
        let mut voices = HashSet::new();
        let mut streams = 0;
        let mut extra_pages = extra_ym2610::PagePlan::default();
        let actions: Vec<_> = commands
            .iter()
            .map(|command| {
                let owner = command.owner(self)?;
                let action = if let CommandAction::ExtraPsg { instance, message } = &command.action
                {
                    if command.frame > MAX_SAFE_INTEGER {
                        return Err("Invalid native command frame".into());
                    }
                    let target = self
                        .extra_psg
                        .get(*instance, self.epoch.load(Ordering::Acquire))?;
                    let prepared = target.prepare(message)?;
                    Action::ExtraPsg(target, prepared)
                } else if let CommandAction::ExtraYm2612 { instance, message } = &command.action {
                    if command.frame > MAX_SAFE_INTEGER {
                        return Err("Invalid native command frame".into());
                    }
                    let target = self
                        .extra_psg
                        .get(*instance, self.epoch.load(Ordering::Acquire))?;
                    target.ym2612()?;
                    let prepared = message.prepare()?;
                    Action::ExtraYm2612(target, prepared)
                } else if let CommandAction::ExtraYm2203 { instance, message } = &command.action {
                    if command.frame > MAX_SAFE_INTEGER {
                        return Err("Invalid native command frame".into());
                    }
                    let target = self
                        .extra_psg
                        .get(*instance, self.epoch.load(Ordering::Acquire))?;
                    target.ym2203()?;
                    if let ym2203::Method::Schedule {
                        duration_samples, ..
                    } = &message.method
                    {
                        let frames =
                            (*duration_samples as u128 * self.rate as u128 + 44099) / 44100;
                        if command.frame.max(self.frame.load(Ordering::Acquire)) as u128 + frames
                            > MAX_SAFE_INTEGER as u128
                        {
                            return Err(
                                "Native YM2203 schedule end exceeds safe frame range".into()
                            );
                        }
                    }
                    Action::ExtraYm2203(target, message.prepare()?)
                } else if let CommandAction::ExtraYm2608 { instance, message } = &command.action {
                    if command.frame > MAX_SAFE_INTEGER {
                        return Err("Invalid native command frame".into());
                    }
                    let target = self
                        .extra_psg
                        .get(*instance, self.epoch.load(Ordering::Acquire))?;
                    target.ym2608()?;
                    let prepared = message.prepare()?;
                    prepared.set_start_frame(command.frame.max(self.frame.load(Ordering::Acquire)));
                    Action::ExtraYm2608(target, prepared)
                } else if let CommandAction::ExtraYm2610 { instance, message } = &command.action {
                    if command.frame > MAX_SAFE_INTEGER {
                        return Err("Invalid native command frame".into());
                    }
                    let target = self
                        .extra_psg
                        .get(*instance, self.epoch.load(Ordering::Acquire))?;
                    target.ym2610()?;
                    let prepared =
                        target
                            .ym2610()?
                            .prepare_message(message, *instance, &mut extra_pages)?;
                    Action::ExtraYm2610(target, prepared)
                } else if let CommandAction::ExtraYm2151 { instance, message } = &command.action {
                    if command.frame > MAX_SAFE_INTEGER {
                        return Err("Invalid native command frame".into());
                    }
                    let target = self
                        .extra_psg
                        .get(*instance, self.epoch.load(Ordering::Acquire))?;
                    target.ym2151()?;
                    let prepared = message.prepare()?;
                    Action::ExtraYm2151(target, prepared)
                } else if let CommandAction::ExtraAy8910 { instance, message } = &command.action {
                    if command.frame > MAX_SAFE_INTEGER {
                        return Err("Invalid native command frame".into());
                    }
                    let target = self
                        .extra_psg
                        .get(*instance, self.epoch.load(Ordering::Acquire))?;
                    target.ay8910()?;
                    let prepared = target.ay8910()?.prepare_message(message)?;
                    Action::ExtraAy8910(target, prepared)
                } else if let CommandAction::ExtraYm2413 { instance, message } = &command.action {
                    if command.frame > MAX_SAFE_INTEGER {
                        return Err("Invalid native command frame".into());
                    }
                    let target = self
                        .extra_psg
                        .get(*instance, self.epoch.load(Ordering::Acquire))?;
                    target.ym2413()?;
                    let prepared = target.ym2413()?.prepare_message(message)?;
                    Action::ExtraYm2413(target, prepared)
                } else if let CommandAction::ExtraYm3526 { instance, message } = &command.action {
                    if command.frame > MAX_SAFE_INTEGER {
                        return Err("Invalid native command frame".into());
                    }
                    let target = self
                        .extra_psg
                        .get(*instance, self.epoch.load(Ordering::Acquire))?;
                    let prepared = target.ym3526()?.prepare_message(message)?;
                    Action::ExtraYm3526(target, prepared)
                } else if let CommandAction::ExtraYm3812 { instance, message } = &command.action {
                    if command.frame > MAX_SAFE_INTEGER {
                        return Err("Invalid native command frame".into());
                    }
                    let target = self
                        .extra_psg
                        .get(*instance, self.epoch.load(Ordering::Acquire))?;
                    let prepared = target.ym3812()?.prepare_message(message)?;
                    Action::ExtraYm3812(target, prepared)
                } else if let CommandAction::ExtraY8950 { instance, message } = &command.action {
                    if command.frame > MAX_SAFE_INTEGER {
                        return Err("Invalid native command frame".into());
                    }
                    let target = self
                        .extra_psg
                        .get(*instance, self.epoch.load(Ordering::Acquire))?;
                    let prepared = target.y8950()?.prepare_message(message)?;
                    Action::ExtraY8950(target, prepared)
                } else if let CommandAction::ExtraYmf262 { instance, message } = &command.action {
                    if command.frame > MAX_SAFE_INTEGER {
                        return Err("Invalid native command frame".into());
                    }
                    let target = self
                        .extra_psg
                        .get(*instance, self.epoch.load(Ordering::Acquire))?;
                    let prepared = target.ymf262()?.prepare_message(message)?;
                    Action::ExtraYmf262(target, prepared)
                } else if let CommandAction::ExtraRf5c164 { instance, message } = &command.action {
                    if command.frame > MAX_SAFE_INTEGER {
                        return Err("Invalid native command frame".into());
                    }
                    let target = self
                        .extra_psg
                        .get(*instance, self.epoch.load(Ordering::Acquire))?;
                    let prepared = target.rf5c164()?.prepare_message(message)?;
                    if prepared.is_owner_cancel() && command.frame != 0 {
                        return Err(
                            "Native RF5C164 owner cancellation requires immediate frame zero"
                                .into(),
                        );
                    }
                    Action::ExtraRf5c164(target, prepared)
                } else if let CommandAction::ExtraOkim6258 { instance, message } = &command.action {
                    if command.frame > MAX_SAFE_INTEGER {
                        return Err("Invalid native command frame".into());
                    }
                    let target = self
                        .extra_psg
                        .get(*instance, self.epoch.load(Ordering::Acquire))?;
                    let prepared = target.okim6258()?.prepare_message(message)?;
                    if prepared.is_owner_cancel() && command.frame != 0 {
                        return Err(
                            "Native OKIM6258 owner cancellation requires immediate frame zero"
                                .into(),
                        );
                    }
                    Action::ExtraOkim6258(target, prepared)
                } else if let CommandAction::ExtraHuc6280 { instance, message } = &command.action {
                    if command.frame > MAX_SAFE_INTEGER {
                        return Err("Invalid native command frame".into());
                    }
                    let target = self
                        .extra_psg
                        .get(*instance, self.epoch.load(Ordering::Acquire))?;
                    let prepared = target.huc6280()?.prepare_message(message)?;
                    if prepared.is_owner_cancel() && command.frame != 0 {
                        return Err(
                            "Native HuC6280 owner cancellation requires immediate frame zero"
                                .into(),
                        );
                    }
                    Action::ExtraHuc6280(target, prepared)
                } else if let CommandAction::ExtraDx7 { instance, message } = &command.action {
                    if command.frame > MAX_SAFE_INTEGER {
                        return Err("Invalid native command frame".into());
                    }
                    let target = self
                        .extra_psg
                        .get(*instance, self.epoch.load(Ordering::Acquire))?;
                    let prepared = target.dx7()?.prepare_message(message)?;
                    if prepared.is_owner_cancel() && command.frame != 0 {
                        return Err(
                            "Native DX7 owner cancellation requires immediate frame zero".into(),
                        );
                    }
                    Action::ExtraDx7(target, prepared)
                } else if let CommandAction::ExtraPwm { instance, message } = &command.action {
                    if command.frame > MAX_SAFE_INTEGER {
                        return Err("Invalid native command frame".into());
                    }
                    let target = self
                        .extra_psg
                        .get(*instance, self.epoch.load(Ordering::Acquire))?;
                    let prepared = target.pwm()?.prepare_message(message)?;
                    if prepared.is_owner_cancel() && command.frame != 0 {
                        return Err(
                            "Native PWM owner cancellation requires immediate frame zero".into(),
                        );
                    }
                    Action::ExtraPwm(target, prepared)
                } else if let CommandAction::ExtraGameboy { instance, message } = &command.action {
                    if command.frame > MAX_SAFE_INTEGER {
                        return Err("Invalid native command frame".into());
                    }
                    let target = self
                        .extra_psg
                        .get(*instance, self.epoch.load(Ordering::Acquire))?;
                    let prepared = target.gameboy()?.prepare_message(message)?;
                    if prepared.is_owner_cancel() && command.frame != 0 {
                        return Err(
                            "Native Game Boy owner cancellation requires immediate frame zero"
                                .into(),
                        );
                    }
                    Action::ExtraGameboy(target, prepared)
                } else if let CommandAction::ExtraNes { instance, message } = &command.action {
                    if command.frame > MAX_SAFE_INTEGER {
                        return Err("Invalid native command frame".into());
                    }
                    let target = self
                        .extra_psg
                        .get(*instance, self.epoch.load(Ordering::Acquire))?;
                    let prepared = target.nes()?.prepare_message(message)?;
                    if prepared.is_owner_cancel() && command.frame != 0 {
                        return Err(
                            "Native NES owner cancellation requires immediate frame zero".into(),
                        );
                    }
                    Action::ExtraNes(target, prepared)
                } else if let CommandAction::Noise { message } = &command.action {
                    Action::Noise(noise_plan.prepare(message, self.rate)?)
                } else {
                    command.resolve(&store, self.rate, owner)?
                };
                if matches!(
                    &action,
                    Action::RfWrite(..)
                        | Action::RfWriteMemory(..)
                        | Action::RfLoad(..)
                        | Action::RfReset
                        | Action::RfClearMemory
                        | Action::RfClock(..)
                        | Action::RfMix(..)
                        | Action::RfKey(..)
                ) && !self.mega_cd
                {
                    return Err("Native RF5C164 requires Mega CD mode".into());
                }
                if matches!(&action, Action::Dx7(_)) && self.dx7.is_none() {
                    return Err("Native DX7 commands require DX7 mode".into());
                }
                if matches!(&action, Action::Nes(_)) && self.nes.is_none() {
                    return Err("Native NES commands require NES mode".into());
                }
                if matches!(&action, Action::Gameboy(_)) && self.gameboy.is_none() {
                    return Err("Native Gameboy requires gameboy mode".into());
                }
                if matches!(&action, Action::Snes(_)) && self.snes.is_none() {
                    return Err("Native Snes requires snes mode".into());
                }
                if matches!(&action, Action::Huc6280(_)) && self.huc6280.is_none() {
                    return Err("Native Huc6280 requires huc6280 mode".into());
                }
                if matches!(&action, Action::Ym2608(_)) && self.ym2608.is_none() {
                    return Err("Native YM2608 requires PC98 mode".into());
                }
                if matches!(&action, Action::Ym2203(_)) && self.ym2203.is_none() {
                    return Err("Native YM2203 requires PC98-26 mode".into());
                }
                if matches!(&action, Action::Ym2610b(_)) && self.ym2610b.is_none() {
                    return Err("Native YM2610B requires Neo Geo mode".into());
                }

                if let Action::MasterSystem(command) = &action {
                    if !self
                        .master_system
                        .as_ref()
                        .is_some_and(|chip| chip.accepts(command))
                    {
                        return Err(
                            "Native PSG command is unavailable in the selected machine mode".into(),
                        );
                    }
                }
                if matches!(&action, Action::Msx(_)) && self.msx.is_none() {
                    return Err("Native MSX commands require MSX mode".into());
                }
                if let Action::Opl(command) = &action {
                    if !self.opl.as_ref().is_some_and(|chip| chip.accepts(command)) {
                        return Err("Native OPL command requires its selected DOS PC mode".into());
                    }
                }
                if matches!(&action, Action::X68000(_)) && self.x68000.is_none() {
                    return Err("Native X68000 commands require X68000 mode".into());
                }
                if let CommandAction::X68000 { message } = &command.action {
                    if command.frame != 0
                        && matches!(
                            &message.method,
                            x68000::Method::MemoryBegin { .. }
                                | x68000::Method::MemoryChunk { .. }
                                | x68000::Method::MemoryCommit { .. }
                        )
                    {
                        return Err(
                            "Native OKIM6258 sample uploads must be prepared before playback"
                                .into(),
                        );
                    }
                }
                if matches!(&action, Action::Pwm(_)) && self.pwm.is_none() {
                    return Err("Native PWM requires 32X mode".into());
                }
                if (self.dx7.is_some()
                    || self.nes.is_some()
                    || self.gameboy.is_some()
                    || self.snes.is_some()
                    || self.huc6280.is_some()
                    || self.ym2608.is_some()
                    || self.ym2203.is_some()
                    || self.ym2610b.is_some()
                    || self.x68000.is_some()
                    || self.master_system.is_some()
                    || self.msx.is_some()
                    || self.opl.is_some())
                    && matches!(
                        &action,
                        Action::Write(..)
                            | Action::PsgWrite(_)
                            | Action::PsgStereo(_)
                            | Action::PsgReset
                            | Action::DacPlay { .. }
                            | Action::DacStream(_)
                            | Action::DacStop
                    )
                {
                    return Err(
                        "Genesis FM, PSG and DAC commands are unavailable in this Native mode"
                            .into(),
                    );
                }
                if let Action::SamplePlay(reservation) = &action {
                    if !voices.insert(reservation.voice) {
                        return Err("Duplicate native sample voice ID in batch".into());
                    }
                }
                if matches!(action, Action::DacStream(_)) {
                    streams += 1;
                }
                let owner = if matches!(&action,Action::ExtraYm2610(_,c)if c.is_memory()) {
                    None
                } else {
                    owner
                };
                Ok((action, owner))
            })
            .collect::<Result<_, String>>()?;
        if store.reservations.len() + voices.len() > media::VOICES {
            return Err("Native sample reservation limit (64) reached".into());
        }
        if streams + self.dac_streams.load(Ordering::Acquire) > 8 {
            return Err("Native DAC stream batch limit (8) reached".into());
        }
        if self
            .pending
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |pending| {
                pending
                    .checked_add(commands.len())
                    .filter(|next| *next <= CAPACITY)
            })
            .is_err()
        {
            self.overflows.fetch_add(1, Ordering::Relaxed);
            return Err("Native command queue is full; playback was not silently truncated".into());
        }
        if let Err(error) = self.retain_extra_psg(&actions, extra_permits) {
            self.pending.fetch_sub(commands.len(), Ordering::AcqRel);
            return Err(error);
        }
        if let Some(dx7) = &self.dx7 {
            let prepared: Vec<_> = actions
                .iter()
                .filter_map(|(action, _)| {
                    if let Action::Dx7(command) = action {
                        Some(command.clone())
                    } else {
                        None
                    }
                })
                .collect();
            if let Err(error) = dx7.retain(&prepared) {
                self.rollback_extra_psg(&actions);
                self.pending.fetch_sub(commands.len(), Ordering::AcqRel);
                return Err(error);
            }
        }
        if let Some(nes) = &self.nes {
            let prepared: Vec<_> = actions
                .iter()
                .filter_map(|(action, _)| {
                    if let Action::Nes(command) = action {
                        Some(command.clone())
                    } else {
                        None
                    }
                })
                .collect();
            if let Err(error) = nes.retain(&prepared) {
                self.rollback_extra_psg(&actions);
                self.pending.fetch_sub(commands.len(), Ordering::AcqRel);
                return Err(error);
            }
        }
        if let Some(chip) = &self.gameboy {
            let prepared: Vec<_> = actions
                .iter()
                .filter_map(|(action, _)| {
                    if let Action::Gameboy(command) = action {
                        Some(command.clone())
                    } else {
                        None
                    }
                })
                .collect();
            if let Err(error) = chip.retain(&prepared) {
                self.rollback_extra_psg(&actions);
                self.pending.fetch_sub(commands.len(), Ordering::AcqRel);
                return Err(error);
            }
        }
        if let Some(chip) = &self.snes {
            let prepared: Vec<_> = actions
                .iter()
                .filter_map(|(action, _)| {
                    if let Action::Snes(command) = action {
                        Some(command.clone())
                    } else {
                        None
                    }
                })
                .collect();
            if let Err(error) = chip.retain(&prepared) {
                self.rollback_extra_psg(&actions);
                self.pending.fetch_sub(commands.len(), Ordering::AcqRel);
                return Err(error);
            }
        }
        if let Some(chip) = &self.huc6280 {
            let prepared: Vec<_> = actions
                .iter()
                .filter_map(|(action, _)| {
                    if let Action::Huc6280(command) = action {
                        Some(command.clone())
                    } else {
                        None
                    }
                })
                .collect();
            if let Err(error) = chip.retain(&prepared) {
                self.rollback_extra_psg(&actions);
                self.pending.fetch_sub(commands.len(), Ordering::AcqRel);
                return Err(error);
            }
        }
        if let Some(chip) = &self.ym2608 {
            let prepared: Vec<_> = actions
                .iter()
                .filter_map(|(action, _)| {
                    if let Action::Ym2608(command) = action {
                        Some(command.clone())
                    } else {
                        None
                    }
                })
                .collect();
            if let Err(error) = chip.retain(&prepared) {
                self.rollback_extra_psg(&actions);
                self.pending.fetch_sub(commands.len(), Ordering::AcqRel);
                return Err(error);
            }
        }
        if let Some(chip) = &self.ym2610b {
            let prepared: Vec<_> = actions
                .iter()
                .filter_map(|(action, _)| {
                    if let Action::Ym2610b(command) = action {
                        Some(command.clone())
                    } else {
                        None
                    }
                })
                .collect();
            if let Err(error) = chip.retain(&prepared) {
                self.rollback_extra_psg(&actions);
                self.pending.fetch_sub(commands.len(), Ordering::AcqRel);
                return Err(error);
            }
        }
        if let Some(chip) = &self.x68000 {
            let prepared: Vec<_> = actions
                .iter()
                .filter_map(|(action, owner)| {
                    if let Action::X68000(command) = action {
                        Some((command.clone(), *owner))
                    } else {
                        None
                    }
                })
                .collect();
            if let Err(error) = chip.retain(&prepared) {
                self.rollback_extra_psg(&actions);
                self.pending.fetch_sub(commands.len(), Ordering::AcqRel);
                return Err(error);
            }
        }
        if let Some(chip) = &self.ym2203 {
            let prepared: Vec<_> = actions
                .iter()
                .filter_map(|(action, owner)| {
                    if let Action::Ym2203(command) = action {
                        Some((command.clone(), *owner))
                    } else {
                        None
                    }
                })
                .collect();
            if let Err(error) = chip.retain(&prepared) {
                self.rollback_extra_psg(&actions);
                self.pending.fetch_sub(commands.len(), Ordering::AcqRel);
                return Err(error);
            }
        }
        if let Some(chip) = &self.master_system {
            let prepared: Vec<_> = actions
                .iter()
                .filter_map(|(action, owner)| {
                    if let Action::MasterSystem(command) = action {
                        Some((command.clone(), *owner))
                    } else {
                        None
                    }
                })
                .collect();
            if let Err(error) = chip.retain(&prepared) {
                self.rollback_extra_psg(&actions);
                self.pending.fetch_sub(commands.len(), Ordering::AcqRel);
                return Err(error);
            }
        }
        if let Some(chip) = &self.msx {
            let prepared: Vec<_> = actions
                .iter()
                .filter_map(|(action, owner)| {
                    if let Action::Msx(command) = action {
                        Some((command.clone(), *owner))
                    } else {
                        None
                    }
                })
                .collect();
            let result = if let Some(opl) = &self.opl {
                let opl_prepared: Vec<_> = actions
                    .iter()
                    .filter_map(|(action, owner)| {
                        if let Action::Opl(command) = action {
                            Some((command.clone(), *owner))
                        } else {
                            None
                        }
                    })
                    .collect();
                chip.retain_then(&prepared, || opl.retain(&opl_prepared))
            } else {
                chip.retain(&prepared)
            };
            if let Err(error) = result {
                self.rollback_extra_psg(&actions);
                self.pending.fetch_sub(commands.len(), Ordering::AcqRel);
                return Err(error);
            }
        } else if let Some(chip) = &self.opl {
            let prepared: Vec<_> = actions
                .iter()
                .filter_map(|(action, owner)| {
                    if let Action::Opl(command) = action {
                        Some((command.clone(), *owner))
                    } else {
                        None
                    }
                })
                .collect();
            if let Err(error) = chip.retain(&prepared) {
                self.rollback_extra_psg(&actions);
                self.pending.fetch_sub(commands.len(), Ordering::AcqRel);
                return Err(error);
            }
        }
        if let Some(chip) = &self.pwm {
            let prepared: Vec<_> = actions
                .iter()
                .filter_map(|(action, _)| {
                    if let Action::Pwm(command) = action {
                        Some(command.clone())
                    } else {
                        None
                    }
                })
                .collect();
            if let Err(error) = chip.retain(&prepared) {
                self.rollback_extra_psg(&actions);
                self.pending.fetch_sub(commands.len(), Ordering::AcqRel);
                return Err(error);
            }
        }
        for (action, _) in &actions {
            if let Action::ExtraYm2610(instance, command) = action {
                instance
                    .ym2610()
                    .expect("YM2610 action target")
                    .commit_pages(command)
            }
        }
        // Scoped RF cleanup invalidates already admitted lookahead only after
        // every family has retained successfully. Before this commit point a
        // later family's rejection must leave all older RF events unchanged.
        for (action, _) in &actions {
            if let Action::ExtraRf5c164(instance, command) = action {
                if instance
                    .rf5c164()
                    .expect("RF5C164 action target")
                    .commit_cancel(command)
                {
                    self.invalidation.fetch_add(1, Ordering::Release);
                }
            }
        }
        for (action, _) in &actions {
            if let Action::ExtraOkim6258(instance, command) = action {
                if instance
                    .okim6258()
                    .expect("OKIM6258 action target")
                    .commit_cancel(command)
                {
                    self.invalidation.fetch_add(1, Ordering::Release);
                }
            }
        }
        for (action, _) in &actions {
            if let Action::ExtraHuc6280(instance, command) = action {
                if instance
                    .huc6280()
                    .expect("HuC6280 action target")
                    .commit_cancel(command)
                {
                    self.invalidation.fetch_add(1, Ordering::Release);
                }
            }
            if let Action::ExtraDx7(instance, command) = action {
                if instance
                    .dx7()
                    .expect("DX7 action target")
                    .commit_cancel(command)
                {
                    self.invalidation.fetch_add(1, Ordering::Release);
                }
            }
            if let Action::ExtraPwm(instance, command) = action {
                if instance
                    .pwm()
                    .expect("PWM action target")
                    .commit_cancel(command)
                {
                    self.invalidation.fetch_add(1, Ordering::Release);
                }
            }
            if let Action::ExtraGameboy(instance, command) = action {
                if instance
                    .gameboy()
                    .expect("Game Boy action target")
                    .commit_cancel(command)
                {
                    self.invalidation.fetch_add(1, Ordering::Release);
                }
            }
            if let Action::ExtraNes(instance, command) = action {
                if instance
                    .nes()
                    .expect("NES action target")
                    .commit_cancel(command)
                {
                    self.invalidation.fetch_add(1, Ordering::Release);
                }
            }
        }
        *noise_control = noise_plan;
        self.dac_streams.fetch_add(streams, Ordering::AcqRel);
        let epoch = self.epoch.load(Ordering::Acquire);
        for (command, (mut action, owner)) in commands.iter().zip(actions) {
            match &action {
                Action::RfLoad(memory) => rf_memory.push(memory.clone()),
                Action::SamplePlay(reservation) => {
                    store
                        .reservations
                        .insert(reservation.voice, reservation.clone());
                }
                Action::SampleStop { voice, name } if command.frame == 0 => {
                    store.cancel(*voice, name.as_deref(), owner);
                    self.invalidation.fetch_add(1, Ordering::Release);
                }
                Action::DacStop if command.frame == 0 => self.cancel_dac_before(owner, *sequence),
                _ => {}
            }
            // Sample-stop names belong to the control side. The render event
            // is only an ordering marker; avoid freeing strings in the callback.
            if let Action::SampleStop { voice, name } = &action {
                action = if command.frame == 0 {
                    Action::SampleRelease
                } else {
                    Action::SampleTimedStop(store.stop_targets(
                        *sequence,
                        *voice,
                        name.as_deref(),
                        owner,
                    ))
                };
            }
            let frame = if matches!(&action,Action::ExtraYm2610(_,c)if c.is_memory()) {
                0
            } else {
                command.frame
            };
            let event = Event {
                frame,
                sequence: *sequence,
                epoch,
                action: RefCell::new(action),
                owner,
            };
            *sequence += 1;
            // Admission includes both the queue and callback heap, so capacity
            // is guaranteed while the only consumer removes queue entries.
            if self.queue.push(event).is_err() {
                unreachable!("Reserved native event capacity");
            }
        }
        Ok(())
    }
    fn retain_extra_psg(
        &self,
        actions: &[(Action, Option<Owner>)],
        extra_permits: Vec<extra_budget::Permit>,
    ) -> Result<(), String> {
        let ym2610_bytes: usize = actions
            .iter()
            .map(|(a, _)| {
                if let Action::ExtraYm2610(_, c) = a {
                    c.upload_bytes()
                } else {
                    0
                }
            })
            .sum();
        if ym2610_bytes > extra_ym2610::MAX_BATCH_UPLOAD_BYTES {
            return Err("Native additional YM2610 batch upload exceeds 32 MiB".into());
        }
        let upload_bytes: usize = actions
            .iter()
            .map(|(a, _)| {
                if let Action::ExtraYm2608(_, c) = a {
                    c.upload_bytes()
                } else {
                    0
                }
            })
            .sum();
        if upload_bytes > extra_ym2608::MAX_BATCH_UPLOAD_BYTES {
            return Err("Native additional YM2608 batch upload exceeds 4 MiB".into());
        }
        self.extra_psg.retain_payloads(
            actions
                .iter()
                .filter_map(|(a, _)| match a {
                    Action::ExtraPsg(_, c) => Some(extra_psg::Payload::Psg(c.clone())),
                    Action::ExtraYm2612(_, c) => Some(extra_psg::Payload::Ym2612(c.clone())),
                    Action::ExtraYm2203(_, c) => Some(extra_psg::Payload::Ym2203(c.clone())),
                    Action::ExtraYm2608(_, c) => Some(extra_psg::Payload::Ym2608(c.clone())),
                    Action::ExtraYm2610(_, c) => Some(extra_psg::Payload::Ym2610(c.clone())),
                    Action::ExtraYm2151(_, c) => Some(extra_psg::Payload::Ym2151(c.clone())),
                    Action::ExtraAy8910(_, c) => Some(extra_psg::Payload::Ay8910(c.clone())),
                    Action::ExtraY8950(_, c) => Some(extra_psg::Payload::Y8950(c.clone())),
                    Action::ExtraYmf262(_, c) => Some(extra_psg::Payload::Ymf262(c.clone())),
                    Action::ExtraRf5c164(_, c) => Some(extra_psg::Payload::Rf5c164(c.clone())),
                    Action::ExtraOkim6258(_, c) => Some(extra_psg::Payload::Okim6258(c.clone())),
                    Action::ExtraGameboy(_, c) => Some(extra_psg::Payload::Gameboy(c.clone())),
                    Action::ExtraHuc6280(_, c) => Some(extra_psg::Payload::Huc6280(c.clone())),
                    Action::ExtraDx7(_, c) => Some(extra_psg::Payload::Dx7(c.clone())),
                    Action::ExtraPwm(_, c) => Some(extra_psg::Payload::Pwm(c.clone())),
                    Action::ExtraNes(_, c) => Some(extra_psg::Payload::Nes(c.clone())),
                    Action::ExtraYm3812(_, c) => Some(extra_psg::Payload::Ym3812(c.clone())),
                    Action::ExtraYm3526(_, c) => Some(extra_psg::Payload::Ym3526(c.clone())),
                    Action::ExtraYm2413(_, c) => Some(extra_psg::Payload::Ym2413(c.clone())),
                    _ => None,
                })
                .collect(),
            extra_permits,
        )?;

        let mut retained = Vec::new();
        let mut seen = HashSet::new();
        for (action, _) in actions {
            let instance = match action {
                Action::ExtraPsg(instance, _)
                | Action::ExtraYm2612(instance, _)
                | Action::ExtraYm2203(instance, _)
                | Action::ExtraYm2608(instance, _)
                | Action::ExtraYm2610(instance, _)
                | Action::ExtraYm2151(instance, _)
                | Action::ExtraAy8910(instance, _)
                | Action::ExtraY8950(instance, _)
                | Action::ExtraYmf262(instance, _)
                | Action::ExtraRf5c164(instance, _)
                | Action::ExtraOkim6258(instance, _)
                | Action::ExtraGameboy(instance, _)
                | Action::ExtraHuc6280(instance, _)
                | Action::ExtraDx7(instance, _)
                | Action::ExtraPwm(instance, _)
                | Action::ExtraNes(instance, _)
                | Action::ExtraYm3812(instance, _)
                | Action::ExtraYm3526(instance, _)
                | Action::ExtraYm2413(instance, _) => instance,
                _ => continue,
            };
            if !seen.insert(instance.id) {
                continue;
            }
            let result = match action {
                Action::ExtraPsg(_, _) => {
                    let prepared: Vec<_> = actions
                        .iter()
                        .filter_map(|(action, owner)| {
                            if let Action::ExtraPsg(target, command) = action {
                                (target.id == instance.id).then(|| (command.clone(), *owner))
                            } else {
                                None
                            }
                        })
                        .collect();
                    instance.psg()?.retain(&prepared).map_err(extra_psg::error)
                }
                Action::ExtraYm2612(_, _) => {
                    let prepared: Vec<_> = actions
                        .iter()
                        .filter_map(|(action, owner)| {
                            if let Action::ExtraYm2612(target, command) = action {
                                (target.id == instance.id).then(|| (command.clone(), *owner))
                            } else {
                                None
                            }
                        })
                        .collect();
                    instance.ym2612()?.retain(&prepared)
                }
                Action::ExtraYm2203(_, _) => {
                    let prepared: Vec<_> = actions
                        .iter()
                        .filter_map(|(action, owner)| {
                            if let Action::ExtraYm2203(target, command) = action {
                                (target.id == instance.id).then(|| (command.clone(), *owner))
                            } else {
                                None
                            }
                        })
                        .collect();
                    instance.ym2203()?.retain(&prepared)
                }
                Action::ExtraYm2608(_, _) => {
                    let prepared: Vec<_> = actions
                        .iter()
                        .filter_map(|(action, owner)| {
                            if let Action::ExtraYm2608(target, command) = action {
                                (target.id == instance.id).then(|| (command.clone(), *owner))
                            } else {
                                None
                            }
                        })
                        .collect();
                    instance.ym2608()?.retain(&prepared)
                }
                Action::ExtraYm2610(_, _) => {
                    let prepared: Vec<_> = actions
                        .iter()
                        .filter_map(|(action, owner)| {
                            if let Action::ExtraYm2610(target, command) = action {
                                (target.id == instance.id).then(|| (command.clone(), *owner))
                            } else {
                                None
                            }
                        })
                        .collect();
                    instance.ym2610()?.retain(&prepared)
                }
                Action::ExtraYm2151(_, _) => {
                    let prepared: Vec<_> = actions
                        .iter()
                        .filter_map(|(action, owner)| {
                            if let Action::ExtraYm2151(target, command) = action {
                                (target.id == instance.id).then(|| (command.clone(), *owner))
                            } else {
                                None
                            }
                        })
                        .collect();
                    instance.ym2151()?.retain(&prepared)
                }
                Action::ExtraYm3526(_, _) => {
                    let prepared: Vec<_> = actions
                        .iter()
                        .filter_map(|(action, owner)| {
                            if let Action::ExtraYm3526(target, command) = action {
                                (target.id == instance.id).then(|| (command.clone(), *owner))
                            } else {
                                None
                            }
                        })
                        .collect();
                    instance.ym3526()?.retain(&prepared)
                }
                Action::ExtraYm3812(_, _) => {
                    let prepared: Vec<_> = actions
                        .iter()
                        .filter_map(|(action, owner)| {
                            if let Action::ExtraYm3812(target, command) = action {
                                (target.id == instance.id).then(|| (command.clone(), *owner))
                            } else {
                                None
                            }
                        })
                        .collect();
                    instance.ym3812()?.retain(&prepared)
                }
                Action::ExtraYmf262(_, _) => {
                    let prepared: Vec<_> = actions
                        .iter()
                        .filter_map(|(action, owner)| {
                            if let Action::ExtraYmf262(target, command) = action {
                                (target.id == instance.id).then(|| (command.clone(), *owner))
                            } else {
                                None
                            }
                        })
                        .collect();
                    instance.ymf262()?.retain(&prepared)
                }
                Action::ExtraRf5c164(_, _) => {
                    let prepared: Vec<_> = actions
                        .iter()
                        .filter_map(|(action, owner)| {
                            if let Action::ExtraRf5c164(target, command) = action {
                                (target.id == instance.id).then(|| (command.clone(), *owner))
                            } else {
                                None
                            }
                        })
                        .collect();
                    instance.rf5c164()?.retain(&prepared)
                }
                Action::ExtraOkim6258(_, _) => {
                    let prepared: Vec<_> = actions
                        .iter()
                        .filter_map(|(action, owner)| {
                            if let Action::ExtraOkim6258(target, command) = action {
                                (target.id == instance.id).then(|| (command.clone(), *owner))
                            } else {
                                None
                            }
                        })
                        .collect();
                    instance.okim6258()?.retain(&prepared)
                }
                Action::ExtraHuc6280(_, _) => {
                    let prepared: Vec<_> = actions
                        .iter()
                        .filter_map(|(action, owner)| {
                            if let Action::ExtraHuc6280(target, command) = action {
                                (target.id == instance.id).then(|| (command.clone(), *owner))
                            } else {
                                None
                            }
                        })
                        .collect();
                    instance.huc6280()?.retain(&prepared)
                }
                Action::ExtraDx7(_, _) => {
                    let prepared: Vec<_> = actions
                        .iter()
                        .filter_map(|(action, owner)| {
                            if let Action::ExtraDx7(target, command) = action {
                                (target.id == instance.id).then(|| (command.clone(), *owner))
                            } else {
                                None
                            }
                        })
                        .collect();
                    instance.dx7()?.retain(&prepared)
                }
                Action::ExtraPwm(_, _) => {
                    let prepared: Vec<_> = actions
                        .iter()
                        .filter_map(|(action, owner)| {
                            if let Action::ExtraPwm(target, command) = action {
                                (target.id == instance.id).then(|| (command.clone(), *owner))
                            } else {
                                None
                            }
                        })
                        .collect();
                    instance.pwm()?.retain(&prepared)
                }
                Action::ExtraGameboy(_, _) => {
                    let prepared: Vec<_> = actions
                        .iter()
                        .filter_map(|(action, owner)| {
                            if let Action::ExtraGameboy(target, command) = action {
                                (target.id == instance.id).then(|| (command.clone(), *owner))
                            } else {
                                None
                            }
                        })
                        .collect();
                    instance.gameboy()?.retain(&prepared)
                }
                Action::ExtraNes(_, _) => {
                    let prepared: Vec<_> = actions
                        .iter()
                        .filter_map(|(action, owner)| {
                            if let Action::ExtraNes(target, command) = action {
                                (target.id == instance.id).then(|| (command.clone(), *owner))
                            } else {
                                None
                            }
                        })
                        .collect();
                    instance.nes()?.retain(&prepared)
                }
                Action::ExtraY8950(_, _) => {
                    let prepared: Vec<_> = actions
                        .iter()
                        .filter_map(|(action, owner)| {
                            if let Action::ExtraY8950(target, command) = action {
                                (target.id == instance.id).then(|| (command.clone(), *owner))
                            } else {
                                None
                            }
                        })
                        .collect();
                    instance.y8950()?.retain(&prepared)
                }
                Action::ExtraAy8910(_, _) => {
                    let prepared: Vec<_> = actions
                        .iter()
                        .filter_map(|(action, owner)| {
                            if let Action::ExtraAy8910(target, command) = action {
                                (target.id == instance.id).then(|| (command.clone(), *owner))
                            } else {
                                None
                            }
                        })
                        .collect();
                    instance.ay8910()?.retain(&prepared)
                }
                Action::ExtraYm2413(_, _) => {
                    let prepared: Vec<_> = actions
                        .iter()
                        .filter_map(|(action, owner)| {
                            if let Action::ExtraYm2413(target, command) = action {
                                (target.id == instance.id).then(|| (command.clone(), *owner))
                            } else {
                                None
                            }
                        })
                        .collect();
                    instance.ym2413()?.retain(&prepared)
                }
                _ => unreachable!(),
            };
            if let Err(error) = result {
                self.rollback_extra_psg(&retained);
                return Err(error);
            }
            retained.extend(
                actions
                    .iter()
                    .filter(|(action, _)| match action {
                        Action::ExtraPsg(target, _)
                        | Action::ExtraYm2612(target, _)
                        | Action::ExtraYm2203(target, _)
                        | Action::ExtraYm2608(target, _)
                        | Action::ExtraYm2610(target, _)
                        | Action::ExtraYm2151(target, _)
                        | Action::ExtraAy8910(target, _)
                        | Action::ExtraY8950(target, _)
                        | Action::ExtraYmf262(target, _)
                        | Action::ExtraRf5c164(target, _)
                        | Action::ExtraOkim6258(target, _)
                        | Action::ExtraGameboy(target, _)
                        | Action::ExtraHuc6280(target, _)
                        | Action::ExtraDx7(target, _)
                        | Action::ExtraPwm(target, _)
                        | Action::ExtraNes(target, _)
                        | Action::ExtraYm3812(target, _)
                        | Action::ExtraYm3526(target, _)
                        | Action::ExtraYm2413(target, _) => target.id == instance.id,
                        _ => false,
                    })
                    .cloned(),
            );
        }
        Ok(())
    }
    fn rollback_extra_psg(&self, actions: &[(Action, Option<Owner>)]) {
        for (action, _) in actions {
            match action {
                Action::ExtraPsg(instance, command) => {
                    let state = instance.psg().expect("PSG action target");
                    state.retire(command.clone());
                    let _ = state.game_gear_status(self.frame.load(Ordering::Acquire));
                }
                Action::ExtraYm2612(instance, command) => {
                    let state = instance.ym2612().expect("YM2612 action target");
                    state.retire(command.clone());
                    let _ = state.status(self.frame.load(Ordering::Acquire));
                }
                Action::ExtraYm2203(instance, command) => {
                    let state = instance.ym2203().expect("YM2203 action target");
                    state.rollback(command.clone());
                    let _ = state.status(self.frame.load(Ordering::Acquire));
                }
                Action::ExtraYm2608(instance, command) => {
                    let state = instance.ym2608().expect("YM2608 action target");
                    state.rollback(command.clone());
                    let _ = state.status(self.frame.load(Ordering::Acquire));
                }
                Action::ExtraYm2610(instance, command) => {
                    let state = instance.ym2610().expect("YM2610 action target");
                    state.rollback(command.clone());
                    let _ = state.status(self.frame.load(Ordering::Acquire));
                }
                Action::ExtraYm2151(instance, command) => {
                    let state = instance.ym2151().expect("YM2151 action target");
                    state.rollback(command.clone());
                    let _ = state.status(self.frame.load(Ordering::Acquire));
                }
                Action::ExtraYm3526(instance, command) => {
                    let state = instance.ym3526().expect("YM3526 action target");
                    state.rollback(command.clone());
                    let _ = state.status(self.frame.load(Ordering::Acquire));
                }
                Action::ExtraYm3812(instance, command) => {
                    let state = instance.ym3812().expect("YM3812 action target");
                    state.rollback(command.clone());
                    let _ = state.status(self.frame.load(Ordering::Acquire));
                }
                Action::ExtraYmf262(instance, command) => {
                    let state = instance.ymf262().expect("YMF262 action target");
                    state.rollback(command.clone());
                    let _ = state.status(self.frame.load(Ordering::Acquire));
                }
                Action::ExtraRf5c164(instance, command) => {
                    let state = instance.rf5c164().expect("RF5C164 action target");
                    state.rollback(command.clone());
                    let _ = state.status(self.frame.load(Ordering::Acquire));
                }
                Action::ExtraOkim6258(instance, command) => {
                    let state = instance.okim6258().expect("OKIM6258 action target");
                    state.rollback(command.clone());
                    let _ = state.status(self.frame.load(Ordering::Acquire));
                }
                Action::ExtraHuc6280(instance, command) => {
                    let state = instance.huc6280().expect("HuC6280 action target");
                    state.rollback(command.clone());
                    let _ = state.status(self.frame.load(Ordering::Acquire));
                }
                Action::ExtraDx7(instance, command) => {
                    let state = instance.dx7().expect("DX7 action target");
                    state.rollback(command.clone());
                    let _ = state.status(self.frame.load(Ordering::Acquire));
                }
                Action::ExtraPwm(instance, command) => {
                    let state = instance.pwm().expect("PWM action target");
                    state.rollback(command.clone());
                    let _ = state.status(self.frame.load(Ordering::Acquire));
                }
                Action::ExtraGameboy(instance, command) => {
                    let state = instance.gameboy().expect("Game Boy action target");
                    state.rollback(command.clone());
                    let _ = state.status(self.frame.load(Ordering::Acquire));
                }
                Action::ExtraNes(instance, command) => {
                    let state = instance.nes().expect("NES action target");
                    state.rollback(command.clone());
                    let _ = state.status(self.frame.load(Ordering::Acquire));
                }
                Action::ExtraY8950(instance, command) => {
                    let state = instance.y8950().expect("Y8950 action target");
                    state.rollback(command.clone());
                    let _ = state.status(self.frame.load(Ordering::Acquire));
                }
                Action::ExtraAy8910(instance, command) => {
                    let state = instance.ay8910().expect("AY8910 action target");
                    state.rollback(command.clone());
                    let _ = state.status(self.frame.load(Ordering::Acquire));
                }
                Action::ExtraYm2413(instance, command) => {
                    let state = instance.ym2413().expect("YM2413 action target");
                    state.rollback(command.clone());
                    let _ = state.status(self.frame.load(Ordering::Acquire));
                }
                _ => {}
            }
        }
    }
    fn create_extra_psg(
        &self,
        instance: u64,
        clock: Option<u32>,
    ) -> Result<master_system::GameGearStatus, String> {
        let mut sequence = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        let next_sequence = sequence
            .checked_add(1)
            .ok_or("Native event sequence has expired")?;
        self.pending
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1).filter(|value| *value <= CAPACITY)
            })
            .map_err(|_| "Native command queue is full")?;
        let epoch = self.epoch.load(Ordering::Acquire);
        let target = match self.extra_psg.create(instance, epoch, self.rate, clock) {
            Ok(target) => target,
            Err(error) => {
                self.pending.fetch_sub(1, Ordering::AcqRel);
                return Err(error);
            }
        };
        let status = target
            .psg()?
            .game_gear_status(self.frame.load(Ordering::Acquire));
        self.queue
            .push(Event {
                frame: 0,
                sequence: *sequence,
                epoch,
                owner: None,
                action: RefCell::new(Action::ExtraPsgInstall(target)),
            })
            .unwrap_or_else(|_| unreachable!("Reserved additional PSG install capacity"));
        *sequence = next_sequence;
        status
    }
    fn create_extra_ym2612(
        &self,
        instance: u64,
        clock: Option<u32>,
    ) -> Result<ym2612::Status, String> {
        let mut sequence = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        let next_sequence = sequence
            .checked_add(1)
            .ok_or("Native event sequence has expired")?;
        self.pending
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1).filter(|value| *value <= CAPACITY)
            })
            .map_err(|_| "Native command queue is full")?;
        let epoch = self.epoch.load(Ordering::Acquire);
        let target = match self
            .extra_psg
            .create_ym2612(instance, epoch, self.rate, clock)
        {
            Ok(target) => target,
            Err(error) => {
                self.pending.fetch_sub(1, Ordering::AcqRel);
                return Err(error);
            }
        };
        let status = target.ym2612()?.status(self.frame.load(Ordering::Acquire));
        self.queue
            .push(Event {
                frame: 0,
                sequence: *sequence,
                epoch,
                owner: None,
                action: RefCell::new(Action::ExtraPsgInstall(target)),
            })
            .unwrap_or_else(|_| unreachable!("Reserved additional YM2612 install capacity"));
        *sequence = next_sequence;
        status
    }
    fn create_extra_ym2203(
        &self,
        instance: u64,
        clock: Option<u32>,
    ) -> Result<ym2203::AdditionalStatus, String> {
        let mut sequence = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        let next_sequence = sequence
            .checked_add(1)
            .ok_or("Native event sequence has expired")?;
        self.pending
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1).filter(|value| *value <= CAPACITY)
            })
            .map_err(|_| "Native command queue is full")?;
        let epoch = self.epoch.load(Ordering::Acquire);
        let target = match self
            .extra_psg
            .create_ym2203(instance, epoch, self.rate, clock)
        {
            Ok(target) => target,
            Err(error) => {
                self.pending.fetch_sub(1, Ordering::AcqRel);
                return Err(error);
            }
        };
        let status = target
            .ym2203()?
            .additional_status(self.frame.load(Ordering::Acquire));
        self.queue
            .push(Event {
                frame: 0,
                sequence: *sequence,
                epoch,
                owner: None,
                action: RefCell::new(Action::ExtraPsgInstall(target)),
            })
            .unwrap_or_else(|_| unreachable!("Reserved additional YM2203 install capacity"));
        *sequence = next_sequence;
        status
    }
    fn create_extra_ym2608(
        &self,
        instance: u64,
        clock: Option<u32>,
    ) -> Result<extra_ym2608::AdditionalStatus, String> {
        let mut sequence = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        let next_sequence = sequence
            .checked_add(1)
            .ok_or("Native event sequence has expired")?;
        self.pending
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1).filter(|value| *value <= CAPACITY)
            })
            .map_err(|_| "Native command queue is full")?;
        let epoch = self.epoch.load(Ordering::Acquire);
        let target = match self
            .extra_psg
            .create_ym2608(instance, epoch, self.rate, clock)
        {
            Ok(target) => target,
            Err(error) => {
                self.pending.fetch_sub(1, Ordering::AcqRel);
                return Err(error);
            }
        };
        let status = target
            .ym2608()?
            .additional_status(self.frame.load(Ordering::Acquire));
        self.queue
            .push(Event {
                frame: 0,
                sequence: *sequence,
                epoch,
                owner: None,
                action: RefCell::new(Action::ExtraPsgInstall(target)),
            })
            .unwrap_or_else(|_| unreachable!("Reserved additional YM2608 install capacity"));
        *sequence = next_sequence;
        status
    }
    fn create_extra_ym2610(
        &self,
        instance: u64,
        clock: Option<u32>,
    ) -> Result<extra_ym2610::AdditionalStatus, String> {
        let mut sequence = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        let next_sequence = sequence
            .checked_add(1)
            .ok_or("Native event sequence has expired")?;
        self.pending
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1).filter(|value| *value <= CAPACITY)
            })
            .map_err(|_| "Native command queue is full")?;
        let epoch = self.epoch.load(Ordering::Acquire);
        let target = match self
            .extra_psg
            .create_ym2610(instance, epoch, self.rate, clock)
        {
            Ok(target) => target,
            Err(error) => {
                self.pending.fetch_sub(1, Ordering::AcqRel);
                return Err(error);
            }
        };
        let status = target
            .ym2610()?
            .additional_status(self.frame.load(Ordering::Acquire));
        self.queue
            .push(Event {
                frame: 0,
                sequence: *sequence,
                epoch,
                owner: None,
                action: RefCell::new(Action::ExtraPsgInstall(target)),
            })
            .unwrap_or_else(|_| unreachable!("Reserved additional YM2610 install capacity"));
        *sequence = next_sequence;
        status
    }
    fn create_extra_ym2151(
        &self,
        instance: u64,
        clock: Option<u32>,
    ) -> Result<extra_ym2151::AdditionalStatus, String> {
        let mut sequence = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        let next_sequence = sequence
            .checked_add(1)
            .ok_or("Native event sequence has expired")?;
        self.pending
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1).filter(|value| *value <= CAPACITY)
            })
            .map_err(|_| "Native command queue is full")?;
        let epoch = self.epoch.load(Ordering::Acquire);
        let target = match self
            .extra_psg
            .create_ym2151(instance, epoch, self.rate, clock)
        {
            Ok(target) => target,
            Err(error) => {
                self.pending.fetch_sub(1, Ordering::AcqRel);
                return Err(error);
            }
        };
        let status = target
            .ym2151()?
            .additional_status(self.frame.load(Ordering::Acquire));
        self.queue
            .push(Event {
                frame: 0,
                sequence: *sequence,
                epoch,
                owner: None,
                action: RefCell::new(Action::ExtraPsgInstall(target)),
            })
            .unwrap_or_else(|_| unreachable!("Reserved additional YM2151 install capacity"));
        *sequence = next_sequence;
        status
    }
    fn create_extra_ym3526(
        &self,
        instance: u64,
        clock: Option<u32>,
    ) -> Result<extra_opl::Status, String> {
        let mut sequence = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        let next_sequence = sequence
            .checked_add(1)
            .ok_or("Native event sequence has expired")?;
        self.pending
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1).filter(|value| *value <= CAPACITY)
            })
            .map_err(|_| "Native command queue is full")?;
        let epoch = self.epoch.load(Ordering::Acquire);
        let target = match self
            .extra_psg
            .create_ym3526(instance, epoch, self.rate, clock)
        {
            Ok(target) => target,
            Err(error) => {
                self.pending.fetch_sub(1, Ordering::AcqRel);
                return Err(error);
            }
        };
        let status = target.ym3526()?.status(self.frame.load(Ordering::Acquire));
        self.queue
            .push(Event {
                frame: 0,
                sequence: *sequence,
                epoch,
                owner: None,
                action: RefCell::new(Action::ExtraPsgInstall(target)),
            })
            .unwrap_or_else(|_| unreachable!("Reserved additional YM3526 install capacity"));
        *sequence = next_sequence;
        status
    }
    fn create_extra_ym3812(
        &self,
        instance: u64,
        clock: Option<u32>,
    ) -> Result<extra_opl::Status, String> {
        let mut sequence = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        let next_sequence = sequence
            .checked_add(1)
            .ok_or("Native event sequence has expired")?;
        self.pending
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1).filter(|value| *value <= CAPACITY)
            })
            .map_err(|_| "Native command queue is full")?;
        let epoch = self.epoch.load(Ordering::Acquire);
        let target = match self
            .extra_psg
            .create_ym3812(instance, epoch, self.rate, clock)
        {
            Ok(target) => target,
            Err(error) => {
                self.pending.fetch_sub(1, Ordering::AcqRel);
                return Err(error);
            }
        };
        let status = target.ym3812()?.status(self.frame.load(Ordering::Acquire));
        self.queue
            .push(Event {
                frame: 0,
                sequence: *sequence,
                epoch,
                owner: None,
                action: RefCell::new(Action::ExtraPsgInstall(target)),
            })
            .unwrap_or_else(|_| unreachable!("Reserved additional YM3812 install capacity"));
        *sequence = next_sequence;
        status
    }
    fn create_extra_ymf262(
        &self,
        instance: u64,
        clock: Option<u32>,
    ) -> Result<extra_opl::Status, String> {
        let mut sequence = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        let next_sequence = sequence
            .checked_add(1)
            .ok_or("Native event sequence has expired")?;
        self.pending
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1).filter(|value| *value <= CAPACITY)
            })
            .map_err(|_| "Native command queue is full")?;
        let epoch = self.epoch.load(Ordering::Acquire);
        let target = match self
            .extra_psg
            .create_ymf262(instance, epoch, self.rate, clock)
        {
            Ok(target) => target,
            Err(error) => {
                self.pending.fetch_sub(1, Ordering::AcqRel);
                return Err(error);
            }
        };
        let status = target.ymf262()?.status(self.frame.load(Ordering::Acquire));
        self.queue
            .push(Event {
                frame: 0,
                sequence: *sequence,
                epoch,
                owner: None,
                action: RefCell::new(Action::ExtraPsgInstall(target)),
            })
            .unwrap_or_else(|_| unreachable!("Reserved additional YMF262 install capacity"));
        *sequence = next_sequence;
        status
    }
    fn create_extra_rf5c164(
        &self,
        instance: u64,
        clock: Option<u32>,
    ) -> Result<extra_rf5c164::Status, String> {
        let mut sequence = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        let next_sequence = sequence
            .checked_add(1)
            .ok_or("Native event sequence has expired")?;
        self.pending
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1).filter(|value| *value <= CAPACITY)
            })
            .map_err(|_| "Native command queue is full")?;
        let epoch = self.epoch.load(Ordering::Acquire);
        let target = match self
            .extra_psg
            .create_rf5c164(instance, epoch, self.rate, clock)
        {
            Ok(target) => target,
            Err(error) => {
                self.pending.fetch_sub(1, Ordering::AcqRel);
                return Err(error);
            }
        };
        let status = target.rf5c164()?.status(self.frame.load(Ordering::Acquire));
        self.queue
            .push(Event {
                frame: 0,
                sequence: *sequence,
                epoch,
                owner: None,
                action: RefCell::new(Action::ExtraPsgInstall(target)),
            })
            .unwrap_or_else(|_| unreachable!("Reserved additional RF5C164 install capacity"));
        *sequence = next_sequence;
        status
    }
    fn create_extra_okim6258(
        &self,
        instance: u64,
        clock: Option<u32>,
    ) -> Result<extra_okim6258::Status, String> {
        let mut sequence = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        let next_sequence = sequence
            .checked_add(1)
            .ok_or("Native event sequence has expired")?;
        self.pending
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1).filter(|value| *value <= CAPACITY)
            })
            .map_err(|_| "Native command queue is full")?;
        let epoch = self.epoch.load(Ordering::Acquire);
        let target = match self
            .extra_psg
            .create_okim6258(instance, epoch, self.rate, clock)
        {
            Ok(target) => target,
            Err(error) => {
                self.pending.fetch_sub(1, Ordering::AcqRel);
                return Err(error);
            }
        };
        let status = target
            .okim6258()?
            .status(self.frame.load(Ordering::Acquire));
        self.queue
            .push(Event {
                frame: 0,
                sequence: *sequence,
                epoch,
                owner: None,
                action: RefCell::new(Action::ExtraPsgInstall(target)),
            })
            .unwrap_or_else(|_| unreachable!("Reserved additional OKIM6258 install capacity"));
        *sequence = next_sequence;
        status
    }
    fn create_extra_huc6280(
        &self,
        instance: u64,
        clock: Option<u32>,
    ) -> Result<extra_huc6280::Status, String> {
        let mut sequence = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        let next_sequence = sequence
            .checked_add(1)
            .ok_or("Native event sequence has expired")?;
        self.pending
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1).filter(|value| *value <= CAPACITY)
            })
            .map_err(|_| "Native command queue is full")?;
        let epoch = self.epoch.load(Ordering::Acquire);
        let target = match self
            .extra_psg
            .create_huc6280(instance, epoch, self.rate, clock)
        {
            Ok(target) => target,
            Err(error) => {
                self.pending.fetch_sub(1, Ordering::AcqRel);
                return Err(error);
            }
        };
        let status = target.huc6280()?.status(self.frame.load(Ordering::Acquire));
        self.queue
            .push(Event {
                frame: 0,
                sequence: *sequence,
                epoch,
                owner: None,
                action: RefCell::new(Action::ExtraPsgInstall(target)),
            })
            .unwrap_or_else(|_| unreachable!("Reserved additional HuC6280 install capacity"));
        *sequence = next_sequence;
        status
    }
    fn create_extra_dx7(&self, instance: u64) -> Result<extra_dx7::Status, String> {
        let mut sequence = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        let next_sequence = sequence
            .checked_add(1)
            .ok_or("Native event sequence has expired")?;
        self.pending
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1).filter(|value| *value <= CAPACITY)
            })
            .map_err(|_| "Native command queue is full")?;
        let epoch = self.epoch.load(Ordering::Acquire);
        let target = match self.extra_psg.create_dx7(instance, epoch, self.rate) {
            Ok(target) => target,
            Err(error) => {
                self.pending.fetch_sub(1, Ordering::AcqRel);
                return Err(error);
            }
        };
        let status = target.dx7()?.status(self.frame.load(Ordering::Acquire));
        self.queue
            .push(Event {
                frame: 0,
                sequence: *sequence,
                epoch,
                owner: None,
                action: RefCell::new(Action::ExtraPsgInstall(target)),
            })
            .unwrap_or_else(|_| unreachable!("Reserved additional DX7 install capacity"));
        *sequence = next_sequence;
        status
    }
    fn create_extra_pwm(&self, instance: u64) -> Result<extra_pwm::Status, String> {
        let mut sequence = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        let next_sequence = sequence
            .checked_add(1)
            .ok_or("Native event sequence has expired")?;
        self.pending
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1).filter(|value| *value <= CAPACITY)
            })
            .map_err(|_| "Native command queue is full")?;
        let epoch = self.epoch.load(Ordering::Acquire);
        let target = match self.extra_psg.create_pwm(instance, epoch, self.rate) {
            Ok(target) => target,
            Err(error) => {
                self.pending.fetch_sub(1, Ordering::AcqRel);
                return Err(error);
            }
        };
        let status = target.pwm()?.status(self.frame.load(Ordering::Acquire));
        self.queue
            .push(Event {
                frame: 0,
                sequence: *sequence,
                epoch,
                owner: None,
                action: RefCell::new(Action::ExtraPsgInstall(target)),
            })
            .unwrap_or_else(|_| unreachable!("Reserved additional PWM install capacity"));
        *sequence = next_sequence;
        status
    }
    fn create_extra_gameboy(
        &self,
        instance: u64,
        clock: Option<u32>,
    ) -> Result<extra_gameboy::Status, String> {
        let mut sequence = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        let next_sequence = sequence
            .checked_add(1)
            .ok_or("Native event sequence has expired")?;
        self.pending
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1).filter(|value| *value <= CAPACITY)
            })
            .map_err(|_| "Native command queue is full")?;
        let epoch = self.epoch.load(Ordering::Acquire);
        let target = match self
            .extra_psg
            .create_gameboy(instance, epoch, self.rate, clock)
        {
            Ok(target) => target,
            Err(error) => {
                self.pending.fetch_sub(1, Ordering::AcqRel);
                return Err(error);
            }
        };
        let status = target.gameboy()?.status(self.frame.load(Ordering::Acquire));
        self.queue
            .push(Event {
                frame: 0,
                sequence: *sequence,
                epoch,
                owner: None,
                action: RefCell::new(Action::ExtraPsgInstall(target)),
            })
            .unwrap_or_else(|_| unreachable!("Reserved additional Game Boy install capacity"));
        *sequence = next_sequence;
        status
    }
    fn create_extra_nes(
        &self,
        instance: u64,
        clock: Option<f64>,
        fds: bool,
    ) -> Result<extra_nes::Status, String> {
        let mut sequence = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        let next_sequence = sequence
            .checked_add(1)
            .ok_or("Native event sequence has expired")?;
        self.pending
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1).filter(|value| *value <= CAPACITY)
            })
            .map_err(|_| "Native command queue is full")?;
        let epoch = self.epoch.load(Ordering::Acquire);
        let target = match self
            .extra_psg
            .create_nes(instance, epoch, self.rate, clock, fds)
        {
            Ok(target) => target,
            Err(error) => {
                self.pending.fetch_sub(1, Ordering::AcqRel);
                return Err(error);
            }
        };
        let status = target.nes()?.status(self.frame.load(Ordering::Acquire));
        self.queue
            .push(Event {
                frame: 0,
                sequence: *sequence,
                epoch,
                owner: None,
                action: RefCell::new(Action::ExtraPsgInstall(target)),
            })
            .unwrap_or_else(|_| unreachable!("Reserved additional NES install capacity"));
        *sequence = next_sequence;
        status
    }
    fn create_extra_y8950(
        &self,
        instance: u64,
        clock: Option<u32>,
    ) -> Result<extra_opl::Status, String> {
        let mut sequence = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        let next_sequence = sequence
            .checked_add(1)
            .ok_or("Native event sequence has expired")?;
        self.pending
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1).filter(|value| *value <= CAPACITY)
            })
            .map_err(|_| "Native command queue is full")?;
        let epoch = self.epoch.load(Ordering::Acquire);
        let target = match self
            .extra_psg
            .create_y8950(instance, epoch, self.rate, clock)
        {
            Ok(target) => target,
            Err(error) => {
                self.pending.fetch_sub(1, Ordering::AcqRel);
                return Err(error);
            }
        };
        let status = target.y8950()?.status(self.frame.load(Ordering::Acquire));
        self.queue
            .push(Event {
                frame: 0,
                sequence: *sequence,
                epoch,
                owner: None,
                action: RefCell::new(Action::ExtraPsgInstall(target)),
            })
            .unwrap_or_else(|_| unreachable!("Reserved additional Y8950 install capacity"));
        *sequence = next_sequence;
        status
    }
    fn create_extra_ay8910(
        &self,
        instance: u64,
        clock: Option<u32>,
    ) -> Result<extra_msx::Ay8910Status, String> {
        let mut sequence = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        let next_sequence = sequence
            .checked_add(1)
            .ok_or("Native event sequence has expired")?;
        self.pending
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1).filter(|value| *value <= CAPACITY)
            })
            .map_err(|_| "Native command queue is full")?;
        let epoch = self.epoch.load(Ordering::Acquire);
        let target = match self
            .extra_psg
            .create_ay8910(instance, epoch, self.rate, clock)
        {
            Ok(target) => target,
            Err(error) => {
                self.pending.fetch_sub(1, Ordering::AcqRel);
                return Err(error);
            }
        };
        let status = target
            .ay8910()?
            .ay8910_status(self.frame.load(Ordering::Acquire));
        self.queue
            .push(Event {
                frame: 0,
                sequence: *sequence,
                epoch,
                owner: None,
                action: RefCell::new(Action::ExtraPsgInstall(target)),
            })
            .unwrap_or_else(|_| unreachable!("Reserved additional AY8910 install capacity"));
        *sequence = next_sequence;
        status
    }
    fn create_extra_ym2413(
        &self,
        instance: u64,
        clock: Option<u32>,
    ) -> Result<extra_msx::Ym2413Status, String> {
        let mut sequence = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        let next_sequence = sequence
            .checked_add(1)
            .ok_or("Native event sequence has expired")?;
        self.pending
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1).filter(|value| *value <= CAPACITY)
            })
            .map_err(|_| "Native command queue is full")?;
        let epoch = self.epoch.load(Ordering::Acquire);
        let target = match self
            .extra_psg
            .create_ym2413(instance, epoch, self.rate, clock)
        {
            Ok(target) => target,
            Err(error) => {
                self.pending.fetch_sub(1, Ordering::AcqRel);
                return Err(error);
            }
        };
        let status = target
            .ym2413()?
            .ym2413_status(self.frame.load(Ordering::Acquire));
        self.queue
            .push(Event {
                frame: 0,
                sequence: *sequence,
                epoch,
                owner: None,
                action: RefCell::new(Action::ExtraPsgInstall(target)),
            })
            .unwrap_or_else(|_| unreachable!("Reserved additional YM2413 install capacity"));
        *sequence = next_sequence;
        status
    }
    fn dispose_extra_psg(&self, instance: u64) -> Result<(), String> {
        let _producer = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        self.extra_psg.dispose(instance)?;
        self.invalidation.fetch_add(1, Ordering::Release);
        Ok(())
    }
    fn extra_psg_status(
        &self,
        instance: u64,
        after: Option<u64>,
    ) -> Result<master_system::GameGearStatus, String> {
        let target = self
            .extra_psg
            .get(instance, self.epoch.load(Ordering::Acquire))?;
        if let Some(id) = after {
            target.psg()?.wait_applied(id).map_err(extra_psg::error)?;
        }
        self.check_session(self.session)?;
        if !target.valid(self.epoch.load(Ordering::Acquire)) {
            return Err("Additional PSG instance has expired".into());
        }
        target
            .psg()?
            .game_gear_status(self.frame.load(Ordering::Acquire))
    }
    fn dispose_extra_ym2612(&self, instance: u64) -> Result<(), String> {
        let _producer = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        self.extra_psg.dispose_ym2612(instance)?;
        self.invalidation.fetch_add(1, Ordering::Release);
        Ok(())
    }
    fn extra_ym2612_status(
        &self,
        instance: u64,
        after: Option<u64>,
    ) -> Result<ym2612::Status, String> {
        let target = self
            .extra_psg
            .get(instance, self.epoch.load(Ordering::Acquire))?;
        if let Some(id) = after {
            target.ym2612()?.wait_applied(id)?;
        }
        self.check_session(self.session)?;
        if !target.valid(self.epoch.load(Ordering::Acquire)) {
            return Err("Additional YM2612 instance has expired".into());
        }
        target.ym2612()?.status(self.frame.load(Ordering::Acquire))
    }
    fn dispose_extra_ym2203(&self, instance: u64) -> Result<(), String> {
        let _producer = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        self.extra_psg.dispose_ym2203(instance)?;
        self.invalidation.fetch_add(1, Ordering::Release);
        Ok(())
    }
    fn dispose_extra_ym2608(&self, instance: u64) -> Result<(), String> {
        let _producer = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        self.extra_psg.dispose_ym2608(instance)?;
        self.invalidation.fetch_add(1, Ordering::Release);
        Ok(())
    }
    fn dispose_extra_ym2610(&self, instance: u64) -> Result<(), String> {
        let _producer = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        self.extra_psg.dispose_ym2610(instance)?;
        self.invalidation.fetch_add(1, Ordering::Release);
        Ok(())
    }
    fn dispose_extra_ym2151(&self, instance: u64) -> Result<(), String> {
        let _producer = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        self.extra_psg.dispose_ym2151(instance)?;
        self.invalidation.fetch_add(1, Ordering::Release);
        Ok(())
    }
    fn dispose_extra_ym3526(&self, instance: u64) -> Result<(), String> {
        let _producer = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        self.extra_psg.dispose_ym3526(instance)?;
        self.invalidation.fetch_add(1, Ordering::Release);
        Ok(())
    }
    fn dispose_extra_ym3812(&self, instance: u64) -> Result<(), String> {
        let _producer = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        self.extra_psg.dispose_ym3812(instance)?;
        self.invalidation.fetch_add(1, Ordering::Release);
        Ok(())
    }
    fn dispose_extra_ymf262(&self, instance: u64) -> Result<(), String> {
        let _producer = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        self.extra_psg.dispose_ymf262(instance)?;
        self.invalidation.fetch_add(1, Ordering::Release);
        Ok(())
    }
    fn dispose_extra_rf5c164(&self, instance: u64) -> Result<(), String> {
        let _producer = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        self.extra_psg.dispose_rf5c164(instance)?;
        self.invalidation.fetch_add(1, Ordering::Release);
        Ok(())
    }
    fn dispose_extra_okim6258(&self, instance: u64) -> Result<(), String> {
        let _producer = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        self.extra_psg.dispose_okim6258(instance)?;
        self.invalidation.fetch_add(1, Ordering::Release);
        Ok(())
    }
    fn dispose_extra_huc6280(&self, instance: u64) -> Result<(), String> {
        let _producer = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        self.extra_psg.dispose_huc6280(instance)?;
        self.invalidation.fetch_add(1, Ordering::Release);
        Ok(())
    }
    fn dispose_extra_dx7(&self, instance: u64) -> Result<(), String> {
        let _producer = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        self.extra_psg.dispose_dx7(instance)?;
        self.invalidation.fetch_add(1, Ordering::Release);
        Ok(())
    }
    fn dispose_extra_pwm(&self, instance: u64) -> Result<(), String> {
        let _producer = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        self.extra_psg.dispose_pwm(instance)?;
        self.invalidation.fetch_add(1, Ordering::Release);
        Ok(())
    }
    fn dispose_extra_gameboy(&self, instance: u64) -> Result<(), String> {
        let _producer = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        self.extra_psg.dispose_gameboy(instance)?;
        self.invalidation.fetch_add(1, Ordering::Release);
        Ok(())
    }
    fn dispose_extra_nes(&self, instance: u64) -> Result<(), String> {
        let _producer = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        self.extra_psg.dispose_nes(instance)?;
        self.invalidation.fetch_add(1, Ordering::Release);
        Ok(())
    }
    fn dispose_extra_y8950(&self, instance: u64) -> Result<(), String> {
        let _producer = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        self.extra_psg.dispose_y8950(instance)?;
        self.invalidation.fetch_add(1, Ordering::Release);
        Ok(())
    }
    fn dispose_extra_ay8910(&self, instance: u64) -> Result<(), String> {
        let _producer = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        self.extra_psg.dispose_ay8910(instance)?;
        self.invalidation.fetch_add(1, Ordering::Release);
        Ok(())
    }
    fn dispose_extra_ym2413(&self, instance: u64) -> Result<(), String> {
        let _producer = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        self.extra_psg.dispose_ym2413(instance)?;
        self.invalidation.fetch_add(1, Ordering::Release);
        Ok(())
    }
    fn extra_ym2203_status(
        &self,
        instance: u64,
        after: Option<u64>,
    ) -> Result<ym2203::AdditionalStatus, String> {
        let target = self
            .extra_psg
            .get(instance, self.epoch.load(Ordering::Acquire))?;
        if let Some(id) = after {
            target.ym2203()?.wait_applied(id)?;
        }
        self.check_session(self.session)?;
        if !target.valid(self.epoch.load(Ordering::Acquire)) {
            return Err("Additional YM2203 instance has expired".into());
        }
        target
            .ym2203()?
            .additional_status(self.frame.load(Ordering::Acquire))
    }
    fn extra_ym2608_status(
        &self,
        instance: u64,
        after: Option<u64>,
    ) -> Result<extra_ym2608::AdditionalStatus, String> {
        let target = self
            .extra_psg
            .get(instance, self.epoch.load(Ordering::Acquire))?;
        if let Some(id) = after {
            target.ym2608()?.wait_applied(id)?;
        }
        self.check_session(self.session)?;
        if !target.valid(self.epoch.load(Ordering::Acquire)) {
            return Err("Additional YM2608 instance has expired".into());
        }
        target
            .ym2608()?
            .additional_status(self.frame.load(Ordering::Acquire))
    }
    fn extra_ym2610_status(
        &self,
        instance: u64,
        after: Option<u64>,
    ) -> Result<extra_ym2610::AdditionalStatus, String> {
        let target = self
            .extra_psg
            .get(instance, self.epoch.load(Ordering::Acquire))?;
        if let Some(id) = after {
            target.ym2610()?.wait_applied(id)?;
        }
        self.check_session(self.session)?;
        if !target.valid(self.epoch.load(Ordering::Acquire)) {
            return Err("Additional YM2610 instance has expired".into());
        }
        target
            .ym2610()?
            .additional_status(self.frame.load(Ordering::Acquire))
    }
    fn extra_ym2151_status(
        &self,
        instance: u64,
        after: Option<u64>,
    ) -> Result<extra_ym2151::AdditionalStatus, String> {
        let target = self
            .extra_psg
            .get(instance, self.epoch.load(Ordering::Acquire))?;
        if let Some(id) = after {
            target.ym2151()?.wait_applied(id)?;
        }
        self.check_session(self.session)?;
        if !target.valid(self.epoch.load(Ordering::Acquire)) {
            return Err("Additional YM2151 instance has expired".into());
        }
        target
            .ym2151()?
            .additional_status(self.frame.load(Ordering::Acquire))
    }
    fn extra_ym3526_status(
        &self,
        instance: u64,
        after: Option<u64>,
        after_layout_revision: Option<u64>,
    ) -> Result<extra_opl::Status, String> {
        let target = self
            .extra_psg
            .get(instance, self.epoch.load(Ordering::Acquire))?;
        if let Some(id) = after {
            target.ym3526()?.wait_applied(id)?;
        }
        if let Some(revision) = after_layout_revision {
            target.ym3526()?.wait_layout(revision)?;
        }
        self.check_session(self.session)?;
        if !target.valid(self.epoch.load(Ordering::Acquire)) {
            return Err("Additional YM3526 instance has expired".into());
        }
        target.ym3526()?.status(self.frame.load(Ordering::Acquire))
    }
    fn extra_ym3812_status(
        &self,
        instance: u64,
        after: Option<u64>,
        after_layout_revision: Option<u64>,
    ) -> Result<extra_opl::Status, String> {
        let target = self
            .extra_psg
            .get(instance, self.epoch.load(Ordering::Acquire))?;
        if let Some(id) = after {
            target.ym3812()?.wait_applied(id)?;
        }
        if let Some(revision) = after_layout_revision {
            target.ym3812()?.wait_layout(revision)?;
        }
        self.check_session(self.session)?;
        if !target.valid(self.epoch.load(Ordering::Acquire)) {
            return Err("Additional YM3812 instance has expired".into());
        }
        target.ym3812()?.status(self.frame.load(Ordering::Acquire))
    }
    fn extra_ymf262_status(
        &self,
        instance: u64,
        after: Option<u64>,
        after_layout_revision: Option<u64>,
    ) -> Result<extra_opl::Status, String> {
        let target = self
            .extra_psg
            .get(instance, self.epoch.load(Ordering::Acquire))?;
        if let Some(id) = after {
            target.ymf262()?.wait_applied(id)?;
        }
        if let Some(revision) = after_layout_revision {
            target.ymf262()?.wait_layout(revision)?;
        }
        self.check_session(self.session)?;
        if !target.valid(self.epoch.load(Ordering::Acquire)) {
            return Err("Additional YMF262 instance has expired".into());
        }
        target.ymf262()?.status(self.frame.load(Ordering::Acquire))
    }
    fn extra_rf5c164_status(
        &self,
        instance: u64,
        after: Option<u64>,
    ) -> Result<extra_rf5c164::Status, String> {
        let target = self
            .extra_psg
            .get(instance, self.epoch.load(Ordering::Acquire))?;
        if let Some(id) = after {
            target.rf5c164()?.wait_applied(id)?;
        }
        self.check_session(self.session)?;
        if !target.valid(self.epoch.load(Ordering::Acquire)) {
            return Err("Additional RF5C164 instance has expired".into());
        }
        target.rf5c164()?.status(self.frame.load(Ordering::Acquire))
    }
    fn extra_okim6258_status(
        &self,
        instance: u64,
        after: Option<u64>,
    ) -> Result<extra_okim6258::Status, String> {
        let target = self
            .extra_psg
            .get(instance, self.epoch.load(Ordering::Acquire))?;
        if let Some(id) = after {
            target.okim6258()?.wait_applied(id)?;
        }
        self.check_session(self.session)?;
        if !target.valid(self.epoch.load(Ordering::Acquire)) {
            return Err("Additional OKIM6258 instance has expired".into());
        }
        target
            .okim6258()?
            .status(self.frame.load(Ordering::Acquire))
    }
    fn extra_huc6280_status(
        &self,
        instance: u64,
        after: Option<u64>,
    ) -> Result<extra_huc6280::Status, String> {
        let target = self
            .extra_psg
            .get(instance, self.epoch.load(Ordering::Acquire))?;
        if let Some(id) = after {
            target.huc6280()?.wait_applied(id)?;
        }
        self.check_session(self.session)?;
        if !target.valid(self.epoch.load(Ordering::Acquire)) {
            return Err("Additional HuC6280 instance has expired".into());
        }
        target.huc6280()?.status(self.frame.load(Ordering::Acquire))
    }
    fn extra_dx7_status(
        &self,
        instance: u64,
        after: Option<u64>,
    ) -> Result<extra_dx7::Status, String> {
        let target = self
            .extra_psg
            .get(instance, self.epoch.load(Ordering::Acquire))?;
        if let Some(id) = after {
            target.dx7()?.wait_applied(id)?;
        }
        self.check_session(self.session)?;
        if !target.valid(self.epoch.load(Ordering::Acquire)) {
            return Err("Additional DX7 instance has expired".into());
        }
        target.dx7()?.status(self.frame.load(Ordering::Acquire))
    }
    fn extra_pwm_status(
        &self,
        instance: u64,
        after: Option<u64>,
    ) -> Result<extra_pwm::Status, String> {
        let target = self
            .extra_psg
            .get(instance, self.epoch.load(Ordering::Acquire))?;
        if let Some(id) = after {
            target.pwm()?.wait_applied(id)?;
        }
        self.check_session(self.session)?;
        if !target.valid(self.epoch.load(Ordering::Acquire)) {
            return Err("Additional PWM instance has expired".into());
        }
        target
            .pwm()?
            .status_after(self.frame.load(Ordering::Acquire), after)
    }
    fn extra_gameboy_status(
        &self,
        instance: u64,
        after: Option<u64>,
    ) -> Result<extra_gameboy::Status, String> {
        let target = self
            .extra_psg
            .get(instance, self.epoch.load(Ordering::Acquire))?;
        if let Some(id) = after {
            target.gameboy()?.wait_applied(id)?;
        }
        self.check_session(self.session)?;
        if !target.valid(self.epoch.load(Ordering::Acquire)) {
            return Err("Additional Game Boy instance has expired".into());
        }
        target.gameboy()?.status(self.frame.load(Ordering::Acquire))
    }
    fn extra_nes_status(
        &self,
        instance: u64,
        after: Option<u64>,
    ) -> Result<extra_nes::Status, String> {
        let target = self
            .extra_psg
            .get(instance, self.epoch.load(Ordering::Acquire))?;
        if let Some(id) = after {
            target.nes()?.wait_applied(id)?;
        }
        self.check_session(self.session)?;
        if !target.valid(self.epoch.load(Ordering::Acquire)) {
            return Err("Additional NES instance has expired".into());
        }
        target.nes()?.status(self.frame.load(Ordering::Acquire))
    }
    fn extra_y8950_status(
        &self,
        instance: u64,
        after: Option<u64>,
        after_layout_revision: Option<u64>,
    ) -> Result<extra_opl::Status, String> {
        let target = self
            .extra_psg
            .get(instance, self.epoch.load(Ordering::Acquire))?;
        if let Some(id) = after {
            target.y8950()?.wait_applied(id)?;
        }
        if let Some(revision) = after_layout_revision {
            target.y8950()?.wait_layout(revision)?;
        }
        self.check_session(self.session)?;
        if !target.valid(self.epoch.load(Ordering::Acquire)) {
            return Err("Additional Y8950 instance has expired".into());
        }
        target.y8950()?.status(self.frame.load(Ordering::Acquire))
    }
    fn extra_ay8910_status(
        &self,
        instance: u64,
        after: Option<u64>,
    ) -> Result<extra_msx::Ay8910Status, String> {
        let target = self
            .extra_psg
            .get(instance, self.epoch.load(Ordering::Acquire))?;
        if let Some(id) = after {
            target.ay8910()?.wait_applied(id)?;
        }
        self.check_session(self.session)?;
        if !target.valid(self.epoch.load(Ordering::Acquire)) {
            return Err("Additional AY8910 instance has expired".into());
        }
        target
            .ay8910()?
            .ay8910_status(self.frame.load(Ordering::Acquire))
    }
    #[cfg(test)]
    fn extra_ym2413_status(
        &self,
        instance: u64,
        after: Option<u64>,
    ) -> Result<extra_msx::Ym2413Status, String> {
        self.extra_ym2413_status_layout(instance, after, None)
    }
    fn extra_ym2413_status_layout(
        &self,
        instance: u64,
        after: Option<u64>,
        after_layout_revision: Option<u64>,
    ) -> Result<extra_msx::Ym2413Status, String> {
        let target = self
            .extra_psg
            .get(instance, self.epoch.load(Ordering::Acquire))?;
        if let Some(id) = after {
            target.ym2413()?.wait_applied(id)?;
        }
        if let Some(revision) = after_layout_revision {
            target.ym2413()?.wait_layout(revision)?;
        }
        self.check_session(self.session)?;
        if !target.valid(self.epoch.load(Ordering::Acquire)) {
            return Err("Additional YM2413 instance has expired".into());
        }
        target
            .ym2413()?
            .ym2413_status(self.frame.load(Ordering::Acquire))
    }
    fn enqueue_special(&self, action: Action) -> Result<(), String> {
        let mut sequence = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        self.pending
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |v| {
                v.checked_add(1).filter(|v| *v <= CAPACITY)
            })
            .map_err(|_| "Native command queue is full")?;
        let event = Event {
            frame: 0,
            sequence: *sequence,
            epoch: self.epoch.load(Ordering::Acquire),
            owner: None,
            action: RefCell::new(action),
        };
        *sequence += 1;
        self.queue
            .push(event)
            .unwrap_or_else(|_| unreachable!("Reserved native event capacity"));
        Ok(())
    }
    fn load_media(
        &self,
        kind: media::Kind,
        name: String,
        rate: f64,
        channels: Vec<Vec<f32>>,
    ) -> Result<Status, String> {
        let _producer = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        let mut store = self
            .media
            .lock()
            .map_err(|_| "Native PCM state is unavailable")?;
        self.reclaim(&mut store);
        store.load(kind, name, rate, channels)?;
        Ok(self.status())
    }
    fn remove_media(&self, kind: media::Kind, name: &str) -> Result<Status, String> {
        let _producer = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        let mut store = self
            .media
            .lock()
            .map_err(|_| "Native PCM state is unavailable")?;
        self.reclaim(&mut store);
        store.remove(kind, name);
        Ok(self.status())
    }
    fn record_start(&self, seconds: f64) -> Result<Status, String> {
        let mut slot = self
            .recording
            .lock()
            .map_err(|_| "Native recording state is unavailable")?;
        if slot.is_some() {
            return Err("Finish the previous native recording before starting another".into());
        }
        let capture = recording::Capture::new(self.rate, seconds)?;
        self.enqueue_special(Action::Record(capture.clone()))?;
        *slot = Some(capture);
        Ok(self.status())
    }
    fn record_stop(&self) -> Result<recording::Recorded, String> {
        let mut slot = self
            .recording
            .lock()
            .map_err(|_| "Native recording state is unavailable")?;
        let capture = slot
            .as_ref()
            .ok_or("No native recording has been started")?;
        capture.stop.store(true, Ordering::Release);
        let start = Instant::now();
        let finished = loop {
            if let Some(finished) = self.recorded.pop() {
                break finished;
            }
            if self.failed.load(Ordering::Acquire) {
                return Err("Native audio stream failed during recording".into());
            }
            if start.elapsed() > Duration::from_secs(2) {
                return Err("Native recording stop timed out".into());
            }
            std::thread::sleep(Duration::from_millis(2));
        };
        let result = finished.result();
        *slot = None;
        result
    }
    fn reclaim(&self, store: &mut media::Store) {
        while let Some(reservation) = self.retired.pop() {
            store.reservations.remove(&reservation.voice);
        }
        while let Some(targets) = self.retired_stops.pop() {
            store.stops.remove(&targets.id);
        }
        store.collect();
    }
    fn cancel_dac_before(&self, owner: Option<Owner>, sequence: u64) {
        if let Some(owner) = owner {
            self.owner_dac_cancel_before[(owner.owner - 1) as usize]
                .fetch_max(sequence, Ordering::AcqRel);
        } else {
            self.dac_cancel_before.fetch_max(sequence, Ordering::AcqRel);
        }
        self.invalidation.fetch_add(1, Ordering::Release);
    }
    fn cancel_nes_before(&self, owner: Option<Owner>, sequence: u64) {
        if let Some(owner) = owner {
            self.owner_nes_cancel_before[(owner.owner - 1) as usize]
                .fetch_max(sequence, Ordering::AcqRel);
        } else {
            self.nes_cancel_before.fetch_max(sequence, Ordering::AcqRel);
        }
        self.invalidation.fetch_add(1, Ordering::Release);
    }
    fn valid_owner(&self, owner: Option<Owner>) -> bool {
        owner.is_none_or(|owner| {
            self.active_performance.load(Ordering::Acquire) == owner.performance
                && !self.cancelled_owners[(owner.owner - 1) as usize].load(Ordering::Acquire)
        })
    }
    fn valid_event(&self, event: &Event) -> bool {
        let action = event.action.borrow();
        self.valid_owner(event.owner)
            && match &*action {
                Action::ExtraPsgInstall(instance) => {
                    instance.valid(self.epoch.load(Ordering::Acquire))
                }
                Action::ExtraPsg(instance, command) => {
                    instance.valid(self.epoch.load(Ordering::Acquire))
                        && instance.psg().expect("PSG action target").valid_event(
                            command,
                            event.sequence,
                            event.owner,
                        )
                }
                Action::ExtraYm2612(instance, command) => {
                    instance.valid(self.epoch.load(Ordering::Acquire))
                        && instance
                            .ym2612()
                            .expect("YM2612 action target")
                            .valid_event(command, event.sequence, event.owner)
                }
                Action::ExtraYm2203(instance, command) => {
                    instance.valid(self.epoch.load(Ordering::Acquire))
                        && instance
                            .ym2203()
                            .expect("YM2203 action target")
                            .valid_event(command, event.sequence, event.owner)
                }
                Action::ExtraYm2608(instance, command) => {
                    instance.valid(self.epoch.load(Ordering::Acquire))
                        && instance
                            .ym2608()
                            .expect("YM2608 action target")
                            .valid_event(command, event.sequence, event.owner)
                }
                Action::ExtraYm2610(instance, command) => {
                    instance.valid(self.epoch.load(Ordering::Acquire))
                        && instance
                            .ym2610()
                            .expect("YM2610 action target")
                            .valid_event(command, event.sequence, event.owner)
                }
                Action::ExtraYm2151(instance, command) => {
                    instance.valid(self.epoch.load(Ordering::Acquire))
                        && instance
                            .ym2151()
                            .expect("YM2151 action target")
                            .valid_event(command, event.sequence, event.owner)
                }
                Action::ExtraYm3526(instance, command) => {
                    instance.valid(self.epoch.load(Ordering::Acquire))
                        && instance
                            .ym3526()
                            .expect("YM3526 action target")
                            .valid_event(command, event.sequence, event.owner)
                }
                Action::ExtraYm3812(instance, command) => {
                    instance.valid(self.epoch.load(Ordering::Acquire))
                        && instance
                            .ym3812()
                            .expect("YM3812 action target")
                            .valid_event(command, event.sequence, event.owner)
                }
                Action::ExtraYmf262(instance, command) => {
                    instance.valid(self.epoch.load(Ordering::Acquire))
                        && instance
                            .ymf262()
                            .expect("YMF262 action target")
                            .valid_event(command, event.sequence, event.owner)
                }
                Action::ExtraRf5c164(instance, command) => {
                    instance.valid(self.epoch.load(Ordering::Acquire))
                        && instance
                            .rf5c164()
                            .expect("RF5C164 action target")
                            .valid_event(command, event.sequence, event.owner)
                }
                Action::ExtraOkim6258(instance, command) => {
                    instance.valid(self.epoch.load(Ordering::Acquire))
                        && instance
                            .okim6258()
                            .expect("OKIM6258 action target")
                            .valid_event(command, event.sequence, event.owner)
                }
                Action::ExtraHuc6280(instance, command) => {
                    instance.valid(self.epoch.load(Ordering::Acquire))
                        && instance
                            .huc6280()
                            .expect("HuC6280 action target")
                            .valid_event(command, event.sequence, event.owner)
                }
                Action::ExtraDx7(instance, command) => {
                    instance.valid(self.epoch.load(Ordering::Acquire))
                        && instance.dx7().expect("DX7 action target").valid_event(
                            command,
                            event.sequence,
                            event.owner,
                        )
                }
                Action::ExtraPwm(instance, command) => {
                    instance.valid(self.epoch.load(Ordering::Acquire))
                        && instance.pwm().expect("PWM action target").valid_event(
                            command,
                            event.sequence,
                            event.owner,
                        )
                }
                Action::ExtraGameboy(instance, command) => {
                    instance.valid(self.epoch.load(Ordering::Acquire))
                        && instance
                            .gameboy()
                            .expect("Game Boy action target")
                            .valid_event(command, event.sequence, event.owner)
                }
                Action::ExtraNes(instance, command) => {
                    instance.valid(self.epoch.load(Ordering::Acquire))
                        && instance.nes().expect("NES action target").valid_event(
                            command,
                            event.sequence,
                            event.owner,
                        )
                }
                Action::ExtraY8950(instance, command) => {
                    instance.valid(self.epoch.load(Ordering::Acquire))
                        && instance.y8950().expect("Y8950 action target").valid_event(
                            command,
                            event.sequence,
                            event.owner,
                        )
                }
                Action::ExtraAy8910(instance, command) => {
                    instance.valid(self.epoch.load(Ordering::Acquire))
                        && instance
                            .ay8910()
                            .expect("AY8910 action target")
                            .valid_event(command, event.sequence, event.owner)
                }
                Action::ExtraYm2413(instance, command) => {
                    instance.valid(self.epoch.load(Ordering::Acquire))
                        && instance
                            .ym2413()
                            .expect("YM2413 action target")
                            .valid_event(command, event.sequence, event.owner)
                }
                _ => true,
            }
            && event.epoch == self.epoch.load(Ordering::Acquire)
            && (!matches!(
                *action,
                Action::Nes(_)
                    | Action::Gameboy(_)
                    | Action::Snes(_)
                    | Action::Huc6280(_)
                    | Action::Ym2608(_)
                    | Action::Ym2610b(_)
                    | Action::Pwm(_)
            ) || event.sequence >= self.nes_cancel_before.load(Ordering::Acquire)
                && event.owner.is_none_or(|owner| {
                    event.sequence
                        >= self.owner_nes_cancel_before[(owner.owner - 1) as usize]
                            .load(Ordering::Acquire)
                }))
            && (if let Action::X68000(command) = &*action {
                self.x68000
                    .as_ref()
                    .is_some_and(|chip| chip.valid_event(command, event.sequence, event.owner))
            } else {
                true
            })
            && (if let Action::MasterSystem(command) = &*action {
                self.master_system
                    .as_ref()
                    .is_some_and(|chip| chip.valid_event(command, event.sequence, event.owner))
            } else {
                true
            })
            && (if let Action::Msx(command) = &*action {
                self.msx
                    .as_ref()
                    .is_some_and(|chip| chip.valid_event(command, event.sequence, event.owner))
            } else {
                true
            })
            && (if let Action::Opl(command) = &*action {
                self.opl
                    .as_ref()
                    .is_some_and(|chip| chip.valid_event(command, event.sequence, event.owner))
            } else {
                true
            })
            && (if let Action::Ym2203(command) = &*action {
                self.ym2203
                    .as_ref()
                    .is_some_and(|chip| chip.valid_event(command, event.sequence, event.owner))
            } else {
                true
            })
            && (!matches!(*action, Action::DacPlay { .. } | Action::DacStream(_))
                || event.sequence >= self.dac_cancel_before.load(Ordering::Acquire)
                    && event.owner.is_none_or(|owner| {
                        event.sequence
                            >= self.owner_dac_cancel_before[(owner.owner - 1) as usize]
                                .load(Ordering::Acquire)
                    }))
            && !matches!(&*action,Action::SamplePlay(reservation) if reservation.cancelled())
    }
    fn retire_action(&self, action: Action) {
        match action {
            Action::ExtraPsg(instance, command) => {
                instance.psg().expect("PSG action target").retire(command);
            }
            Action::ExtraPsgInstall(_) => {}
            Action::ExtraYm2612(instance, command) => {
                instance
                    .ym2612()
                    .expect("YM2612 action target")
                    .retire(command);
            }
            Action::ExtraYm2203(instance, command) => {
                instance
                    .ym2203()
                    .expect("YM2203 action target")
                    .retire(command);
            }
            Action::ExtraYm2608(instance, command) => {
                instance
                    .ym2608()
                    .expect("YM2608 action target")
                    .retire(command);
            }
            Action::ExtraYm2610(instance, command) => {
                instance
                    .ym2610()
                    .expect("YM2610 action target")
                    .retire(command);
            }
            Action::ExtraYm2151(instance, command) => {
                instance
                    .ym2151()
                    .expect("YM2151 action target")
                    .retire(command);
            }
            Action::ExtraYm3526(instance, command) => instance
                .ym3526()
                .expect("YM3526 action target")
                .retire(command),
            Action::ExtraYm3812(instance, command) => instance
                .ym3812()
                .expect("YM3812 action target")
                .retire(command),
            Action::ExtraYmf262(instance, command) => instance
                .ymf262()
                .expect("YMF262 action target")
                .retire(command),
            Action::ExtraRf5c164(instance, command) => instance
                .rf5c164()
                .expect("RF5C164 action target")
                .retire(command),
            Action::ExtraOkim6258(instance, command) => instance
                .okim6258()
                .expect("OKIM6258 action target")
                .retire(command),
            Action::ExtraHuc6280(instance, command) => instance
                .huc6280()
                .expect("HuC6280 action target")
                .retire(command),
            Action::ExtraDx7(instance, command) => {
                instance.dx7().expect("DX7 action target").retire(command)
            }
            Action::ExtraPwm(instance, command) => {
                instance.pwm().expect("PWM action target").retire(command)
            }
            Action::ExtraGameboy(instance, command) => instance
                .gameboy()
                .expect("Game Boy action target")
                .retire(command),
            Action::ExtraNes(instance, command) => {
                instance.nes().expect("NES action target").retire(command)
            }
            Action::ExtraY8950(instance, command) => instance
                .y8950()
                .expect("Y8950 action target")
                .retire(command),
            Action::ExtraAy8910(instance, command) => {
                instance
                    .ay8910()
                    .expect("AY8910 action target")
                    .retire(command);
            }
            Action::ExtraYm2413(instance, command) => {
                instance
                    .ym2413()
                    .expect("YM2413 action target")
                    .retire(command);
            }
            Action::Nes(command) => {
                if let Some(nes) = &self.nes {
                    nes.retire(command);
                }
            }
            Action::Gameboy(command) => {
                if let Some(chip) = &self.gameboy {
                    chip.retire(command);
                }
            }
            Action::Snes(command) => {
                if let Some(chip) = &self.snes {
                    chip.retire(command);
                }
            }
            Action::Huc6280(command) => {
                if let Some(chip) = &self.huc6280 {
                    chip.retire(command);
                }
            }
            Action::Ym2608(command) => {
                if let Some(chip) = &self.ym2608 {
                    chip.retire(command);
                }
            }
            Action::Ym2203(command) => {
                if let Some(chip) = &self.ym2203 {
                    chip.retire(command);
                }
            }
            Action::Ym2610b(command) => {
                if let Some(chip) = &self.ym2610b {
                    chip.retire(command);
                }
            }
            Action::X68000(command) => {
                if let Some(chip) = &self.x68000 {
                    chip.retire(command);
                }
            }
            Action::MasterSystem(command) => {
                if let Some(chip) = &self.master_system {
                    chip.retire(command);
                }
            }
            Action::Msx(command) => {
                if let Some(chip) = &self.msx {
                    chip.retire(command);
                }
            }
            Action::Opl(command) => {
                if let Some(chip) = &self.opl {
                    chip.retire(command);
                }
            }
            Action::Pwm(command) => {
                if let Some(chip) = &self.pwm {
                    chip.retire(command);
                }
            }
            Action::Dx7(command) => {
                if let Some(dx7) = &self.dx7 {
                    dx7.retire(command);
                }
            }
            Action::RfLoad(memory) => {
                self.retired_rf
                    .push(memory)
                    .unwrap_or_else(|_| unreachable!("Bounded RF5C164 upload retirement"));
            }
            Action::Record(capture) => {
                capture.stop.store(true, Ordering::Release);
                capture.sample(self.frame.load(Ordering::Acquire), [0.0; 2]);
                self.recorded
                    .push(capture)
                    .unwrap_or_else(|_| unreachable!("Single recording completion"));
            }
            Action::SampleTimedStop(targets) => {
                self.retired_stops
                    .push(targets)
                    .unwrap_or_else(|_| unreachable!("Bounded sample stop retirement"));
            }
            Action::SamplePlay(reservation) => {
                self.retired
                    .push(reservation)
                    .unwrap_or_else(|_| unreachable!("Bounded reservation retirement"));
            }
            Action::DacStream(_) => {
                self.dac_streams.fetch_sub(1, Ordering::AcqRel);
            }
            _ => {}
        }
    }
    fn performance(
        &self,
        op: &str,
        performance: u64,
        owner: Option<u32>,
    ) -> Result<Status, String> {
        if performance == 0 || performance > MAX_SAFE_INTEGER {
            return Err("Invalid native performance ID".into());
        }
        let _producer = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        match op {
            "start" => {
                self.active_performance.store(0, Ordering::Release);
                for cancelled in &self.owner_dac_cancel_before {
                    cancelled.store(0, Ordering::Release);
                }
                for cancelled in &self.owner_nes_cancel_before {
                    cancelled.store(0, Ordering::Release);
                }
                for cancelled in &self.cancelled_owners {
                    cancelled.store(false, Ordering::Release);
                }
                self.active_performance
                    .store(performance, Ordering::Release);
            }
            "stop" if self.active_performance.load(Ordering::Acquire) == performance => {
                self.active_performance.store(0, Ordering::Release);
            }
            "cancel-owner" if self.active_performance.load(Ordering::Acquire) == performance => {
                let owner = owner
                    .filter(|v| *v > 0 && *v <= OWNERS as u32)
                    .ok_or("Invalid native performance owner")?;
                self.cancelled_owners[(owner - 1) as usize].store(true, Ordering::Release);
            }
            // Stop can race a global Stop or a new performance across the
            // admin and Worker ports. Stale cleanup must be harmless and must
            // never invalidate the replacement performance.
            "stop" | "cancel-owner" => return Ok(self.status()),
            _ => return Err("Unknown native performance operation".into()),
        }
        self.invalidation.fetch_add(1, Ordering::Release);
        Ok(self.status())
    }
    fn check_session(&self, session: u64) -> Result<(), String> {
        if self.session != session {
            return Err("Native Genesis session has expired".into());
        }
        if self.failed.load(Ordering::Acquire) {
            return Err("Native audio device stream failed; select the output again".into());
        }
        Ok(())
    }
    fn status(&self) -> Status {
        Status {
            session: self.session,
            engine: self.genesis_engine,
            mode: if self.pwm.is_some() {
                if self.nuked {
                    "genesis-full-nuked"
                } else {
                    "32x"
                }
            } else if self.gameboy.is_some() {
                "gameboy"
            } else if self.snes.is_some() {
                "snes"
            } else if let Some(chip) = &self.ym2610b {
                chip.mode()
            } else if self.msx.is_some() {
                if self.opl.as_ref().is_some_and(|chip| chip.mode() == "y8950") {
                    "msx-audio"
                } else {
                    "msx"
                }
            } else if let Some(chip) = &self.opl {
                chip.mode()
            } else if let Some(chip) = &self.master_system {
                chip.mode()
            } else if self.x68000.is_some() {
                "x68000"
            } else if self.ym2608.is_some() {
                "pc98"
            } else if self.ym2203.is_some() {
                "pc98-26"
            } else if self.huc6280.is_some() {
                "huc6280"
            } else if self.dx7.is_some() {
                "dx7"
            } else if self.nes.is_some() {
                "nes"
            } else if self.nuked {
                "genesis-nuked"
            } else if self.mega_cd {
                "mega-cd"
            } else {
                "genesis"
            },
            rf5c164_clock: self.mega_cd.then(|| self.rf5c164.clock()),
            nes_clock: self.nes.as_ref().map(nes::Shared::clock),
            gameboy_clock: self.gameboy.as_ref().map(gameboy::Shared::clock),
            snes_clock: self.snes.as_ref().map(snes::Shared::clock),
            huc6280_clock: self.huc6280.as_ref().map(huc6280::Shared::clock),
            ym2608_clock: self.ym2608.as_ref().map(ym2608::Shared::clock),
            ym2203_clock: self.ym2203.as_ref().map(ym2203::Shared::clock),
            ym2610b_clock: self.ym2610b.as_ref().map(ym2610b::Shared::clock),
            ym2151_clock: self.x68000.as_ref().map(x68000::Shared::clock),
            okim6258_clock: self.x68000.as_ref().map(x68000::Shared::oki_clock),
            ym2413_clock: self
                .master_system
                .as_ref()
                .filter(|chip| !chip.is_game_gear())
                .map(master_system::Shared::opll_clock)
                .or_else(|| self.msx.as_ref().map(msx::Shared::opll_clock)),
            ay8910_clock: self.msx.as_ref().map(msx::Shared::clock),
            y8950_clock: self
                .opl
                .as_ref()
                .filter(|chip| chip.mode() == "y8950")
                .map(opl::Shared::clock),
            pwm_clock: self.pwm.as_ref().map(pwm::Shared::clock),
            sample_rate: self.rate,
            frame: self.frame.load(Ordering::Acquire),
            ym_clock: self
                .ym2610b
                .as_ref()
                .map(ym2610b::Shared::clock)
                .or_else(|| self.ym2608.as_ref().map(ym2608::Shared::clock))
                .or_else(|| self.ym2203.as_ref().map(ym2203::Shared::clock))
                .or_else(|| self.x68000.as_ref().map(x68000::Shared::clock))
                .or_else(|| {
                    self.master_system
                        .as_ref()
                        .map(master_system::Shared::opll_clock)
                })
                .or_else(|| self.opl.as_ref().map(opl::Shared::clock))
                .or_else(|| self.msx.as_ref().map(msx::Shared::opll_clock))
                .unwrap_or(7_670_454),
            psg_clock: if let Some(chip) = &self.master_system {
                chip.clock()
            } else if self.ym2608.is_some()
                || self.ym2203.is_some()
                || self.ym2610b.is_some()
                || self.x68000.is_some()
                || self.msx.is_some()
                || self.opl.is_some()
            {
                0
            } else {
                3_579_545
            },
            running: !self.failed.load(Ordering::Acquire),
            queued: self.pending.load(Ordering::Acquire),
            late_commands: self.late.load(Ordering::Relaxed),
            queue_overflows: self.overflows.load(Ordering::Relaxed),
            live_fx: self.livefx.status(),
            error: self
                .failed
                .load(Ordering::Acquire)
                .then(|| "Native audio device stream failed".into()),
        }
    }
    fn msx_audio_status(&self) -> Result<MsxAudioStatus, String> {
        let pair = self
            .msx
            .as_ref()
            .ok_or("Native MSX Audio status requires MSX Audio mode")?;
        let opl = self
            .opl
            .as_ref()
            .filter(|chip| chip.mode() == "y8950")
            .ok_or("Native MSX Audio status requires MSX Audio mode")?;
        let start = Instant::now();
        loop {
            let frame = self.frame.load(Ordering::Acquire);
            let m = pair.status(frame)?;
            let o = opl.status(frame)?;
            if self.frame.load(Ordering::Acquire) == frame {
                return Ok(MsxAudioStatus {
                    y8950: o.y8950.ok_or("Missing Native Y8950 state")?,
                    ym2413: m.ym2413,
                    ay8910: m.ay8910,
                    frame,
                    sample_rate: self.rate,
                    revision: m.revision.saturating_add(o.revision),
                });
            }
            if start.elapsed() > Duration::from_millis(50) {
                return Err("Native MSX Audio state changed while reading; retry".into());
            }
            std::thread::yield_now();
        }
    }
    fn stop(&self) -> Result<Status, String> {
        let _producer = self
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        self.extra_psg.deactivate_all()?;
        let epoch = self.epoch.fetch_add(1, Ordering::AcqRel) + 1;
        self.active_performance.store(0, Ordering::Release);
        let start = Instant::now();
        while self.stopped_epoch.load(Ordering::Acquire) < epoch {
            if self.failed.load(Ordering::Acquire) {
                return Err("Native audio device stream failed during stop".into());
            }
            if start.elapsed() > Duration::from_secs(2) {
                return Err("Native audio stop timed out".into());
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        self.extra_psg.collect_retired()?;
        self.noise
            .lock()
            .map_err(|_| "Native noise state is unavailable")?
            .clear();
        // Runtime Stop disposes run FX. The callback has released every graph
        // at the fade barrier; clear the control model and reclaim off-thread.
        let mut fx = self
            .fx
            .lock()
            .map_err(|_| "Native FX state is unavailable")?;
        let _ = fx.command(FxMessage::Reset, self.rate)?;
        fx.collect();
        self.livefx.wait_reset()?;
        if let Some(chip) = &self.x68000 {
            chip.invalidate_upload();
        }
        Ok(self.status())
    }
}

struct DacVoice {
    bank: Arc<media::Bank>,
    offset: usize,
    size: usize,
    index: usize,
    start: u64,
    owner: Option<Owner>,
}
struct DacStream {
    bank: Arc<media::Bank>,
    index: usize,
    start: u64,
    owner: Option<Owner>,
}

struct Renderer {
    extra_psg: extra_psg::Renderer,
    core: Core,
    dx7: Option<dx7::Renderer>,
    nes: Option<nes::Renderer>,
    gameboy: Option<gameboy::Renderer>,
    snes: Option<snes::Renderer>,
    huc6280: Option<huc6280::Renderer>,
    ym2608: Option<ym2608::Renderer>,
    ym2203: Option<ym2203::Renderer>,
    ym2610b: Option<ym2610b::Renderer>,
    x68000: Option<x68000::Renderer>,
    master_system: Option<master_system::Renderer>,
    msx: Option<msx::Renderer>,
    opl: Option<opl::Renderer>,
    pwm: Option<pwm::Renderer>,

    dx7_master: f32,
    dx7_master_target: f32,
    dx7_master_ramp: u32,
    rf5c164: Option<rf5c164::Renderer>,
    fx: FxRenderer,
    livefx: livefx::Renderer,
    noise: noise::Renderer,
    voices: [Option<media::Voice>; media::VOICES],
    dac: Option<DacVoice>,
    streams: [Option<DacStream>; 8],
    pan_register: u8,
    raw_dac_owner: Option<Owner>,
    raw_dac_enabled: bool,
    fm_owners: [Option<Owner>; 6],
    fm_key_masks: [u8; 6],
    fm_mode_register: u8,
    csm_owner: Option<Owner>,
    psg_owners: [Option<Owner>; 4],
    psg_latch: u8,
    invalidation: u64,
    record: Option<Arc<recording::Capture>>,
    monitor_writer: monitor::Writer,
    heap: BinaryHeap<Event>,
    frame: u64,
    epoch: u64,
    fade_remaining: u32,
    fade_frames: u32,
    resume_remaining: u32,
    idle: bool,
}
impl Renderer {
    #[cfg(test)]
    fn new(rate: u32) -> Result<Self, String> {
        Self::new_mode(rate, false)
    }
    #[cfg(test)]
    fn new_mode(rate: u32, mega_cd: bool) -> Result<Self, String> {
        Self::new_primary(rate, mega_cd, false)
    }
    #[cfg(test)]
    fn new_primary(rate: u32, mega_cd: bool, dx7: bool) -> Result<Self, String> {
        Self::new_config(rate, mega_cd, dx7, false)
    }
    #[cfg(test)]
    fn new_config(rate: u32, mega_cd: bool, dx7: bool, nes: bool) -> Result<Self, String> {
        Self::new_modes(rate, mega_cd, dx7, nes, None)
    }
    fn new_modes(
        rate: u32,
        mega_cd: bool,
        dx7: bool,
        nes: bool,
        console: Option<&str>,
    ) -> Result<Self, String> {
        Ok(Self {
            extra_psg: extra_psg::Renderer::default(),
            core: if dx7
                || nes
                || console
                    .is_some_and(|mode| !matches!(mode, "32x" | "32x-nuked" | "genesis-nuked"))
            {
                Core::empty()
            } else if matches!(console, Some("32x-nuked" | "genesis-nuked")) {
                Core::new_nuked(rate)?
            } else {
                Core::new(rate)?
            },
            nes: if nes {
                Some(nes::Renderer::new(rate)?)
            } else {
                None
            },
            dx7: if dx7 {
                Some(dx7::Renderer::new(rate)?)
            } else {
                None
            },
            gameboy: if console == Some("gameboy") {
                Some(gameboy::Renderer::new(rate)?)
            } else {
                None
            },
            snes: if console == Some("snes") {
                Some(snes::Renderer::new(rate)?)
            } else {
                None
            },
            huc6280: if console == Some("huc6280") {
                Some(huc6280::Renderer::new(rate)?)
            } else {
                None
            },
            ym2608: if console == Some("ym2608") {
                Some(ym2608::Renderer::new(rate)?)
            } else {
                None
            },
            ym2203: if console == Some("ym2203") {
                Some(ym2203::Renderer::new(rate)?)
            } else {
                None
            },
            ym2610b: if matches!(console, Some("ym2610" | "ym2610b")) {
                Some(ym2610b::Renderer::new(rate, console == Some("ym2610b"))?)
            } else {
                None
            },
            x68000: if console == Some("x68000") {
                Some(x68000::Renderer::new(rate)?)
            } else {
                None
            },
            master_system: match console {
                Some("master-system") => Some(master_system::Renderer::new(rate)?),
                Some("game-gear") => Some(master_system::Renderer::new_game_gear(rate)?),
                _ => None,
            },
            msx: if matches!(console, Some("msx" | "msx-audio")) {
                Some(msx::Renderer::new(rate)?)
            } else {
                None
            },
            opl: if let Some(chip) = console.and_then(opl::Chip::from_mode) {
                Some(opl::Renderer::new(rate, chip)?)
            } else {
                None
            },
            pwm: if matches!(console, Some("32x" | "32x-nuked")) {
                Some(pwm::Renderer::new(rate)?)
            } else {
                None
            },
            dx7_master: 1.0,
            dx7_master_target: 1.0,
            dx7_master_ramp: 0,
            rf5c164: if mega_cd {
                Some(rf5c164::Renderer::new(rate)?)
            } else {
                None
            },
            fx: FxRenderer::new(rate),
            livefx: livefx::Renderer::new(),
            noise: noise::Renderer::new(rate)?,
            voices: std::array::from_fn(|_| None),
            dac: None,
            streams: std::array::from_fn(|_| None),
            pan_register: 0,
            raw_dac_owner: None,
            raw_dac_enabled: false,
            fm_owners: [None; 6],
            fm_key_masks: [0; 6],
            fm_mode_register: 0,
            csm_owner: None,
            psg_owners: [None; 4],
            psg_latch: 0,
            invalidation: 0,
            record: None,
            monitor_writer: monitor::Writer::default(),
            heap: BinaryHeap::with_capacity(CAPACITY),
            frame: 0,
            epoch: 0,
            fade_remaining: 0,
            fade_frames: (rate / 100).max(1),
            resume_remaining: 0,
            idle: true,
        })
    }
    fn prepare(&mut self, shared: &Shared) {
        let epoch = shared.epoch.load(Ordering::Acquire);
        if epoch != self.epoch {
            self.epoch = epoch;
            self.livefx.begin_stop();
            shared.livefx.transport.invalidate(true);
            let mut removed = self.heap.len();
            while let Some(event) = self.heap.pop() {
                shared.retire_action(event.action.into_inner());
            }
            while let Some(event) = shared.queue.pop() {
                shared.retire_action(event.action.into_inner());
                removed += 1;
            }
            shared.pending.fetch_sub(removed, Ordering::AcqRel);
            self.fade_remaining = self.fade_frames;
            self.resume_remaining = 0;
        }
        let invalidation = shared.invalidation.load(Ordering::Acquire);
        if invalidation != self.invalidation {
            self.invalidation = invalidation;
            // Invalidate cancelled performance owners and future media starts before
            // processing new events. BinaryHeap::retain keeps its allocated capacity.
            let mut removed = 0;
            self.heap.retain(|event| {
                let keep = shared.valid_event(event);
                if !keep {
                    removed += 1;
                    // Move the reference out before publishing retirement.
                    // Cloning then dropping the original could let control
                    // reclaim first and leave the callback as final owner.
                    shared.retire_action(event.action.replace(Action::Reset));
                }
                keep
            });
            shared.pending.fetch_sub(removed, Ordering::AcqRel);
            if self.csm_owner.is_some() && !shared.valid_owner(self.csm_owner) {
                // CSM can sound CH3 from Timer A without register 0x28. Stop
                // that source independently of a still-valid explicit/manual
                // CH3 key gate, preserving other voices and Timer B.
                self.fm_mode_register &= !0x81;
                self.core
                    .apply(Action::Write(0, 0x27, self.fm_mode_register));
                self.csm_owner = None;
                if self.fm_key_masks[2] == 0 || !shared.valid_owner(self.fm_owners[2]) {
                    self.core.apply(Action::Write(0, 0x28, 2));
                    self.fm_key_masks[2] = 0;
                    self.fm_owners[2] = None;
                }
            }
            for channel in 0..6 {
                if self.fm_owners[channel].is_some() && !shared.valid_owner(self.fm_owners[channel])
                {
                    self.core.apply(Action::Write(
                        0,
                        0x28,
                        (channel % 3 + if channel >= 3 { 4 } else { 0 }) as u8,
                    ));
                    self.fm_owners[channel] = None;
                    self.fm_key_masks[channel] = 0;
                }
            }
            for channel in 0..4 {
                if self.psg_owners[channel].is_some()
                    && !shared.valid_owner(self.psg_owners[channel])
                {
                    if !self.core.0.is_null() {
                        unsafe { native_genesis_psg_mute(self.core.0, channel as u8) };
                    }
                    self.psg_owners[channel] = None;
                }
            }
            if let (Some(dx7), Some(state)) = (&mut self.dx7, &shared.dx7) {
                dx7.cancel_invalid(|owner| shared.valid_owner(owner), state);
            }
            if let (Some(nes), Some(state)) = (&mut self.nes, &shared.nes) {
                nes.cancel_invalid(|owner| shared.valid_owner(owner), state);
            }
            if let (Some(chip), Some(state)) = (&mut self.gameboy, &shared.gameboy) {
                chip.cancel_invalid(|owner| shared.valid_owner(owner), state);
            }
            if let (Some(chip), Some(state)) = (&mut self.snes, &shared.snes) {
                chip.cancel_invalid(|owner| shared.valid_owner(owner), state);
            }
            if let (Some(chip), Some(state)) = (&mut self.huc6280, &shared.huc6280) {
                chip.cancel_invalid(|owner| shared.valid_owner(owner), state);
            }
            if let (Some(chip), Some(state)) = (&mut self.ym2608, &shared.ym2608) {
                chip.cancel_invalid(|owner| shared.valid_owner(owner), state);
            }
            if let (Some(chip), Some(state)) = (&mut self.ym2203, &shared.ym2203) {
                chip.cancel_invalid(|owner| shared.valid_owner(owner), state);
            }
            if let (Some(chip), Some(state)) = (&mut self.ym2610b, &shared.ym2610b) {
                chip.cancel_invalid(|owner| shared.valid_owner(owner), state);
            }
            if let (Some(chip), Some(state)) = (&mut self.x68000, &shared.x68000) {
                chip.cancel_invalid(|owner| shared.valid_owner(owner), state);
            }
            if let (Some(chip), Some(state)) = (&mut self.master_system, &shared.master_system) {
                chip.cancel_invalid(|owner| shared.valid_owner(owner), state);
            }
            if let (Some(chip), Some(state)) = (&mut self.msx, &shared.msx) {
                chip.cancel_invalid(|owner| shared.valid_owner(owner), state);
            }
            if let (Some(chip), Some(state)) = (&mut self.opl, &shared.opl) {
                chip.cancel_invalid(|owner| shared.valid_owner(owner), state);
            }
            if let (Some(chip), Some(state)) = (&mut self.pwm, &shared.pwm) {
                chip.cancel_invalid(|owner| shared.valid_owner(owner), state);
            }
            self.noise.cancel_invalid(|owner| shared.valid_owner(owner));
            if let Some(rf) = &mut self.rf5c164 {
                rf.cancel_invalid(|owner| shared.valid_owner(owner), &shared.rf5c164);
            }
            for voice in self.voices.iter_mut().flatten() {
                if !shared.valid_owner(voice.owner) {
                    voice.reservation.cancel();
                }
            }
            if self
                .dac
                .as_ref()
                .is_some_and(|v| !shared.valid_owner(v.owner))
            {
                self.stop_dac();
            }
            if self.raw_dac_enabled && !shared.valid_owner(self.raw_dac_owner) {
                self.core.apply(Action::Write(0, 0x2a, 128));
                self.core.apply(Action::Write(0, 0x2b, 0));
                self.raw_dac_enabled = false;
                self.raw_dac_owner = None;
            }
            for stream in &mut self.streams {
                if stream
                    .as_ref()
                    .is_some_and(|v| !shared.valid_owner(v.owner))
                {
                    *stream = None;
                    shared.dac_streams.fetch_sub(1, Ordering::AcqRel);
                }
            }
        }
        self.extra_psg.prune(
            self.epoch,
            |owner| shared.valid_owner(owner),
            self.fade_remaining > 0,
        );
        // Fixed admission bounds guarantee this never reallocates. Limit work
        // per callback; large incoming batches cannot monopolize audio forever.
        for _ in 0..MAX_BATCH {
            let Some(event) = shared.queue.pop() else {
                break;
            };
            if event.epoch == self.epoch && shared.valid_event(&event) {
                self.heap.push(event);
            } else {
                shared.pending.fetch_sub(1, Ordering::AcqRel);
                shared.retire_action(event.action.into_inner());
            }
        }
    }
    fn publish_primary(&self, shared: &Shared) {
        if let (Some(nes), Some(state)) = (&self.nes, &shared.nes) {
            nes.publish(state);
        }
        if let (Some(chip), Some(state)) = (&self.gameboy, &shared.gameboy) {
            chip.publish(state);
        }
        if let (Some(chip), Some(state)) = (&self.snes, &shared.snes) {
            chip.publish(state);
        }
        if let (Some(chip), Some(state)) = (&self.huc6280, &shared.huc6280) {
            chip.publish(state);
        }
        if let (Some(chip), Some(state)) = (&self.ym2608, &shared.ym2608) {
            chip.publish(state);
        }
        if let (Some(chip), Some(state)) = (&self.ym2203, &shared.ym2203) {
            chip.publish(state);
        }
        if let (Some(chip), Some(state)) = (&self.ym2610b, &shared.ym2610b) {
            chip.publish(state);
        }
        if let (Some(chip), Some(state)) = (&self.x68000, &shared.x68000) {
            chip.publish(state);
        }
        if let (Some(chip), Some(state)) = (&self.master_system, &shared.master_system) {
            chip.publish(state);
        }
        if let (Some(chip), Some(state)) = (&self.msx, &shared.msx) {
            chip.publish(state);
        }
        if let (Some(chip), Some(state)) = (&self.opl, &shared.opl) {
            chip.publish(state);
        }
        if let (Some(chip), Some(state)) = (&self.pwm, &shared.pwm) {
            chip.publish(state);
        }
    }
    fn stop_dac(&mut self) {
        if self.dac.take().is_some() || self.raw_dac_enabled {
            self.core.apply(Action::Write(0, 0x2a, 128));
            self.core.apply(Action::Write(0, 0x2b, 0));
        }
        self.raw_dac_owner = None;
        self.raw_dac_enabled = false;
    }
    fn clear_media(&mut self, shared: &Shared) {
        self.stop_dac();
        for stream in &mut self.streams {
            if stream.take().is_some() {
                shared.dac_streams.fetch_sub(1, Ordering::AcqRel);
            }
        }
        for voice in &mut self.voices {
            if let Some(voice) = voice.take() {
                shared
                    .retired
                    .push(voice.reservation)
                    .unwrap_or_else(|_| unreachable!("Bounded reservation retirement"));
            }
        }
    }
    fn apply(&mut self, event: Event, shared: &Shared) {
        let owner = event.owner;
        let sequence = event.sequence;
        match event.action.into_inner() {
            Action::ExtraPsgInstall(instance) => self.extra_psg.install_at(instance, self.frame),
            Action::ExtraPsg(instance, command) => {
                if command.stops() {
                    instance.psg().expect("PSG action target").cancel_before(
                        &command,
                        sequence.saturating_add(1),
                        owner,
                    );
                    shared.invalidation.fetch_add(1, Ordering::Release);
                }
                self.extra_psg.apply(&instance, &command, owner);
                instance.psg().expect("PSG action target").retire(command);
            }
            Action::ExtraYm2612(instance, command) => {
                let state = instance.ym2612().expect("YM2612 action target");
                if command.stops() {
                    state.cancel_before(&command, sequence.saturating_add(1), owner);
                    shared.invalidation.fetch_add(1, Ordering::Release);
                }
                self.extra_psg.apply_ym2612(&instance, &command, owner);
                state.retire(command);
            }
            Action::ExtraYm2203(instance, command) => {
                let state = instance.ym2203().expect("YM2203 action target");
                if command.stops() {
                    state.cancel_before(&command, sequence.saturating_add(1), owner);
                    shared.invalidation.fetch_add(1, Ordering::Release);
                }
                if command.invalidates_schedule() {
                    shared.invalidation.fetch_add(1, Ordering::Release);
                }
                self.extra_psg.apply_ym2203(&instance, &command, owner);
                if !command.deferred_retirement() {
                    state.retire(command);
                }
            }
            Action::ExtraYm2608(instance, command) => {
                let state = instance.ym2608().expect("YM2608 action target");
                if command.stops() {
                    state.cancel_before(&command, sequence.saturating_add(1), owner);
                    shared.invalidation.fetch_add(1, Ordering::Release);
                }
                if command.invalidates_schedule() {
                    shared.invalidation.fetch_add(1, Ordering::Release);
                }
                self.extra_psg.apply_ym2608(&instance, &command, owner);
                if !command.deferred_retirement() {
                    state.retire(command);
                }
            }
            Action::ExtraYm2610(instance, command) => {
                let state = instance.ym2610().expect("YM2610 action target");
                if command.stops() {
                    state.cancel_before(&command, sequence.saturating_add(1), owner);
                    shared.invalidation.fetch_add(1, Ordering::Release);
                }
                self.extra_psg.apply_ym2610(&instance, &command, owner);
                state.retire(command);
            }
            Action::ExtraYm2151(instance, command) => {
                let state = instance.ym2151().expect("YM2151 action target");
                if command.stops() {
                    state.cancel_before(&command, sequence.saturating_add(1), owner);
                    shared.invalidation.fetch_add(1, Ordering::Release);
                }
                self.extra_psg.apply_ym2151(&instance, &command, owner);
                state.retire(command);
            }
            Action::ExtraYm3526(instance, command) => {
                let state = instance.ym3526().expect("YM3526 action target");
                if command.stops() {
                    state.cancel_before(&command, sequence.saturating_add(1), owner);
                    shared.invalidation.fetch_add(1, Ordering::Release);
                }
                self.extra_psg.apply_ym3526(&instance, &command, owner);
                state.retire(command);
            }
            Action::ExtraYm3812(instance, command) => {
                let state = instance.ym3812().expect("YM3812 action target");
                if command.stops() {
                    state.cancel_before(&command, sequence.saturating_add(1), owner);
                    shared.invalidation.fetch_add(1, Ordering::Release);
                }
                self.extra_psg.apply_ym3812(&instance, &command, owner);
                state.retire(command);
            }
            Action::ExtraYmf262(instance, command) => {
                let state = instance.ymf262().expect("YMF262 action target");
                if command.stops() {
                    state.cancel_before(&command, sequence.saturating_add(1), owner);
                    shared.invalidation.fetch_add(1, Ordering::Release);
                }
                self.extra_psg.apply_ymf262(&instance, &command, owner);
                state.retire(command);
            }
            Action::ExtraRf5c164(instance, command) => {
                let state = instance.rf5c164().expect("RF5C164 action target");
                if command.stops() {
                    state.cancel_before(&command, sequence.saturating_add(1), owner);
                    shared.invalidation.fetch_add(1, Ordering::Release);
                }
                self.extra_psg.apply_rf5c164(&instance, &command, owner);
                state.retire(command);
            }
            Action::ExtraOkim6258(instance, command) => {
                let state = instance.okim6258().expect("OKIM6258 action target");
                if command.stops() {
                    state.cancel_before(&command, sequence.saturating_add(1), owner);
                    shared.invalidation.fetch_add(1, Ordering::Release);
                }
                self.extra_psg.apply_okim6258(&instance, &command, owner);
                state.retire(command);
            }
            Action::ExtraHuc6280(instance, command) => {
                let state = instance.huc6280().expect("HuC6280 action target");
                if command.stops() {
                    state.cancel_before(&command, sequence.saturating_add(1), owner);
                    shared.invalidation.fetch_add(1, Ordering::Release);
                }
                self.extra_psg.apply_huc6280(&instance, &command, owner);
                state.retire(command);
            }
            Action::ExtraDx7(instance, command) => {
                let state = instance.dx7().expect("DX7 action target");
                if command.stops() {
                    state.cancel_before(&command, sequence.saturating_add(1), owner);
                    shared.invalidation.fetch_add(1, Ordering::Release);
                }
                self.extra_psg.apply_dx7(&instance, &command, owner);
                state.retire(command);
            }
            Action::ExtraPwm(instance, command) => {
                let state = instance.pwm().expect("PWM action target");
                if command.stops() {
                    state.cancel_before(&command, sequence.saturating_add(1), owner);
                    shared.invalidation.fetch_add(1, Ordering::Release);
                }
                self.extra_psg
                    .apply_pwm(&instance, &command, owner, self.frame);
                state.retire(command);
            }
            Action::ExtraGameboy(instance, command) => {
                let state = instance.gameboy().expect("Game Boy action target");
                if command.stops() {
                    state.cancel_before(&command, sequence.saturating_add(1), owner);
                    shared.invalidation.fetch_add(1, Ordering::Release);
                }
                self.extra_psg.apply_gameboy(&instance, &command, owner);
                state.retire(command);
            }
            Action::ExtraNes(instance, command) => {
                let state = instance.nes().expect("NES action target");
                if command.stops() {
                    state.cancel_before(&command, sequence.saturating_add(1), owner);
                    shared.invalidation.fetch_add(1, Ordering::Release);
                }
                self.extra_psg.apply_nes(&instance, &command, owner);
                state.retire(command);
            }
            Action::ExtraY8950(instance, command) => {
                let state = instance.y8950().expect("Y8950 action target");
                if command.stops() {
                    state.cancel_before(&command, sequence.saturating_add(1), owner);
                    shared.invalidation.fetch_add(1, Ordering::Release);
                }
                self.extra_psg.apply_y8950(&instance, &command, owner);
                state.retire(command);
            }
            Action::ExtraAy8910(instance, command) => {
                let state = instance.ay8910().expect("AY8910 action target");
                if command.stops() {
                    state.cancel_before(&command, sequence.saturating_add(1), owner);
                    shared.invalidation.fetch_add(1, Ordering::Release);
                }
                self.extra_psg.apply_ay8910(&instance, &command, owner);
                state.retire(command);
            }
            Action::ExtraYm2413(instance, command) => {
                let state = instance.ym2413().expect("YM2413 action target");
                if command.stops() {
                    state.cancel_before(&command, sequence.saturating_add(1), owner);
                    shared.invalidation.fetch_add(1, Ordering::Release);
                }
                self.extra_psg.apply_ym2413(&instance, &command, owner);
                state.retire(command);
            }
            Action::Nes(command) => {
                if command.stops() {
                    shared.cancel_nes_before(owner, sequence.saturating_add(1));
                }
                if let (Some(nes), Some(state)) = (&mut self.nes, &shared.nes) {
                    nes.apply(&command, owner, state);
                }
                shared.retire_action(Action::Nes(command));
            }
            Action::Gameboy(command) => {
                if command.stops() {
                    shared.cancel_nes_before(owner, sequence.saturating_add(1));
                }
                if let (Some(chip), Some(state)) = (&mut self.gameboy, &shared.gameboy) {
                    chip.apply(&command, owner, state);
                }
                shared.retire_action(Action::Gameboy(command));
            }
            Action::Snes(command) => {
                if command.stops() {
                    shared.cancel_nes_before(owner, sequence.saturating_add(1));
                }
                if let (Some(chip), Some(state)) = (&mut self.snes, &shared.snes) {
                    chip.apply(&command, owner, state);
                }
                shared.retire_action(Action::Snes(command));
            }
            Action::Huc6280(command) => {
                if command.stops() {
                    shared.cancel_nes_before(owner, sequence.saturating_add(1));
                }
                if let (Some(chip), Some(state)) = (&mut self.huc6280, &shared.huc6280) {
                    chip.apply(&command, owner, state);
                }
                shared.retire_action(Action::Huc6280(command));
            }
            Action::Ym2608(command) => {
                if command.stops() {
                    shared.cancel_nes_before(owner, sequence.saturating_add(1));
                }
                if let (Some(chip), Some(state)) = (&mut self.ym2608, &shared.ym2608) {
                    chip.apply(&command, owner, state);
                }
                shared.retire_action(Action::Ym2608(command));
            }
            Action::Ym2610b(command) => {
                if command.stops() {
                    shared.cancel_nes_before(owner, sequence.saturating_add(1));
                }
                if let (Some(chip), Some(state)) = (&mut self.ym2610b, &shared.ym2610b) {
                    chip.apply(&command, owner, state);
                }
                shared.retire_action(Action::Ym2610b(command));
            }
            Action::X68000(command) => {
                if let (Some(chip), Some(state)) = (&mut self.x68000, &shared.x68000) {
                    if command.stops() {
                        state.cancel_before(&command, sequence.saturating_add(1), owner);
                        shared.invalidation.fetch_add(1, Ordering::Release);
                    }
                    chip.apply(&command, owner, state);
                }
                shared.retire_action(Action::X68000(command));
            }
            Action::MasterSystem(command) => {
                if let (Some(chip), Some(state)) = (&mut self.master_system, &shared.master_system)
                {
                    if command.stops() {
                        state.cancel_before(&command, sequence.saturating_add(1), owner);
                        shared.invalidation.fetch_add(1, Ordering::Release);
                    }
                    chip.apply(&command, owner, state);
                }
                shared.retire_action(Action::MasterSystem(command));
            }
            Action::Msx(command) => {
                if let (Some(chip), Some(state)) = (&mut self.msx, &shared.msx) {
                    if command.stops() {
                        state.cancel_before(&command, sequence.saturating_add(1), owner);
                        shared.invalidation.fetch_add(1, Ordering::Release);
                    }
                    chip.apply(&command, owner, state);
                }
                shared.retire_action(Action::Msx(command));
            }
            Action::Opl(command) => {
                if let (Some(chip), Some(state)) = (&mut self.opl, &shared.opl) {
                    if command.stops() {
                        state.cancel_before(&command, sequence.saturating_add(1), owner);
                        shared.invalidation.fetch_add(1, Ordering::Release);
                    }
                    chip.apply(&command, owner, state);
                }
                shared.retire_action(Action::Opl(command));
            }
            Action::Ym2203(command) => {
                if let (Some(chip), Some(state)) = (&mut self.ym2203, &shared.ym2203) {
                    if command.stops() {
                        state.cancel_before(&command, sequence.saturating_add(1), owner);
                        shared.invalidation.fetch_add(1, Ordering::Release);
                    }
                    chip.apply(&command, owner, state);
                }
                shared.retire_action(Action::Ym2203(command));
            }
            Action::Pwm(command) => {
                if command.stops() {
                    shared.cancel_nes_before(owner, sequence.saturating_add(1));
                }
                if let (Some(chip), Some(state)) = (&mut self.pwm, &shared.pwm) {
                    chip.apply(&command, owner, state);
                }
                shared.retire_action(Action::Pwm(command));
            }
            Action::Dx7(command) => {
                if let (Some(dx7), Some(state)) = (&mut self.dx7, &shared.dx7) {
                    dx7.apply(&command, owner, state);
                }
                shared.retire_action(Action::Dx7(command));
            }
            Action::RfWrite(register, value, audition) => {
                if let Some(rf) = &mut self.rf5c164 {
                    rf.write(register, value, owner, audition, &shared.rf5c164);
                }
            }
            Action::RfWriteMemory(offset, value) => {
                if let Some(rf) = &mut self.rf5c164 {
                    rf.write_memory(offset, value);
                }
            }
            Action::RfLoad(memory) => {
                if let Some(rf) = &mut self.rf5c164 {
                    rf.load(&memory);
                }
                shared
                    .retired_rf
                    .push(memory)
                    .unwrap_or_else(|_| unreachable!("Bounded RF5C164 upload retirement"));
            }
            Action::RfReset => {
                if let Some(rf) = &mut self.rf5c164 {
                    rf.reset(&shared.rf5c164);
                }
            }
            Action::RfClearMemory => {
                if let Some(rf) = &mut self.rf5c164 {
                    rf.clear_memory();
                }
            }
            Action::RfClock(clock) => {
                if let Some(rf) = &mut self.rf5c164 {
                    rf.set_clock(clock, &shared.rf5c164);
                }
            }
            Action::RfMix(gains) => {
                if let Some(rf) = &mut self.rf5c164 {
                    rf.set_mix(gains);
                }
            }
            Action::RfKey(channel, on, voice) => {
                if let Some(rf) = &mut self.rf5c164 {
                    rf.key(channel, on, owner, voice, &shared.rf5c164);
                }
            }
            Action::DacPlay {
                bank,
                offset,
                size,
                pan,
            } => {
                self.dac = Some(DacVoice {
                    bank,
                    offset,
                    size,
                    index: 0,
                    start: self.frame,
                    owner,
                });
                self.raw_dac_owner = owner;
                self.raw_dac_enabled = true;
                self.core
                    .apply(Action::Write(1, 0xb6, (self.pan_register & 0x3f) | pan));
                self.core.apply(Action::Write(0, 0x2a, 128));
                self.core.apply(Action::Write(0, 0x2b, 0x80));
            }
            Action::DacStream(bank) => {
                if let Some(slot) = self.streams.iter_mut().find(|slot| slot.is_none()) {
                    *slot = Some(DacStream {
                        bank,
                        index: 0,
                        start: self.frame,
                        owner,
                    });
                } else {
                    shared.failed.store(true, Ordering::Release);
                    shared.dac_streams.fetch_sub(1, Ordering::AcqRel);
                }
            }
            Action::DacStop => {
                shared.cancel_dac_before(owner, sequence);
                if owner.is_none()
                    || self.dac.as_ref().is_some_and(|v| v.owner == owner)
                    || self.raw_dac_owner == owner
                {
                    self.stop_dac();
                }
                for stream in &mut self.streams {
                    if stream
                        .as_ref()
                        .is_some_and(|v| owner.is_none() || v.owner == owner)
                    {
                        *stream = None;
                        shared.dac_streams.fetch_sub(1, Ordering::AcqRel);
                    }
                }
            }
            Action::SamplePlay(reservation) => {
                if reservation.cancelled() {
                    shared
                        .retired
                        .push(reservation)
                        .unwrap_or_else(|_| unreachable!("Bounded reservation retirement"));
                } else if let Some(slot) = self.voices.iter_mut().find(|slot| slot.is_none()) {
                    *slot = Some(media::Voice::new(reservation, shared.rate, owner));
                } else {
                    shared
                        .retired
                        .push(reservation)
                        .unwrap_or_else(|_| unreachable!("Bounded reservation retirement"));
                    shared.failed.store(true, Ordering::Release);
                }
            }
            Action::SampleTimedStop(targets) => {
                targets.cancel();
                shared.invalidation.fetch_add(1, Ordering::Release);
                shared
                    .retired_stops
                    .push(targets)
                    .unwrap_or_else(|_| unreachable!("Bounded sample stop retirement"));
            }
            Action::SampleStop { .. } | Action::SampleRelease => {} // cancellation atomics were published at admission.
            Action::Record(capture) => {
                self.record = Some(capture);
            }
            Action::Fx(action) => self.fx.apply(action),
            Action::FxReset(action) => {
                self.fx.apply(action);
                shared.livefx.reset();
            }
            Action::Noise(command) => {
                if let Some(command) = command {
                    self.noise.apply(command, owner, &shared.noise_completed);
                }
            }
            action => {
                match &action {
                    Action::Write(port, reg, value) => {
                        if *port == 1 && *reg == 0xb6 {
                            self.pan_register = *value;
                        }
                        if *port == 0 && *reg == 0x2b {
                            self.raw_dac_enabled = value & 0x80 != 0;
                            self.raw_dac_owner = if self.raw_dac_enabled { owner } else { None };
                        }
                        if *port == 0 && *reg == 0x2a && self.raw_dac_enabled {
                            self.raw_dac_owner = owner;
                        }
                        if *port == 0 && *reg == 0x27 {
                            self.fm_mode_register = *value;
                            // Nuked selects CSM only for mode 2; mode 3 is CH3
                            // special without CSM in the pinned OPN2 core.
                            let csm = if shared.nuked {
                                value & 0xc1 == 0x81
                            } else {
                                value & 0x81 == 0x81
                            };
                            self.csm_owner = if csm { owner } else { None };
                        }
                        if *port == 0 && *reg == 0x28 {
                            let low = (value & 3) as usize;
                            if low < 3 {
                                let channel = low + if value & 4 != 0 { 3 } else { 0 };
                                self.fm_owners[channel] =
                                    if value & 0xf0 != 0 { owner } else { None };
                                self.fm_key_masks[channel] = value >> 4;
                            }
                        }
                    }
                    Action::PsgWrite(value) => {
                        if value & 0x80 != 0 {
                            self.psg_latch = (value >> 4) & 7;
                        }
                        if self.psg_latch & 1 != 0 {
                            self.psg_owners[(self.psg_latch >> 1) as usize] =
                                if value & 15 != 15 { owner } else { None };
                        }
                    }
                    Action::PsgReset => {
                        self.psg_owners = [None; 4];
                        self.psg_latch = 0;
                    }
                    Action::Mix(gains)
                        if self.dx7.is_some()
                            || self.nes.is_some()
                            || self.gameboy.is_some()
                            || self.snes.is_some()
                            || self.huc6280.is_some()
                            || self.ym2608.is_some()
                            || self.ym2203.is_some()
                            || self.ym2610b.is_some()
                            || self.x68000.is_some()
                            || self.master_system.is_some()
                            || self.msx.is_some()
                            || self.opl.is_some() =>
                    {
                        self.dx7_master_target = gains[4];
                        self.dx7_master_ramp = (shared.rate / 200).max(1);
                    }
                    Action::Reset => {
                        self.clear_media(shared);
                        if let (Some(dx7), Some(state)) = (&mut self.dx7, &shared.dx7) {
                            dx7.stop(state);
                        }
                        if let (Some(nes), Some(state)) = (&mut self.nes, &shared.nes) {
                            nes.stop(state);
                        }
                        if let (Some(chip), Some(state)) = (&mut self.gameboy, &shared.gameboy) {
                            chip.stop(state);
                        }
                        if let (Some(chip), Some(state)) = (&mut self.snes, &shared.snes) {
                            chip.stop(state);
                        }
                        if let (Some(chip), Some(state)) = (&mut self.huc6280, &shared.huc6280) {
                            chip.stop(state);
                        }
                        if let (Some(chip), Some(state)) = (&mut self.ym2608, &shared.ym2608) {
                            chip.stop(state);
                        }
                        if let (Some(chip), Some(state)) = (&mut self.ym2203, &shared.ym2203) {
                            chip.stop(state);
                        }
                        if let (Some(chip), Some(state)) = (&mut self.ym2610b, &shared.ym2610b) {
                            chip.stop(state);
                        }
                        if let (Some(chip), Some(state)) = (&mut self.x68000, &shared.x68000) {
                            chip.stop(state);
                        }
                        if let (Some(chip), Some(state)) =
                            (&mut self.master_system, &shared.master_system)
                        {
                            chip.stop(state);
                        }
                        if let (Some(chip), Some(state)) = (&mut self.msx, &shared.msx) {
                            chip.stop(state);
                        }
                        if let (Some(chip), Some(state)) = (&mut self.opl, &shared.opl) {
                            chip.stop(state);
                        }
                        if let Some(rf) = &mut self.rf5c164 {
                            rf.reset(&shared.rf5c164);
                        }
                        if let (Some(chip), Some(state)) = (&mut self.pwm, &shared.pwm) {
                            chip.reset(state);
                        }
                        self.pan_register = 0;
                        self.fm_owners = [None; 6];
                        self.fm_key_masks = [0; 6];
                        self.fm_mode_register = 0;
                        self.csm_owner = None;
                        self.psg_owners = [None; 4];
                        self.psg_latch = 0;
                    }
                    _ => {}
                }
                self.core.apply(action);
            }
        }
    }
    fn advance_dac(&mut self, shared: &Shared) {
        if let Some(voice) = &mut self.dac {
            loop {
                let next = voice.start
                    + (voice.index as f64 * shared.rate as f64 / voice.bank.rate).ceil() as u64;
                if next > self.frame {
                    break;
                }
                if voice.index == voice.size {
                    self.core.apply(Action::Write(0, 0x2a, 128));
                    self.core.apply(Action::Write(0, 0x2b, 0));
                    self.dac = None;
                    self.raw_dac_owner = None;
                    self.raw_dac_enabled = false;
                    break;
                }
                self.core.apply(Action::Write(
                    0,
                    0x2a,
                    voice.bank.bytes()[voice.offset + voice.index],
                ));
                voice.index += 1;
            }
        }
        for slot in &mut self.streams {
            if let Some(stream) = slot {
                let data = stream.bank.bytes();
                while stream.index < data.len() {
                    let offset = stream.index;
                    let samples = u32::from_le_bytes([
                        data[offset],
                        data[offset + 1],
                        data[offset + 2],
                        data[offset + 3],
                    ]);
                    let frame = stream.start
                        + (samples as f64 * shared.rate as f64 / 44100.0).round() as u64;
                    if frame > self.frame {
                        break;
                    }
                    self.core.apply(Action::Write(0, 0x2a, data[offset + 4]));
                    stream.index += 5;
                }
                if stream.index == data.len() {
                    *slot = None;
                    shared.dac_streams.fetch_sub(1, Ordering::AcqRel);
                }
            }
        }
    }
    fn sample(&mut self, shared: &Shared) -> [f32; 2] {
        while self
            .heap
            .peek()
            .is_some_and(|event| event.frame <= self.frame)
        {
            let event = self.heap.pop().unwrap();
            shared.pending.fetch_sub(1, Ordering::AcqRel);
            // Frame zero is the explicit ASAP path used by live controls.
            if event.frame != 0 && event.frame < self.frame {
                shared.late.fetch_add(1, Ordering::Relaxed);
            }
            if !shared.valid_event(&event) {
                shared.retire_action(event.action.into_inner());
                continue;
            }
            let wakes_audio = {
                let action = event.action.borrow();
                match &*action {
                    Action::ExtraPsgInstall(_) => false,
                    Action::ExtraPsg(_, command) => command.starts(),
                    Action::ExtraYm2612(_, command) => command.starts(),
                    Action::ExtraYm2203(_, command) => command.starts(),
                    Action::ExtraYm2608(_, command) => command.starts(),
                    Action::ExtraYm2610(_, command) => command.starts(),
                    Action::ExtraYm2151(_, command) => command.starts(),
                    Action::ExtraAy8910(_, command) => command.starts(),
                    Action::ExtraY8950(_, command) => command.starts(),
                    Action::ExtraYmf262(_, command) => command.starts(),
                    Action::ExtraRf5c164(_, command) => command.starts(),
                    Action::ExtraOkim6258(_, command) => command.starts(),
                    Action::ExtraGameboy(_, command) => command.starts(),
                    Action::ExtraHuc6280(_, command) => command.starts(),
                    Action::ExtraDx7(_, command) => command.starts(),
                    Action::ExtraPwm(_, command) => command.starts(),
                    Action::ExtraNes(_, command) => command.starts(),
                    Action::ExtraYm3812(_, command) => command.starts(),
                    Action::ExtraYm3526(_, command) => command.starts(),
                    Action::ExtraYm2413(_, command) => command.starts(),
                    Action::Noise(command) => command.is_some_and(|command| command.starts()),
                    Action::Dx7(command) => command.starts(),
                    Action::Nes(command) => command.starts(),
                    Action::Gameboy(command) => command.starts(),
                    Action::Snes(command) => command.starts(),
                    Action::Huc6280(command) => command.starts(),
                    Action::Ym2608(command) => command.starts(),
                    Action::Ym2203(command) => command.starts(),
                    Action::Ym2610b(command) => command.starts(),
                    Action::X68000(command) => command.starts(),
                    Action::MasterSystem(command) => command.starts(),
                    Action::Msx(command) => command.starts(),
                    Action::Opl(command) => command.starts(),
                    Action::Pwm(command) => command.starts(),
                    Action::Mix(_)
                    | Action::RfMix(_)
                    | Action::RfLoad(_)
                    | Action::RfWriteMemory(..)
                    | Action::RfClearMemory
                    | Action::RfClock(_)
                    | Action::RfReset
                    | Action::Reset
                    | Action::Record(_)
                    | Action::Fx(_)
                    | Action::FxReset(_) => false,
                    _ => true,
                }
            };
            if self.idle && wakes_audio {
                self.idle = false;
                // An already running JS generator owns the audible bus. A
                // first chip note must not fade that existing output to zero.
                if !self.livefx.has_output() {
                    self.resume_remaining = (shared.rate / 200).max(1);
                }
            }
            self.apply(event, shared);
        }
        self.advance_dac(shared);
        let mut pcm = self.core.sample_bus();
        if let Some(dx7) = &mut self.dx7 {
            dx7.mix(&mut pcm);
        }
        if let Some(nes) = &mut self.nes {
            nes.mix(&mut pcm);
        }
        if let Some(chip) = &mut self.gameboy {
            chip.mix(&mut pcm);
        }
        if let Some(chip) = &mut self.snes {
            chip.mix(&mut pcm);
        }
        if let Some(chip) = &mut self.huc6280 {
            chip.mix(&mut pcm);
        }
        if let Some(chip) = &mut self.ym2608 {
            chip.mix(&mut pcm);
        }
        if let Some(chip) = &mut self.ym2203 {
            chip.mix(&mut pcm);
        }
        if let Some(chip) = &mut self.ym2610b {
            chip.mix(&mut pcm);
        }
        if let Some(chip) = &mut self.x68000 {
            chip.mix(&mut pcm);
        }
        if let Some(chip) = &mut self.master_system {
            chip.mix(&mut pcm);
        }
        if let Some(chip) = &mut self.msx {
            chip.mix(&mut pcm);
        }
        if let Some(chip) = &mut self.opl {
            chip.mix(&mut pcm);
        }
        if let Some(rf) = &mut self.rf5c164 {
            rf.mix(&mut pcm);
        }
        if let Some(chip) = &mut self.pwm {
            chip.mix(&mut pcm);
        }
        self.extra_psg.mix(&mut pcm);
        self.noise.mix(&mut pcm, &shared.noise_completed);
        for slot in &mut self.voices {
            if let Some(voice) = slot {
                if !voice.sample(&mut pcm) {
                    let voice = slot.take().unwrap();
                    shared
                        .retired
                        .push(voice.reservation)
                        .unwrap_or_else(|_| unreachable!("Bounded reservation retirement"));
                }
            }
        }
        self.fx.process(&mut pcm);
        pcm = self
            .livefx
            .process(pcm, self.frame, shared.rate, &shared.livefx.transport);
        let master = if self.dx7.is_some()
            || self.nes.is_some()
            || self.gameboy.is_some()
            || self.snes.is_some()
            || self.huc6280.is_some()
            || self.ym2608.is_some()
            || self.ym2203.is_some()
            || self.ym2610b.is_some()
            || self.x68000.is_some()
            || self.master_system.is_some()
            || self.msx.is_some()
            || self.opl.is_some()
        {
            if self.dx7_master_ramp > 0 {
                self.dx7_master +=
                    (self.dx7_master_target - self.dx7_master) / self.dx7_master_ramp as f32;
                self.dx7_master_ramp -= 1;
            }
            self.dx7_master
        } else {
            self.core.master()
        };
        pcm[0] *= master;
        pcm[1] *= master;
        if self.fade_remaining > 0 {
            let gain = self.fade_remaining as f32 / self.fade_frames as f32;
            pcm[0] *= gain;
            pcm[1] *= gain;
            self.fade_remaining -= 1;
            if self.fade_remaining == 0 {
                unsafe {
                    if !self.core.0.is_null() {
                        native_genesis_stop_voices(self.core.0);
                    }
                }
                self.clear_media(shared);
                if let (Some(dx7), Some(state)) = (&mut self.dx7, &shared.dx7) {
                    dx7.stop(state);
                }
                if let (Some(nes), Some(state)) = (&mut self.nes, &shared.nes) {
                    nes.stop(state);
                }
                if let (Some(chip), Some(state)) = (&mut self.gameboy, &shared.gameboy) {
                    chip.stop(state);
                }
                if let (Some(chip), Some(state)) = (&mut self.snes, &shared.snes) {
                    chip.stop(state);
                }
                if let (Some(chip), Some(state)) = (&mut self.huc6280, &shared.huc6280) {
                    chip.stop(state);
                }
                if let (Some(chip), Some(state)) = (&mut self.ym2608, &shared.ym2608) {
                    chip.stop(state);
                }
                if let (Some(chip), Some(state)) = (&mut self.ym2203, &shared.ym2203) {
                    chip.stop(state);
                }
                if let (Some(chip), Some(state)) = (&mut self.ym2610b, &shared.ym2610b) {
                    chip.stop(state);
                }
                if let (Some(chip), Some(state)) = (&mut self.x68000, &shared.x68000) {
                    chip.stop(state);
                }
                if let (Some(chip), Some(state)) = (&mut self.master_system, &shared.master_system)
                {
                    chip.stop(state);
                }
                if let (Some(chip), Some(state)) = (&mut self.msx, &shared.msx) {
                    chip.stop(state);
                }
                if let (Some(chip), Some(state)) = (&mut self.opl, &shared.opl) {
                    chip.stop(state);
                }
                if let (Some(chip), Some(state)) = (&mut self.pwm, &shared.pwm) {
                    chip.stop(state);
                }
                if let Some(rf) = &mut self.rf5c164 {
                    rf.stop(&shared.rf5c164);
                }
                self.extra_psg.clear();
                self.noise.reset(&shared.noise_completed);
                self.fx.stop();
                self.livefx.end_stop();
                self.fm_owners = [None; 6];
                self.fm_key_masks = [0; 6];
                self.fm_mode_register &= 0xc0;
                self.csm_owner = None;
                self.psg_owners = [None; 4];
                self.idle = true;
                shared.stopped_epoch.store(self.epoch, Ordering::Release);
            }
        } else if self.idle && !self.livefx.has_output() {
            pcm = [0.0; 2];
        } else if self.resume_remaining > 0 {
            let total = (shared.rate / 200).max(1);
            let gain = 1.0 - self.resume_remaining as f32 / total as f32;
            pcm[0] *= gain;
            pcm[1] *= gain;
            self.resume_remaining -= 1;
        }
        // Match the device conversion limit; capture the delivered signal.
        pcm[0] = if pcm[0].is_finite() {
            pcm[0].clamp(-1.0, 1.0)
        } else {
            0.0
        };
        pcm[1] = if pcm[1].is_finite() {
            pcm[1].clamp(-1.0, 1.0)
        } else {
            0.0
        };
        if let Some(capture) = &self.record {
            if !capture.sample(self.frame, pcm) {
                let capture = self.record.take().unwrap();
                shared
                    .recorded
                    .push(capture)
                    .unwrap_or_else(|_| unreachable!("Only one capture may be active"));
            }
        }
        shared
            .monitor
            .write(&mut self.monitor_writer, self.frame, pcm);
        self.frame += 1;
        pcm
    }
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    session: u64,
    mode: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    engine: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rf5c164_clock: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    nes_clock: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gameboy_clock: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    snes_clock: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    huc6280_clock: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ym2608_clock: Option<u32>,
    ym2203_clock: Option<u32>,
    ym2610b_clock: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ym2151_clock: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    okim6258_clock: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ym2413_clock: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ay8910_clock: Option<u32>,
    y8950_clock: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pwm_clock: Option<u32>,
    sample_rate: u32,
    frame: u64,
    ym_clock: u32,
    psg_clock: u32,
    running: bool,
    queued: usize,
    late_commands: u64,
    queue_overflows: u64,
    live_fx: livefx::Status,
    error: Option<String>,
}
struct Running {
    shared: Arc<Shared>,
    stop: mpsc::Sender<()>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.shared.stop();
        let _ = self.stop.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
#[derive(Clone, Default)]
pub struct NativeGenesis {
    running: Arc<Mutex<Option<Running>>>,
}
impl NativeGenesis {
    #[cfg(test)]
    fn start(&self) -> Result<Status, String> {
        self.replace_running(Self::launch)
    }
    fn mode_config(mode: Option<&str>) -> Result<(bool, bool, bool, Option<&'static str>), String> {
        Ok(match mode.unwrap_or("genesis") {
            "genesis" => (false, false, false, None),
            "32x" => (true, false, false, Some("32x")),
            "genesis-full-nuked" => (true,false,false,Some("32x-nuked")),
            "genesis-nuked" => (false,false,false,Some("genesis-nuked")),
            "mega-cd" => (true, false, false, None),
            "dx7" => (false, true, false, None),
            "nes" => (false, false, true, None),
            "gameboy" => (false,false,false,Some("gameboy")),
            "snes" => (false,false,false,Some("snes")),
            "pc-engine" | "huc6280" => (false,false,false,Some("huc6280")),
            "pc98" | "ym2608" => (false,false,false,Some("ym2608")),
            "pc98-26" | "ym2203" => (false,false,false,Some("ym2203")),
            "neogeo" | "ym2610" => (false,false,false,Some("ym2610")),
            "neogeo-b" | "ym2610b" => (false,false,false,Some("ym2610b")),
            "x68000" => (false,false,false,Some("x68000")),
            "master-system" => (false,false,false,Some("master-system")),
            "game-gear" => (false,false,false,Some("game-gear")),
            "msx" => (false,false,false,Some("msx")),
            "msx-audio" => (false,false,false,Some("msx-audio")),
            "ym3526" => (false,false,false,Some("ym3526")),
            "ym3812" => (false,false,false,Some("ym3812")),
            "ymf262" => (false,false,false,Some("ymf262")),
            _ => return Err("Native output supports Genesis, Mega CD, 32X, 32X Nuked, Genesis Nuked, DX7, NES, Game Boy, SNES, PC Engine, PC98, PC98-26, Neo Geo, X68000, Master System, Game Gear, MSX, MSX Audio and DOS PC modes".into()),
        })
    }
    fn start_mode(&self, mode: Option<&str>) -> Result<Status, String> {
        let (mega_cd, dx7, nes, console) = Self::mode_config(mode)?;
        self.replace_running(|session| Self::launch_config(session, mega_cd, dx7, nes, console))
    }
    fn replace_running(
        &self,
        launch: impl FnOnce(u64) -> Result<Running, String>,
    ) -> Result<Status, String> {
        let mut guard = self
            .running
            .lock()
            .map_err(|_| "Native audio state is unavailable")?;
        // An explicit start always establishes new page ownership. A delayed
        // pagehide shutdown from the old page cannot close this new session.
        drop(guard.take());
        let session = NEXT_SESSION.fetch_add(1, Ordering::Relaxed);
        let running = launch(session)?;
        let status = running.shared.status();
        *guard = Some(running);
        Ok(status)
    }
    #[cfg(test)]
    fn launch(session: u64) -> Result<Running, String> {
        Self::launch_mode(session, false)
    }
    #[cfg(test)]
    fn launch_mode(session: u64, mega_cd: bool) -> Result<Running, String> {
        Self::launch_primary(session, mega_cd, false)
    }
    #[cfg(test)]
    fn launch_primary(session: u64, mega_cd: bool, dx7: bool) -> Result<Running, String> {
        Self::launch_config(session, mega_cd, dx7, false, None)
    }
    fn launch_config(
        session: u64,
        mega_cd: bool,
        dx7: bool,
        nes: bool,
        console: Option<&'static str>,
    ) -> Result<Running, String> {
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (stop, stopped) = mpsc::channel();
        let thread =
            std::thread::spawn(
                move || match start_device(session, mega_cd, dx7, nes, console) {
                    Ok((stream, shared)) => {
                        let _ = ready_tx.send(Ok(shared));
                        let _ = stopped.recv();
                        drop(stream);
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                    }
                },
            );
        match ready_rx.recv().map_err(|e| e.to_string())? {
            Ok(shared) => Ok(Running {
                shared,
                stop,
                thread: Some(thread),
            }),
            Err(error) => {
                let _ = thread.join();
                Err(error)
            }
        }
    }
    fn session_shared(&self, session: u64) -> Result<Arc<Shared>, String> {
        let guard = self
            .running
            .lock()
            .map_err(|_| "Native audio state is unavailable")?;
        let shared = &guard
            .as_ref()
            .ok_or("Native Genesis output is not running")?
            .shared;
        shared.check_session(session)?;
        Ok(shared.clone())
    }
    fn with_session<T>(
        &self,
        session: u64,
        f: impl FnOnce(&Shared) -> Result<T, String>,
    ) -> Result<T, String> {
        let guard = self
            .running
            .lock()
            .map_err(|_| "Native audio state is unavailable")?;
        let shared = &guard
            .as_ref()
            .ok_or("Native Genesis output is not running")?
            .shared;
        shared.check_session(session)?;
        f(shared)
    }
    pub fn shutdown(&self) {
        if let Ok(mut guard) = self.running.lock() {
            guard.take();
        }
    }
    fn shutdown_session(&self, session: u64) -> Result<(), String> {
        let mut guard = self
            .running
            .lock()
            .map_err(|_| "Native audio state is unavailable")?;
        if let Some(running) = guard.as_ref() {
            if running.shared.session != session {
                return Err("Native Genesis session has expired".into());
            }
        }
        guard.take();
        Ok(())
    }
}

fn start_device(
    session: u64,
    mega_cd: bool,
    dx7: bool,
    nes: bool,
    console: Option<&'static str>,
) -> Result<(cpal::Stream, Arc<Shared>), String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let device = cpal::default_host()
        .default_output_device()
        .ok_or("No default native audio output device")?;
    let supported = device
        .default_output_config()
        .map_err(|e| format!("Native audio configuration failed: {e}"))?;
    let config: cpal::StreamConfig = supported.clone().into();
    let shared = Arc::new(Shared::new_modes(
        config.sample_rate.0,
        session,
        mega_cd,
        dx7,
        nes,
        console,
    ));
    let stream = match supported.sample_format() {
        cpal::SampleFormat::F32 => build_stream::<f32>(&device, &config, shared.clone()),
        cpal::SampleFormat::I16 => build_stream::<i16>(&device, &config, shared.clone()),
        cpal::SampleFormat::U16 => build_stream::<u16>(&device, &config, shared.clone()),
        format => Err(format!(
            "Unsupported native output sample format: {format:?}"
        )),
    }?;
    stream
        .play()
        .map_err(|e| format!("Native audio output could not start: {e}"))?;
    Ok((stream, shared))
}
fn build_stream<T: cpal::SizedSample + cpal::FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    shared: Arc<Shared>,
) -> Result<cpal::Stream, String> {
    use cpal::traits::DeviceTrait;
    let mut renderer = Renderer::new_modes(
        config.sample_rate.0,
        shared.mega_cd,
        shared.dx7.is_some(),
        shared.nes.is_some(),
        if shared.pwm.is_some() {
            Some(if shared.nuked { "32x-nuked" } else { "32x" })
        } else if shared.nuked {
            Some("genesis-nuked")
        } else if shared.gameboy.is_some() {
            Some("gameboy")
        } else if shared.snes.is_some() {
            Some("snes")
        } else if let Some(chip) = &shared.ym2610b {
            Some(chip.variant())
        } else if shared.msx.is_some() {
            Some(
                if shared
                    .opl
                    .as_ref()
                    .is_some_and(|chip| chip.mode() == "y8950")
                {
                    "msx-audio"
                } else {
                    "msx"
                },
            )
        } else if let Some(chip) = &shared.opl {
            Some(chip.mode())
        } else if let Some(chip) = &shared.master_system {
            Some(chip.mode())
        } else if shared.x68000.is_some() {
            Some("x68000")
        } else if shared.ym2608.is_some() {
            Some("ym2608")
        } else if shared.ym2203.is_some() {
            Some("ym2203")
        } else if shared.huc6280.is_some() {
            Some("huc6280")
        } else {
            None
        },
    )?;
    // Publish the actual initialized core before start() exposes the session.
    renderer.publish_primary(&shared);
    let channels = config.channels as usize;
    if channels == 0 {
        return Err("Native audio device has no output channels".into());
    }
    let errors = shared.clone();
    device
        .build_output_stream(
            config,
            move |buffer: &mut [T], _| {
                renderer.prepare(&shared);
                for frame in buffer.chunks_mut(channels) {
                    let pcm = renderer.sample(&shared);
                    for (channel, sample) in frame.iter_mut().enumerate() {
                        let value = if channels == 1 {
                            (pcm[0] + pcm[1]) * 0.5
                        } else if channel < 2 {
                            pcm[channel]
                        } else {
                            0.0
                        };
                        *sample = T::from_sample(value.clamp(-1.0, 1.0));
                    }
                }
                shared.frame.store(renderer.frame, Ordering::Release);
                renderer.publish_primary(&shared);
            },
            move |_| {
                errors.failed.store(true, Ordering::Release);
            },
            None,
        )
        .map_err(|e| format!("Native audio stream creation failed: {e}"))
}

#[tauri::command]
pub async fn native_genesis_start(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    mode: Option<String>,
) -> Result<Status, String> {
    crate::allowed(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.start_mode(mode.as_deref()))
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn native_genesis_nes_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    after: Option<u64>,
) -> Result<nes::Status, String> {
    crate::allowed(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_session(session, |shared| {
            let nes = shared.nes.as_ref().ok_or("Native NES requires NES mode")?;
            if let Some(id) = after {
                nes.wait_applied(id)?;
            }
            nes.status(shared.frame.load(Ordering::Acquire))
        })
    })
    .await
    .map_err(|error| error.to_string())?
}
#[tauri::command]
pub async fn native_genesis_gameboy_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    after: Option<u64>,
) -> Result<gameboy::Status, String> {
    crate::allowed(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_session(session, |shared| {
            let chip = shared
                .gameboy
                .as_ref()
                .ok_or("Native gameboy requires gameboy mode")?;
            if let Some(id) = after {
                chip.wait_applied(id)?;
            }
            chip.status(shared.frame.load(Ordering::Acquire))
        })
    })
    .await
    .map_err(|error| error.to_string())?
}
#[tauri::command]
pub async fn native_genesis_snes_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    after: Option<u64>,
    ram: Option<bool>,
) -> Result<snes::Status, String> {
    crate::allowed(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_session(session, |shared| {
            let chip = shared
                .snes
                .as_ref()
                .ok_or("Native snes requires snes mode")?;
            if let Some(id) = after {
                chip.wait_applied(id)?;
            }
            if ram.unwrap_or(false) {
                chip.status_with_ram(shared.frame.load(Ordering::Acquire))
            } else {
                chip.status(shared.frame.load(Ordering::Acquire))
            }
        })
    })
    .await
    .map_err(|error| error.to_string())?
}
#[tauri::command]
pub async fn native_genesis_huc6280_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    after: Option<u64>,
) -> Result<huc6280::Status, String> {
    crate::allowed(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_session(session, |shared| {
            let chip = shared
                .huc6280
                .as_ref()
                .ok_or("Native huc6280 requires huc6280 mode")?;
            if let Some(id) = after {
                chip.wait_applied(id)?;
            }
            chip.status(shared.frame.load(Ordering::Acquire))
        })
    })
    .await
    .map_err(|error| error.to_string())?
}
#[tauri::command]
pub async fn native_genesis_ym2608_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    after: Option<u64>,
) -> Result<ym2608::Status, String> {
    crate::allowed(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_session(session, |shared| {
            let chip = shared
                .ym2608
                .as_ref()
                .ok_or("Native ym2608 requires ym2608 mode")?;
            if let Some(id) = after {
                chip.wait_applied(id)?;
            }
            chip.status(shared.frame.load(Ordering::Acquire))
        })
    })
    .await
    .map_err(|error| error.to_string())?
}
#[tauri::command]
pub async fn native_genesis_ym2203_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    after: Option<u64>,
) -> Result<ym2203::Status, String> {
    crate::allowed(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_session(session, |shared| {
            let chip = shared
                .ym2203
                .as_ref()
                .ok_or("Native YM2203 status requires PC98-26 mode")?;
            if let Some(id) = after {
                chip.wait_applied(id)?;
            }
            chip.status(shared.frame.load(Ordering::Acquire))
        })
    })
    .await
    .map_err(|error| error.to_string())?
}
#[tauri::command]
pub async fn native_genesis_ym2610b_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    after: Option<u64>,
) -> Result<ym2610b::Status, String> {
    crate::allowed(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_session(session, |shared| {
            let chip = shared
                .ym2610b
                .as_ref()
                .ok_or("Native ym2610b requires ym2610b mode")?;
            if let Some(id) = after {
                chip.wait_applied(id)?;
            }
            chip.status(shared.frame.load(Ordering::Acquire))
        })
    })
    .await
    .map_err(|error| error.to_string())?
}
#[tauri::command]
pub async fn native_genesis_x68000_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    after: Option<u64>,
) -> Result<x68000::Status, String> {
    crate::allowed(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_session(session, |shared| {
            let chip = shared
                .x68000
                .as_ref()
                .ok_or("Native X68000 status requires X68000 mode")?;
            if let Some(id) = after {
                chip.wait_applied(id)?;
            }
            chip.status(shared.frame.load(Ordering::Acquire))
        })
    })
    .await
    .map_err(|error| error.to_string())?
}
#[tauri::command]
pub async fn native_genesis_master_system_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    after: Option<u64>,
) -> Result<master_system::Status, String> {
    crate::allowed(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_session(session, |shared| {
            let chip = shared
                .master_system
                .as_ref()
                .filter(|chip| !chip.is_game_gear())
                .ok_or("Native Master System status requires Master System mode")?;
            if let Some(id) = after {
                chip.wait_applied(id)?;
            }
            chip.status(shared.frame.load(Ordering::Acquire))
        })
    })
    .await
    .map_err(|error| error.to_string())?
}
#[tauri::command]
pub fn native_genesis_extra_psg_create(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
    clock: Option<u32>,
) -> Result<master_system::GameGearStatus, String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| shared.create_extra_psg(instance, clock))
}
#[tauri::command]
pub fn native_genesis_extra_psg_dispose(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
) -> Result<(), String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| shared.dispose_extra_psg(instance))
}
#[tauri::command]
pub async fn native_genesis_extra_psg_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
    after: Option<u64>,
) -> Result<master_system::GameGearStatus, String> {
    crate::allowed(&window)?;
    let shared = state.session_shared(session)?;
    let status =
        tauri::async_runtime::spawn_blocking(move || shared.extra_psg_status(instance, after))
            .await
            .map_err(|error| error.to_string())??;
    state.with_session(session, |_| Ok(()))?;
    Ok(status)
}
#[tauri::command]
pub fn native_genesis_extra_ym2612_create(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
    clock: Option<u32>,
) -> Result<ym2612::Status, String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| {
        shared.create_extra_ym2612(instance, clock)
    })
}
#[tauri::command]
pub fn native_genesis_extra_ym2612_dispose(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
) -> Result<(), String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| shared.dispose_extra_ym2612(instance))
}
#[tauri::command]
pub async fn native_genesis_extra_ym2612_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
    after: Option<u64>,
) -> Result<ym2612::Status, String> {
    crate::allowed(&window)?;
    let shared = state.session_shared(session)?;
    let status =
        tauri::async_runtime::spawn_blocking(move || shared.extra_ym2612_status(instance, after))
            .await
            .map_err(|error| error.to_string())??;
    state.with_session(session, |_| Ok(()))?;
    Ok(status)
}
#[tauri::command]
pub fn native_genesis_extra_ym2203_create(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
    clock: Option<u32>,
) -> Result<ym2203::AdditionalStatus, String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| {
        shared.create_extra_ym2203(instance, clock)
    })
}
#[tauri::command]
pub fn native_genesis_extra_ym2608_create(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
    clock: Option<u32>,
) -> Result<extra_ym2608::AdditionalStatus, String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| {
        shared.create_extra_ym2608(instance, clock)
    })
}
#[tauri::command]
pub fn native_genesis_extra_ym2610_create(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
    clock: Option<u32>,
) -> Result<extra_ym2610::AdditionalStatus, String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| {
        shared.create_extra_ym2610(instance, clock)
    })
}
#[tauri::command]
pub fn native_genesis_extra_ym2151_create(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
    clock: Option<u32>,
) -> Result<extra_ym2151::AdditionalStatus, String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| {
        shared.create_extra_ym2151(instance, clock)
    })
}
#[tauri::command]
pub fn native_genesis_extra_ay8910_create(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
    clock: Option<u32>,
) -> Result<extra_msx::Ay8910Status, String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| {
        shared.create_extra_ay8910(instance, clock)
    })
}
#[tauri::command]
pub fn native_genesis_extra_ym2413_create(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
    clock: Option<u32>,
) -> Result<extra_msx::Ym2413Status, String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| {
        shared.create_extra_ym2413(instance, clock)
    })
}
#[tauri::command]
pub fn native_genesis_extra_ym2203_dispose(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
) -> Result<(), String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| shared.dispose_extra_ym2203(instance))
}
#[tauri::command]
pub fn native_genesis_extra_ym2608_dispose(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
) -> Result<(), String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| shared.dispose_extra_ym2608(instance))
}
#[tauri::command]
pub fn native_genesis_extra_ym2610_dispose(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
) -> Result<(), String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| shared.dispose_extra_ym2610(instance))
}
#[tauri::command]
pub fn native_genesis_extra_ym2151_dispose(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
) -> Result<(), String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| shared.dispose_extra_ym2151(instance))
}
#[tauri::command]
pub fn native_genesis_extra_ay8910_dispose(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
) -> Result<(), String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| shared.dispose_extra_ay8910(instance))
}
#[tauri::command]
pub fn native_genesis_extra_ym2413_dispose(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
) -> Result<(), String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| shared.dispose_extra_ym2413(instance))
}
#[tauri::command]
pub async fn native_genesis_extra_ym2203_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
    after: Option<u64>,
) -> Result<ym2203::AdditionalStatus, String> {
    crate::allowed(&window)?;
    let shared = state.session_shared(session)?;
    let status =
        tauri::async_runtime::spawn_blocking(move || shared.extra_ym2203_status(instance, after))
            .await
            .map_err(|error| error.to_string())??;
    state.with_session(session, |_| Ok(()))?;
    Ok(status)
}
#[tauri::command]
pub async fn native_genesis_extra_ym2608_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
    after: Option<u64>,
) -> Result<extra_ym2608::AdditionalStatus, String> {
    crate::allowed(&window)?;
    let shared = state.session_shared(session)?;
    let status =
        tauri::async_runtime::spawn_blocking(move || shared.extra_ym2608_status(instance, after))
            .await
            .map_err(|error| error.to_string())??;
    state.with_session(session, |_| Ok(()))?;
    Ok(status)
}
#[tauri::command]
pub async fn native_genesis_extra_ym2610_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
    after: Option<u64>,
) -> Result<extra_ym2610::AdditionalStatus, String> {
    crate::allowed(&window)?;
    let shared = state.session_shared(session)?;
    let status =
        tauri::async_runtime::spawn_blocking(move || shared.extra_ym2610_status(instance, after))
            .await
            .map_err(|error| error.to_string())??;
    state.with_session(session, |_| Ok(()))?;
    Ok(status)
}
#[tauri::command]
pub async fn native_genesis_extra_ym2151_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
    after: Option<u64>,
) -> Result<extra_ym2151::AdditionalStatus, String> {
    crate::allowed(&window)?;
    let shared = state.session_shared(session)?;
    let status =
        tauri::async_runtime::spawn_blocking(move || shared.extra_ym2151_status(instance, after))
            .await
            .map_err(|error| error.to_string())??;
    state.with_session(session, |_| Ok(()))?;
    Ok(status)
}
#[tauri::command]
pub async fn native_genesis_extra_ay8910_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
    after: Option<u64>,
) -> Result<extra_msx::Ay8910Status, String> {
    crate::allowed(&window)?;
    let shared = state.session_shared(session)?;
    let status =
        tauri::async_runtime::spawn_blocking(move || shared.extra_ay8910_status(instance, after))
            .await
            .map_err(|error| error.to_string())??;
    state.with_session(session, |_| Ok(()))?;
    Ok(status)
}
#[tauri::command]
pub async fn native_genesis_extra_ym2413_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
    after: Option<u64>,
    after_layout_revision: Option<u64>,
) -> Result<extra_msx::Ym2413Status, String> {
    crate::allowed(&window)?;
    let shared = state.session_shared(session)?;
    let status = tauri::async_runtime::spawn_blocking(move || {
        shared.extra_ym2413_status_layout(instance, after, after_layout_revision)
    })
    .await
    .map_err(|error| error.to_string())??;
    state.with_session(session, |_| Ok(()))?;
    Ok(status)
}
#[tauri::command]
pub async fn native_genesis_game_gear_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    after: Option<u64>,
) -> Result<master_system::GameGearStatus, String> {
    crate::allowed(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_session(session, |shared| {
            let chip = shared
                .master_system
                .as_ref()
                .filter(|chip| chip.is_game_gear())
                .ok_or("Native Game Gear status requires Game Gear mode")?;
            if let Some(id) = after {
                chip.wait_applied(id)?;
            }
            chip.game_gear_status(shared.frame.load(Ordering::Acquire))
        })
    })
    .await
    .map_err(|error| error.to_string())?
}
#[tauri::command]
pub async fn native_genesis_msx_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    after: Option<u64>,
) -> Result<msx::Status, String> {
    crate::allowed(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_session(session, |shared| {
            let chip = shared
                .msx
                .as_ref()
                .ok_or("Native MSX status requires MSX mode")?;
            if let Some(id) = after {
                chip.wait_applied(id)?;
            }
            chip.status(shared.frame.load(Ordering::Acquire))
        })
    })
    .await
    .map_err(|error| error.to_string())?
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MsxAudioStatus {
    pub y8950: opl::ChipStatus,
    pub ym2413: msx::OpllStatus,
    pub ay8910: msx::AyStatus,
    pub frame: u64,
    pub sample_rate: u32,
    pub revision: u64,
}
#[tauri::command]
pub async fn native_genesis_msx_audio_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
) -> Result<MsxAudioStatus, String> {
    crate::allowed(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_session(session, |shared| shared.msx_audio_status())
    })
    .await
    .map_err(|error| error.to_string())?
}
#[tauri::command]
pub async fn native_genesis_opl_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    after: Option<u64>,
) -> Result<opl::Status, String> {
    crate::allowed(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_session(session, |shared| {
            let chip = shared
                .opl
                .as_ref()
                .ok_or("Native OPL status requires DOS PC mode")?;
            if let Some(id) = after {
                chip.wait_applied(id)?;
            }
            chip.status(shared.frame.load(Ordering::Acquire))
        })
    })
    .await
    .map_err(|error| error.to_string())?
}
#[tauri::command]
pub async fn native_genesis_pwm_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    after: Option<u64>,
) -> Result<pwm::Status, String> {
    crate::allowed(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_session(session, |shared| {
            let chip = shared.pwm.as_ref().ok_or("Native PWM requires 32X mode")?;
            if let Some(id) = after {
                chip.wait_applied(id)?;
            }
            chip.status(shared.frame.load(Ordering::Acquire))
        })
    })
    .await
    .map_err(|error| error.to_string())?
}
#[tauri::command]
pub fn native_genesis_dx7_status(
    window: tauri::WebviewWindow,
    state: tauri::State<NativeGenesis>,
    session: u64,
) -> Result<dx7::Status, String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| {
        shared
            .dx7
            .as_ref()
            .ok_or("Native DX7 requires DX7 mode")?
            .status(shared.frame.load(Ordering::Acquire))
    })
}
#[tauri::command]
pub fn native_genesis_rf5c164_status(
    window: tauri::WebviewWindow,
    state: tauri::State<NativeGenesis>,
    session: u64,
) -> Result<rf5c164::Status, String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| {
        if !shared.mega_cd {
            return Err("Native RF5C164 requires Mega CD mode".into());
        }
        shared
            .rf5c164
            .status(shared.rate, shared.frame.load(Ordering::Acquire))
    })
}
#[tauri::command]
pub fn native_genesis_status(
    window: tauri::WebviewWindow,
    state: tauri::State<NativeGenesis>,
    session: u64,
) -> Result<Status, String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| Ok(shared.status()))
}
#[tauri::command]
pub fn native_genesis_commands(
    window: tauri::WebviewWindow,
    state: tauri::State<NativeGenesis>,
    session: u64,
    commands: Vec<Command>,
) -> Result<Status, String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| {
        shared.enqueue(session, &commands)?;
        Ok(shared.status())
    })
}
#[tauri::command]
pub async fn native_genesis_stop(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
) -> Result<Status, String> {
    crate::allowed(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.with_session(session, Shared::stop))
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn native_genesis_shutdown(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
) -> Result<(), String> {
    crate::allowed(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.shutdown_session(session))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn native_genesis_media_load(
    window: tauri::WebviewWindow,
    state: tauri::State<NativeGenesis>,
    session: u64,
    kind: media::Kind,
    name: String,
    sample_rate: f64,
    channels: Vec<Vec<f32>>,
) -> Result<Status, String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| {
        shared.load_media(kind, name, sample_rate, channels)
    })
}
#[tauri::command]
pub fn native_genesis_media_remove(
    window: tauri::WebviewWindow,
    state: tauri::State<NativeGenesis>,
    session: u64,
    kind: media::Kind,
    name: String,
) -> Result<Status, String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| shared.remove_media(kind, &name))
}
#[tauri::command]
pub fn native_genesis_record_start(
    window: tauri::WebviewWindow,
    state: tauri::State<NativeGenesis>,
    session: u64,
    seconds: f64,
) -> Result<Status, String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| shared.record_start(seconds))
}
#[tauri::command]
pub async fn native_genesis_record_stop(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
) -> Result<recording::Recorded, String> {
    crate::allowed(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.with_session(session, Shared::record_stop))
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
pub fn native_genesis_performance(
    window: tauri::WebviewWindow,
    state: tauri::State<NativeGenesis>,
    session: u64,
    op: String,
    performance: u64,
    owner: Option<u32>,
) -> Result<Status, String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| {
        shared.performance(&op, performance, owner)
    })
}

#[tauri::command]
pub fn native_genesis_fx(
    window: tauri::WebviewWindow,
    state: tauri::State<NativeGenesis>,
    session: u64,
    message: FxMessage,
    frame: Option<u64>,
    performance: Option<u64>,
    owner: Option<u32>,
) -> Result<Status, String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| {
        let mut sequence = shared
            .producer
            .lock()
            .map_err(|_| "Native producer state is unavailable")?;
        let frame = frame.unwrap_or(0);
        if frame > MAX_SAFE_INTEGER {
            return Err("Invalid native FX frame".into());
        }
        let tag = Command {
            frame,
            performance,
            owner,
            action: CommandAction::Reset,
        }
        .owner(shared)?;
        if shared.pending.load(Ordering::Acquire) >= CAPACITY {
            return Err("Native command queue is full".into());
        }
        let mut fx = shared
            .fx
            .lock()
            .map_err(|_| "Native FX state is unavailable")?;
        fx.collect();
        let reset_livefx = matches!(&message, FxMessage::Reset);
        if reset_livefx && frame == 0 {
            shared.livefx.reset();
        }
        if let Some(action) = fx.command(message, shared.rate)? {
            shared.pending.fetch_add(1, Ordering::AcqRel);
            let event = Event {
                frame,
                sequence: *sequence,
                epoch: shared.epoch.load(Ordering::Acquire),
                owner: tag,
                action: RefCell::new(if reset_livefx && frame != 0 {
                    Action::FxReset(action)
                } else {
                    Action::Fx(action)
                }),
            };
            *sequence += 1;
            shared
                .queue
                .push(event)
                .unwrap_or_else(|_| unreachable!("Reserved native FX event capacity"));
        }
        Ok(shared.status())
    })
}

#[tauri::command]
pub fn native_genesis_live_fx(
    window: tauri::WebviewWindow,
    state: tauri::State<NativeGenesis>,
    session: u64,
    message: serde_json::Value,
    frame: Option<u64>,
    performance: Option<u64>,
    owner: Option<u32>,
) -> Result<Status, String> {
    crate::allowed(&window)?;
    let shared = state.session_shared(session)?;
    let _producer = shared
        .producer
        .lock()
        .map_err(|_| "Native producer state is unavailable")?;
    let frame = frame.unwrap_or(0);
    let tag = Command {
        frame,
        performance,
        owner,
        action: CommandAction::Reset,
    }
    .owner(&shared)?;
    shared.livefx.command(&shared, message, frame, tag)?;
    Ok(shared.status())
}

#[tauri::command]
pub async fn native_genesis_monitor(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    mode: monitor::Mode,
    channel: Option<usize>,
) -> Result<Option<monitor::Snapshot>, String> {
    crate::allowed(&window)?;
    let state = state.inner().clone();
    let shared = state.session_shared(session)?;
    let channel = channel.unwrap_or(0);
    if channel > 1 {
        return Err("Native monitor channel must be 0 or 1".into());
    }
    if mode == monitor::Mode::Off {
        shared.monitor.off();
        return Ok(None);
    }
    let expected = shared.monitor.generation();
    if !shared.monitor.try_reserve_read() {
        return Ok(None);
    }
    // Snapshot/FFT work neither holds the running-session mutex nor executes
    // on the device or WebView/UI thread. At most one read enters the FFT.
    let result = tauri::async_runtime::spawn_blocking(move || {
        shared.monitor.read_reserved(
            mode,
            channel,
            session,
            shared.frame.load(Ordering::Acquire),
            expected,
        )
    })
    .await
    .map_err(|error| error.to_string())??;
    state.with_session(session, |_| Ok(()))?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn command(frame: u64, action: CommandAction) -> Command {
        Command {
            frame,
            action,
            performance: None,
            owner: None,
        }
    }
    fn render(renderer: &mut Renderer, shared: &Shared, count: usize) -> Vec<[f32; 2]> {
        renderer.prepare(shared);
        (0..count).map(|_| renderer.sample(shared)).collect()
    }
    fn tone(frame: u64) -> Vec<Command> {
        vec![
            command(frame, CommandAction::PsgWrite { value: 0x8e }),
            command(frame, CommandAction::PsgWrite { value: 0x0f }),
            command(frame, CommandAction::PsgWrite { value: 0x94 }),
        ]
    }
    fn fm_patch(core: &mut Core) {
        for slot in [0, 4, 8, 12] {
            core.apply(Action::Write(0, 0x30 + slot, 1));
            core.apply(Action::Write(
                0,
                0x40 + slot,
                if slot == 12 { 20 } else { 127 },
            ));
            core.apply(Action::Write(0, 0x50 + slot, 31));
            core.apply(Action::Write(0, 0x60 + slot, 0));
            core.apply(Action::Write(0, 0x70 + slot, 0));
            core.apply(Action::Write(0, 0x80 + slot, 15));
        }
        core.apply(Action::Write(0, 0xb0, 7));
        core.apply(Action::Write(0, 0xb4, 0xc0));
        core.apply(Action::Write(0, 0xa4, 0x22));
        core.apply(Action::Write(0, 0xa0, 0x84));
    }
    #[test]
    fn native_idle_signal_reset_and_stereo_are_finite() {
        for rate in [44100, 48000, 96000] {
            let shared = Shared::new(rate, 1);
            let mut renderer = Renderer::new(rate).unwrap();
            assert!(render(&mut renderer, &shared, 256)
                .iter()
                .all(|v| *v == [0.0; 2]));
            shared.enqueue(1, &tone(256)).unwrap();
            let signal = render(&mut renderer, &shared, 4096);
            assert!(signal
                .iter()
                .flatten()
                .all(|v| v.is_finite() && v.abs() <= 1.0));
            assert!(signal.iter().any(|v| v[0].abs() > 0.01));
            assert!(signal.iter().all(|v| v[0] == v[1]));
            renderer.core.apply(Action::Reset);
            assert!(render(&mut renderer, &shared, 256)
                .iter()
                .all(|v| *v == [0.0; 2]));
        }
    }
    #[test]
    fn scheduling_is_chunk_invariant_and_keeps_equal_time_order() {
        let shared_a = Shared::new(48000, 1);
        let shared_b = Shared::new(48000, 2);
        let mut a = Renderer::new(48000).unwrap();
        let mut b = Renderer::new(48000).unwrap();
        let mut commands = tone(200);
        commands.push(command(1200, CommandAction::PsgWrite { value: 0x9f }));
        // Immediate commands are allowed after already submitted future events.
        commands.push(command(0, CommandAction::PsgStereo { value: 0x10 }));
        shared_a.enqueue(1, &commands).unwrap();
        shared_b.enqueue(2, &commands).unwrap();
        let expected = render(&mut a, &shared_a, 2000);
        let mut actual = Vec::new();
        for count in [1, 127, 53, 512, 31, 1276] {
            actual.extend(render(&mut b, &shared_b, count));
        }
        assert_eq!(actual, expected);
        assert!(actual[..200].iter().all(|v| *v == [0.0; 2]));
        assert!(actual[500..1200].iter().any(|v| v[0] != 0.0));
        assert!(actual.iter().all(|v| v[1] == 0.0));
        assert_eq!(shared_a.pending.load(Ordering::Relaxed), 0);
    }
    #[test]
    fn admission_is_bounded_atomic_and_validates_before_mutation() {
        let shared = Shared::new(48000, 8);
        let batch: Vec<_> = (0..MAX_BATCH)
            .map(|_| command(48000, CommandAction::Reset))
            .collect();
        for _ in 0..(CAPACITY / MAX_BATCH) {
            shared.enqueue(8, &batch).unwrap();
        }
        assert!(shared.enqueue(8, &tone(0)).is_err());
        assert_eq!(shared.pending.load(Ordering::Relaxed), CAPACITY);
        assert_eq!(shared.queue.len(), CAPACITY);
        assert_eq!(shared.overflows.load(Ordering::Relaxed), 1);
        let mut renderer = Renderer::new(48000).unwrap();
        let capacity = renderer.heap.capacity();
        for _ in 0..(CAPACITY / MAX_BATCH) {
            renderer.prepare(&shared);
        }
        assert_eq!(renderer.heap.len(), CAPACITY);
        assert_eq!(renderer.heap.capacity(), capacity);
        assert!(shared.queue.is_empty());
        assert_eq!(shared.pending.load(Ordering::Acquire), CAPACITY);
        shared.epoch.store(1, Ordering::Release);
        renderer.prepare(&shared);
        assert!(renderer.heap.is_empty());
        assert_eq!(renderer.heap.capacity(), capacity);
        assert_eq!(shared.pending.load(Ordering::Acquire), 0);
        assert!(shared.enqueue(7, &[]).unwrap_err().contains("expired"));
        let other = Shared::new(48000, 9);
        let invalid = vec![
            command(0, CommandAction::Reset),
            command(
                0,
                CommandAction::Write {
                    port: 2,
                    register: 0,
                    value: 0,
                },
            ),
        ];
        assert!(other.enqueue(9, &invalid).is_err());
        assert_eq!(other.queue.len(), 0);
        assert_eq!(other.pending.load(Ordering::Relaxed), 0);
    }
    #[test]
    fn stop_cancels_future_notes_fades_resets_and_can_resume() {
        let shared = Shared::new(96000, 1);
        let mut renderer = Renderer::new(96000).unwrap();
        let mut commands = tone(0);
        commands.extend(tone(3000));
        shared.enqueue(1, &commands).unwrap();
        assert!(render(&mut renderer, &shared, 1000)
            .iter()
            .any(|v| v[0] != 0.0));
        shared.epoch.store(1, Ordering::Release);
        let stopped = render(&mut renderer, &shared, 3000);
        assert!(stopped[960..].iter().all(|v| *v == [0.0; 2]));
        assert_eq!(shared.stopped_epoch.load(Ordering::Acquire), 1);
        assert_eq!(shared.pending.load(Ordering::Relaxed), 0);
        shared.enqueue(1, &tone(4000)).unwrap();
        assert!(render(&mut renderer, &shared, 1000)
            .iter()
            .any(|v| v[0] != 0.0));
        assert_eq!(renderer.frame, 5000);
    }
    #[test]
    fn future_events_are_applied_in_timestamp_then_submission_order() {
        let shared = Shared::new(48000, 1);
        let mut renderer = Renderer::new(48000).unwrap();
        shared.enqueue(1, &tone(100)).unwrap();
        shared
            .enqueue(1, &[command(100, CommandAction::PsgWrite { value: 0x9f })])
            .unwrap();
        let output = render(&mut renderer, &shared, 1024);
        assert!(output.iter().all(|v| *v == [0.0; 2]));
        assert_eq!(shared.late.load(Ordering::Relaxed), 0);
    }
    #[test]
    fn ffi_core_render_matches_fresh_reset_and_mix_ramp_settles() {
        let mut core = Core::new(96000).unwrap();
        let mut fresh = Core::new(96000).unwrap();
        core.apply(Action::PsgWrite(0x90));
        for _ in 0..301 {
            core.sample();
        }
        core.apply(Action::Reset);
        for _ in 0..301 {
            assert_eq!(core.sample(), fresh.sample());
        }
        core.apply(Action::PsgWrite(0x90));
        core.apply(Action::Mix([1.0, 1.0, 1.0, 1.0, 0.0]));
        for _ in 0..480 {
            core.sample();
        }
        assert_eq!(core.sample(), [0.0; 2]);
    }
    #[test]
    fn stop_preserves_fm_raw_patch_without_retriggering_voices() {
        let mut core = Core::new(48000).unwrap();
        fm_patch(&mut core);
        // The chip's high latch is shared: A5 then A0 commits the A5 value to
        // CH1. Stop must retain the effective frequency, not replay stale A4.
        core.apply(Action::Write(0, 0xa5, 0x2a));
        core.apply(Action::Write(0, 0xa0, 0x44));
        core.apply(Action::Write(0, 0x28, 0xf0));
        let before: Vec<_> = (0..4096).map(|_| core.sample()).collect();
        assert!(before.iter().any(|v| v[0].abs() > 0.01));
        unsafe {
            native_genesis_stop_voices(core.0);
        }
        assert!((0..1024).all(|_| core.sample() == [0.0; 2]));
        core.apply(Action::Write(0, 0x28, 0xf0));
        let after: Vec<_> = (0..4096).map(|_| core.sample()).collect();
        let mut reference = Core::new(48000).unwrap();
        fm_patch(&mut reference);
        reference.apply(Action::Write(0, 0xa5, 0x2a));
        reference.apply(Action::Write(0, 0xa0, 0x44));
        for _ in 0..1024 {
            reference.sample();
        }
        reference.apply(Action::Write(0, 0x28, 0xf0));
        let expected: Vec<_> = (0..4096).map(|_| reference.sample()).collect();
        assert!(after.iter().any(|v| v[0].abs() > 0.01));
        assert!(
            after
                .iter()
                .zip(expected)
                .all(|(actual, expected)| *actual == expected),
            "Stop must preserve the patch and frequency"
        );
    }
    #[test]
    fn ipc_command_contract_and_gain_ranges_are_explicit() {
        let parsed: Command = serde_json::from_value(serde_json::json!({
            "frame": 17, "type": "mix", "fmLeft": 2, "fmRight": 1,
            "psgLeft": 0.5, "psgRight": 0, "master": 3.8,
        }))
        .unwrap();
        assert!(matches!(
            parsed.validate().unwrap(),
            Action::Mix([2.0, 1.0, 0.5, 0.0, 3.8])
        ));
        let invalid = command(
            0,
            CommandAction::Mix {
                fm_left: 2.01,
                fm_right: 1.0,
                psg_left: 1.0,
                psg_right: 1.0,
                master: 1.0,
            },
        );
        assert!(invalid.validate().is_err());
        let dac = command(
            0,
            CommandAction::Write {
                port: 0,
                register: 0x2b,
                value: 0x80,
            },
        );
        assert!(matches!(
            dac.validate().unwrap(),
            Action::Write(0, 0x2b, 0x80)
        ));
    }
    #[test]
    fn replacement_session_rejects_old_commands_and_delayed_pagehide_shutdown() {
        fn headless(session: u64) -> Result<Running, String> {
            let shared = Arc::new(Shared::new(48000, session));
            let callback = shared.clone();
            let (stop, stopped) = mpsc::channel();
            let mut renderer = Renderer::new(48000)?;
            let thread = std::thread::spawn(move || loop {
                if stopped.try_recv().is_ok() {
                    break;
                }
                renderer.prepare(&callback);
                for _ in 0..128 {
                    renderer.sample(&callback);
                }
                callback.frame.store(renderer.frame, Ordering::Release);
                std::thread::sleep(Duration::from_millis(1));
            });
            Ok(Running {
                shared,
                stop,
                thread: Some(thread),
            })
        }
        let rack = NativeGenesis::default();
        let first = rack.replace_running(headless).unwrap();
        rack.with_session(first.session, |shared| {
            shared.enqueue(first.session, &tone(0))
        })
        .unwrap();
        let second = rack.replace_running(headless).unwrap();
        assert_ne!(first.session, second.session);
        assert!(rack
            .with_session(first.session, |shared| shared
                .enqueue(first.session, &tone(0)))
            .unwrap_err()
            .contains("expired"));
        assert!(rack
            .shutdown_session(first.session)
            .unwrap_err()
            .contains("expired"));
        let current = rack
            .with_session(second.session, |shared| Ok(shared.status()))
            .unwrap();
        assert!(current.running);
        assert_eq!(current.session, second.session);
        rack.shutdown_session(second.session).unwrap();
        assert!(rack
            .with_session(second.session, |shared| Ok(shared.status()))
            .is_err());
    }
    #[test]
    #[ignore = "opens the system default audio output; sends silence only"]
    fn native_device_silent_smoke() {
        fn wait_for(
            rack: &NativeGenesis,
            session: u64,
            deadline: Instant,
            ready: impl Fn(&Status) -> bool,
        ) -> Status {
            loop {
                let status = rack
                    .with_session(session, |shared| Ok(shared.status()))
                    .expect("The native stream must remain available");
                assert!(status.running && status.error.is_none(), "{status:?}");
                if ready(&status) {
                    return status;
                }
                assert!(
                    Instant::now() < deadline,
                    "Native callback did not advance: {status:?}"
                );
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        let mute = || {
            command(
                0,
                CommandAction::Mix {
                    fm_left: 1.0,
                    fm_right: 1.0,
                    psg_left: 1.0,
                    psg_right: 1.0,
                    master: 0.0,
                },
            )
        };
        // Prove the startup and master-ramp path is exactly silent before
        // opening the device. No voice is keyed on and PSG volumes stay muted.
        let offline = Shared::new(48000, 1);
        let mut renderer = Renderer::new(48000).unwrap();
        assert!(render(&mut renderer, &offline, 128)
            .iter()
            .all(|pcm| *pcm == [0.0; 2]));
        offline.enqueue(1, &[mute()]).unwrap();
        assert!(render(&mut renderer, &offline, 512)
            .iter()
            .all(|pcm| *pcm == [0.0; 2]));

        let rack = NativeGenesis::default();
        let started = rack.start().expect("Open the system default native output");
        assert!(started.running && started.error.is_none(), "{started:?}");
        let deadline = Instant::now() + Duration::from_secs(2);
        rack.with_session(started.session, |shared| {
            shared.enqueue(started.session, &[mute()])
        })
        .unwrap();
        let drained = wait_for(&rack, started.session, deadline, |status| {
            status.queued == 0
        });
        // A completed callback beyond the full 5 ms ramp is the mute barrier.
        let muted = wait_for(&rack, started.session, deadline, |status| {
            status.frame > drained.frame + u64::from(status.sample_rate / 200)
        });
        rack.with_session(started.session, |shared| {
            shared.enqueue(
                started.session,
                &[command(
                    0,
                    CommandAction::Write {
                        port: 0,
                        register: 0x40,
                        value: 127,
                    },
                )],
            )
        })
        .unwrap();
        let written = wait_for(&rack, started.session, deadline, |status| {
            status.queued == 0 && status.frame > muted.frame
        });
        let stopped = rack
            .with_session(started.session, Shared::stop)
            .expect("Native stop fade barrier");
        assert!(stopped.running && stopped.error.is_none(), "{stopped:?}");
        assert_eq!(stopped.queued, 0);
        assert!(stopped.frame >= written.frame);
        wait_for(&rack, started.session, deadline, |status| {
            status.frame > stopped.frame
        });
        rack.shutdown_session(started.session)
            .expect("Close the native stream");
        assert!(rack.running.lock().unwrap().is_none());
    }
    fn media_load(
        shared: &Shared,
        kind: media::Kind,
        name: &str,
        rate: f64,
        channels: Vec<Vec<f32>>,
    ) {
        shared
            .load_media(kind, name.into(), rate, channels)
            .unwrap();
    }
    fn pcm_options() -> media::Options {
        serde_json::from_value(serde_json::json!({})).unwrap()
    }
    #[test]
    fn media_admission_is_atomic_bounded_and_stop_retires_future_reservations() {
        let shared = Shared::new(48000, 1);
        let mut renderer = Renderer::new(48000).unwrap();
        media_load(
            &shared,
            media::Kind::Sample,
            "pcm",
            44100.0,
            vec![vec![0.25; 32]],
        );
        let invalid = vec![
            command(
                0,
                CommandAction::SamplePlay {
                    name: "pcm".into(),
                    voice: 1,
                    options: pcm_options(),
                },
            ),
            command(
                0,
                CommandAction::DacPlay {
                    name: "missing".into(),
                    offset: 0,
                    size: None,
                    pan: "both".into(),
                },
            ),
        ];
        assert!(shared.enqueue(1, &invalid).is_err());
        assert!(shared.media.lock().unwrap().reservations.is_empty());
        assert_eq!(shared.pending.load(Ordering::Acquire), 0);
        shared
            .enqueue(
                1,
                &[command(
                    10000,
                    CommandAction::SamplePlay {
                        name: "pcm".into(),
                        voice: 1,
                        options: pcm_options(),
                    },
                )],
            )
            .unwrap();
        shared
            .enqueue(
                1,
                &[command(
                    0,
                    CommandAction::SampleStop {
                        voice: Some(1),
                        name: None,
                    },
                )],
            )
            .unwrap();
        assert!(render(&mut renderer, &shared, 128)
            .iter()
            .all(|v| *v == [0.0; 2]));
        shared.enqueue(1, &[]).unwrap();
        assert!(shared.media.lock().unwrap().reservations.is_empty());
        assert!(shared
            .load_media(
                media::Kind::Dac,
                "invalid".into(),
                22050.0,
                vec![vec![256.0]]
            )
            .is_err());
        assert!(shared
            .load_media(
                media::Kind::DacStream,
                "invalid".into(),
                44100.0,
                vec![vec![1.0; 4]]
            )
            .is_err());
        assert!(shared
            .load_media(
                media::Kind::Sample,
                "invalid".into(),
                44100.0,
                vec![vec![f32::NAN]]
            )
            .is_err());
    }
    #[test]
    fn dac_stop_cancels_future_pcm_and_packed_stream_starts() {
        let shared = Shared::new(48000, 1);
        let mut renderer = Renderer::new(48000).unwrap();
        media_load(
            &shared,
            media::Kind::Dac,
            "dac",
            22050.0,
            vec![vec![200.0; 32]],
        );
        media_load(
            &shared,
            media::Kind::DacStream,
            "packed",
            44100.0,
            vec![vec![0.0, 0.0, 0.0, 0.0, 200.0]],
        );
        shared
            .enqueue(
                1,
                &[
                    command(
                        1000,
                        CommandAction::DacPlay {
                            name: "dac".into(),
                            offset: 0,
                            size: None,
                            pan: "both".into(),
                        },
                    ),
                    command(
                        1000,
                        CommandAction::DacStreamPlay {
                            name: "packed".into(),
                        },
                    ),
                ],
            )
            .unwrap();
        shared
            .enqueue(1, &[command(0, CommandAction::DacStop)])
            .unwrap();
        assert!(render(&mut renderer, &shared, 2000)
            .iter()
            .all(|v| *v == [0.0; 2]));
        assert_eq!(shared.pending.load(Ordering::Acquire), 0);
        assert_eq!(shared.dac_streams.load(Ordering::Acquire), 0);
    }
    #[test]
    fn performance_cancellation_preserves_interactive_replacements_and_cancels_future_notes() {
        let shared = Shared::new(48000, 1);
        let mut renderer = Renderer::new(48000).unwrap();
        shared.performance("start", 17, None).unwrap();
        let tagged = |frame, action| Command {
            frame,
            action,
            performance: Some(17),
            owner: Some(1),
        };
        let mut events = tone(0);
        for event in &mut events {
            event.performance = Some(17);
            event.owner = Some(1);
        }
        events.push(tagged(5000, CommandAction::PsgWrite { value: 0x90 }));
        shared.enqueue(1, &events).unwrap();
        render(&mut renderer, &shared, 512);
        // A manual replacement of the same PSG channel owns the current voice.
        shared
            .enqueue(1, &[command(0, CommandAction::PsgWrite { value: 0x92 })])
            .unwrap();
        render(&mut renderer, &shared, 128);
        shared.performance("cancel-owner", 17, Some(1)).unwrap();
        assert!(shared
            .enqueue(1, &[tagged(0, CommandAction::PsgWrite { value: 0x90 })])
            .is_err());
        let remaining = render(&mut renderer, &shared, 6000);
        assert!(remaining.iter().any(|v| v[0] != 0.0));
        assert_eq!(shared.pending.load(Ordering::Acquire), 0);
        assert_eq!(renderer.psg_owners[0], None);
        shared.performance("start", 18, None).unwrap();
        let mut owned = tone(0);
        for event in &mut owned {
            event.performance = Some(18);
            event.owner = Some(2);
        }
        shared.enqueue(1, &owned).unwrap();
        render(&mut renderer, &shared, 128);
        shared.performance("cancel-owner", 18, Some(2)).unwrap();
        assert!(render(&mut renderer, &shared, 128)
            .iter()
            .all(|v| *v == [0.0; 2]));
    }
    #[test]
    fn owned_sample_stop_and_owner_cancellation_preserve_manual_pcm() {
        let shared = Shared::new(48000, 1);
        let mut renderer = Renderer::new(48000).unwrap();
        media_load(
            &shared,
            media::Kind::Sample,
            "pcm",
            48000.0,
            vec![vec![0.2; 64]],
        );
        shared.performance("start", 9, None).unwrap();
        let options: media::Options =
            serde_json::from_value(serde_json::json!({"loop":true,"pan":-1})).unwrap();
        let worker = |frame, action| Command {
            frame,
            action,
            performance: Some(9),
            owner: Some(1),
        };
        shared
            .enqueue(
                1,
                &[
                    command(
                        0,
                        CommandAction::SamplePlay {
                            name: "pcm".into(),
                            voice: 1,
                            options,
                        },
                    ),
                    worker(
                        0,
                        CommandAction::SamplePlay {
                            name: "pcm".into(),
                            voice: 2,
                            options,
                        },
                    ),
                    worker(
                        10000,
                        CommandAction::SamplePlay {
                            name: "pcm".into(),
                            voice: 3,
                            options,
                        },
                    ),
                ],
            )
            .unwrap();
        assert!(render(&mut renderer, &shared, 512).last().unwrap()[0] > 0.39);
        shared
            .enqueue(
                1,
                &[worker(
                    0,
                    CommandAction::SampleStop {
                        voice: None,
                        name: Some("pcm".into()),
                    },
                )],
            )
            .unwrap();
        let output = render(&mut renderer, &shared, 512);
        assert!(output
            .iter()
            .all(|v| (v[0] - 0.2).abs() < 1e-7 && v[1].abs() < 1e-7));
        shared.enqueue(1, &[]).unwrap();
        assert_eq!(shared.media.lock().unwrap().reservations.len(), 1);
        shared.performance("cancel-owner", 9, Some(1)).unwrap();
        assert!(render(&mut renderer, &shared, 10000)
            .iter()
            .all(|v| (v[0] - 0.2).abs() < 1e-7));
        assert_eq!(renderer.voices.iter().flatten().count(), 1);
    }
    #[test]
    fn fm_owner_cancellation_preserves_new_manual_keyon_and_drops_future_keyon() {
        let shared = Shared::new(48000, 1);
        let mut renderer = Renderer::new(48000).unwrap();
        fm_patch(&mut renderer.core);
        shared.performance("start", 1, None).unwrap();
        let worker = |frame, value| Command {
            frame,
            action: CommandAction::Write {
                port: 0,
                register: 0x28,
                value,
            },
            performance: Some(1),
            owner: Some(1),
        };
        shared
            .enqueue(1, &[worker(0, 0xf0), worker(5000, 0xf0)])
            .unwrap();
        render(&mut renderer, &shared, 512);
        shared
            .enqueue(
                1,
                &[command(
                    0,
                    CommandAction::Write {
                        port: 0,
                        register: 0x28,
                        value: 0xf0,
                    },
                )],
            )
            .unwrap();
        render(&mut renderer, &shared, 128);
        shared.performance("cancel-owner", 1, Some(1)).unwrap();
        let output = render(&mut renderer, &shared, 6000);
        assert!(output[1000..].iter().any(|v| v[0].abs() > 0.01));
        assert_eq!(renderer.fm_owners[0], None);
        assert_eq!(shared.pending.load(Ordering::Acquire), 0);
        shared.performance("start", 2, None).unwrap();
        shared
            .enqueue(
                1,
                &[Command {
                    frame: 0,
                    action: CommandAction::Write {
                        port: 0,
                        register: 0x28,
                        value: 0xf0,
                    },
                    performance: Some(2),
                    owner: Some(1),
                }],
            )
            .unwrap();
        render(&mut renderer, &shared, 128);
        shared.performance("stop", 2, None).unwrap();
        render(&mut renderer, &shared, 128);
        assert_eq!(renderer.fm_owners[0], None);
        assert!((0..4096)
            .map(|_| renderer.sample(&shared))
            .last()
            .unwrap()
            .iter()
            .all(|v| v.abs() < 0.0001));
    }
    #[test]
    fn psg_only_reset_keeps_fm_and_generic_pcm_and_sample_master_is_shared() {
        let shared = Shared::new(48000, 1);
        let mut renderer = Renderer::new(48000).unwrap();
        fm_patch(&mut renderer.core);
        let mut commands = tone(0);
        commands.push(command(
            0,
            CommandAction::Write {
                port: 0,
                register: 0x28,
                value: 0xf0,
            },
        ));
        shared.enqueue(1, &commands).unwrap();
        render(&mut renderer, &shared, 512);
        shared
            .enqueue(1, &[command(0, CommandAction::PsgReset)])
            .unwrap();
        let output = render(&mut renderer, &shared, 512);
        assert!(output.iter().any(|v| v[0].abs() > 0.01));
        assert_eq!(renderer.psg_owners, [None; 4]);
        let other = Shared::new(48000, 2);
        let mut sample = Renderer::new(48000).unwrap();
        media_load(
            &other,
            media::Kind::Sample,
            "pcm",
            48000.0,
            vec![vec![0.4; 64]],
        );
        let options: media::Options =
            serde_json::from_value(serde_json::json!({"loop":true,"pan":-1})).unwrap();
        other
            .enqueue(
                2,
                &[
                    command(
                        0,
                        CommandAction::Mix {
                            fm_left: 1.0,
                            fm_right: 1.0,
                            psg_left: 1.0,
                            psg_right: 1.0,
                            master: 0.5,
                        },
                    ),
                    command(
                        0,
                        CommandAction::SamplePlay {
                            name: "pcm".into(),
                            voice: 1,
                            options,
                        },
                    ),
                ],
            )
            .unwrap();
        let output = render(&mut sample, &other, 1024);
        assert!((output.last().unwrap()[0] - 0.2).abs() < 1e-7);
    }
    #[test]
    fn removed_banks_survive_accepted_playback_then_reclaim_on_control() {
        let shared = Shared::new(48000, 1);
        let mut renderer = Renderer::new(48000).unwrap();
        media_load(
            &shared,
            media::Kind::Sample,
            "pcm",
            48000.0,
            vec![vec![0.25; 1024]],
        );
        let bank = shared
            .media
            .lock()
            .unwrap()
            .get(media::Kind::Sample, "pcm")
            .unwrap();
        let weak = Arc::downgrade(&bank);
        drop(bank);
        shared
            .enqueue(
                1,
                &[command(
                    0,
                    CommandAction::SamplePlay {
                        name: "pcm".into(),
                        voice: 1,
                        options: pcm_options(),
                    },
                )],
            )
            .unwrap();
        shared.remove_media(media::Kind::Sample, "pcm").unwrap();
        assert!(weak.upgrade().is_some());
        let pcm = render(&mut renderer, &shared, 2048);
        assert!(pcm.iter().any(|v| v[0].abs() > 0.01));
        assert!(
            weak.upgrade().is_some(),
            "callback retains a control-side lifetime guard"
        );
        shared.enqueue(1, &[]).unwrap();
        assert!(
            weak.upgrade().is_none(),
            "only control reclaims completed bank data"
        );
    }
    #[test]
    fn future_media_stop_waits_for_its_audio_frame_and_cancels_prior_future_starts() {
        let shared = Shared::new(48000, 1);
        let mut renderer = Renderer::new(48000).unwrap();
        media_load(
            &shared,
            media::Kind::Sample,
            "pcm",
            48000.0,
            vec![vec![0.2; 64]],
        );
        let options: media::Options =
            serde_json::from_value(serde_json::json!({"loop":true,"pan":-1,"fadeOut":0.003}))
                .unwrap();
        shared
            .enqueue(
                1,
                &[
                    command(
                        0,
                        CommandAction::SamplePlay {
                            name: "pcm".into(),
                            voice: 1,
                            options,
                        },
                    ),
                    command(
                        26000,
                        CommandAction::SamplePlay {
                            name: "pcm".into(),
                            voice: 2,
                            options,
                        },
                    ),
                    command(
                        24000,
                        CommandAction::SampleStop {
                            voice: None,
                            name: None,
                        },
                    ),
                ],
            )
            .unwrap();
        let pcm = render(&mut renderer, &shared, 28000);
        assert!(pcm[512..24000].iter().all(|v| (v[0] - 0.2).abs() < 1e-7));
        assert!(pcm[24001][0] < pcm[24000][0]);
        assert!(pcm[24144..].iter().all(|v| *v == [0.0; 2]));
        shared.enqueue(1, &[]).unwrap();
        assert!(shared.media.lock().unwrap().reservations.is_empty());
        assert!(shared.media.lock().unwrap().stops.is_empty());
        let dac = Shared::new(48000, 2);
        let mut player = Renderer::new(48000).unwrap();
        media_load(
            &dac,
            media::Kind::Dac,
            "dac",
            22050.0,
            vec![vec![200.0; 100000]],
        );
        dac.enqueue(
            2,
            &[
                command(
                    0,
                    CommandAction::DacPlay {
                        name: "dac".into(),
                        offset: 0,
                        size: None,
                        pan: "left".into(),
                    },
                ),
                command(
                    26000,
                    CommandAction::DacPlay {
                        name: "dac".into(),
                        offset: 0,
                        size: None,
                        pan: "left".into(),
                    },
                ),
                command(24000, CommandAction::DacStop),
            ],
        )
        .unwrap();
        let pcm = render(&mut player, &dac, 28000);
        assert!(pcm[512..24000].iter().any(|v| v[0].abs() > 0.01));
        assert!(pcm[24002..].iter().all(|v| *v == [0.0; 2]));
        assert_eq!(dac.pending.load(Ordering::Acquire), 0);
    }
    #[test]
    fn stale_performance_cleanup_cannot_cancel_replacement_performance() {
        let shared = Shared::new(48000, 1);
        let mut renderer = Renderer::new(48000).unwrap();
        shared.performance("start", 1, None).unwrap();
        shared.performance("stop", 1, None).unwrap();
        shared.performance("start", 2, None).unwrap();
        shared.performance("stop", 1, None).unwrap();
        shared.performance("cancel-owner", 1, Some(1)).unwrap();
        assert!(shared.valid_owner(Some(Owner {
            performance: 2,
            owner: 1
        })));
        let mut commands = tone(0);
        for command in &mut commands {
            command.performance = Some(2);
            command.owner = Some(1);
        }
        shared.enqueue(1, &commands).unwrap();
        assert!(render(&mut renderer, &shared, 512)
            .iter()
            .any(|v| v[0].abs() > 0.01));
    }
    #[test]
    fn global_stop_resets_fx_control_model_and_keeps_recording_available() {
        let shared = Arc::new(Shared::new(48000, 1));
        let message = |value| serde_json::from_value::<FxMessage>(value).unwrap();
        let unit = serde_json::json!({"type":"gain","slot":0,"values":{"gain":0.5}});
        let graph=shared.fx.lock().unwrap().command(message(serde_json::json!({"op":"chain","children":[{"type":"gain","slot":0}],"units":[unit.clone()],"beatSeconds":0.5})),48000).unwrap().unwrap();
        shared.enqueue_special(Action::Fx(graph)).unwrap();
        shared.enqueue(1, &tone(0)).unwrap();
        shared.record_start(0.03).unwrap();
        let callback = shared.clone();
        let stopped = Arc::new(AtomicBool::new(false));
        let exit = stopped.clone();
        let thread = std::thread::spawn(move || {
            let mut renderer = Renderer::new(48000).unwrap();
            while !exit.load(Ordering::Acquire) {
                renderer.prepare(&callback);
                for _ in 0..128 {
                    renderer.sample(&callback);
                }
                callback.frame.store(renderer.frame, Ordering::Release);
                std::thread::sleep(Duration::from_millis(1));
            }
        });
        let stop = shared.stop();
        let record = shared.record_stop();
        let parameter=shared.fx.lock().unwrap().command(message(serde_json::json!({"op":"parameter","unit":unit,"key":"gain","value":0.9,"seconds":0,"beatSeconds":0.5})),48000);
        stopped.store(true, Ordering::Release);
        thread.join().unwrap();
        assert!(stop.is_ok());
        assert!(record.is_ok());
        assert!(parameter.unwrap_err().contains("disposed"));
        assert!(shared.recording.lock().unwrap().is_none());
    }
    #[test]
    fn bounded_recording_captures_final_signal_and_returns_after_ownership_barrier() {
        let shared = Shared::new(48000, 1);
        let mut renderer = Renderer::new(48000).unwrap();
        shared.enqueue(1, &tone(0)).unwrap();
        shared.record_start(0.01).unwrap();
        assert!(shared.record_start(0.01).is_err());
        let pcm = render(&mut renderer, &shared, 1024);
        let result = shared.record_stop().unwrap();
        assert_eq!(result.frames, 480);
        assert!(result.complete);
        assert_eq!(result.first_frame, 0);
        assert_eq!(
            result.channels[0],
            pcm[..480].iter().map(|v| v[0]).collect::<Vec<_>>()
        );
        assert_eq!(
            result.channels[1],
            pcm[..480].iter().map(|v| v[1]).collect::<Vec<_>>()
        );
        assert!(shared.record_stop().is_err());
        shared.record_start(0.01).unwrap();
        render(&mut renderer, &shared, 128);
        shared
            .recording
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .stop
            .store(true, Ordering::Release);
        render(&mut renderer, &shared, 128);
        assert_eq!(shared.record_stop().unwrap().frames, 128);
        assert!(shared.record_start(31.0).is_err());
    }
    #[test]
    fn enabling_native_monitor_leaves_rendered_audio_bit_identical() {
        let a = Shared::new(48000, 1);
        let b = Shared::new(48000, 2);
        let mut expected = Renderer::new(48000).unwrap();
        let mut observed = Renderer::new(48000).unwrap();
        a.enqueue(1, &tone(0)).unwrap();
        b.enqueue(2, &tone(0)).unwrap();
        b.monitor.read(monitor::Mode::Waveform, 0, 2, 0).unwrap();
        let left = render(&mut expected, &a, 8192);
        let right = render(&mut observed, &b, 8192);
        assert_eq!(left, right);
        let snapshot = b
            .monitor
            .read(monitor::Mode::Waveform, 0, 2, 8192)
            .unwrap()
            .unwrap();
        assert_eq!(snapshot.frames, 4096);
        assert_eq!(
            snapshot.samples.unwrap(),
            right[4096..].iter().map(|v| v[0]).collect::<Vec<_>>()
        );
        b.monitor.off();
        assert_eq!(
            render(&mut expected, &a, 8192),
            render(&mut observed, &b, 8192)
        );
    }
    #[test]
    fn native_media_pcm_fixture() {
        let mut fixtures = Vec::new();
        for rate in [44100, 48000, 96000] {
            for scenario in ["dac", "packed", "sample"] {
                let shared = Shared::new(rate, 1);
                let mut renderer = Renderer::new(rate).unwrap();
                renderer.idle = false;
                let mut events = Vec::new();
                if scenario == "dac" {
                    let bytes: Vec<f32> = (0..257)
                        .map(|i| (128.0 + 110.0 * (i as f64 * 0.19).sin()).round() as f32)
                        .collect();
                    media_load(&shared, media::Kind::Dac, "dac", 22050.0, vec![bytes]);
                    events.push(command(
                        137,
                        CommandAction::DacPlay {
                            name: "dac".into(),
                            offset: 13,
                            size: Some(211),
                            pan: "left".into(),
                        },
                    ));
                } else if scenario == "packed" {
                    let mut packed = Vec::new();
                    for i in 0..257u32 {
                        packed.extend((i * 2).to_le_bytes().map(|v| v as f32));
                        packed.push((128.0 + 100.0 * (i as f64 * 0.2).sin()).round() as f32);
                    }
                    media_load(
                        &shared,
                        media::Kind::DacStream,
                        "packed",
                        44100.0,
                        vec![packed],
                    );
                    events.push(command(
                        137,
                        CommandAction::Write {
                            port: 1,
                            register: 0xb6,
                            value: 0x40,
                        },
                    ));
                    events.push(command(
                        137,
                        CommandAction::Write {
                            port: 0,
                            register: 0x2b,
                            value: 0x80,
                        },
                    ));
                    events.push(command(
                        137,
                        CommandAction::DacStreamPlay {
                            name: "packed".into(),
                        },
                    ));
                    events.push(command(
                        3000,
                        CommandAction::Write {
                            port: 0,
                            register: 0x2b,
                            value: 0,
                        },
                    ));
                } else {
                    let left: Vec<f32> = (0..1024)
                        .map(|i| (i as f64 * 0.12).sin() as f32 * 0.2)
                        .collect();
                    let right: Vec<f32> = (0..1024)
                        .map(|i| (i as f64 * 0.17).cos() as f32 * 0.15)
                        .collect();
                    media_load(
                        &shared,
                        media::Kind::Sample,
                        "pcm",
                        44100.0,
                        vec![left, right],
                    );
                    let options:media::Options=serde_json::from_value(serde_json::json!({"playbackRate":-0.75,"gain":0.7,"pan":-0.35,"offset":0.001,"duration":0.035,"loop":true,"loopStart":0.002,"loopEnd":0.018,"fadeIn":0.002,"fadeOut":0.003})).unwrap();
                    events.push(command(
                        137,
                        CommandAction::SamplePlay {
                            name: "pcm".into(),
                            voice: 1,
                            options,
                        },
                    ));
                }
                shared.enqueue(1, &events).unwrap();
                let pcm = render(&mut renderer, &shared, 8192);
                assert!(pcm.iter().flatten().all(|v| v.is_finite()));
                assert!(pcm.iter().any(|v| v[0].abs() + v[1].abs() > 0.01));
                fixtures.push(serde_json::json!({"sampleRate":rate,"scenario":scenario,"pcm":pcm}));
            }
        }
        if let Ok(path) = std::env::var("TETORICA_NATIVE_MEDIA_FIXTURE") {
            std::fs::write(path, serde_json::to_vec(&fixtures).unwrap()).unwrap();
        }
    }
}

#[tauri::command]
pub fn native_genesis_extra_ym3526_create(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
    clock: Option<u32>,
) -> Result<extra_opl::Status, String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| {
        shared.create_extra_ym3526(instance, clock)
    })
}
#[tauri::command]
pub fn native_genesis_extra_ym3526_dispose(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
) -> Result<(), String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| shared.dispose_extra_ym3526(instance))
}
#[tauri::command]
pub async fn native_genesis_extra_ym3526_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
    after: Option<u64>,
    after_layout_revision: Option<u64>,
) -> Result<extra_opl::Status, String> {
    crate::allowed(&window)?;
    let shared = state.session_shared(session)?;
    let status = tauri::async_runtime::spawn_blocking(move || {
        shared.extra_ym3526_status(instance, after, after_layout_revision)
    })
    .await
    .map_err(|error| error.to_string())??;
    state.with_session(session, |_| Ok(()))?;
    Ok(status)
}

#[tauri::command]
pub fn native_genesis_extra_ym3812_create(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
    clock: Option<u32>,
) -> Result<extra_opl::Status, String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| {
        shared.create_extra_ym3812(instance, clock)
    })
}
#[tauri::command]
pub fn native_genesis_extra_ym3812_dispose(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
) -> Result<(), String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| shared.dispose_extra_ym3812(instance))
}
#[tauri::command]
pub async fn native_genesis_extra_ym3812_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
    after: Option<u64>,
    after_layout_revision: Option<u64>,
) -> Result<extra_opl::Status, String> {
    crate::allowed(&window)?;
    let shared = state.session_shared(session)?;
    let status = tauri::async_runtime::spawn_blocking(move || {
        shared.extra_ym3812_status(instance, after, after_layout_revision)
    })
    .await
    .map_err(|error| error.to_string())??;
    state.with_session(session, |_| Ok(()))?;
    Ok(status)
}

#[tauri::command]
pub fn native_genesis_extra_y8950_create(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
    clock: Option<u32>,
) -> Result<extra_opl::Status, String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| shared.create_extra_y8950(instance, clock))
}
#[tauri::command]
pub fn native_genesis_extra_y8950_dispose(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
) -> Result<(), String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| shared.dispose_extra_y8950(instance))
}
#[tauri::command]
pub async fn native_genesis_extra_y8950_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
    after: Option<u64>,
    after_layout_revision: Option<u64>,
) -> Result<extra_opl::Status, String> {
    crate::allowed(&window)?;
    let shared = state.session_shared(session)?;
    let status = tauri::async_runtime::spawn_blocking(move || {
        shared.extra_y8950_status(instance, after, after_layout_revision)
    })
    .await
    .map_err(|error| error.to_string())??;
    state.with_session(session, |_| Ok(()))?;
    Ok(status)
}

#[tauri::command]
pub fn native_genesis_extra_ymf262_create(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
    clock: Option<u32>,
) -> Result<extra_opl::Status, String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| {
        shared.create_extra_ymf262(instance, clock)
    })
}

#[tauri::command]
pub fn native_genesis_extra_rf5c164_create(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
    clock: Option<u32>,
) -> Result<extra_rf5c164::Status, String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| {
        shared.create_extra_rf5c164(instance, clock)
    })
}
#[tauri::command]
pub fn native_genesis_extra_okim6258_create(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
    clock: Option<u32>,
) -> Result<extra_okim6258::Status, String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| {
        shared.create_extra_okim6258(instance, clock)
    })
}
#[tauri::command]
pub fn native_genesis_extra_huc6280_create(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
    clock: Option<u32>,
) -> Result<extra_huc6280::Status, String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| {
        shared.create_extra_huc6280(instance, clock)
    })
}
#[tauri::command]
pub fn native_genesis_extra_dx7_create(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
) -> Result<extra_dx7::Status, String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| shared.create_extra_dx7(instance))
}
#[tauri::command]
pub fn native_genesis_extra_pwm_create(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
) -> Result<extra_pwm::Status, String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| shared.create_extra_pwm(instance))
}
#[tauri::command]
pub fn native_genesis_extra_gameboy_create(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
    clock: Option<u32>,
) -> Result<extra_gameboy::Status, String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| {
        shared.create_extra_gameboy(instance, clock)
    })
}
#[tauri::command]
pub fn native_genesis_extra_nes_create(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
    clock: Option<f64>,
    fds: Option<bool>,
) -> Result<extra_nes::Status, String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| {
        shared.create_extra_nes(instance, clock, fds.unwrap_or(false))
    })
}
#[tauri::command]
pub fn native_genesis_extra_ymf262_dispose(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
) -> Result<(), String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| shared.dispose_extra_ymf262(instance))
}

#[tauri::command]
pub fn native_genesis_extra_rf5c164_dispose(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
) -> Result<(), String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| shared.dispose_extra_rf5c164(instance))
}
#[tauri::command]
pub fn native_genesis_extra_okim6258_dispose(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
) -> Result<(), String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| shared.dispose_extra_okim6258(instance))
}
#[tauri::command]
pub fn native_genesis_extra_huc6280_dispose(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
) -> Result<(), String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| shared.dispose_extra_huc6280(instance))
}
#[tauri::command]
pub fn native_genesis_extra_dx7_dispose(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
) -> Result<(), String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| shared.dispose_extra_dx7(instance))
}
#[tauri::command]
pub fn native_genesis_extra_pwm_dispose(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
) -> Result<(), String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| shared.dispose_extra_pwm(instance))
}
#[tauri::command]
pub fn native_genesis_extra_gameboy_dispose(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
) -> Result<(), String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| shared.dispose_extra_gameboy(instance))
}
#[tauri::command]
pub fn native_genesis_extra_nes_dispose(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
) -> Result<(), String> {
    crate::allowed(&window)?;
    state.with_session(session, |shared| shared.dispose_extra_nes(instance))
}
#[tauri::command]
pub async fn native_genesis_extra_ymf262_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
    after: Option<u64>,
    after_layout_revision: Option<u64>,
) -> Result<extra_opl::Status, String> {
    crate::allowed(&window)?;
    let shared = state.session_shared(session)?;
    let status = tauri::async_runtime::spawn_blocking(move || {
        shared.extra_ymf262_status(instance, after, after_layout_revision)
    })
    .await
    .map_err(|error| error.to_string())??;
    state.with_session(session, |_| Ok(()))?;
    Ok(status)
}

#[tauri::command]
pub async fn native_genesis_extra_rf5c164_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
    after: Option<u64>,
) -> Result<extra_rf5c164::Status, String> {
    crate::allowed(&window)?;
    let shared = state.session_shared(session)?;
    let status =
        tauri::async_runtime::spawn_blocking(move || shared.extra_rf5c164_status(instance, after))
            .await
            .map_err(|error| error.to_string())??;
    state.with_session(session, |_| Ok(()))?;
    Ok(status)
}
#[tauri::command]
pub async fn native_genesis_extra_okim6258_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
    after: Option<u64>,
) -> Result<extra_okim6258::Status, String> {
    crate::allowed(&window)?;
    let shared = state.session_shared(session)?;
    let status =
        tauri::async_runtime::spawn_blocking(move || shared.extra_okim6258_status(instance, after))
            .await
            .map_err(|error| error.to_string())??;
    state.with_session(session, |_| Ok(()))?;
    Ok(status)
}
#[tauri::command]
pub async fn native_genesis_extra_huc6280_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
    after: Option<u64>,
) -> Result<extra_huc6280::Status, String> {
    crate::allowed(&window)?;
    let shared = state.session_shared(session)?;
    let status =
        tauri::async_runtime::spawn_blocking(move || shared.extra_huc6280_status(instance, after))
            .await
            .map_err(|error| error.to_string())??;
    state.with_session(session, |_| Ok(()))?;
    Ok(status)
}
#[tauri::command]
pub async fn native_genesis_extra_dx7_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
    after: Option<u64>,
) -> Result<extra_dx7::Status, String> {
    crate::allowed(&window)?;
    let shared = state.session_shared(session)?;
    let status =
        tauri::async_runtime::spawn_blocking(move || shared.extra_dx7_status(instance, after))
            .await
            .map_err(|error| error.to_string())??;
    state.with_session(session, |_| Ok(()))?;
    Ok(status)
}
#[tauri::command]
pub async fn native_genesis_extra_pwm_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
    after: Option<u64>,
) -> Result<extra_pwm::Status, String> {
    crate::allowed(&window)?;
    let shared = state.session_shared(session)?;
    let status =
        tauri::async_runtime::spawn_blocking(move || shared.extra_pwm_status(instance, after))
            .await
            .map_err(|error| error.to_string())??;
    state.with_session(session, |_| Ok(()))?;
    Ok(status)
}
#[tauri::command]
pub async fn native_genesis_extra_gameboy_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
    after: Option<u64>,
) -> Result<extra_gameboy::Status, String> {
    crate::allowed(&window)?;
    let shared = state.session_shared(session)?;
    let status =
        tauri::async_runtime::spawn_blocking(move || shared.extra_gameboy_status(instance, after))
            .await
            .map_err(|error| error.to_string())??;
    state.with_session(session, |_| Ok(()))?;
    Ok(status)
}
#[tauri::command]
pub async fn native_genesis_extra_nes_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeGenesis>,
    session: u64,
    instance: u64,
    after: Option<u64>,
) -> Result<extra_nes::Status, String> {
    crate::allowed(&window)?;
    let shared = state.session_shared(session)?;
    let status =
        tauri::async_runtime::spawn_blocking(move || shared.extra_nes_status(instance, after))
            .await
            .map_err(|error| error.to_string())??;
    state.with_session(session, |_| Ok(()))?;
    Ok(status)
}
