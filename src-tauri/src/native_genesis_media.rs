//! Native-owned immutable PCM banks and allocation-free playback state.
//! Uploads and releases happen on the control thread; the callback never owns
//! the final bank reference. Control reclaims unreferenced banks within a 64 MiB budget.
//! Voice interpolation/envelopes match native/audio_effect/sample.c; the real
//! native/WASM PCM comparison guards this port against audible divergence.
use serde::Deserialize;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

pub const BANK_BUDGET: usize = 64 * 1024 * 1024;
pub const VOICES: usize = 64;
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Dac,
    DacStream,
    Sample,
}
#[derive(Debug)]
pub enum Data {
    Bytes(Vec<u8>),
    Float(Vec<f32>),
}
#[derive(Debug)]
pub struct Bank {
    pub name: String,
    pub rate: f64,
    pub frames: usize,
    pub channels: usize,
    pub data: Data,
}
impl Bank {
    pub fn bytes(&self) -> &[u8] {
        if let Data::Bytes(data) = &self.data {
            data
        } else {
            &[]
        }
    }
    fn read(&self, channel: usize, index: usize, reverse: bool) -> f64 {
        let index = if reverse {
            self.frames - 1 - index
        } else {
            index
        };
        if let Data::Float(data) = &self.data {
            data[channel * self.frames + index] as f64
        } else {
            0.0
        }
    }
}
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Options {
    #[serde(default = "one")]
    pub playback_rate: f64,
    #[serde(default = "one")]
    pub gain: f64,
    #[serde(default)]
    pub pan: f64,
    #[serde(default)]
    pub offset: f64,
    #[serde(default = "negative")]
    pub duration: f64,
    #[serde(default)]
    pub r#loop: bool,
    #[serde(default)]
    pub loop_start: f64,
    #[serde(default)]
    pub loop_end: f64,
    #[serde(default)]
    pub fade_in: f64,
    #[serde(default)]
    pub fade_out: f64,
}
fn one() -> f64 {
    1.0
}
fn negative() -> f64 {
    -1.0
}
impl Options {
    pub fn validate(&self, bank: &Bank, rate: u32) -> Result<(), String> {
        let values = [
            self.playback_rate,
            self.gain,
            self.pan,
            self.offset,
            self.duration,
            self.loop_start,
            self.loop_end,
            self.fade_in,
            self.fade_out,
        ];
        let step = self.playback_rate.abs() * bank.rate / rate as f64;
        if values.iter().any(|v| !v.is_finite())
            || self.playback_rate == 0.0
            || self.offset < 0.0
            || self.duration < -1.0
            || self.loop_start < 0.0
            || self.loop_end < 0.0
            || self.fade_in < 0.0
            || self.fade_out < 0.0
            || !step.is_finite()
            || step <= 0.0
        {
            return Err("Invalid native sample playback parameters".into());
        }
        Ok(())
    }
}
#[derive(Debug)]
pub struct Reservation {
    pub voice: u64,
    pub bank: Arc<Bank>,
    pub options: Options,
    pub owner: Option<super::Owner>,
    pub cancelled: AtomicBool,
}
impl Reservation {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
    pub fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}
#[derive(Debug)]
pub struct StopTargets {
    pub id: u64,
    pub targets: Vec<Arc<Reservation>>,
}
impl StopTargets {
    pub fn cancel(&self) {
        for target in &self.targets {
            target.cancel();
        }
    }
}

