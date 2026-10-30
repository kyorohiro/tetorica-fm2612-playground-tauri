//! Callback-owned SCSP core and Browser-equivalent fixed-rate conversion.
//! No JS, device access, locks or allocation in render/write/upload/reset.
//! Scheduling, admission and voice ownership belong to the caller. This layer
//! deliberately does not make the SCSP mode available in the Native UI yet.
use std::{ffi::c_void, ptr::NonNull};

pub const RAM_BYTES: usize = 512 * 1024;
pub const NATIVE_RATE: u32 = 44100;
extern "C" {
    fn scsp_create() -> *mut c_void;
    fn scsp_destroy(handle: *mut c_void);
    fn scsp_reset(handle: *mut c_void, clear_ram: i32) -> i32;
    fn scsp_write(handle: *mut c_void, offset: u32, value: u16, mask: u16) -> i32;
    fn scsp_read(handle: *mut c_void, offset: u32) -> u32;
    fn scsp_load_ram(handle: *mut c_void, address: u32, data: *const u8, bytes: u32) -> i32;
    fn scsp_read_ram(handle: *mut c_void, address: u32, data: *mut u8, bytes: u32) -> i32;
    fn scsp_render(handle: *mut c_void, left: *mut f32, right: *mut f32, frames: u32) -> i32;
}

