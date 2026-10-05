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
    
    frame_counter: usize,
    hop_size: usize,
}

impl VocalTuner {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            detector: PitchDetector::new(sample_rate),
            quantizer: ScaleQuantizer::new(Note::C, Scale::Chromatic),
            shifter: PitchShifter::new(sample_rate),
            retune_speed_ms: 20.0, // 20 ms natural vocal retune speed
            strength: 0.85,        // 85% correction strength
            bypassed: false,
            detected_freq_hz: None,
            target_freq_hz: None,
            cents_deviation: 0.0,
            current_note: "--",
            confidence: 0.0,
            frame_counter: 0,
            hop_size: 128,         // pitch analysis hop size (~2.6ms at 48kHz)
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
    }

    #[inline(always)]
    fn process_sample(&mut self, input: f32) -> f32 {
        if self.bypassed {
            return input;
        }

        // Push to pitch detection buffer
        self.detector.push_sample(input);
        self.frame_counter += 1;

        // Perform periodic pitch detection hop
        if self.frame_counter >= self.hop_size {
            self.frame_counter = 0;
            let (pitch_opt, conf) = self.detector.detect_pitch();
            self.confidence = conf;

            if let Some(f_in) = pitch_opt {
                if conf > 0.65 {
                    let (f_target, note_str, cents) = self.quantizer.quantize(f_in);
                    self.detected_freq_hz = Some(f_in);
                    self.target_freq_hz = Some(f_target);
                    self.cents_deviation = cents;
                    self.current_note = note_str;

                    // Compute correction ratio
                    let ideal_ratio = f_target / f_in;
                    // Blend with strength
                    let effective_ratio = 1.0 + (ideal_ratio - 1.0) * self.strength;
                    self.shifter.set_ratio(effective_ratio, self.retune_speed_ms);
                } else {
                    // Low confidence / unvoiced
                    self.shifter.set_ratio(1.0, self.retune_speed_ms);
                }
            } else {
                self.shifter.set_ratio(1.0, self.retune_speed_ms);
            }
        }

        self.shifter.process_sample(input)
    }
}