#[derive(Default)]
pub struct Store {
    names: HashMap<(Kind, String), Arc<Bank>>,
    retained: Vec<Arc<Bank>>,
    bytes: usize,
    pub reservations: HashMap<u64, Arc<Reservation>>,
    pub stops: HashMap<u64, Arc<StopTargets>>,
}
impl Store {
    pub fn collect(&mut self) {
        let mut freed = 0;
        self.retained.retain(|bank| {
            if Arc::strong_count(bank) == 1 {
                freed += match &bank.data {
                    Data::Bytes(data) => data.len(),
                    Data::Float(data) => data.len() * 4,
                };
                false
            } else {
                true
            }
        });
        self.bytes -= freed;
    }
    pub fn load(
        &mut self,
        kind: Kind,
        name: String,
        rate: f64,
        channels: Vec<Vec<f32>>,
    ) -> Result<(), String> {
        if name.is_empty()
            || name.len() > 1024
            || !rate.is_finite()
            || rate <= 0.0
            || rate > 384000.0
            || channels.is_empty()
            || channels.len() > 2
            || channels[0].is_empty()
        {
            return Err("Expected a named mono/stereo PCM bank with a positive sample rate".into());
        }
        let frames = channels[0].len();
        if frames > 10_000_000
            || channels
                .iter()
                .any(|c| c.len() != frames || c.iter().any(|v| !v.is_finite()))
        {
            return Err("Invalid native PCM bank".into());
        }
        if kind != Kind::Sample
            && (channels.len() != 1
                || channels[0]
                    .iter()
                    .any(|v| *v < 0.0 || *v > 255.0 || v.fract() != 0.0))
        {
            return Err("DAC banks require unsigned 8-bit mono PCM".into());
        }
        if kind == Kind::DacStream {
            if frames % 5 != 0 {
                return Err("Packed DAC stream must contain five-byte events".into());
            }
            let mut previous = 0;
            for event in channels[0].chunks_exact(5) {
                let offset = u32::from_le_bytes([
                    event[0] as u8,
                    event[1] as u8,
                    event[2] as u8,
                    event[3] as u8,
                ]);
                if offset < previous {
                    return Err("Packed DAC stream offsets must be nondecreasing".into());
                }
                previous = offset;
            }
        }
        let count = if kind == Kind::Sample {
            channels.len() * frames * 4
        } else {
            frames
        };
        if self
            .bytes
            .checked_add(count)
            .is_none_or(|v| v > BANK_BUDGET)
        {
            return Err("Native PCM bank budget (64 MiB) exhausted; unload unused banks before uploading more".into());
        }
        let key = (kind, name.clone());
        if !self.names.contains_key(&key)
            && self.names.keys().filter(|(k, _)| *k == kind).count() >= 64
        {
            return Err("Native PCM bank limit (64 per kind) reached".into());
        }
        let channel_count = channels.len();
        let data = if kind == Kind::Sample {
            Data::Float(channels.into_iter().flatten().collect())
        } else {
            Data::Bytes(
                channels
                    .into_iter()
                    .next()
                    .unwrap()
                    .into_iter()
                    .map(|v| v as u8)
                    .collect(),
            )
        };
        let bank = Arc::new(Bank {
            name,
            rate,
            frames,
            channels: channel_count,
            data,
        });
        self.bytes += count;
        self.retained.push(bank.clone());
        self.names.insert(key, bank);
        Ok(())
    }
    pub fn get(&self, kind: Kind, name: &str) -> Result<Arc<Bank>, String> {
        self.names
            .get(&(kind, name.into()))
            .cloned()
            .ok_or_else(|| format!("Unknown native PCM bank: {name}"))
    }
    pub fn remove(&mut self, kind: Kind, name: &str) {
        self.names.remove(&(kind, name.into()));
        self.collect();
    }
    pub fn stop_targets(
        &mut self,
        id: u64,
        voice: Option<u64>,
        name: Option<&str>,
        owner: Option<super::Owner>,
    ) -> Arc<StopTargets> {
        let targets = self
            .reservations
            .values()
            .filter(|reservation| {
                owner.is_none_or(|value| reservation.owner == Some(value))
                    && voice.is_none_or(|value| value == reservation.voice)
                    && name.is_none_or(|value| value == reservation.bank.name)
            })
            .cloned()
            .collect();
        let targets = Arc::new(StopTargets { id, targets });
        self.stops.insert(id, targets.clone());
        targets
    }
    pub fn cancel(&self, voice: Option<u64>, name: Option<&str>, owner: Option<super::Owner>) {
        for reservation in self.reservations.values() {
            if owner.is_none_or(|value| reservation.owner == Some(value))
                && voice.is_none_or(|v| v == reservation.voice)
                && name.is_none_or(|n| n == reservation.bank.name)
            {
                reservation.cancel();
            }
        }
    }
}

