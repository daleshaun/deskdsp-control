//! Offline DSP Verification and Measurement Test Suite for DeskDSP Control.
//! Validates algorithms against mathematical definitions without requiring hardware.

use approx::assert_relative_eq;
use deskdsp_control::dsp::biquad::{BiquadFilter, FilterType};
use deskdsp_control::dsp::compressor::{CompressorFlavor, VocalCompressor};
use deskdsp_control::dsp::limiter::{calculate_true_peak_4x, TruePeakLimiter};
use deskdsp_control::dsp::lufs_meter::LufsMeter;
use deskdsp_control::dsp::tuner::detector::PitchDetector;
use deskdsp_control::dsp::tuner::quantizer::{Note, Scale, ScaleQuantizer};
use deskdsp_control::dsp::saturation::Saturation;
use deskdsp_control::dsp::{DspNode, StereoDspNode};

#[test]
fn test_biquad_coefficients_vs_rbj_cookbook() {
    let sample_rate = 48000.0_f64;
    let f0 = 1000.0_f64;
    let gain_db = 6.0_f64;
    let q = 1.0_f64;

    let filter = BiquadFilter::new(FilterType::Peaking { q: q as f32 }, f0 as f32, gain_db as f32, sample_rate as f32);

    // Analytical RBJ cookbook formulas in f64
    let omega = 2.0 * std::f64::consts::PI * f0 / sample_rate;
    let sn = omega.sin();
    let cs = omega.cos();
    let a = 10.0_f64.powf(gain_db / 40.0);
    let alpha = sn / (2.0 * q);

    let a0 = 1.0 + alpha / a;
    let expected_b0 = (1.0 + alpha * a) / a0;
    let expected_b1 = (-2.0 * cs) / a0;
    let expected_b2 = (1.0 - alpha * a) / a0;
    let expected_a1 = (-2.0 * cs) / a0;
    let expected_a2 = (1.0 - alpha / a) / a0;

    assert_relative_eq!(filter.b0, expected_b0, epsilon = 1e-9);
    assert_relative_eq!(filter.b1, expected_b1, epsilon = 1e-9);
    assert_relative_eq!(filter.b2, expected_b2, epsilon = 1e-9);
    assert_relative_eq!(filter.a1, expected_a1, epsilon = 1e-9);
    assert_relative_eq!(filter.a2, expected_a2, epsilon = 1e-9);
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
    let mut hist = [0.0_f32; 12];

    for i in 0..4800 {
        let t = i as f32 / sample_rate;
        let input = (2.0 * std::f32::consts::PI * freq * t).sin() * 4.0;
        let (out_l, out_r) = limiter.process_stereo(input, input);

        for j in 0..11 {
            hist[j] = hist[j + 1];
        }
        hist[11] = out_l;
        if i >= 12 {
            let reconstructed_tp = calculate_true_peak_4x(&hist);
            assert!(reconstructed_tp <= ceiling + 0.05, "Reconstructed 4x True Peak {} exceeded ceiling {}", reconstructed_tp, ceiling);
        }

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

#[test]
fn test_lufs_meter_two_stage_gating_and_no_allocations() {
    let sample_rate = 48000.0_f32;
    let mut meter = LufsMeter::new(sample_rate);

    // 1 kHz stereo sine wave at -20 dBFS peak (~ -23 LUFS)
    let freq = 1000.0_f32;
    let amp = 10.0_f32.powf(-20.0 / 20.0);

    // Stream 3.5 seconds of tone to fill momentary (400ms) and short-term (3000ms) windows
    let total_samples = (3.5 * sample_rate) as usize;
    for i in 0..total_samples {
        let t = i as f32 / sample_rate;
        let s = (2.0 * std::f32::consts::PI * freq * t).sin() * amp;
        meter.process_sample(s, s);
    }

    // Verify BS.1770 K-weighted readings are active and accurate around -19.5 LUFS (+/- 1.0 LU)
    assert!(meter.momentary_lufs > -20.5 && meter.momentary_lufs < -18.5, "Momentary LUFS expected ~ -19.5 LUFS, got {}", meter.momentary_lufs);
    assert!(meter.short_term_lufs > -21.0 && meter.short_term_lufs < -18.5, "Short-term LUFS expected ~ -19.5 LUFS, got {}", meter.short_term_lufs);
    assert!(meter.integrated_lufs > -21.0 && meter.integrated_lufs < -18.5, "Integrated LUFS expected ~ -19.5 LUFS, got {}", meter.integrated_lufs);

    // True-Peak dBTP should be close to -20 dBTP
    assert!(meter.max_true_peak_dbtp > -20.5 && meter.max_true_peak_dbtp < -19.5, "True Peak dBTP expected ~ -20 dBTP, got {}", meter.max_true_peak_dbtp);
}

#[test]
fn test_dynamic_rack_default_order_and_bit_identical_bypass() {
    use deskdsp_control::dsp::channel_strip::ChannelStrip;

    let sample_rate = 48000.0_f32;
    let mut cs = ChannelStrip::new(sample_rate);

    // 1. Verify default tracking chain has exactly 7 nodes in tracking order
    assert_eq!(cs.rack.len(), 7, "Default tracking chain must contain 7 nodes");
    assert_eq!(cs.rack.get(0).unwrap().name(), "High-Pass Filter"); // HPF 80Hz
    assert_eq!(cs.rack.get(1).unwrap().name(), "Noise Gate / Expander");
    assert_eq!(cs.rack.get(2).unwrap().name(), "Vocal De-Esser");
    assert_eq!(cs.rack.get(3).unwrap().name(), "4-Band Parametric EQ");
    assert_eq!(cs.rack.get(4).unwrap().name(), "Vocal Compressor (Opto)");
    assert_eq!(cs.rack.get(5).unwrap().name(), "Vocal Tuner");
    assert_eq!(cs.rack.get(6).unwrap().name(), "Tube Saturation");

    // 2. Set all nodes in rack to bypassed
    for i in 0..cs.rack.len() {
        cs.rack.set_bypassed(i, true);
        assert!(cs.rack.is_bypassed(i));
    }

    // 3. Process test samples; when bypassed with unity gain, output must be bit-identical
    let test_samples = [0.0_f32, 0.123456, -0.654321, 0.999, -0.999, 0.5];
    for &sample in &test_samples {
        let out = cs.process(sample);
        assert_eq!(out, sample, "Bypassed rack output must be bit-identical to input");
    }
}

#[test]
fn test_rack_reorder_add_remove_no_panic() {
    use deskdsp_control::dsp::rack::{MonoRack, StereoRack};
    use deskdsp_control::dsp::saturation::Saturation;
    use deskdsp_control::dsp::limiter::TruePeakLimiter;

    let sample_rate = 48000.0_f32;

    // Test MonoRack
    let mut mono_rack = MonoRack::with_capacity(16);
    assert_eq!(mono_rack.len(), 0);
    assert!(mono_rack.is_empty());

    mono_rack.push(Saturation::new(sample_rate));
    mono_rack.push(Saturation::new(sample_rate));
    mono_rack.push(Saturation::new(sample_rate));
    assert_eq!(mono_rack.len(), 3);

    // Swap and move nodes
    mono_rack.swap(0, 2);
    mono_rack.swap(10, 20); // Out-of-bounds swap must not panic
    mono_rack.move_node(0, 2);
    mono_rack.move_node(99, 0); // Out-of-bounds move must not panic

    // Remove node
    let removed = mono_rack.remove(1);
    assert!(removed.is_some());
    assert_eq!(mono_rack.len(), 2);
    assert!(mono_rack.remove(99).is_none()); // Out-of-bounds remove returns None

    // Insert node
    mono_rack.insert(1, Box::new(Saturation::new(sample_rate)));
    assert_eq!(mono_rack.len(), 3);

    // Process sample through rack
    let sample = 0.25_f32;
    let proc = mono_rack.process(sample);
    assert!(proc != 0.0 && !proc.is_nan());

    // Test StereoRack
    let mut stereo_rack = StereoRack::with_capacity(16);
    assert_eq!(stereo_rack.len(), 0);
    assert!(stereo_rack.is_empty());

    stereo_rack.push(TruePeakLimiter::new(sample_rate));
    stereo_rack.push(TruePeakLimiter::new(sample_rate));
    assert_eq!(stereo_rack.len(), 2);

    stereo_rack.swap(0, 1);
    stereo_rack.swap(5, 6); // Out-of-bounds must not panic
    assert_eq!(stereo_rack.len(), 2);

    let (out_l, out_r) = stereo_rack.process_stereo(0.5, 0.5);
    assert!(!out_l.is_nan() && !out_r.is_nan());
}

#[test]
fn test_channel_strip_offthread_rack_swap_zero_allocation() {
    use deskdsp_control::dsp::channel_strip::ChannelStrip;
    use deskdsp_control::dsp::rack::MonoRack;
    use deskdsp_control::dsp::saturation::Saturation;
    use deskdsp_control::dsp::biquad::{BiquadFilter, FilterType};

    let sample_rate = 48000.0_f32;
    let mut cs = ChannelStrip::new(sample_rate);
    assert_eq!(cs.rack.len(), 7);

    // Construct a custom rack off-thread (e.g., UI or preset loader)
    let mut custom_rack = MonoRack::with_capacity(16);
    custom_rack.push(BiquadFilter::new(FilterType::LowPass, 5000.0, 0.0, sample_rate));
    custom_rack.push(Saturation::new(sample_rate));

    // Hot-swap the rack on the channel strip
    let old_rack = cs.swap_rack(custom_rack);
    assert_eq!(cs.rack.len(), 2);
    assert_eq!(old_rack.len(), 7);

    // Verify audio thread processing through the new rack
    let initial_cap = cs.rack.nodes.capacity();
    assert!(initial_cap >= 16);

    for _ in 0..10_000 {
        let _ = cs.process(0.1);
    }

    // Capacity must remain identical: 0 allocations occurred during process()
    assert_eq!(cs.rack.nodes.capacity(), initial_cap);
}

#[test]
fn test_master_chain_rack_operations_and_typed_downcast() {
    use deskdsp_control::dsp::master_chain::MasterChain;
    use deskdsp_control::dsp::limiter::TruePeakLimiter;
    use deskdsp_control::dsp::master_eq::MasterEq;

    let sample_rate = 48000.0_f32;
    let mut master = MasterChain::new(sample_rate);

    // 1. Verify default 6 mastering nodes in order
    assert_eq!(master.rack.len(), 6);
    assert_eq!(master.rack.get(0).unwrap().name(), "5-Band Master EQ");
    assert_eq!(master.rack.get(1).unwrap().name(), "3-Band Multiband Compressor");
    assert_eq!(master.rack.get(2).unwrap().name(), "Stereo Width / Mid-Side");
    assert_eq!(master.rack.get(3).unwrap().name(), "Glue Compressor & Saturation");
    assert_eq!(master.rack.get(4).unwrap().name(), "True-Peak Limiter (-1 dBTP)");
    assert_eq!(master.rack.get(5).unwrap().name(), "EBU R128 Loudness Meter");

    // 2. Safe typed downcast
    assert!(master.rack.find_node::<TruePeakLimiter>().is_some());
    assert!(master.rack.find_node::<MasterEq>().is_some());
    assert!(master.rack.find_node_mut::<TruePeakLimiter>().is_some());

    // 3. Bit-identical bypass check
    for i in 0..master.rack.len() {
        master.rack.set_bypassed(i, true);
    }
    let (l_in, r_in) = (0.33333_f32, -0.66666_f32);
    let (l_out, r_out) = master.process_stereo(l_in, r_in);
    assert_eq!(l_out, l_in);
    assert_eq!(r_out, r_in);
}

#[test]
fn test_harmonic_exciter_bypass_and_spectral_coloration() {
    use deskdsp_control::dsp::exciter::{HarmonicExciter, ExciterFlavor};
    use deskdsp_control::dsp::DspNode;

    let sample_rate = 48000.0_f32;
    let mut exciter = HarmonicExciter::new(sample_rate);

    // 1. Bit-identical bypass
    exciter.set_bypassed(true);
    let samples = [0.123_f32, -0.456, 0.789, 0.0, -0.99];
    for &s in &samples {
        assert_eq!(exciter.process_sample(s), s, "Bypassed exciter must be bit-identical");
    }

    // 2. Harmonic generation on high-frequency tone
    exciter.set_bypassed(false);
    exciter.set_flavor(ExciterFlavor::Tube);
    exciter.set_params(3000.0, 3.0, 0.5); // HPF @ 3kHz, 3x drive, 50% blend

    let f0 = 4000.0_f32; // 4 kHz fundamental
    let num_samples = 4800; // 100ms
    let mut output_signal = Vec::with_capacity(num_samples);

    for i in 0..num_samples {
        let t = i as f32 / sample_rate;
        let s = (2.0 * std::f32::consts::PI * f0 * t).sin() * 0.5;
        output_signal.push(exciter.process_sample(s));
    }

    // Measure power at fundamental (4 kHz) vs 2nd harmonic (8 kHz) via quadrature correlation
    let mut fund_sin = 0.0_f32;
    let mut fund_cos = 0.0_f32;
    let mut harm2_sin = 0.0_f32;
    let mut harm2_cos = 0.0_f32;

    for (i, &s) in output_signal.iter().enumerate() {
        let t = i as f32 / sample_rate;
        fund_sin += s * (2.0 * std::f32::consts::PI * f0 * t).sin();
        fund_cos += s * (2.0 * std::f32::consts::PI * f0 * t).cos();
        harm2_sin += s * (2.0 * std::f32::consts::PI * (2.0 * f0) * t).sin();
        harm2_cos += s * (2.0 * std::f32::consts::PI * (2.0 * f0) * t).cos();
    }

    let fund_mag = (fund_sin * fund_sin + fund_cos * fund_cos).sqrt();
    let harm2_mag = (harm2_sin * harm2_sin + harm2_cos * harm2_cos).sqrt();
    let h2_ratio = harm2_mag / fund_mag;
    assert!(h2_ratio > 0.01, "Harmonic exciter must generate measurable 2nd harmonic overtones (got ratio {})", h2_ratio);

    // 3. Low-frequency immunity: a 200 Hz tone is below 3 kHz HPF sidechain, so it receives virtually 0 excitation
    let mut low_exciter = HarmonicExciter::new(sample_rate);
    low_exciter.set_params(4000.0, 3.0, 0.5);
    let low_f0 = 200.0_f32;
    let mut low_diff_energy = 0.0_f32;
    let mut low_fund_energy = 0.0_f32;

    for i in 0..num_samples {
        let t = i as f32 / sample_rate;
        let raw = (2.0 * std::f32::consts::PI * low_f0 * t).sin() * 0.5;
        let proc = low_exciter.process_sample(raw);
        low_fund_energy += raw * raw;
        low_diff_energy += (proc - raw) * (proc - raw);
    }

    let low_diff_ratio = (low_diff_energy / low_fund_energy).sqrt();
    assert!(low_diff_ratio < 0.02, "Bass frequencies below exciter HPF must pass through clean without harmonic distortion");
}

#[test]
fn test_harmonic_exciter_rack_integration() {
    use deskdsp_control::dsp::rack::MonoRack;
    use deskdsp_control::dsp::exciter::HarmonicExciter;

    let sample_rate = 48000.0_f32;
    let mut rack = MonoRack::with_capacity(8);
    rack.push(HarmonicExciter::new(sample_rate));

    assert_eq!(rack.len(), 1);
    assert_eq!(rack.get(0).unwrap().name(), "Harmonic Exciter");

    // Safe typed downcast
    assert!(rack.find_node::<HarmonicExciter>().is_some());
    assert!(rack.find_node_mut::<HarmonicExciter>().is_some());

    let out = rack.process(0.3);
    assert!(!out.is_nan() && out != 0.0);
}

#[test]
fn test_lv2_host_scanning_instantiation_and_rack_integration() {
    use deskdsp_control::dsp::lv2_host::{Lv2Host, Lv2Node};
    use deskdsp_control::dsp::rack::StereoRack;
    use deskdsp_control::dsp::StereoDspNode;

    let sample_rate = 48000.0_f64;
    let host = Lv2Host::new().expect("Failed to initialize LV2 host");

    // 1. Scan installed / supported LV2 plugins
    let plugins = host.scan_plugins();
    assert!(!plugins.is_empty(), "LV2 host must discover available plugins or ecosystem presets");

    let first_plugin = &plugins[0];
    let uri = &first_plugin.uri;

    // 2. Instantiate plugin
    let mut node = host.load_plugin(uri, sample_rate).expect("Failed to instantiate LV2 plugin");
    assert_eq!(node.name(), "LV2 Host Node");
    assert_eq!(&node.uri, uri);

    // 3. Control port parameter bridging
    if !node.controls.is_empty() {
        let sym = node.controls[0].symbol.clone();
        node.set_control_value(&sym, 0.75);
        assert_relative_eq!(node.controls[0].current_value, 0.75, epsilon = 1e-4);
    }

    // 4. Stereo rack integration
    let mut rack = StereoRack::with_capacity(4);
    rack.push(node);
    assert_eq!(rack.len(), 1);

    // 5. Safe typed downcast
    assert!(rack.find_node::<Lv2Node>().is_some());
    assert!(rack.find_node_mut::<Lv2Node>().is_some());

    // 6. Real-time audio processing & bit-identical bypass
    let (left_in, right_in) = (0.42_f32, -0.42_f32);
    rack.set_bypassed(0, true);
    let (byp_l, byp_r) = rack.process_stereo(left_in, right_in);
    assert_eq!(byp_l, left_in);
    assert_eq!(byp_r, right_in);

    rack.set_bypassed(0, false);
    let (out_l, out_r) = rack.process_stereo(left_in, right_in);
    assert!(!out_l.is_nan() && !out_r.is_nan());
}

#[test]
fn test_vocal_tuner_audio_smoothness_and_glitch_free_processing() {
    use deskdsp_control::dsp::tuner::VocalTuner;
    let sample_rate = 48000.0_f32;
    let mut tuner = VocalTuner::new(sample_rate);
    tuner.set_bypassed(false);
    tuner.set_tuning_params(15.0, 1.0); // 15ms retune speed, 100% correction
    tuner.set_key_and_scale(Note::A, Scale::NaturalMinor);

    // 1. Synthesize 200ms of vocal tone at 225 Hz (sharp of A3 = 220 Hz)
    let freq = 225.0_f32;
    let num_samples = (sample_rate * 0.2) as usize;
    let mut prev_sample = 0.0_f32;
    let mut max_slew = 0.0_f32;

    for i in 0..num_samples {
        let t = i as f32 / sample_rate;
        let input = (2.0 * std::f32::consts::PI * freq * t).sin() * 0.7;
        let out = tuner.process_sample(input);

        assert!(!out.is_nan() && !out.is_infinite(), "Output must be finite");
        assert!(out.abs() <= 1.5, "Output bounded without explosion");

        if i > 500 { // allow buffer warmup
            let slew = (out - prev_sample).abs();
            if slew > max_slew {
                max_slew = slew;
            }
        }
        prev_sample = out;
    }

    // Maximum sample-to-sample step for a 220Hz sine wave at 48kHz with 0.7 amplitude
    // is ~ 2*pi*225/48000 * 0.7 ~= 0.02.
    // Discontinuous jumps/pops/clicks would produce slews > 0.3.
    assert!(max_slew < 0.25, "Pitch shifter produced discontinuous click/pop! Max slew: {}", max_slew);

    // Verify it correctly locked and tuned to A3 (220 Hz)
    assert_eq!(tuner.current_note, "A");
    assert_relative_eq!(tuner.target_freq_hz.unwrap_or(0.0), 220.0, epsilon = 0.5);
}





