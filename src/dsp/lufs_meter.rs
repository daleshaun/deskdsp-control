//! EBU R128 & ITU-R BS.1770 Loudness Meter (LUFS) + True-Peak Detector.

use super::biquad::{BiquadFilter, FilterType};

#[derive(Debug, Clone)]
pub struct LufsMeter {
    // K-weighting pre-filter (Stage 1: High shelf +4dB @ 1.5kHz; Stage 2: High pass @ 100Hz)
    k_stage1_l: BiquadFilter,
    k_stage1_r: BiquadFilter,
    k_stage2_l: BiquadFilter,
    k_stage2_r: BiquadFilter,

    // Circular buffers for momentary (400ms) and short-term (3000ms) energy
    sample_rate: f32,
    momentary_samples: usize,
    short_term_samples: usize,
    
    // Ring buffer storing mean square power per 100ms block
    block_size: usize,
    block_counter: usize,
    current_block_energy: f32,

    momentary_blocks: Vec<f32>,
    short_term_blocks: Vec<f32>,
    m_idx: usize,
    s_idx: usize,

    // Integrated loudness gating
    all_blocks: Vec<f32>,

    // Telemetry readouts
    pub momentary_lufs: f32,
    pub short_term_lufs: f32,
    pub integrated_lufs: f32,
    pub max_true_peak_dbtp: f32,
}

impl LufsMeter {
    pub fn new(sample_rate: f32) -> Self {
        // ITU-R BS.1770 K-weighting filter coefficients
        let k_stage1_l = BiquadFilter::new(FilterType::HighShelf { q: 0.707 }, 1500.0, 4.0, sample_rate);
        let k_stage1_r = BiquadFilter::new(FilterType::HighShelf { q: 0.707 }, 1500.0, 4.0, sample_rate);
        let k_stage2_l = BiquadFilter::new(FilterType::HighPass, 100.0, 0.0, sample_rate);
        let k_stage2_r = BiquadFilter::new(FilterType::HighPass, 100.0, 0.0, sample_rate);

        let block_size = (sample_rate * 0.1) as usize; // 100ms blocks
        let momentary_blocks_count = 4; // 4 * 100ms = 400ms
        let short_term_blocks_count = 30; // 30 * 100ms = 3000ms

        Self {
            k_stage1_l,
            k_stage1_r,
            k_stage2_l,
            k_stage2_r,
            sample_rate,
            momentary_samples: (sample_rate * 0.4) as usize,
            short_term_samples: (sample_rate * 3.0) as usize,
            block_size,
            block_counter: 0,
            current_block_energy: 0.0,
            momentary_blocks: vec![0.0; momentary_blocks_count],
            short_term_blocks: vec![0.0; short_term_blocks_count],
            m_idx: 0,
            s_idx: 0,
            all_blocks: Vec::with_capacity(1024),
            momentary_lufs: -100.0,
            short_term_lufs: -100.0,
            integrated_lufs: -100.0,
            max_true_peak_dbtp: -100.0,
        }
    }

    pub fn reset(&mut self) {
        self.k_stage1_l.reset();
        self.k_stage1_r.reset();
        self.k_stage2_l.reset();
        self.k_stage2_r.reset();
        self.momentary_blocks.fill(0.0);
        self.short_term_blocks.fill(0.0);
        self.all_blocks.clear();
        self.block_counter = 0;
        self.current_block_energy = 0.0;
        self.momentary_lufs = -100.0;
        self.short_term_lufs = -100.0;
        self.integrated_lufs = -100.0;
        self.max_true_peak_dbtp = -100.0;
    }

    #[inline(always)]
    pub fn process_sample(&mut self, left: f32, right: f32) {
        // True-peak tracking
        let peak = left.abs().max(right.abs());
        let peak_db = if peak > 1e-6 { 20.0 * peak.log10() } else { -120.0 };
        if peak_db > self.max_true_peak_dbtp {
            self.max_true_peak_dbtp = peak_db;
        }

        // Apply K-weighting pre-filtering
        let k_l = self.k_stage2_l.process_sample(self.k_stage1_l.process_sample(left));
        let k_r = self.k_stage2_r.process_sample(self.k_stage1_r.process_sample(right));

        // Mean square sum for stereo: z[i] = y_L[i]^2 + y_R[i]^2
        self.current_block_energy += k_l * k_l + k_r * k_r;
        self.block_counter += 1;

        if self.block_counter >= self.block_size {
            let mean_sq = self.current_block_energy / self.block_size as f32;
            self.current_block_energy = 0.0;
            self.block_counter = 0;

            // Store in momentary circular buffer
            self.momentary_blocks[self.m_idx] = mean_sq;
            self.m_idx = (self.m_idx + 1) % self.momentary_blocks.len();

            // Store in short-term circular buffer
            self.short_term_blocks[self.s_idx] = mean_sq;
            self.s_idx = (self.s_idx + 1) % self.short_term_blocks.len();

            // Calculate Momentary LUFS (400ms)
            let m_mean: f32 = self.momentary_blocks.iter().sum::<f32>() / self.momentary_blocks.len() as f32;
            self.momentary_lufs = if m_mean > 1e-10 {
                -0.691 + 10.0 * m_mean.log10()
            } else {
                -100.0
            };

            // Calculate Short-term LUFS (3000ms)
            let s_mean: f32 = self.short_term_blocks.iter().sum::<f32>() / self.short_term_blocks.len() as f32;
            self.short_term_lufs = if s_mean > 1e-10 {
                -0.691 + 10.0 * s_mean.log10()
            } else {
                -100.0
            };

            // Cumulative integrated (gated at -70 LUFS)
            if self.momentary_lufs > -70.0 {
                self.all_blocks.push(mean_sq);
                if self.all_blocks.len() > 10000 {
                    self.all_blocks.remove(0);
                }
                let int_mean: f32 = self.all_blocks.iter().sum::<f32>() / self.all_blocks.len() as f32;
                self.integrated_lufs = -0.691 + 10.0 * int_mean.log10();
            }
        }
    }
}
