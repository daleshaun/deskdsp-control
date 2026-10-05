//! Offline DSP Verification and Measurement Test Suite for DeskDSP Control.
//! Validates algorithms against mathematical definitions without requiring hardware.

use approx::assert_relative_eq;
use deskdsp_control::dsp::biquad::{BiquadFilter, FilterType};
use deskdsp_control::dsp::compressor::{CompressorFlavor, VocalCompressor};
use deskdsp_control::dsp::limiter::TruePeakLimiter;
use deskdsp_control::dsp::tuner::detector::PitchDetector;
use deskdsp_control::dsp::tuner::quantizer::{Note, Scale, ScaleQuantizer};
use deskdsp_control::dsp::saturation::Saturation;
use deskdsp_control::dsp::{DspNode, StereoDspNode};

#[test]
fn test_biquad_coefficients_vs_rbj_cookbook() {
    let sample_rate = 48000.0_f32;
    let f0 = 1000.0_f32;
    let gain_db = 6.0_f32;
    let q = 1.0_f32;

    let filter = BiquadFilter::new(FilterType::Peaking { q }, f0, gain_db, sample_rate);

    // Analytical RBJ cookbook formulas
    let omega = 2.0 * std::f32::consts::PI * f0 / sample_rate;
    let sn = omega.sin();
    let cs = omega.cos();
    let a = 10.0_f32.powf(gain_db / 40.0);
    let alpha = sn / (2.0 * q);

    let a0 = 1.0 + alpha / a;
    let expected_b0 = (1.0 + alpha * a) / a0;
    let expected_b1 = (-2.0 * cs) / a0;
    let expected_b2 = (1.0 - alpha * a) / a0;
    let expected_a1 = (-2.0 * cs) / a0;
    let expected_a2 = (1.0 - alpha / a) / a0;

    assert_relative_eq!(filter.b0, expected_b0, epsilon = 1e-5);
    assert_relative_eq!(filter.b1, expected_b1, epsilon = 1e-5);
    assert_relative_eq!(filter.b2, expected_b2, epsilon = 1e-5);
    assert_relative_eq!(filter.a1, expected_a1, epsilon = 1e-5);
    assert_relative_eq!(filter.a2, expected_a2, epsilon = 1e-5);
}

#[test]
fn test_compressor_transfer_curve() {
    let sample_rate = 48000.0_f32;
    let mut comp = VocalCompressor::new(sample_rate);
    comp.set_flavor(CompressorFlavor::Clean);
    comp.set_params(-18.0, 4.0, 0.1, 10.0, 0.0); // Fast attack, 0 makeup

    // 1. Below threshold: -30 dBFS
    let sub_thresh_linear = 10.0_f32.powf(-30.0 / 20.0);
    for _ in 0..2000 {
        comp.process_sample(sub_thresh_linear);
    }
    assert_relative_eq!(comp.gain_reduction_db(), 0.0, epsilon = 0.5);

    // 2. Above threshold: -6 dBFS (12 dB above -18 dB threshold)
    // Expected reduction: (1 - 1/ratio) * delta = (1 - 1/4) * 12 = 9.0 dB
    comp.reset();
    let over_thresh_linear = 10.0_f32.powf(-6.0 / 20.0);
    for _ in 0..5000 {
        comp.process_sample(over_thresh_linear);
    }
    assert_relative_eq!(comp.gain_reduction_db(), 9.0, epsilon = 0.8);
}

#[test]
fn test_true_peak_limiter_ceiling_never_exceeded() {
    let sample_rate = 48000.0_f32;
    let mut limiter = TruePeakLimiter::new(sample_rate);
    limiter.set_ceiling_dbtp(-1.0); // -1.0 dBTP = ~0.89125 linear

    let ceiling = 10.0_f32.powf(-1.0 / 20.0);

    // Feed a burst of extreme +12 dBFS overload (linear amp = 4.0)
    let freq = 1250.0_f32;
    let mut max_output_peak = 0.0_f32;

    for i in 0..4800 {
        let t = i as f32 / sample_rate;
        let input = (2.0 * std::f32::consts::PI * freq * t).sin() * 4.0;
        let (out_l, out_r) = limiter.process_stereo(input, input);

        max_output_peak = max_output_peak.max(out_l.abs()).max(out_r.abs());
        
        // Assert every single sample is bounded at or below ceiling
        assert!(out_l.abs() <= ceiling + 1e-4, "Left output {} exceeded ceiling {}", out_l, ceiling);
        assert!(out_r.abs() <= ceiling + 1e-4, "Right output {} exceeded ceiling {}", out_r, ceiling);
    }

    assert!(max_output_peak > 0.80, "Limiter passed signal");
    assert!(max_output_peak <= ceiling + 1e-4);
}

