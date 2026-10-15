use std::ffi::{c_int, c_uint, c_ulong, c_void};
type Metric = unsafe extern "C" fn(*mut c_void, *mut f64) -> c_int;
type Peak = unsafe extern "C" fn(*mut c_void, c_uint, *mut f64) -> c_int;
extern "C" {
    fn ebur128_get_version(major: *mut c_int, minor: *mut c_int, patch: *mut c_int);
    fn ebur128_init(channels: c_uint, rate: c_ulong, mode: c_int) -> *mut c_void;
    fn ebur128_destroy(state: *mut *mut c_void);
    fn ebur128_set_channel(state: *mut c_void, channel: c_uint, kind: c_int) -> c_int;
    fn ebur128_add_frames_float(state: *mut c_void, samples: *const f32, frames: usize) -> c_int;
    fn ebur128_loudness_global(state: *mut c_void, out: *mut f64) -> c_int;
    fn ebur128_loudness_range(state: *mut c_void, out: *mut f64) -> c_int;
    fn ebur128_loudness_shortterm(state: *mut c_void, out: *mut f64) -> c_int;
    fn ebur128_loudness_momentary(state: *mut c_void, out: *mut f64) -> c_int;
    fn ebur128_sample_peak(state: *mut c_void, channel: c_uint, out: *mut f64) -> c_int;
    fn ebur128_true_peak(state: *mut c_void, channel: c_uint, out: *mut f64) -> c_int;
}
pub(super) struct Api {
    pub version: String,
}
impl Api {
    /// Inspect the bundled native meter ABI.
    /// Args: none. Returns its real compiled version and pinned source identity; no dynamic fallback is used.
    pub fn open() -> Result<Self, String> {
        let (mut major, mut minor, mut patch) = (0, 0, 0);
        unsafe {
            ebur128_get_version(&mut major, &mut minor, &mut patch);
        }
        if (major, minor, patch) != (1, 2, 6) {
            return Err("Bundled meter version differs from pinned source".into());
        }
        Ok(Self {
            version: format!(
                "libebur128{major}.{minor}.{patch} static; source{}",
                env!("EBUR128_SOURCE_SHA256")
            ),
        })
    }
}
pub(super) struct Meter {
    pub api: Api,
    state: *mut c_void,
}
impl Drop for Meter {
    fn drop(&mut self) {
        unsafe { ebur128_destroy(&mut self.state) }
    }
}
impl Meter {
    /// Prepare bounded loudness/peak measurement.
    /// Args: actual rate/count and canonical mask. Returns native state plus whether layout weights are qualified; unknown wide layouts retain peak-only measurement.
    pub fn new(rate: u32, channels: u16, mask: Option<u32>) -> Result<(Self, bool), String> {
        let api = Api::open()?;
        let supported = match mask {
            Some(m) => m.count_ones() == u32::from(channels) && m & !0x63f == 0,
            None => channels <= 2,
        };
        let state = unsafe {
            ebur128_init(
                u32::from(channels),
                rate as c_ulong,
                if supported { 127 } else { 49 },
            )
        };
        if state.is_null() {
            return Err("Cannot allocate bounded delivery loudness meter".into());
        }
        let meter = Self { api, state };
        let map = if let Some(mask) = mask {
            (0..32)
                .filter(|bit| mask & (1 << bit) != 0)
                .map(|bit| match bit {
                    0 => 1,
                    1 => 2,
                    2 => 3,
                    3 => 0,
                    4 => 13,
                    5 => 14,
                    9 => 11,
                    10 => 12,
                    _ => 0,
                })
                .collect::<Vec<_>>()
        } else if channels == 1 {
            vec![3]
        } else if channels == 2 {
            vec![1, 2]
        } else {
            vec![0; channels as usize]
        };
        if map.len() != channels as usize {
            return Err("QC mask differs from encoded channel count".into());
        }
        for (i, channel) in map.into_iter().enumerate() {
            if unsafe { ebur128_set_channel(state, i as u32, channel) } != 0 {
                return Err("Meter refused canonical speaker channel".into());
            }
        }
        Ok((meter, supported))
    }
    /// Feed complete decoded frames.
    /// Args: finite interleaved samples and channel count. Returns after the actual native meter accepts them.
    pub fn add(&self, samples: &[f32], channels: usize) -> Result<(), String> {
        if unsafe {
            ebur128_add_frames_float(self.state, samples.as_ptr(), samples.len() / channels)
        } != 0
        {
            return Err("Native loudness meter failed".into());
        }
        Ok(())
    }
    /// Read a standards metric.
    /// Args: integrated/range/short/momentary selector. Returns a finite value or None for ungated silence.
    pub fn metric(&self, kind: u8) -> Result<Option<f64>, String> {
        let f: Metric = match kind {
            0 => ebur128_loudness_global,
            1 => ebur128_loudness_range,
            2 => ebur128_loudness_shortterm,
            _ => ebur128_loudness_momentary,
        };
        let mut out = 0.;
        if unsafe { f(self.state, &mut out) } != 0 {
            return Err("Native loudness metric unavailable".into());
        }
        Ok(out.is_finite().then_some(out))
    }
    /// Read an actual sample or interpolated peak.
    /// Args: channel and peak kind. Returns finite linear amplitude, refusing unsupported native state.
    pub fn peak(&self, channel: u32, true_peak: bool) -> Result<f64, String> {
        let f: Peak = if true_peak {
            ebur128_true_peak
        } else {
            ebur128_sample_peak
        };
        let mut out = 0.;
        if unsafe { f(self.state, channel, &mut out) } != 0 || !out.is_finite() {
            return Err("Native peak metric unavailable".into());
        }
        Ok(out)
    }
}