pub struct Voice {
    pub reservation: Arc<Reservation>,
    pub owner: Option<super::Owner>,
    pos: f64,
    step: f64,
    end: f64,
    loop_start: f64,
    loop_end: f64,
    reverse: bool,
    age: f64,
    travel: f64,
    limit: f64,
    length: f64,
    release_start: f64,
    fade_in: f64,
    fade_out: f64,
    stop_age: f64,
    stop_gain: Option<f64>,
    pan_c: f64,
    pan_s: f64,
}
impl Voice {
    pub fn new(reservation: Arc<Reservation>, rate: u32, owner: Option<super::Owner>) -> Self {
        let o = reservation.options;
        let bank = &reservation.bank;
        let frames = bank.frames as f64;
        let step = o.playback_rate.abs() * (bank.rate / rate as f64);
        let reverse = o.playback_rate < 0.0;
        let mut loop_start = (o.loop_start * bank.rate).min(frames);
        let mut loop_end = if o.loop_end > 0.0 {
            (o.loop_end * bank.rate).min(frames)
        } else {
            frames
        };
        if loop_end <= loop_start {
            loop_start = 0.0;
            loop_end = frames;
        }
        if reverse {
            let start = loop_start;
            loop_start = frames - loop_end;
            loop_end = frames - start;
        }
        let mut pos = o.offset * bank.rate;
        if o.r#loop && pos >= loop_end {
            pos = loop_start;
        }
        let limit = if o.duration < 0.0 {
            -1.0
        } else {
            o.duration * bank.rate
        };
        let mut length = if o.r#loop {
            f64::INFINITY
        } else {
            ((frames - pos).max(0.0) / step).ceil()
        };
        if limit >= 0.0 {
            length = length.min((limit / step).ceil());
        }
        let fade_in = o.fade_in * rate as f64;
        let fade_out = o.fade_out * rate as f64;
        let release_start = if fade_out > 0.0 && length.is_finite() {
            length - fade_out
        } else {
            f64::INFINITY
        };
        let pan = o.pan.clamp(-1.0, 1.0);
        let angle = if bank.channels == 1 {
            (pan + 1.0) * std::f64::consts::PI / 4.0
        } else if pan < 0.0 {
            (pan + 1.0) * std::f64::consts::PI / 2.0
        } else {
            pan * std::f64::consts::PI / 2.0
        };
        Self {
            reservation,
            owner,
            pos,
            step,
            end: frames,
            loop_start,
            loop_end,
            reverse,
            age: 0.0,
            travel: 0.0,
            limit,
            length,
            release_start,
            fade_in,
            fade_out,
            stop_age: 0.0,
            stop_gain: None,
            pan_c: angle.cos(),
            pan_s: angle.sin(),
        }
    }
    fn envelope(&self) -> f64 {
        let mut value = if self.age < self.fade_in {
            self.age / self.fade_in
        } else {
            1.0
        };
        if self.age > self.release_start {
            value = value.min(((self.length - self.age) / self.fade_out).max(0.0));
        }
        value
    }
    pub fn sample(&mut self, output: &mut [f32; 2]) -> bool {
        let o = self.reservation.options;
        let bank = &self.reservation.bank;
        if o.r#loop && self.pos >= self.loop_end {
            self.pos =
                self.loop_start + (self.pos - self.loop_start) % (self.loop_end - self.loop_start);
        }
        if self.reservation.cancelled() && self.stop_gain.is_none() {
            self.stop_gain = Some(self.envelope());
        }
        if self.pos >= self.end
            || (self.limit >= 0.0 && self.travel >= self.limit)
            || (self.stop_gain.is_some() && self.stop_age >= self.fade_out)
        {
            return false;
        }
        let index = self.pos as usize;
        let fraction = self.pos - index as f64;
        let mut next = index + 1;
        if o.r#loop && next as f64 >= self.loop_end {
            next = self.loop_start as usize;
        } else if next >= bank.frames {
            next = index;
        }
        let mut left = bank.read(0, index, self.reverse) * (1.0 - fraction)
            + bank.read(0, next, self.reverse) * fraction;
        let mut right = if bank.channels == 2 {
            bank.read(1, index, self.reverse) * (1.0 - fraction)
                + bank.read(1, next, self.reverse) * fraction
        } else {
            left
        };
        let mut env = self.envelope();
        if let Some(gain) = self.stop_gain {
            env = env.min(gain * (1.0 - self.stop_age / self.fade_out));
        }
        if bank.channels == 1 {
            left *= self.pan_c;
            right *= self.pan_s;
        } else if o.pan < 0.0 {
            left += right * self.pan_c;
            right *= self.pan_s;
        } else {
            right += left * self.pan_s;
            left *= self.pan_c;
        }
        output[0] += (left * o.gain * env) as f32;
        output[1] += (right * o.gain * env) as f32;
        self.pos += self.step;
        self.travel += self.step;
        self.age += 1.0;
        if self.stop_gain.is_some() {
            self.stop_age += 1.0;
        }
        true
    }
}
