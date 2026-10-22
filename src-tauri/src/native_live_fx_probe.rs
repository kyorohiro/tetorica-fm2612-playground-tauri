//! Single-owner QuickJS-NG rack used by the native generation worker and probe.
//!
//! A `LiveFxHost` and every call into it must remain on one owner thread. Neither
//! the Native Genesis audio callback nor a Tauri IPC command constructs this host.
//! The C++ host itself is lazy: its VM and PCM storage appear on first register.
#![allow(dead_code)]

use std::{
    ffi::{c_char, c_void},
    marker::PhantomData,
    ptr::NonNull,
    rc::Rc,
};

const ERROR_CAPACITY: usize = 8192;
const REPORT_CAPACITY: usize = 16 * 1024 + 1;
pub(crate) const MAX_COMMAND_BYTES: usize = 256 * 1024;

extern "C" {
    fn native_genesis_livefx_create(
        rate: u32,
        max_frames: u32,
        memory_limit: usize,
        budget_us: u64,
    ) -> *mut c_void;
    fn native_genesis_livefx_destroy(host: *mut c_void);
    fn native_genesis_livefx_command(
        host: *mut c_void,
        action: i32,
        command: *const c_char,
        length: usize,
        error: *mut c_char,
        capacity: usize,
    ) -> i32;
    fn native_genesis_livefx_process(
        host: *mut c_void,
        stereo: *mut f32,
        frames: u32,
        error: *mut c_char,
        capacity: usize,
    ) -> i32;
    fn native_genesis_livefx_clear(host: *mut c_void);
    fn native_genesis_livefx_active(host: *mut c_void) -> u32;
    fn native_genesis_livefx_initialized(host: *mut c_void) -> u32;
    fn native_genesis_livefx_faulted(host: *mut c_void) -> u32;
    fn native_genesis_livefx_process_calls(host: *mut c_void) -> u64;
    fn native_genesis_livefx_initializations(host: *mut c_void) -> u64;
    fn native_genesis_livefx_reports(
        host: *mut c_void,
        output: *mut c_char,
        capacity: usize,
    ) -> usize;
}

pub(crate) struct LiveFxHost {
    host: NonNull<c_void>,
    max_frames: u32,
    // Prevent sharing or moving an initialized QuickJS context across threads.
    _owner: PhantomData<Rc<()>>,
}

impl LiveFxHost {
    pub(crate) fn new(
        rate: u32,
        max_frames: u32,
        memory_limit: usize,
        budget_us: u64,
    ) -> Result<Self, String> {
        let host = NonNull::new(unsafe {
            native_genesis_livefx_create(rate, max_frames, memory_limit, budget_us)
        })
        .ok_or("Invalid native LiveFX host configuration or allocation failure")?;
        Ok(Self {
            host,
            max_frames,
            _owner: PhantomData,
        })
    }

    pub(crate) fn command(&mut self, command: &serde_json::Value) -> Result<(), String> {
        let action = match command.get("action").and_then(|value| value.as_str()) {
            Some("register") => 0,
            Some("context") => 1,
            Some("remove") => 2,
            _ => return Err("Unknown native LiveFX action".into()),
        };
        let source = serde_json::to_vec(command).map_err(|error| error.to_string())?;
        if source.len() > MAX_COMMAND_BYTES {
            return Err("Native LiveFX command exceeds 256 KiB limit".into());
        }
        let mut error = [0u8; ERROR_CAPACITY];
        let status = unsafe {
            native_genesis_livefx_command(
                self.host.as_ptr(),
                action,
                source.as_ptr().cast(),
                source.len(),
                error.as_mut_ptr().cast(),
                error.len(),
            )
        };
        Self::result(status, &error)
    }

    pub(crate) fn process(&mut self, stereo: &mut [f32]) -> Result<(), String> {
        if stereo.is_empty() || stereo.len() % 2 != 0 || stereo.len() / 2 > self.max_frames as usize
        {
            return Err("Invalid native LiveFX stereo block".into());
        }
        let mut error = [0u8; ERROR_CAPACITY];
        let status = unsafe {
            native_genesis_livefx_process(
                self.host.as_ptr(),
                stereo.as_mut_ptr(),
                (stereo.len() / 2) as u32,
                error.as_mut_ptr().cast(),
                error.len(),
            )
        };
        Self::result(status, &error)
    }

    fn result(status: i32, error: &[u8]) -> Result<(), String> {
        if status != 0 {
            Ok(())
        } else {
            let end = error
                .iter()
                .position(|&byte| byte == 0)
                .unwrap_or(error.len());
            Err(String::from_utf8_lossy(&error[..end]).into_owned())
        }
    }

    pub(crate) fn clear(&mut self) {
        unsafe { native_genesis_livefx_clear(self.host.as_ptr()) }
    }

