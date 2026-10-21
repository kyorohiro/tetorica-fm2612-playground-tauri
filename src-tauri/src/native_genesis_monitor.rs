//! Best-effort native output observation. The audio callback only writes a
//! fixed atomic ring. Snapshots and the 4096-point FFT run on control threads.
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Mutex, TryLockError,
};

pub const SIZE: usize = 4096;
const MASK: usize = SIZE - 1;
const LEASE_SECONDS: u64 = 2;
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Envelope,
    Waveform,
    Spectrum,
    Off,
}
struct Slot {
    stamp: AtomicU64,
    stereo: AtomicU64,
}
impl Default for Slot {
    fn default() -> Self {
        Self {
            stamp: AtomicU64::new(0),
            stereo: AtomicU64::new(0),
        }
    }
}
#[derive(Default)]
pub struct Writer {
    generation: u64,
}
struct Reader {
    samples: Box<[f32]>,
    real: Box<[f64]>,
    imag: Box<[f64]>,
    window: Box<[f64]>,
    twiddle_real: Box<[f64]>,
    twiddle_imag: Box<[f64]>,
}
impl Reader {
    fn new() -> Self {
        let angle = 2.0 * std::f64::consts::PI / SIZE as f64;
        Self {
            samples: vec![0.0; SIZE].into_boxed_slice(),
            real: vec![0.0; SIZE].into_boxed_slice(),
            imag: vec![0.0; SIZE].into_boxed_slice(),
            window: (0..SIZE)
                .map(|n| {
                    0.42 - 0.5 * (angle * n as f64).cos() + 0.08 * (2.0 * angle * n as f64).cos()
                })
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            twiddle_real: (0..SIZE / 2)
                .map(|n| (-angle * n as f64).cos())
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            twiddle_imag: (0..SIZE / 2)
                .map(|n| (-angle * n as f64).sin())
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        }
    }
    fn spectrum(&mut self) -> Vec<f32> {
        for n in 0..SIZE {
            self.real[n] = self.samples[n] as f64 * self.window[n];
            self.imag[n] = 0.0;
        }
        for n in 0..SIZE {
            let reversed = n.reverse_bits() >> (usize::BITS - 12);
            if n < reversed {
                self.real.swap(n, reversed);
                self.imag.swap(n, reversed);
            }
        }
        let mut length = 2;
        while length <= SIZE {
            let half = length / 2;
            let step = SIZE / length;
            for start in (0..SIZE).step_by(length) {
                for index in 0..half {
                    let i = start + index;
                    let j = i + half;
                    let w = index * step;
                    let right_real =
                        self.real[j] * self.twiddle_real[w] - self.imag[j] * self.twiddle_imag[w];
                    let right_imag =
                        self.real[j] * self.twiddle_imag[w] + self.imag[j] * self.twiddle_real[w];
                    let left_real = self.real[i];
                    let left_imag = self.imag[i];
                    self.real[i] = left_real + right_real;
                    self.imag[i] = left_imag + right_imag;
                    self.real[j] = left_real - right_real;
                    self.imag[j] = left_imag - right_imag;
                }
            }
            length *= 2;
        }
        (0..SIZE / 2)
            .map(|n| {
                let magnitude = self.real[n].hypot(self.imag[n]) / SIZE as f64;
                if magnitude > 0.0 {
                    (20.0 * magnitude.log10()).clamp(-100.0, 0.0) as f32
                } else {
                    -100.0
                }
            })
            .collect()
    }
}

