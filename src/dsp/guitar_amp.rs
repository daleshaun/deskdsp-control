//! Guitar Amp Emulation with 2x Oversampled Tube Waveshaper and 3-Band Tone Stack.
//! All buffers allocated in `::new` with zero real-time audio thread allocations.

#![allow(dead_code)]

use super::biquad::{BiquadFilter, FilterType};
use super::dc_blocker::DcBlocker;
use super::DspNode;

#[derive(Debug, Clone)]
pub struct GuitarAmp {
    sample_rate: f32,
    pub drive: f32,
    pub bass_db: f32,
    pub mid_db: f32,
    pub treble_db: f32,
    pub level_db: f32,
    pub bypassed: bool,

    // Pre-filtering (tight high-pass at 60 Hz to prevent flubby low end)
    pre_hpf: BiquadFilter,

    // Tone Stack
    tone_low: BiquadFilter,
    tone_mid: BiquadFilter,
    tone_high: BiquadFilter,

    // Post DC Blocker
    dc_blocker: DcBlocker,

    // 2x Oversampling anti-aliasing / decimation filter
    aa_filter1: BiquadFilter,
    aa_filter2: BiquadFilter,
    prev_input: f32,
}

impl GuitarAmp {
    pub fn new(sample_rate: f32) -> Self {
        // Pre-HPF to keep bottom-end articulate through high gain
        let pre_hpf = BiquadFilter::new(FilterType::HighPass, 70.0, 0.0, sample_rate);

        // Standard tone stack EQ
        let tone_low = BiquadFilter::new(FilterType::LowShelf { q: 0.707 }, 150.0, 0.0, sample_rate);
        let tone_mid = BiquadFilter::new(FilterType::Peaking { q: 1.0 }, 800.0, 0.0, sample_rate);
        let tone_high = BiquadFilter::new(FilterType::HighShelf { q: 0.707 }, 3200.0, 0.0, sample_rate);

        // Anti-aliasing decimation filters at 2x rate
        let double_sr = sample_rate * 2.0;
        let aa_filter1 = BiquadFilter::new(FilterType::LowPass, sample_rate * 0.45, 0.0, double_sr);
        let aa_filter2 = BiquadFilter::new(FilterType::LowPass, sample_rate * 0.45, 0.0, double_sr);

        Self {
            sample_rate,
            drive: 3.5,
            bass_db: 0.0,
            mid_db: 0.0,
            treble_db: 0.0,
            level_db: -6.0, // Default master attenuation to preserve headroom
            bypassed: false,
            pre_hpf,
            tone_low,
            tone_mid,
            tone_high,
            dc_blocker: DcBlocker::new(sample_rate),
            aa_filter1,
            aa_filter2,
            prev_input: 0.0,
        }
    }

    pub fn set_params(&mut self, drive: f32, bass_db: f32, mid_db: f32, treble_db: f32, level_db: f32) {
        self.drive = drive.clamp(1.0, 25.0);
        self.bass_db = bass_db.clamp(-12.0, 12.0);
        self.mid_db = mid_db.clamp(-12.0, 12.0);
        self.treble_db = treble_db.clamp(-12.0, 12.0);
        self.level_db = level_db.clamp(-24.0, 12.0);

        self.tone_low.set_gain_db(self.bass_db);
        self.tone_mid.set_gain_db(self.mid_db);
        self.tone_high.set_gain_db(self.treble_db);
    }

    #[inline(always)]
    fn tube_transfer(x: f32, drive: f32) -> f32 {
        let biased = x * drive + 0.16;
        // Asymmetric tube soft clip
        if biased >= 0.0 {
            biased.tanh() - 0.16_f32.tanh()
        } else {
            // Softer knee for negative half-cycle
            let v = biased * 0.8;
            (v / (1.0 + v * v).sqrt()) - 0.16_f32.tanh()
        }
    }
}

impl DspNode for GuitarAmp {
    fn name(&self) -> &'static str {
        "Guitar Amp"
    }

    fn is_bypassed(&self) -> bool {
        self.bypassed
    }

    fn set_bypassed(&mut self, bypassed: bool) {
        self.bypassed = bypassed;
    }

    fn reset(&mut self) {
        self.pre_hpf.reset();
        self.tone_low.reset();
        self.tone_mid.reset();
        self.tone_high.reset();
        self.dc_blocker.reset();
        self.aa_filter1.reset();
        self.aa_filter2.reset();
        self.prev_input = 0.0;
    }

    #[inline(always)]
    fn process_sample(&mut self, input: f32) -> f32 {
        if self.bypassed {
            return input;
        }

        if !input.is_finite() {
            return 0.0;
        }

        // 1. Pre-filtering to clean sub-bass rumble
        let filtered = self.pre_hpf.process_sample(input.clamp(-2.0, 2.0));

        // 2. 2x Oversampled Nonlinearity
        // Interpolate 2x mid-sample
        let mid = 0.5 * (self.prev_input + filtered);
        self.prev_input = filtered;

        // Apply tube saturation at 2x rate
        let sat1 = Self::tube_transfer(mid, self.drive);
        let sat2 = Self::tube_transfer(filtered, self.drive);

        // Anti-aliasing lowpass filtering
        let aa1 = self.aa_filter1.process_sample(sat1);
        let aa2 = self.aa_filter2.process_sample(sat2);

        // Decimate back to 1x rate
        let mut sample = 0.5 * (aa1 + aa2);

        // 3. DC offset removal
        sample = self.dc_blocker.process_sample(sample);

        // 4. Tone Stack (Bass, Mid, Treble)
        sample = self.tone_low.process_sample(sample);
        sample = self.tone_mid.process_sample(sample);
        sample = self.tone_high.process_sample(sample);

        // 5. Output Level
        let linear_gain = 10.0_f32.powf(self.level_db / 20.0);
        (sample * linear_gain).clamp(-1.0, 1.0)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