/// Exclusive ownership makes the SDK's single-thread-at-a-time contract explicit.
/// Move onto a callback thread, but never share it across concurrent callbacks.
pub struct Core {
    handle: NonNull<c_void>,
    output_rate: u32,
    phase: u64,
    previous: [f32; 2],
    next: [f32; 2],
    primed: bool,
    native_frames: u64,
    output_frames: u64,
}
unsafe impl Send for Core {}
impl Drop for Core {
    fn drop(&mut self) {
        unsafe { scsp_destroy(self.handle.as_ptr()) }
    }
}
impl Core {
    pub fn new(output_rate: u32) -> Result<Self, &'static str> {
        if !(8000..=384000).contains(&output_rate) {
            return Err("Native SCSP output rate must be 8000..384000 Hz");
        }
        let handle =
            NonNull::new(unsafe { scsp_create() }).ok_or("Could not allocate Native SCSP")?;
        Ok(Self {
            handle,
            output_rate,
            phase: 0,
            previous: [0.; 2],
            next: [0.; 2],
            primed: false,
            native_frames: 0,
            output_frames: 0,
        })
    }
    pub fn output_rate(&self) -> u32 {
        self.output_rate
    }
    pub fn native_frames(&self) -> u64 {
        self.native_frames
    }
    pub fn output_frames(&self) -> u64 {
        self.output_frames
    }
    pub fn write(&mut self, offset: u32, value: u16, mask: u16) -> Result<(), &'static str> {
        if unsafe { scsp_write(self.handle.as_ptr(), offset, value, mask) } == 1 {
            Ok(())
        } else {
            Err("Invalid or unsupported Native SCSP register write")
        }
    }
    pub fn read(&self, offset: u32) -> Result<u16, &'static str> {
        u16::try_from(unsafe { scsp_read(self.handle.as_ptr(), offset) })
            .map_err(|_| "Invalid Native SCSP register read")
    }
    fn range(address: u32, bytes: usize) -> Result<u32, &'static str> {
        if address as usize > RAM_BYTES || bytes > RAM_BYTES - address as usize {
            Err("Native SCSP RAM range exceeds 512 KiB")
        } else {
            Ok(bytes as u32)
        }
    }
    pub fn load_ram(&mut self, address: u32, bytes: &[u8]) -> Result<(), &'static str> {
        let count = Self::range(address, bytes.len())?;
        if unsafe { scsp_load_ram(self.handle.as_ptr(), address, bytes.as_ptr(), count) } == 1 {
            Ok(())
        } else {
            Err("Native SCSP RAM upload failed")
        }
    }
    pub fn read_ram(&self, address: u32, bytes: &mut [u8]) -> Result<(), &'static str> {
        let count = Self::range(address, bytes.len())?;
        if unsafe { scsp_read_ram(self.handle.as_ptr(), address, bytes.as_mut_ptr(), count) } == 1 {
            Ok(())
        } else {
            Err("Native SCSP RAM read failed")
        }
    }
    /// Matches ScspSynth.stopVoices: clear DSP/registers/phase, retain RAM.
    /// Authored presets and sample handles live above this callback layer.
    pub fn stop_voices(&mut self) {
        self.reset(false);
        self.write(0x400, 0x020f, 0xffff).unwrap();
    }
    pub fn reset(&mut self, clear_ram: bool) {
        let ok = unsafe { scsp_reset(self.handle.as_ptr(), clear_ram as i32) };
        debug_assert_eq!(ok, 1);
        self.phase = 0;
        self.previous = [0.; 2];
        self.next = [0.; 2];
        self.primed = false;
        self.native_frames = 0;
        self.output_frames = 0;
    }
    fn pull(&mut self) -> [f32; 2] {
        let mut left = 0.;
        let mut right = 0.;
        let ok = unsafe { scsp_render(self.handle.as_ptr(), &mut left, &mut right, 1) };
        debug_assert_eq!(ok, 1);
        self.native_frames += 1;
        [left, right]
    }
    /// One host frame, with the same look-ahead and double arithmetic as
    /// Pcm44100Resampler. At 44.1 kHz there is no look-ahead or interpolation.
    pub fn sample(&mut self) -> [f32; 2] {
        self.output_frames += 1;
        if self.output_rate == NATIVE_RATE {
            return self.pull();
        }
        if !self.primed {
            self.previous = self.pull();
            self.next = self.pull();
            self.primed = true;
        }
        let fraction = self.phase as f64 / self.output_rate as f64;
        let sample = std::array::from_fn(|i| {
            let a = self.previous[i] as f64;
            let b = self.next[i] as f64;
            (a + (b - a) * fraction) as f32
        });
        self.phase += NATIVE_RATE as u64;
        while self.phase >= self.output_rate as u64 {
            self.phase -= self.output_rate as u64;
            self.previous = self.next;
            self.next = self.pull();
        }
        sample
    }
    /// Caller-owned output; validation does not advance either clock.
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) -> Result<(), &'static str> {
        if left.len() != right.len() {
            return Err("Native SCSP stereo lengths differ");
        }
        for (l, r) in left.iter_mut().zip(right) {
            let pcm = self.sample();
            *l = pcm[0];
            *r = pcm[1];
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounds_are_atomic_and_instances_are_independent() {
        assert!(Core::new(7999).is_err());
        assert!(Core::new(384001).is_err());
        let mut a = Core::new(48000).unwrap();
        let b = Core::new(48000).unwrap();
        a.load_ram(524286, &[1, 2]).unwrap();
        assert!(a.load_ram(524287, &[3, 4]).is_err());
        assert!(a.load_ram(u32::MAX, &[]).is_err());
        a.load_ram(524288, &[]).unwrap();
        let mut data = [0; 2];
        a.read_ram(524286, &mut data).unwrap();
        assert_eq!(data, [1, 2]);
        b.read_ram(524286, &mut data).unwrap();
        assert_eq!(data, [0, 0]);
        assert!(a.write(0x401, 0, 65535).is_err());
        a.write(0x400, 0x020f, 65535).unwrap();
        assert!(a.write(0x400, 0, 65535).is_err());
        assert_eq!(a.read(0x400), Ok(0x020f));
        assert!(a.read(0x1000).is_err());
        let mut l = [1.; 3];
        let mut r = [1.; 2];
        assert!(a.render(&mut l, &mut r).is_err());
        assert_eq!(a.native_frames(), 0);
        a.render(&mut [], &mut []).unwrap();
        assert_eq!(a.native_frames(), 0);
    }
    #[test]
    fn stop_and_clear_reset_have_different_ram_contracts() {
        let mut a = Core::new(48000).unwrap();
        a.load_ram(0, &[42]).unwrap();
        a.sample();
        assert_eq!(a.native_frames(), 2);
        assert_eq!(a.output_frames(), 1);
        a.stop_voices();
        assert_eq!(a.native_frames(), 0);
        assert_eq!(a.output_frames(), 0);
        assert_eq!(a.read(0x400), Ok(0x020f));
        let mut data = [0];
        a.read_ram(0, &mut data).unwrap();
        assert_eq!(data, [42]);
        a.reset(true);
        a.read_ram(0, &mut data).unwrap();
        assert_eq!(data, [0]);
        for rate in [8000, 44100, 48000, 96000, 384000] {
            let mut c = Core::new(rate).unwrap();
            assert_eq!(c.output_rate(), rate);
            for _ in 0..1000 {
                assert_eq!(c.sample(), [0.; 2]);
            }
            let expected = if rate == 44100 {
                1000
            } else {
                2 + 1000 * 44100 / rate as u64
            };
            assert_eq!(c.native_frames(), expected);
        }
    }
}
