//! DeskDSP Vocal Strip DAW Plugin (CLAP and VST3)
//!
//! Exposes DeskDSP's vocal channel strip (HPF, Noise Gate, De-Esser, 4-Band EQ,
//! Vocal Compressor, Vocal Tuner, Analog Saturation) for use in DAWs (Reaper,
//! Ableton Live, Bitwig Studio, FL Studio, etc.).
//!
//! Technical Invariants:
//! - 100% allocation-free audio callback in `process()`.
//! - Reuses core `deskdsp_control::dsp::ChannelStrip` dynamic rack directly.
//! - Exports CLAP and VST3 entry points via `nih_plug`.

use std::num::NonZeroU32;
use std::sync::Arc;
use nih_plug::prelude::*;

use deskdsp_control::dsp::channel_strip::ChannelStrip;

#[derive(Params)]
pub struct DeskDspVocalStripParams {
    #[id = "input_gain"]
    pub input_gain: FloatParam,

    #[id = "output_gain"]
    pub output_gain: FloatParam,

    #[id = "gate_threshold"]
    pub gate_threshold: FloatParam,

    #[id = "comp_threshold"]
    pub comp_threshold: FloatParam,

    #[id = "comp_ratio"]
    pub comp_ratio: FloatParam,

    #[id = "sat_drive"]
    pub sat_drive: FloatParam,

    #[id = "tuner_bypass"]
    pub tuner_bypass: BoolParam,

    #[id = "bypass"]
    pub bypass: BoolParam,
}

impl Default for DeskDspVocalStripParams {
    fn default() -> Self {
        Self {
            input_gain: FloatParam::new(
                "Input Gain",
                0.0,
                FloatRange::Linear { min: -24.0, max: 24.0 },
            )
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),

            output_gain: FloatParam::new(
                "Output Gain",
                0.0,
                FloatRange::Linear { min: -24.0, max: 24.0 },
            )
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),

            gate_threshold: FloatParam::new(
                "Gate Threshold",
                -42.0,
                FloatRange::Linear { min: -70.0, max: -10.0 },
            )
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),

            comp_threshold: FloatParam::new(
                "Comp Threshold",
                -18.0,
                FloatRange::Linear { min: -50.0, max: 0.0 },
            )
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),

            comp_ratio: FloatParam::new(
                "Comp Ratio",
                4.0,
                FloatRange::Linear { min: 1.5, max: 12.0 },
            )
            .with_unit(":1")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),

            sat_drive: FloatParam::new(
                "Sat Drive",
                1.5,
                FloatRange::Linear { min: 1.0, max: 10.0 },
            )
            .with_unit("x")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),

            tuner_bypass: BoolParam::new("Tuner Bypass", false),

            bypass: BoolParam::new("Bypass", false),
        }
    }
}

pub struct DeskDspVocalStrip {
    params: Arc<DeskDspVocalStripParams>,
    channel_l: ChannelStrip,
    channel_r: ChannelStrip,
    sample_rate: f32,
}

impl Default for DeskDspVocalStrip {
    fn default() -> Self {
        Self {
            params: Arc::new(DeskDspVocalStripParams::default()),
            channel_l: ChannelStrip::new(48000.0),
            channel_r: ChannelStrip::new(48000.0),
            sample_rate: 48000.0,
        }
    }
}