    pub(crate) fn reports(&mut self) -> String {
        let mut output = [0u8; REPORT_CAPACITY];
        let written = unsafe {
            native_genesis_livefx_reports(
                self.host.as_ptr(),
                output.as_mut_ptr().cast(),
                output.len(),
            )
        }
        .min(output.len());
        let end = output[..written]
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(written);
        String::from_utf8_lossy(&output[..end]).into_owned()
    }

    pub(crate) fn initialized(&self) -> bool {
        unsafe { native_genesis_livefx_initialized(self.host.as_ptr()) != 0 }
    }

    pub(crate) fn active(&self) -> u32 {
        unsafe { native_genesis_livefx_active(self.host.as_ptr()) }
    }

    pub(crate) fn faulted(&self) -> bool {
        unsafe { native_genesis_livefx_faulted(self.host.as_ptr()) != 0 }
    }

    pub(crate) fn process_calls(&self) -> u64 {
        unsafe { native_genesis_livefx_process_calls(self.host.as_ptr()) }
    }

    pub(crate) fn initializations(&self) -> u64 {
        unsafe { native_genesis_livefx_initializations(self.host.as_ptr()) }
    }
}

impl Drop for LiveFxHost {
    fn drop(&mut self) {
        unsafe { native_genesis_livefx_destroy(self.host.as_ptr()) }
    }
}

#[cfg(test)]
mod tests {
    use super::{LiveFxHost as Probe, MAX_COMMAND_BYTES};
    use serde_json::json;
    use std::time::{Duration, Instant};

    fn probe() -> Probe {
        Probe::new(48_000, 128, 16 * 1024 * 1024, 50_000).unwrap()
    }

    fn register(probe: &mut Probe, source: &str, context: serde_json::Value) {
        probe
            .command(&json!({"op":"live-fx","action":"register","name":"test",
                "source":source,"context":context,"resetState":false}))
            .unwrap();
    }

    const GAIN: &str = "process(input, output, state, context) { for(let c=0;c<2;c++) for(let i=0;i<input[c].length;i++) output[c][i]=input[c][i]*context.gain; }";

    #[test]
    fn empty_rack_has_no_vm_or_javascript_processing() {
        let mut probe = probe();
        assert!(!probe.initialized());
        assert_eq!(probe.initializations(), 0);
        let mut pcm = vec![0.25, -0.5, 0.75, -0.125];
        let expected = pcm.clone();
        for _ in 0..128 {
            probe.process(&mut pcm).unwrap();
        }
        assert_eq!(pcm, expected);
        assert_eq!(probe.active(), 0);
        assert_eq!(probe.process_calls(), 0);
        assert_eq!(probe.initializations(), 0);
        assert!(!probe.initialized());
    }

    #[test]
    fn actual_device_sample_rate_is_a_read_only_global_for_existing_dsp() {
        for rate in [44_100, 48_000, 96_000] {
            let mut probe = Probe::new(rate, 128, 16 * 1024 * 1024, 50_000).unwrap();
            register(
                &mut probe,
                "process(input, output, state, context) { const rate=Reflect.get(globalThis,'sampleRate'); const descriptor=Object.getOwnPropertyDescriptor(globalThis,'sampleRate'); if(rate!==context.rate||descriptor.writable||descriptor.configurable) throw new Error('Native sampleRate changed'); for(let c=0;c<2;c++) output[c].fill(rate/1000000); }",
                json!({"rate":rate}),
            );
            let mut pcm = [0.; 4];
            probe.process(&mut pcm).unwrap();
            assert_eq!(pcm, [rate as f32 / 1_000_000.; 4]);
            assert!(probe.reports().is_empty());
        }
    }

    #[test]
    fn actual_engine_processes_stereo_and_releases_vm_on_clear() {
        let mut probe = probe();
        register(&mut probe, GAIN, json!({"gain":0.5}));
        assert!(probe.initialized());
        assert_eq!(probe.active(), 1);
        let mut pcm = vec![0.8, -0.6, 0.4, -0.2];
        probe.process(&mut pcm).unwrap();
        assert_eq!(pcm, vec![0.4, -0.3, 0.2, -0.1]);
        assert_eq!(probe.process_calls(), 1);
        assert!(probe.reports().is_empty());
        probe.clear();
        assert!(!probe.initialized());
        assert_eq!(probe.active(), 0);
        let original = pcm.clone();
        probe.process(&mut pcm).unwrap();
        assert_eq!(pcm, original);
        assert_eq!(probe.process_calls(), 1);
    }