pub struct Monitor {
    slots: Box<[Slot]>,
    /// Low bit is enabled; high bits identify a capture generation. Off and
    /// re-enabling after an expired lease invalidate every older in-flight read.
    control: AtomicU64,
    lease: AtomicU64,
    published: AtomicU64,
    first: AtomicU64,
    head: AtomicU64,
    reader: Mutex<Reader>,
    reading: AtomicBool,
    rate: u32,
}
impl Monitor {
    pub fn new(rate: u32) -> Self {
        Self {
            slots: (0..SIZE)
                .map(|_| Slot::default())
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            control: AtomicU64::new(0),
            lease: AtomicU64::new(0),
            published: AtomicU64::new(0),
            first: AtomicU64::new(0),
            head: AtomicU64::new(0),
            reader: Mutex::new(Reader::new()),
            reading: AtomicBool::new(false),
            rate,
        }
    }
    pub fn off(&self) {
        // Clear the lease first. A newer read that sees the disabled generation
        // installs its own lease, while an older read can never re-enable that word.
        self.lease.store(0, Ordering::Release);
        let _ = self
            .control
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                Some(((value >> 1).wrapping_add(1)) << 1)
            });
    }
    fn begin(&self, expected: u64, frame: u64) -> Option<u64> {
        if self.control.load(Ordering::Acquire) != expected {
            return None;
        }
        let deadline = frame.saturating_add(self.rate as u64 * LEASE_SECONDS);
        let expired = self.lease.load(Ordering::Acquire) <= frame;
        self.lease.store(deadline, Ordering::Release);
        if expected & 1 != 0 && !expired {
            return (self.control.load(Ordering::Acquire) == expected).then_some(expected);
        }
        let next = (((expected >> 1).wrapping_add(1)) << 1) | 1;
        self.control
            .compare_exchange(expected, next, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| next)
    }
    /// This is the only callback operation: no allocation, lock, FFT, clock query,
    /// wait or PCM message. Disabled monitoring only reads one control atomic.
    pub fn write(&self, writer: &mut Writer, frame: u64, pcm: [f32; 2]) {
        let generation = self.control.load(Ordering::Acquire);
        if generation & 1 == 0 || frame >= self.lease.load(Ordering::Acquire) {
            return;
        }
        let changed = writer.generation != generation;
        if changed {
            writer.generation = generation;
            self.first.store(frame, Ordering::Relaxed);
        }
        let slot = &self.slots[frame as usize & MASK];
        let stamp = (frame + 1) << 1;
        // If the reader observes new PCM through the Acquire load, the preceding
        // busy stamp also happens-before its final stamp check. This prevents an
        // old stamp/new sample pair without locking or retrying the audio writer.
        slot.stamp.store(stamp | 1, Ordering::Relaxed);
        slot.stereo.store(
            pcm[0].to_bits() as u64 | ((pcm[1].to_bits() as u64) << 32),
            Ordering::Release,
        );
        slot.stamp.store(stamp, Ordering::Release);
        self.head.store(frame + 1, Ordering::Release);
        if changed {
            self.published.store(generation, Ordering::Release);
        }
    }
    fn copy(&self, generation: u64, channel: usize, reader: &mut Reader) -> Option<(u64, usize)> {
        reader.samples.fill(0.0);
        if self.published.load(Ordering::Acquire) != generation {
            return Some((0, 0));
        }
        let head = self.head.load(Ordering::Acquire);
        let first = self.first.load(Ordering::Acquire);
        let count = head.saturating_sub(first).min(SIZE as u64) as usize;
        let start = head - count as u64;
        for offset in 0..count {
            let frame = start + offset as u64;
            let slot = &self.slots[frame as usize & MASK];
            let expected = (frame + 1) << 1;
            if slot.stamp.load(Ordering::Acquire) != expected {
                return None;
            }
            let bits = slot.stereo.load(Ordering::Acquire);
            if slot.stamp.load(Ordering::Acquire) != expected {
                return None;
            }
            let value = f32::from_bits((bits >> (channel * 32)) as u32);
            reader.samples[SIZE - count + offset] = if value.is_finite() { value } else { 0.0 };
        }
        if self.control.load(Ordering::Acquire) != generation
            || self.published.load(Ordering::Acquire) != generation
        {
            return None;
        }
        Some((head, count))
    }
    pub fn try_reserve_read(&self) -> bool {
        self.reading
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
    pub fn read_reserved(
        &self,
        mode: Mode,
        channel: usize,
        session: u64,
        frame: u64,
        expected: u64,
    ) -> Result<Option<Snapshot>, String> {
        struct Release<'a>(&'a AtomicBool);
        impl Drop for Release<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }
        let _release = Release(&self.reading);
        self.read_expected(mode, channel, session, frame, expected)
    }
    pub fn generation(&self) -> u64 {
        self.control.load(Ordering::Acquire)
    }
    #[cfg(test)]
    pub fn read(
        &self,
        mode: Mode,
        channel: usize,
        session: u64,
        frame: u64,
    ) -> Result<Option<Snapshot>, String> {
        self.read_expected(mode, channel, session, frame, self.generation())
    }
    pub fn read_expected(
        &self,
        mode: Mode,
        channel: usize,
        session: u64,
        frame: u64,
        expected: u64,
    ) -> Result<Option<Snapshot>, String> {
        if channel > 1 {
            return Err("Native monitor channel must be 0 or 1".into());
        }
        if mode == Mode::Off {
            self.off();
            return Ok(None);
        }
        let mut reader = match self.reader.try_lock() {
            Ok(reader) => reader,
            Err(TryLockError::WouldBlock) => return Ok(None),
            Err(TryLockError::Poisoned(_)) => {
                return Err("Native audio monitor is unavailable".into())
            }
        };
        let Some(generation) = self.begin(expected, frame) else {
            return Ok(None);
        };
        let mut snapshot = None;
        for _ in 0..3 {
            if let Some(value) = self.copy(generation, channel, &mut reader) {
                snapshot = Some(value);
                break;
            }
        }
        let Some((head, count)) = snapshot else {
            return Ok(None);
        };
        let mut energy = 0.0f64;
        let mut peak = 0.0f32;
        for &value in &reader.samples[SIZE - count..] {
            energy += (value as f64) * (value as f64);
            peak = peak.max(value.abs());
        }
        let samples = if mode == Mode::Waveform {
            Some(reader.samples.to_vec())
        } else {
            None
        };
        let bins = if mode == Mode::Spectrum {
            Some(reader.spectrum())
        } else {
            None
        };
        if self.control.load(Ordering::Acquire) != generation {
            return Ok(None);
        }
        Ok(Some(Snapshot {
            session,
            frame: if count == 0 { frame } else { head },
            sample_rate: self.rate,
            frames: count,
            rms: if count == 0 {
                0.0
            } else {
                (energy / count as f64).sqrt() as f32
            },
            peak,
            samples,
            bins,
        }))
    }
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub session: u64,
    pub frame: u64,
    pub sample_rate: u32,
    pub frames: usize,
    pub rms: f32,
    pub peak: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub samples: Option<Vec<f32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bins: Option<Vec<f32>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn enabled(rate: u32) -> (Monitor, Writer) {
        let monitor = Monitor::new(rate);
        monitor.read(Mode::Envelope, 0, 7, 0).unwrap();
        (monitor, Writer::default())
    }
    #[test]
    fn rms_uses_valid_frames_and_waveform_is_left_padded_stereo() {
        let (monitor, mut writer) = enabled(48000);
        for (frame, pcm) in [[0.3, 0.8], [-0.4, 0.6], [0.0, -0.2]]
            .into_iter()
            .enumerate()
        {
            monitor.write(&mut writer, frame as u64, pcm);
        }
        let left = monitor.read(Mode::Waveform, 0, 7, 3).unwrap().unwrap();
        assert_eq!(left.frames, 3);
        assert_eq!(left.frame, 3);
        assert_eq!(left.session, 7);
        assert!((left.rms - (0.25f32 / 3.0).sqrt()).abs() < 1e-7);
        assert_eq!(left.peak, 0.4);
        let values = left.samples.unwrap();
        assert_eq!(values.len(), SIZE);
        assert!(values[..SIZE - 3].iter().all(|v| *v == 0.0));
        assert_eq!(&values[SIZE - 3..], &[0.3, -0.4, 0.0]);
        let right = monitor.read(Mode::Envelope, 1, 7, 3).unwrap().unwrap();
        assert!((right.rms - (1.04f32 / 3.0).sqrt()).abs() < 1e-7);
        assert_eq!(right.peak, 0.8);
        assert!(right.samples.is_none() && right.bins.is_none());
    }
    #[test]
    fn ring_wrap_keeps_only_the_latest_4096_frames() {
        let (monitor, mut writer) = enabled(48000);
        for frame in 0..(SIZE * 3) {
            monitor.write(
                &mut writer,
                frame as u64,
                [(frame % 97) as f32 / 100.0, 0.0],
            );
        }
        let result = monitor
            .read(Mode::Waveform, 0, 1, (SIZE * 3) as u64)
            .unwrap()
            .unwrap();
        assert_eq!(result.frame, (SIZE * 3) as u64);
        assert_eq!(result.frames, SIZE);
        let samples = result.samples.unwrap();
        for (offset, value) in samples.iter().enumerate() {
            assert_eq!(*value, ((SIZE * 2 + offset) % 97) as f32 / 100.0);
        }
    }
    #[test]
    fn blackman_fft_matches_tone_amplitude_bin_and_channel_with_no_smoothing() {
        let (monitor, mut writer) = enabled(48000);
        for frame in 0..SIZE {
            let phase = 2.0 * std::f64::consts::PI * frame as f64 / SIZE as f64;
            monitor.write(
                &mut writer,
                frame as u64,
                [
                    (phase * 64.0).sin() as f32 * 0.5,
                    (phase * 192.0).sin() as f32 * 0.25,
                ],
            );
        }
        for (channel, bin, amplitude) in [(0, 64, 0.5f64), (1, 192, 0.25)] {
            let result = monitor
                .read(Mode::Spectrum, channel, 1, SIZE as u64)
                .unwrap()
                .unwrap();
            let bins = result.bins.unwrap();
            assert_eq!(bins.len(), SIZE / 2);
            assert!(bins
                .iter()
                .all(|v| v.is_finite() && (-100.0..=0.0).contains(v)));
            let peak = bins
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .unwrap()
                .0;
            assert_eq!(peak, bin);
            let expected = 20.0 * (0.21 * amplitude).log10();
            assert!((bins[bin] as f64 - expected).abs() < 1e-5);
            assert!((result.rms as f64 - amplitude / std::f64::consts::SQRT_2).abs() < 1e-7);
            let next = monitor
                .read(Mode::Spectrum, channel, 1, SIZE as u64)
                .unwrap()
                .unwrap()
                .bins
                .unwrap();
            assert_eq!(bins, next);
        }
    }
    #[test]
    fn silence_is_finite_and_lease_expiry_and_off_remove_old_history() {
        let (monitor, mut writer) = enabled(8000);
        for frame in 0..16001 {
            monitor.write(&mut writer, frame, [0.5, -0.5]);
        }
        assert_eq!(monitor.head.load(Ordering::Acquire), 16000);
        let expired = monitor.read(Mode::Waveform, 0, 1, 18000).unwrap().unwrap();
        assert_eq!(expired.frames, 0);
        assert!(expired.samples.unwrap().iter().all(|v| *v == 0.0));
        monitor.write(&mut writer, 18000, [0.0, 0.0]);
        let result = monitor.read(Mode::Spectrum, 1, 1, 18001).unwrap().unwrap();
        assert_eq!(result.frames, 1);
        assert_eq!(result.rms, 0.0);
        assert_eq!(result.peak, 0.0);
        assert!(result.bins.unwrap().iter().all(|v| *v == -100.0));
        let old = monitor.control.load(Ordering::Acquire);
        assert!(monitor.read(Mode::Off, 0, 1, 18001).unwrap().is_none());
        assert!(monitor.begin(old, 18001).is_none());
        assert!(monitor
            .read_expected(Mode::Spectrum, 0, 1, 18001, old)
            .unwrap()
            .is_none());
        monitor.write(&mut writer, 18001, [0.8, 0.8]);
        assert_eq!(monitor.head.load(Ordering::Acquire), 18001);
        let fresh = monitor.read(Mode::Waveform, 0, 1, 18002).unwrap().unwrap();
        assert_eq!(fresh.frames, 0);
    }
    #[test]
    fn busy_reads_skip_without_waiting_and_invalid_channels_do_not_enable() {
        let monitor = Monitor::new(48000);
        assert!(monitor.read(Mode::Envelope, 2, 1, 0).is_err());
        assert_eq!(monitor.control.load(Ordering::Acquire), 0);
        assert!(monitor.try_reserve_read());
        assert!(!monitor.try_reserve_read());
        monitor
            .read_reserved(Mode::Envelope, 0, 1, 0, monitor.generation())
            .unwrap();
        assert!(!monitor.reading.load(Ordering::Acquire));
        let _busy = monitor.reader.lock().unwrap();
        assert!(monitor.read(Mode::Spectrum, 0, 1, 0).unwrap().is_none());
        monitor.off();
        assert_eq!(monitor.control.load(Ordering::Acquire) & 1, 0);
    }
    #[test]
    fn concurrent_ring_snapshots_never_observe_torn_stereo_samples() {
        use std::sync::Arc;
        let monitor = Arc::new(Monitor::new(48000));
        monitor.read(Mode::Envelope, 0, 1, 0).unwrap();
        let writer_monitor = monitor.clone();
        let thread = std::thread::spawn(move || {
            let mut writer = Writer::default();
            for frame in 0..90000 {
                let value = (frame % 1024) as f32 / 1024.0;
                writer_monitor.write(&mut writer, frame, [value, -value]);
            }
        });
        for _ in 0..64 {
            if let Some(snapshot) = monitor.read(Mode::Waveform, 0, 1, 0).unwrap() {
                let count = snapshot.frames;
                let values = snapshot.samples.unwrap();
                if count > 0 {
                    let first = snapshot.frame - count as u64;
                    for index in 0..count {
                        assert_eq!(
                            values[SIZE - count + index],
                            ((first + index as u64) % 1024) as f32 / 1024.0
                        );
                    }
                }
            }
        }
        thread.join().unwrap();
    }
    #[test]
    fn monitor_fft_fixture() {
        let (monitor, mut writer) = enabled(48000);
        let samples: Vec<f32> = (0..SIZE)
            .map(|frame| {
                let phase = 2.0 * std::f64::consts::PI * frame as f64 / SIZE as f64;
                (0.5 * (phase * 64.0).sin() + 0.1 * (phase * 192.0).cos() + 0.05) as f32
            })
            .collect();
        for (frame, &value) in samples.iter().enumerate() {
            monitor.write(&mut writer, frame as u64, [value, -value]);
        }
        let snapshot = monitor
            .read(Mode::Spectrum, 0, 1, SIZE as u64)
            .unwrap()
            .unwrap();
        assert!(snapshot.bins.as_ref().unwrap()[64] > -20.0);
        if let Ok(path) = std::env::var("TETORICA_NATIVE_MONITOR_FIXTURE") {
            let fixture = serde_json::json!({"sampleRate":48000,"samples":samples,"bins":snapshot.bins,"rms":snapshot.rms,"peak":snapshot.peak});
            std::fs::write(path, serde_json::to_vec(&fixture).unwrap()).unwrap();
        }
    }
    #[test]
    #[ignore = "explicit hardware-free timing sample; no output device is opened"]
    fn monitor_microbenchmark() {
        use std::{hint::black_box, time::Instant};
        let frames = 48000 * 5;
        let monitor = Monitor::new(48000);
        let mut writer = Writer::default();
        let disabled = Instant::now();
        for frame in 0..frames {
            monitor.write(black_box(&mut writer), frame, [0.25, -0.5]);
        }
        let disabled = disabled.elapsed();
        let mut enabled = std::time::Duration::ZERO;
        for second in 0..5 {
            monitor.read(Mode::Envelope, 0, 1, second * 48000).unwrap();
            let start = Instant::now();
            for frame in second * 48000..(second + 1) * 48000 {
                monitor.write(black_box(&mut writer), frame, [0.25, -0.5]);
            }
            enabled += start.elapsed();
        }
        let start = Instant::now();
        for _ in 0..30 {
            black_box(monitor.read(Mode::Spectrum, 0, 1, frames).unwrap());
        }
        eprintln!(
            "monitor 240000 frames disabled={disabled:?}, enabled={enabled:?}; 30 FFT reads={:?}",
            start.elapsed()
        );
    }
}
