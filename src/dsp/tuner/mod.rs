//! Vocal Tuner node combining YIN pitch detection, scale quantizing, and pitch shifting.

pub mod detector;
pub mod quantizer;
pub mod shifter;

pub use detector::PitchDetector;
pub use quantizer::{Note, Scale, ScaleQuantizer};
pub use shifter::PitchShifter;

use super::DspNode;

#[derive(Debug, Clone)]
pub struct VocalTuner {
    pub detector: PitchDetector,
    pub quantizer: ScaleQuantizer,
    pub shifter: PitchShifter,

    // Parameters
    pub retune_speed_ms: f32,
    pub strength: f32,
    pub bypassed: bool,

    // Telemetry for UI / Monitoring
    pub detected_freq_hz: Option<f32>,
    pub target_freq_hz: Option<f32>,
    pub cents_deviation: f32,
    pub current_note: &'static str,
    pub confidence: f32,

    sample_rate: f32,
    frame_counter: usize,
    hop_size: usize,

    // Signal level tracking (smooth RMS/peak envelope)
    rms_env: f32,

    // 3-point median filter on detected frequency for rock-solid stability
    pitch_history: [f32; 3],
    history_idx: usize,
    history_count: usize,

    // Voiced tracking state
    voiced_counter: usize,
}

impl VocalTuner {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            detector: PitchDetector::new(sample_rate),
            quantizer: ScaleQuantizer::new(Note::C, Scale::Chromatic),
            shifter: PitchShifter::new(sample_rate),
            retune_speed_ms: 20.0, // 20 ms natural vocal retune speed
            strength: 0.85,        // 85% correction strength
            bypassed: true,        // SAFE DEFAULT: Off until explicitly enabled
            detected_freq_hz: None,
            target_freq_hz: None,
            cents_deviation: 0.0,
            current_note: "--",
            confidence: 0.0,
            sample_rate,
            frame_counter: 0,
            hop_size: 128,         // pitch analysis hop size (~2.6ms at 48kHz)
            rms_env: 0.0,
            pitch_history: [0.0; 3],
            history_idx: 0,
            history_count: 0,
            voiced_counter: 0,
        }
    }

    pub fn set_key_and_scale(&mut self, root: Note, scale: Scale) {
        self.quantizer.root = root;
        self.quantizer.scale = scale;
    }

    pub fn set_tuning_params(&mut self, retune_speed_ms: f32, strength: f32) {
        self.retune_speed_ms = retune_speed_ms.clamp(0.1, 200.0);
        self.strength = strength.clamp(0.0, 1.0);
    }
}

impl DspNode for VocalTuner {
    fn name(&self) -> &'static str {
        "Vocal Tuner"
    }

    fn is_bypassed(&self) -> bool {
        self.bypassed
    }

    fn set_bypassed(&mut self, bypassed: bool) {
        self.bypassed = bypassed;
    }

    fn reset(&mut self) {
        self.shifter.reset();
        self.detected_freq_hz = None;
        self.target_freq_hz = None;
        self.cents_deviation = 0.0;
        self.current_note = "--";
        self.confidence = 0.0;
        self.frame_counter = 0;
        self.rms_env = 0.0;
        self.history_count = 0;
        self.voiced_counter = 0;
    }

    #[inline(always)]
    fn process_sample(&mut self, input: f32) -> f32 {
        if self.bypassed {
            return input;
        }

        // Push to pitch detection buffer
        self.detector.push_sample(input);

        // Smooth signal envelope follower to avoid zero-crossing dropouts
        let abs_in = input.abs();
        let coeff = if abs_in > self.rms_env { 0.05 } else { 0.001 };
        self.rms_env += (abs_in - self.rms_env) * coeff;

        self.frame_counter += 1;

        // Perform periodic pitch detection hop
        if self.frame_counter >= self.hop_size {
            self.frame_counter = 0;
            let (pitch_opt, conf) = self.detector.detect_pitch();
            self.confidence = conf;

            let signal_present = self.rms_env > 0.003; // > -50 dBFS

            if let Some(raw_f) = pitch_opt {
                if conf >= 0.70 && signal_present && raw_f >= 75.0 && raw_f <= 900.0 {
                    // Push to 3-point median filter for outlier rejection
                    self.pitch_history[self.history_idx] = raw_f;
                    self.history_idx = (self.history_idx + 1) % 3;
                    if self.history_count < 3 {
                        self.history_count += 1;
                    }

                    let f_in = if self.history_count >= 3 {
                        let mut sorted = self.pitch_history;
                        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                        sorted[1]
                    } else {
                        raw_f
                    };

                    let (f_target, note_str, cents) = self.quantizer.quantize_with_hysteresis(f_in);
                    self.detected_freq_hz = Some(f_in);
                    self.target_freq_hz = Some(f_target);
                    self.cents_deviation = cents;
                    self.current_note = note_str;

                    // Musical correction ratio (clamped to +/- 2.5 semitones)
                    let ideal_ratio = (f_target / f_in).clamp(0.85, 1.18);
                    let effective_ratio = 1.0 + (ideal_ratio - 1.0) * self.strength;

                    self.shifter.set_ratio(effective_ratio, self.retune_speed_ms);
                    self.shifter.set_pitch_period(self.sample_rate / f_in);
                    self.voiced_counter = 0;
                } else {
                    self.voiced_counter += 1;
                    if self.voiced_counter > 4 { // ~10ms unvoiced debounce
                        self.shifter.set_ratio(1.0, 15.0);
                        self.current_note = "--";
                        self.cents_deviation = 0.0;
                        self.history_count = 0;
                    }
                }
            } else {
                self.voiced_counter += 1;
                if self.voiced_counter > 4 {
                    self.shifter.set_ratio(1.0, 15.0);
                    self.current_note = "--";
                    self.cents_deviation = 0.0;
                    self.history_count = 0;
                }
            }
        }

        // Continuous delay line processing: clean, bit-perfect passthrough at ratio 1.0,
        // and artifact-free pitch modification when retuned.
        self.shifter.process_sample(input)
    }

    fn telemetry(&self) -> super::NodeTelemetry {
        super::NodeTelemetry {
            detected_freq_hz: self.detected_freq_hz.unwrap_or(0.0),
            cents_deviation: self.cents_deviation,
            ..Default::default()
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