    #[test]
    fn command_uses_only_the_requested_bytes_without_a_c_string_terminator() {
        let mut probe = probe();
        let mut source = serde_json::to_vec(&json!({
            "action":"register", "name":"test", "source":GAIN, "context":{"gain":0.5}
        }))
        .unwrap();
        let length = source.len();
        // A Rust byte slice need not be NUL terminated. Keep an invalid JSON
        // sentinel immediately beyond its logical end, as MSVC's debug heap did.
        source.extend_from_slice(&[0xcc, 0]);
        let mut error = [0u8; super::ERROR_CAPACITY];
        let status = unsafe {
            super::native_genesis_livefx_command(
                probe.host.as_ptr(),
                0,
                source.as_ptr().cast(),
                length,
                error.as_mut_ptr().cast(),
                error.len(),
            )
        };
        Probe::result(status, &error).unwrap();
        assert_eq!(probe.active(), 1);
        let mut pcm = [0.8, -0.6];
        probe.process(&mut pcm).unwrap();
        assert_eq!(pcm, [0.4, -0.3]);
        assert_eq!(&source[length..], &[0xcc, 0]);
    }

    #[test]
    fn last_removal_releases_vm_and_hot_readdition_retains_fade() {
        let mut probe = probe();
        register(&mut probe, GAIN, json!({"gain":0.5}));
        probe.process(&mut [0.8; 256]).unwrap();
        probe
            .command(&json!({"action":"remove","name":"test"}))
            .unwrap();
        assert_eq!(probe.active(), 0);
        assert!(!probe.initialized());
        let calls = probe.process_calls();
        let mut dry = [0.8; 256];
        probe.process(&mut dry).unwrap();
        assert_eq!(dry, [0.8; 256]);
        assert_eq!(probe.process_calls(), calls);
        register(&mut probe, GAIN, json!({"gain":0.25}));
        assert!(probe.initialized());
        assert_eq!(probe.initializations(), 2);
        let mut pcm = [0.8; 256];
        probe.process(&mut pcm).unwrap();
        assert_eq!(pcm[0], 0.8);
        assert!(pcm[254] > 0.2 && pcm[254] < 0.8);
        pcm.fill(0.8);
        probe.process(&mut pcm).unwrap();
        assert_eq!(pcm[254], 0.2);
    }

    #[test]
    fn state_context_and_replacement_follow_shared_rack_contract() {
        let mut probe = probe();
        let source = "process(input, output, state, context) { state.n=(state.n||0)+1; for(let c=0;c<2;c++) output[c].fill((state.n+context.offset)*context.gain); }";
        register(&mut probe, source, json!({"gain":0.1,"offset":1}));
        let mut pcm = vec![0.; 4];
        probe.process(&mut pcm).unwrap();
        assert_eq!(pcm, vec![0.2; 4]);
        probe
            .command(&json!({"action":"context","name":"test","context":{"offset":2}}))
            .unwrap();
        probe.process(&mut pcm).unwrap();
        assert_eq!(pcm, vec![0.4; 4]);
        register(&mut probe, source, json!({"gain":0.1,"offset":0}));
        probe.process(&mut pcm).unwrap();
        assert_eq!(pcm, vec![0.3; 4]);
        probe
            .command(&json!({"action":"register","name":"test","source":source,
            "context":{"gain":0.1,"offset":0},"resetState":true}))
            .unwrap();
        probe.process(&mut pcm).unwrap();
        assert_eq!(pcm, vec![0.1; 4]);
    }

    #[test]
    fn throwing_effect_is_reported_and_bypassed_until_replacement() {
        let mut probe = probe();
        register(
            &mut probe,
            "process() { throw new Error('probe failure'); }",
            json!({}),
        );
        let mut pcm = vec![0.8, -0.6];
        probe.process(&mut pcm).unwrap();
        assert_eq!(pcm, vec![0.8, -0.6]);
        let report = probe.reports();
        assert!(report.contains("[liveFx:test]"), "{report}");
        assert!(report.contains("probe failure"), "{report}");
        assert!(report.contains("Effect bypassed"), "{report}");
        probe.process(&mut pcm).unwrap();
        assert!(probe.reports().is_empty());
        register(&mut probe, GAIN, json!({"gain":0.5}));
        probe.process(&mut pcm).unwrap();
        assert_eq!(pcm, vec![0.4, -0.3]);
    }

    #[test]
    fn invalid_nonfinite_output_does_not_escape_to_pcm() {
        let mut probe = probe();
        register(
            &mut probe,
            "process(input, output) { output[0].fill(NaN); output[1].fill(Infinity); }",
            json!({}),
        );
        let mut pcm = vec![0.2, -0.3, 0.4, -0.5];
        let expected = pcm.clone();
        probe.process(&mut pcm).unwrap();
        assert_eq!(pcm, expected);
        let report = probe.reports();
        assert!(report.contains("[liveFx:test]"), "{report}");
        assert!(report.contains("Non-finite output"), "{report}");
    }

