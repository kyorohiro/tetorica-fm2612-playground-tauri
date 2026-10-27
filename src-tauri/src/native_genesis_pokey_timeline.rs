//! Compact immutable POKEY register trace. Decode/validation is control-thread only.
use serde::Deserialize;
pub const MAX_BYTES: usize = 8 * 1024 * 1024;
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Input {
    pub version: u32,
    pub clock: u32,
    pub duration_samples: u32,
    pub data: Vec<u8>,
}
#[derive(Debug)]
pub struct Timeline {
    pub clock: u32,
    pub duration: u32,
    pub data: Vec<u8>,
}
fn delta(bytes: &[u8], position: &mut usize) -> u32 {
    let mut value = 0;
    let mut shift = 0;
    loop {
        let byte = bytes[*position];
        *position += 1;
        value |= u32::from(byte & 127) << shift;
        if byte & 128 == 0 {
            return value;
        }
        shift += 7;
    }
}
impl Input {
    pub fn bytes(&self) -> usize {
        self.data.len()
    }
    pub fn prepare(&self) -> Result<Timeline, String> {
        let invalid = || "Invalid Native POKEY compact timeline".to_string();
        if self.version != 1
            || !(100_000..=100_000_000).contains(&self.clock)
            || self.data.len() > MAX_BYTES
        {
            return Err(invalid());
        }
        let mut pos = 0;
        let mut sample = 0u32;
        while pos < self.data.len() {
            let mut value = 0;
            let mut finished = false;
            for part in 0..5 {
                let byte = *self.data.get(pos).ok_or_else(invalid)?;
                pos += 1;
                if part == 4 && byte > 15 {
                    return Err(invalid());
                }
                value |= u32::from(byte & 127) << (part * 7);
                if byte & 128 == 0 {
                    if part > 0 && byte == 0 {
                        return Err(invalid());
                    }
                    finished = true;
                    break;
                }
            }
            if !finished || pos + 2 > self.data.len() || self.data[pos] > 15 {
                return Err(invalid());
            }
            sample = sample.checked_add(value).ok_or_else(invalid)?;
            if sample > self.duration_samples {
                return Err(invalid());
            }
            pos += 2;
        }
        Ok(Timeline {
            clock: self.clock,
            duration: self.duration_samples,
            data: self.data.clone(),
        })
    }
}
impl Timeline {
    pub fn bytes(&self) -> usize {
        self.data.len()
    }
    pub fn output_frame(&self, sample: u32, rate: u32) -> u64 {
        (u64::from(sample) * u64::from(rate)).div_ceil(44100)
    }
    pub fn validate_density(&self, rate: u32) -> Result<(), String> {
        let mut window = [0u64; 512];
        let mut count = 0;
        let mut cursor = Cursor::default();
        while let Some((sample, _, _)) = cursor.next_event(self) {
            let frame = self.output_frame(sample, rate);
            if count >= 512 && frame - window[count % 512] < 128 {
                return Err("Native POKEY timeline exceeds 512 writes in 128 output frames".into());
            }
            window[count % 512] = frame;
            count += 1;
        }
        Ok(())
    }
}
#[derive(Default)]
pub struct Cursor {
    position: usize,
    sample: u32,
    pub frame: u64,
}
impl Cursor {
    fn next_event(&mut self, trace: &Timeline) -> Option<(u32, u8, u8)> {
        if self.position >= trace.data.len() {
            return None;
        }
        self.sample += delta(&trace.data, &mut self.position);
        let register = trace.data[self.position];
        let value = trace.data[self.position + 1];
        self.position += 2;
        Some((self.sample, register, value))
    }
    pub fn next(&mut self, trace: &Timeline, rate: u32) -> Option<(u8, u8)> {
        let old_position = self.position;
        let old_sample = self.sample;
        let (sample, register, value) = self.next_event(trace)?;
        if trace.output_frame(sample, rate) > self.frame {
            self.position = old_position;
            self.sample = old_sample;
            return None;
        }
        Some((register, value))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn input(data: Vec<u8>) -> Input {
        Input {
            version: 1,
            clock: 1789790,
            duration_samples: 10,
            data,
        }
    }
    #[test]
    fn compact_order_and_fractional_frames() {
        let t = input(vec![0, 15, 3, 0, 1, 168, 2, 0, 70, 0, 1, 160])
            .prepare()
            .unwrap();
        for rate in [44100, 48000] {
            let mut c = Cursor::default();
            let mut out = vec![];
            for frame in 0..12 {
                c.frame = frame;
                while let Some((r, v)) = c.next(&t, rate) {
                    out.push((frame, r, v));
                }
            }
            assert_eq!(
                out,
                vec![
                    (0, 15, 3),
                    (0, 1, 168),
                    (t.output_frame(2, rate), 0, 70),
                    (t.output_frame(2, rate), 1, 160)
                ]
            );
        }
    }
    #[test]
    fn malformed_is_rejected_atomically() {
        for data in [
            vec![128, 0, 1, 2],
            vec![0, 16, 0],
            vec![0, 1],
            vec![255, 255, 255, 255, 16, 1, 1],
            vec![11, 1, 1],
        ] {
            assert!(input(data).prepare().is_err());
        }
    }
    #[test]
    fn exact_rolling_density_limit() {
        let t = input(vec![vec![0, 1, 1]; 512].concat()).prepare().unwrap();
        assert!(t.validate_density(48000).is_ok());
        let t = input(vec![vec![0, 1, 1]; 513].concat()).prepare().unwrap();
        assert!(t.validate_density(48000).is_err());
    }
}