#[test]
fn test_pitch_detector_on_synthetic_tones() {
    let sample_rate = 48000.0_f32;
    let test_pitches = [
        (220.0_f32, "A3"),
        (261.63_f32, "C4"),
        (440.0_f32, "A4"),
    ];

    for (target_freq, name) in test_pitches {
        let mut detector = PitchDetector::new(sample_rate);
        
        // Synthesize 100ms of pure sine tone
        let num_samples = (sample_rate * 0.1) as usize;
        for i in 0..num_samples {
            let t = i as f32 / sample_rate;
            let sample = (2.0 * std::f32::consts::PI * target_freq * t).sin();
            detector.push_sample(sample);
        }

        let (detected, conf) = detector.detect_pitch();
        assert!(detected.is_some(), "Pitch not detected for {}", name);
        assert!(conf > 0.80, "Confidence too low ({}) for {}", conf, name);

        let freq = detected.unwrap();
        assert_relative_eq!(freq, target_freq, epsilon = 0.6);
    }
}

#[test]
fn test_scale_quantizer_and_tuner_snapping() {
    let quantizer = ScaleQuantizer::new(Note::A, Scale::NaturalMinor);

    // Glissando sweep around A3 (220 Hz): 215 Hz to 225 Hz should all snap to A3
    for f in 216..=224 {
        let (target, note_str, cents) = quantizer.quantize(f as f32);
        assert_eq!(note_str, "A", "Frequency {} Hz should quantize to A", f);
        assert_relative_eq!(target, 220.0, epsilon = 0.1);
        assert!(cents.abs() < 50.0);
    }

    // In A minor: D# (semitone 3 from A) is not in natural minor; should snap to D or E
    let d_sharp_4 = 311.13_f32;
    let (target, note_str, _) = quantizer.quantize(d_sharp_4);
    assert!(note_str == "D" || note_str == "E", "D# must snap to allowed A minor scale note, got {}", note_str);
    assert!(target > 290.0 && target < 335.0);
}

#[test]
fn test_node_bypass_is_bit_identical() {
    let sample_rate = 48000.0_f32;
    
    // Test biquad bypass
    let mut biquad = BiquadFilter::new(FilterType::HighPass, 100.0, 0.0, sample_rate);
    biquad.set_bypassed(true);
    let test_sample = 0.424242_f32;
    assert_eq!(biquad.process_sample(test_sample), test_sample);

    // Test saturation bypass
    let mut sat = Saturation::new(sample_rate);
    sat.set_bypassed(true);
    assert_eq!(sat.process_sample(test_sample), test_sample);

    // Test compressor bypass
    let mut comp = VocalCompressor::new(sample_rate);
    comp.set_bypassed(true);
    assert_eq!(comp.process_sample(test_sample), test_sample);
}

#[test]
fn test_thd_measurement_harness() {
    let sample_rate = 48000.0_f32;
    let mut sat = Saturation::new(sample_rate);
    sat.set_params(3.0, 1.0); // 3x drive

    // 1 kHz test sine wave
    let f0 = 1000.0_f32;
    let num_samples = 4800; // 100ms
    let mut output_signal = Vec::with_capacity(num_samples);

    for i in 0..num_samples {
        let t = i as f32 / sample_rate;
        let input = (2.0 * std::f32::consts::PI * f0 * t).sin() * 0.7;
        output_signal.push(sat.process_sample(input));
    }

    // Measure power at fundamental (1 kHz) vs 2nd harmonic (2 kHz) and 3rd (3 kHz) via discrete correlation
    let mut fundamental_energy = 0.0_f32;
    let mut harmonic2_energy = 0.0_f32;
    let mut harmonic3_energy = 0.0_f32;

    for (i, &s) in output_signal.iter().enumerate() {
        let t = i as f32 / sample_rate;
        fundamental_energy += s * (2.0 * std::f32::consts::PI * f0 * t).sin();
        harmonic2_energy += s * (2.0 * std::f32::consts::PI * (2.0 * f0) * t).cos(); // even harmonic
        harmonic3_energy += s * (2.0 * std::f32::consts::PI * (3.0 * f0) * t).sin(); // odd harmonic
    }

    let h2_ratio = (harmonic2_energy / fundamental_energy).abs();
    let h3_ratio = (harmonic3_energy / fundamental_energy).abs();

    // Verify presence of tube coloration harmonics
    assert!(h2_ratio > 0.01, "2nd harmonic must be generated by asymmetric tube bias");
    assert!(h3_ratio > 0.02, "3rd harmonic must be generated by tanh soft clipping");
}