    #[test]
    fn unbounded_user_loop_is_interrupted_offline() {
        let mut probe = Probe::new(48_000, 128, 16 * 1024 * 1024, 5_000).unwrap();
        register(&mut probe, "process() { for (;;) {} }", json!({}));
        let mut pcm = vec![0.2, -0.3];
        let expected = pcm.clone();
        let start = Instant::now();
        let result = probe.process(&mut pcm);
        assert!(start.elapsed() < Duration::from_secs(2));
        assert_eq!(pcm, expected);
        assert!(result.is_err() || probe.faulted() || !probe.reports().is_empty());
        probe.clear();
        register(&mut probe, GAIN, json!({"gain":0.5}));
        probe.process(&mut pcm).unwrap();
        assert_eq!(pcm, vec![0.1, -0.15]);
    }

    #[test]
    fn javascript_heap_allocation_is_bounded() {
        let mut probe = Probe::new(48_000, 128, 4 * 1024 * 1024, 50_000).unwrap();
        register(&mut probe, "process(input, output, state) { state.large=new Uint8Array(64*1024*1024); output[0].fill(1); output[1].fill(1); }", json!({}));
        let mut pcm = vec![0.2, -0.3];
        let expected = pcm.clone();
        let result = probe.process(&mut pcm);
        assert_eq!(pcm, expected);
        assert!(result.is_err() || probe.faulted() || !probe.reports().is_empty());
    }

    #[test]
    fn rust_adapter_rejects_out_of_bounds_blocks_without_entering_js() {
        let mut probe = probe();
        assert!(probe.process(&mut []).is_err());
        assert!(probe.process(&mut [0.; 3]).is_err());
        assert!(probe.process(&mut [0.; 258]).is_err());
        assert_eq!(probe.process_calls(), 0);
        assert!(!probe.initialized());
        assert!(probe.command(&json!({"action":"unknown"})).is_err());
    }

    #[test]
    fn oversized_command_is_rejected_before_initializing_engine() {
        let mut probe = probe();
        let error = probe
            .command(&json!({"action":"register","name":"large",
                "source":" ".repeat(MAX_COMMAND_BYTES),"context":{}}))
            .unwrap_err();
        assert!(error.contains("256 KiB"), "{error}");
        assert!(!probe.initialized());
        assert_eq!(probe.initializations(), 0);
    }

    #[test]
    fn failed_first_registration_releases_the_unused_vm_and_can_retry() {
        let mut probe = probe();
        assert!(probe
            .command(&json!({"action":"register","name":"test","source":"invalid {", "context":{}}))
            .is_err());
        assert!(!probe.initialized());
        assert_eq!(probe.active(), 0);
        assert!(!probe.faulted());
        register(&mut probe, GAIN, json!({"gain":0.5}));
        let mut pcm = [0.5, -0.25];
        probe.process(&mut pcm).unwrap();
        assert_eq!(pcm, [0.25, -0.125]);
    }

    #[test]
    fn bounded_reports_are_not_truncated_by_rust_and_clear_discards_old_reports() {
        let mut probe = probe();
        let source = "process() { throw new Error('x'.repeat(20000)); }";
        register(&mut probe, source, json!({}));
        probe.process(&mut [0.1, -0.1]).unwrap();
        let report = probe.reports();
        assert_eq!(report.len(), 16 * 1024);
        assert!(report.starts_with("[liveFx:test]"), "{report}");
        assert!(probe.reports().is_empty());
        register(&mut probe, source, json!({}));
        probe.process(&mut [0.1, -0.1]).unwrap();
        probe.clear();
        assert!(probe.reports().is_empty());
        assert!(!probe.initialized());
    }

    #[test]
    fn fatal_rack_stays_dry_until_explicit_clear_and_then_can_be_reused() {
        let mut probe = Probe::new(48_000, 128, 16 * 1024 * 1024, 5_000).unwrap();
        register(&mut probe, "process() { for (;;) {} }", json!({}));
        let mut pcm = [0.2, -0.3];
        assert!(probe.process(&mut pcm).is_err());
        assert!(probe.faulted());
        assert_eq!(pcm, [0.2, -0.3]);
        let calls = probe.process_calls();
        for _ in 0..16 {
            probe.process(&mut pcm).unwrap();
        }
        assert_eq!(probe.process_calls(), calls);
        assert_eq!(pcm, [0.2, -0.3]);
        let error = probe
            .command(&json!({"action":"register","name":"test", "source":GAIN,
                "context":{"gain":0.5}}))
            .unwrap_err();
        assert!(error.contains("Stop and Run"), "{error}");
        probe.clear();
        assert!(!probe.faulted());
        register(&mut probe, GAIN, json!({"gain":0.5}));
        probe.process(&mut pcm).unwrap();
        assert_eq!(pcm, [0.1, -0.15]);
    }
}
