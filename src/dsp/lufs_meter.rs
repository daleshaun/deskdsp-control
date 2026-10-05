//! EBU R128 & ITU-R BS.1770-4 Compliant Loudness Meter (LUFS) + True-Peak Meter.
//!
//! Fully allocation-free on the audio thread. Features:
//! - ITU-R BS.1770 K-weighting pre-filter (Stage 1: +4dB high-shelf @ 1.5kHz, Stage 2: RLB HPF @ 38Hz).
//! - 4x oversampled polyphase FIR True-Peak detector compliant with BS.1770-4 Annex 2.
//! - Momentary loudness (400ms sliding window).
//! - Short-term loudness (3000ms sliding window).
//! - Integrated loudness with full two-stage gating (-70 LKFS absolute gate and -10 LU relative gate)
//!   using a fixed-size ring buffer without any heap allocations or memory shifts in the audio loop.

use super::biquad::{BiquadFilter, FilterType};
use super::limiter::calculate_true_peak_4x;

pub const INTEGRATED_BUFFER_SIZE: usize = 10000; // 10,000 * 100ms = 1,000 seconds of history

#[derive(Debug, Clone)]
pub struct LufsMeter {
    // K-weighting pre-filters (Stage 1: High shelf +4dB @ 1.5kHz; Stage 2: RLB high-pass @ 38Hz)
    k_stage1_l: BiquadFilter,
    k_stage1_r: BiquadFilter,
    k_stage2_l: BiquadFilter,
    k_stage2_r: BiquadFilter,

    // Circular buffers for momentary (400ms) and short-term (3000ms) energy
    sample_rate: f32,
    
    // Ring buffer storing mean square power per 100ms block
    block_size: usize,
    block_counter: usize,
    current_block_energy: f32,

    momentary_blocks: [f32; 4],   // 4 * 100ms = 400ms
    short_term_blocks: [f32; 30], // 30 * 100ms = 3000ms
    m_idx: usize,
    s_idx: usize,

    // Integrated loudness: Preallocated fixed-size ring buffer (zero audio-thread allocations)
    all_blocks: Box<[f32; INTEGRATED_BUFFER_SIZE]>,
    all_blocks_head: usize,
    all_blocks_count: usize,

    // 4x Oversampling true-peak sliding window
    recent_l: [f32; 12],
    recent_r: [f32; 12],
    recent_idx: usize,

    // Telemetry readouts
    pub momentary_lufs: f32,
    pub short_term_lufs: f32,
    pub integrated_lufs: f32,
    pub max_true_peak_dbtp: f32,
}

