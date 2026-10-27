//! Compact, immutable WonderSwan trace. Validation never runs on the callback.
use serde::Deserialize;

pub const MAX_BYTES: usize = 8 * 1024 * 1024;
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Input {
    version: u32,
    pub clock: u32,
    pub duration_samples: u32,
    events: Vec<Vec<u32>>,
    pcm_data: Vec<u8>,
}
#[derive(Debug)]
pub struct Timeline {
    pub clock: u32,
    pub duration: u32,
    pub events: Vec<[u32; 6]>,
    pub pcm: Vec<u8>,
}
pub fn delta(bytes: &[u8], position: &mut usize) -> u32 {
    let mut value = 0u32;
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
        self.pcm_data
            .len()
            .saturating_add(self.events.len().saturating_mul(32))
    }
    pub fn prepare(&self) -> Result<Timeline, String> {
        let invalid = || "Invalid Native WonderSwan compact timeline".to_string();
        if self.version != 1
            || !(100_000..=100_000_000).contains(&self.clock)
            || self.bytes() > MAX_BYTES
        {
            return Err(invalid());
        }
        let mut events = Vec::with_capacity(self.events.len());
        let mut previous = 0;
        let mut voice_pcm = false;
        let mut work_bytes = self.events.len() * 32;
        for (index, source) in self.events.iter().enumerate() {
            if source.len() < 2 || source[0] < previous || source[0] > self.duration_samples {
                return Err(invalid());
            }
            let mut event = [0; 6];
            match source[1] {
                0 | 1 => {
                    if source.len() != 4
                        || source[3] > 255
                        || source[1] == 0 && !(0x80..=0x9f).contains(&source[2])
                        || source[1] == 1 && source[2] >= 16384
                    {
                        return Err(invalid());
                    }
                    event[..4].copy_from_slice(source);
                    if source[1] == 0 && source[2] == 0x90 {
                        voice_pcm = source[3] & 32 != 0;
                    }
                }
                2 => {
                    if source.len() != 6
                        || source[3] == 0
                        || source[4] == 0
                        || !voice_pcm
                        || source[5] < source[0]
                        || source[5] > self.duration_samples
                        || self
                            .events
                            .get(index + 1)
                            .is_some_and(|next| next.first().is_none_or(|time| source[5] > *time))
                    {
                        return Err(invalid());
                    }
                    work_bytes = work_bytes
                        .checked_add(source[3] as usize)
                        .ok_or_else(invalid)?;
                    if work_bytes > MAX_BYTES {
                        return Err(invalid());
                    }
                    let begin = source[2] as usize;
                    let end = begin.checked_add(source[3] as usize).ok_or_else(invalid)?;
                    if end > self.pcm_data.len() {
                        return Err(invalid());
                    }
                    let mut position = begin;
                    let mut sample = source[0];
                    for entry in 0..source[4] {
                        let mut value = 0u32;
                        let mut finished = false;
                        for part in 0..5 {
                            if position >= end {
                                return Err(invalid());
                            }
                            let byte = self.pcm_data[position];
                            position += 1;
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
                        if !finished || entry == 0 && value != 0 || position >= end {
                            return Err(invalid());
                        }
                        sample = sample.checked_add(value).ok_or_else(invalid)?;
                        if sample > source[5] {
                            return Err(invalid());
                        }
                        position += 1;
                    }
                    if position != end || sample != source[5] {
                        return Err(invalid());
                    }
                    event.copy_from_slice(source);
                }
                _ => return Err(invalid()),
            }
            previous = source[0];
            events.push(event);
        }
        Ok(Timeline {
            clock: self.clock,
            duration: self.duration_samples,
            events,
            pcm: self.pcm_data.clone(),
        })
    }
}
impl Timeline {
    pub fn bytes(&self) -> usize {
        self.pcm.len() + self.events.len() * 32
    }
    pub fn output_frame(&self, sample: u32, rate: u32) -> u64 {
        (u64::from(sample) * u64::from(rate)).div_ceil(44100)
    }
    pub fn validate_density(&self, rate: u32) -> Result<(), String> {
        let mut window = [0u64; 512];
        let mut count = 0usize;
        let mut observe = |sample| -> Result<(), String> {
            let frame = self.output_frame(sample, rate);
            if count >= 512 && frame - window[count % 512] < 128 {
                return Err(
                    "Native WonderSwan timeline exceeds 512 writes in 128 output frames".into(),
                );
            }
            window[count % 512] = frame;
            count += 1;
            Ok(())
        };
        for event in &self.events {
            if event[1] != 2 {
                observe(event[0])?;
                continue;
            }
            let mut position = event[2] as usize;
            let mut sample = event[0];
            for _ in 0..event[4] {
                sample += delta(&self.pcm, &mut position);
                position += 1;
                observe(sample)?;
            }
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct Cursor {
    pub frame: u64,
    pub event: usize,
    pub pcm_position: usize,
    pub pcm_remaining: u32,
    pub pcm_sample: u32,
}
impl Cursor {
    /// The returned register/RAM writes preserve even equal-sample source order.
    pub fn next(&mut self, timeline: &Timeline, rate: u32) -> Option<(u32, u32, u8)> {
        loop {
            if self.pcm_remaining > 0 {
                if timeline.output_frame(self.pcm_sample, rate) > self.frame {
                    return None;
                }
                let value = timeline.pcm[self.pcm_position];
                self.pcm_position += 1;
                self.pcm_remaining -= 1;
                if self.pcm_remaining > 0 {
                    self.pcm_sample += delta(&timeline.pcm, &mut self.pcm_position);
                }
                return Some((0, 0x89, value));
            }
            let event = *timeline.events.get(self.event)?;
            if timeline.output_frame(event[0], rate) > self.frame {
                return None;
            }
            self.event += 1;
            if event[1] < 2 {
                return Some((event[1], event[2], event[3] as u8));
            }
            self.pcm_position = event[2] as usize;
            self.pcm_remaining = event[4];
            self.pcm_sample = event[0] + delta(&timeline.pcm, &mut self.pcm_position);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn input() -> Input {
        serde_json::from_value(json!({"version":1,"clock":3072000,"durationSamples":20,"events":[[0,0,144,34],[0,2,0,6,3,4],[4,0,137,99]],"pcmData":[0,10,2,20,2,30]})).unwrap()
    }
    #[test]
    fn compact_cursor_exact_order_at_44100_and_48000() {
        let timeline = input().prepare().unwrap();
        for rate in [44100, 48000] {
            let mut cursor = Cursor::default();
            let mut actual = Vec::new();
            for frame in 0..8 {
                cursor.frame = frame;
                while let Some((kind, address, value)) = cursor.next(&timeline, rate) {
                    actual.push((frame, kind, address, value));
                }
            }
            assert_eq!(
                actual,
                vec![
                    (0, 0, 144, 34),
                    (0, 0, 137, 10),
                    (timeline.output_frame(2, rate), 0, 137, 20),
                    (timeline.output_frame(4, rate), 0, 137, 30),
                    (timeline.output_frame(4, rate), 0, 137, 99)
                ]
            );
        }
    }
    #[test]
    fn rejects_malformed_compact_traces() {
        let good = input();
        let mut bad = good.clone();
        bad.pcm_data[0] = 1;
        assert!(bad.prepare().is_err());
        let mut bad = good.clone();
        bad.pcm_data = vec![128, 0, 10, 2, 20, 2, 30];
        assert!(bad.prepare().is_err());
        let mut bad = good.clone();
        bad.events[1][4] = 4;
        assert!(bad.prepare().is_err());
        let mut bad = good.clone();
        bad.events[1][5] = 3;
        assert!(bad.prepare().is_err());
        let mut bad = good.clone();
        bad.events[2][0] = 3;
        assert!(bad.prepare().is_err());
        let mut bad = good.clone();
        bad.events[0][3] = 0;
        assert!(bad.prepare().is_err());
        let mut bad = good.clone();
        bad.pcm_data.resize(MAX_BYTES, 0);
        assert!(bad.prepare().is_err());
    }
}