impl Plugin for DeskDspVocalStrip {
    const NAME: &'static str = "DeskDSP Vocal Strip";
    const VENDOR: &'static str = "DeskDSP";
    const URL: &'static str = "https://github.com/daleshaun/deskdsp-control";
    const EMAIL: &'static str = "shaun@deskdsp.internal";

    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(2),
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        },
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(1),
            main_output_channels: NonZeroU32::new(1),
            ..AudioIOLayout::const_default()
        },
    ];

    const MIDI_INPUT: MidiConfig = MidiConfig::None;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::None;

    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.sample_rate = buffer_config.sample_rate;
        self.channel_l = ChannelStrip::new(self.sample_rate);
        self.channel_r = ChannelStrip::new(self.sample_rate);
        true
    }

    fn reset(&mut self) {
        self.channel_l.reset_all();
        self.channel_r.reset_all();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        if self.params.bypass.value() {
            return ProcessStatus::Normal;
        }

        // Apply parameter updates to channel strip nodes
        let in_gain = self.params.input_gain.value();
        let out_gain = self.params.output_gain.value();
        let gate_th = self.params.gate_threshold.value();
        let comp_th = self.params.comp_threshold.value();
        let comp_rat = self.params.comp_ratio.value();
        let sat_dr = self.params.sat_drive.value();
        let tuner_byp = self.params.tuner_bypass.value();

        self.channel_l.input_gain_db = in_gain;
        self.channel_r.input_gain_db = in_gain;
        self.channel_l.output_gain_db = out_gain;
        self.channel_r.output_gain_db = out_gain;

        if let Some(gate) = self.channel_l.gate_mut() {
            gate.threshold_db = gate_th;
        }
        if let Some(gate) = self.channel_r.gate_mut() {
            gate.threshold_db = gate_th;
        }

        if let Some(comp) = self.channel_l.compressor_mut() {
            comp.threshold_db = comp_th;
            comp.ratio = comp_rat;
        }
        if let Some(comp) = self.channel_r.compressor_mut() {
            comp.threshold_db = comp_th;
            comp.ratio = comp_rat;
        }

        if let Some(sat) = self.channel_l.saturation_mut() {
            sat.set_drive(sat_dr);
        }
        if let Some(sat) = self.channel_r.saturation_mut() {
            sat.set_drive(sat_dr);
        }

        if let Some(tuner) = self.channel_l.tuner_mut() {
            tuner.bypassed = tuner_byp;
        }
        if let Some(tuner) = self.channel_r.tuner_mut() {
            tuner.bypassed = tuner_byp;
        }

        // Process samples in-place (allocation-free real-time audio loop)
        let num_channels = buffer.channels();
        if num_channels == 1 {
            let samples = buffer.as_slice();
            for sample in samples[0].iter_mut() {
                *sample = self.channel_l.process(*sample);
            }
        } else if num_channels >= 2 {
            let samples = buffer.as_slice();
            let (l_slice, rest) = samples.split_at_mut(1);
            let (r_slice, _) = rest.split_at_mut(1);

            for (l, r) in l_slice[0].iter_mut().zip(r_slice[0].iter_mut()) {
                *l = self.channel_l.process(*l);
                *r = self.channel_r.process(*r);
            }
        }

        ProcessStatus::Normal
    }
}

impl ClapPlugin for DeskDspVocalStrip {
    const CLAP_ID: &'static str = "com.deskdsp.vocal-strip";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("DeskDSP Vocal Tracking & Processing Channel Strip");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Stereo,
        ClapFeature::Mono,
        ClapFeature::PitchCorrection,
        ClapFeature::Compressor,
        ClapFeature::Equalizer,
    ];
}

impl Vst3Plugin for DeskDspVocalStrip {
    const VST3_CLASS_ID: [u8; 16] = *b"DeskDspVocalStrp";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[
        Vst3SubCategory::Fx,
        Vst3SubCategory::Dynamics,
        Vst3SubCategory::Eq,
        Vst3SubCategory::PitchShift,
    ];
}

nih_export_clap!(DeskDspVocalStrip);
nih_export_vst3!(DeskDspVocalStrip);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plugin_instantiation_and_parameter_defaults() {
        let plugin = DeskDspVocalStrip::default();
        assert_eq!(DeskDspVocalStrip::NAME, "DeskDSP Vocal Strip");
        assert_eq!(DeskDspVocalStrip::VENDOR, "DeskDSP");

        // Verify default parameter values
        assert_eq!(plugin.params.input_gain.value(), 0.0);
        assert_eq!(plugin.params.output_gain.value(), 0.0);
        assert_eq!(plugin.params.gate_threshold.value(), -42.0);
        assert_eq!(plugin.params.comp_threshold.value(), -18.0);
        assert_eq!(plugin.params.comp_ratio.value(), 4.0);
        assert_eq!(plugin.params.sat_drive.value(), 1.5);
        assert!(!plugin.params.bypass.value());
        assert!(!plugin.params.tuner_bypass.value());
    }

    #[test]
    fn test_plugin_channel_strip_processing_and_reset() {
        let mut plugin = DeskDspVocalStrip::default();

        // Feed test samples directly through underlying channel strip processing
        let input = 0.25_f32;
        let proc = plugin.channel_l.process(input);
        assert!(!proc.is_nan());
        assert!(proc != 0.0);

        // Reset channel strips
        plugin.reset();
        let proc_after_reset = plugin.channel_l.process(input);
        assert!(!proc_after_reset.is_nan());
    }
}