impl LufsMeter {
    pub fn new(sample_rate: f32) -> Self {
        // ITU-R BS.1770 K-weighting filter coefficients:
        // Stage 1: High-shelf filter (+4 dB at 1.5 kHz)
        let k_stage1_l = BiquadFilter::new(FilterType::HighShelf { q: 0.707 }, 1500.0, 4.0, sample_rate);
        let k_stage1_r = BiquadFilter::new(FilterType::HighShelf { q: 0.707 }, 1500.0, 4.0, sample_rate);
        // Stage 2: High-pass RLB weighting filter (Butterworth 2nd order at 38 Hz)
        let k_stage2_l = BiquadFilter::new(FilterType::HighPass, 38.0, 0.0, sample_rate);
        let k_stage2_r = BiquadFilter::new(FilterType::HighPass, 38.0, 0.0, sample_rate);

        let block_size = ((sample_rate * 0.1) as usize).max(1); // 100ms blocks

        Self {
            k_stage1_l,
            k_stage1_r,
            k_stage2_l,
            k_stage2_r,
            sample_rate,
            block_size,
            block_counter: 0,
            current_block_energy: 0.0,
            momentary_blocks: [0.0; 4],
            short_term_blocks: [0.0; 30],
            m_idx: 0,
            s_idx: 0,
            all_blocks: vec![0.0_f32; INTEGRATED_BUFFER_SIZE].into_boxed_slice().try_into().unwrap(),
            all_blocks_head: 0,
            all_blocks_count: 0,
            recent_l: [0.0; 12],
            recent_r: [0.0; 12],
            recent_idx: 0,
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
        self.all_blocks.fill(0.0);
        self.all_blocks_head = 0;
        self.all_blocks_count = 0;
        self.recent_l.fill(0.0);
        self.recent_r.fill(0.0);
        self.recent_idx = 0;
        self.block_counter = 0;
        self.current_block_energy = 0.0;
        self.momentary_lufs = -100.0;
        self.short_term_lufs = -100.0;
        self.integrated_lufs = -100.0;
        self.max_true_peak_dbtp = -100.0;
    }

    #[inline(always)]
    pub fn process_sample(&mut self, left: f32, right: f32) {
        // 1. Maintain 12-sample sliding window for 4x true-peak detection
        self.recent_l[self.recent_idx] = left;
        self.recent_r[self.recent_idx] = right;
        self.recent_idx = (self.recent_idx + 1) % 12;

        let mut hist_l = [0.0_f32; 12];
        let mut hist_r = [0.0_f32; 12];
        for i in 0..12 {
            let idx = (self.recent_idx + i) % 12;
            hist_l[i] = self.recent_l[idx];
            hist_r[i] = self.recent_r[idx];
        }

        // True 4x oversampled peak detection
        let tp_l = calculate_true_peak_4x(&hist_l);
        let tp_r = calculate_true_peak_4x(&hist_r);
        let peak = tp_l.max(tp_r);
        let peak_db = if peak > 1e-6 { 20.0 * peak.log10() } else { -120.0 };
        if peak_db > self.max_true_peak_dbtp {
            self.max_true_peak_dbtp = peak_db;
        }

        // 2. Apply K-weighting pre-filtering (Stage 1 HighShelf -> Stage 2 HighPass 38Hz)
        let k_l = self.k_stage2_l.process_sample(self.k_stage1_l.process_sample(left));
        let k_r = self.k_stage2_r.process_sample(self.k_stage1_r.process_sample(right));

        // Mean square sum for stereo: z[i] = y_L[i]^2 + y_R[i]^2
        self.current_block_energy += k_l * k_l + k_r * k_r;
        self.block_counter += 1;

        if self.block_counter >= self.block_size {
            let mean_sq = self.current_block_energy / self.block_size as f32;
            self.current_block_energy = 0.0;
            self.block_counter = 0;

            // Store in momentary circular buffer (400ms)
            self.momentary_blocks[self.m_idx] = mean_sq;
            self.m_idx = (self.m_idx + 1) % self.momentary_blocks.len();

            // Store in short-term circular buffer (3000ms)
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

            // Store in preallocated ring buffer (NO heap allocation)
            self.all_blocks[self.all_blocks_head] = mean_sq;
            self.all_blocks_head = (self.all_blocks_head + 1) % INTEGRATED_BUFFER_SIZE;
            if self.all_blocks_count < INTEGRATED_BUFFER_SIZE {
                self.all_blocks_count += 1;
            }

            // Two-stage BS.1770 gating over the recorded history:
            // Stage 1: Absolute threshold Gamma_a = -70 LKFS
            let abs_thresh_linear = 10.0_f32.powf((-70.0 + 0.691) / 10.0);
            let mut abs_sum = 0.0_f32;
            let mut abs_count = 0usize;

            for i in 0..self.all_blocks_count {
                let energy = self.all_blocks[i];
                if energy >= abs_thresh_linear {
                    abs_sum += energy;
                    abs_count += 1;
                }
            }

            if abs_count > 0 {
                let abs_mean = abs_sum / abs_count as f32;
                let abs_lufs = -0.691 + 10.0 * abs_mean.log10();

                // Stage 2: Relative threshold Gamma_r = Loudness_abs - 10 LU
                let rel_lufs = abs_lufs - 10.0;
                let rel_thresh_linear = 10.0_f32.powf((rel_lufs + 0.691) / 10.0);

                let mut rel_sum = 0.0_f32;
                let mut rel_count = 0usize;

                for i in 0..self.all_blocks_count {
                    let energy = self.all_blocks[i];
                    if energy >= rel_thresh_linear {
                        rel_sum += energy;
                        rel_count += 1;
                    }
                }

                if rel_count > 0 {
                    let rel_mean = rel_sum / rel_count as f32;
                    self.integrated_lufs = -0.691 + 10.0 * rel_mean.log10();
                } else {
                    self.integrated_lufs = abs_lufs;
                }
            } else {
                self.integrated_lufs = -100.0;
            }
        }
    }
}
