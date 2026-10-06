//! Partitioned Overlap-Save FFT Convolution Engine.
//!
//! Shared core used for both Guitar Cab Simulation (`CabSim`) and Microphone
//! Profiling (`MicImage`). All FFT plans, frequency-domain partition spectra,
//! and scratch buffers are allocated strictly at construction (`new`).
//! Audio-thread processing is 100% allocation-free (`assert_no_alloc`).

use std::sync::Arc;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};
use rustfft::num_complex::Complex32;

#[derive(Clone)]
pub struct ConvEngine {
    pub partition_len: usize,
    pub fft_len: usize,
    pub num_partitions: usize,

    // Precomputed frequency-domain partitions: [num_partitions][spectrum_len]
    h_freq: Vec<Complex32>,
    spectrum_len: usize,

    // Circular history of input spectra: [num_partitions][spectrum_len]
    x_freq_history: Vec<Complex32>,
    history_idx: usize,

    // Previous input block of size partition_len for overlap
    prev_input: Vec<f32>,

    // FFT plans & preallocated scratch vectors
    fft_forward: Arc<dyn RealToComplex<f32>>,
    fft_inverse: Arc<dyn ComplexToReal<f32>>,
    forward_scratch: Vec<Complex32>,
    inverse_scratch: Vec<Complex32>,

    // Time-domain and frequency-domain working buffers
    time_buf_in: Vec<f32>,       // 2 * partition_len
    time_buf_out: Vec<f32>,      // 2 * partition_len
    curr_input_freq: Vec<Complex32>, // spectrum_len
    accum_freq: Vec<Complex32>,  // spectrum_len

    // Sample-by-sample bridging FIFO
    sample_buf_in: Vec<f32>,
    sample_in_count: usize,
    sample_buf_out: Vec<f32>,
    sample_out_pos: usize,
    sample_out_count: usize,

    fft_scale: f32,
}

impl ConvEngine {
    /// Creates a new partitioned convolution engine from an impulse response.
    ///
    /// - `ir`: Impulse response samples.
    /// - `partition_len`: Block size (typically 128 or 256 samples). Latency equals `partition_len`.
    pub fn new(ir: &[f32], partition_len: usize) -> Self {
        let partition_len = partition_len.max(32).next_power_of_two();
        let fft_len = partition_len * 2;
        let spectrum_len = fft_len / 2 + 1;

        let num_partitions = if ir.is_empty() {
            1
        } else {
            ((ir.len() + partition_len - 1) / partition_len).max(1)
        };

        let mut planner = RealFftPlanner::<f32>::new();
        let fft_forward = planner.plan_fft_forward(fft_len);
        let fft_inverse = planner.plan_fft_inverse(fft_len);
        let forward_scratch = fft_forward.make_scratch_vec();
        let inverse_scratch = fft_inverse.make_scratch_vec();

        // Precompute partitions: pad each partition with L zeros at the end
        let mut h_freq = vec![Complex32::new(0.0, 0.0); num_partitions * spectrum_len];
        let mut time_scratch = vec![0.0_f32; fft_len];
        let mut freq_scratch = vec![Complex32::new(0.0, 0.0); spectrum_len];
        let mut scratch = fft_forward.make_scratch_vec();

        for p in 0..num_partitions {
            time_scratch.fill(0.0);
            let start = p * partition_len;
            let end = (start + partition_len).min(ir.len());
            if start < ir.len() {
                time_scratch[..end - start].copy_from_slice(&ir[start..end]);
            }

            fft_forward
                .process_with_scratch(&mut time_scratch, &mut freq_scratch, &mut scratch)
                .expect("FFT transform failed");

            let dest_slice = &mut h_freq[p * spectrum_len..(p + 1) * spectrum_len];
            dest_slice.copy_from_slice(&freq_scratch);
        }

        Self {
            partition_len,
            fft_len,
            num_partitions,
            h_freq,
            spectrum_len,
            x_freq_history: vec![Complex32::new(0.0, 0.0); num_partitions * spectrum_len],
            history_idx: 0,
            prev_input: vec![0.0; partition_len],
            fft_forward,
            fft_inverse,
            forward_scratch,
            inverse_scratch,
            time_buf_in: vec![0.0; fft_len],
            time_buf_out: vec![0.0; fft_len],
            curr_input_freq: vec![Complex32::new(0.0, 0.0); spectrum_len],
            accum_freq: vec![Complex32::new(0.0, 0.0); spectrum_len],
            sample_buf_in: vec![0.0; partition_len],
            sample_in_count: 0,
            sample_buf_out: vec![0.0; partition_len],
            sample_out_pos: 0,
            sample_out_count: 0,
            fft_scale: 1.0 / (fft_len as f32),
        }
    }

    /// Resets all delay lines and internal history buffers.
    pub fn reset(&mut self) {
        self.x_freq_history.fill(Complex32::new(0.0, 0.0));
        self.history_idx = 0;
        self.prev_input.fill(0.0);
        self.time_buf_in.fill(0.0);
        self.time_buf_out.fill(0.0);
        self.curr_input_freq.fill(Complex32::new(0.0, 0.0));
        self.accum_freq.fill(Complex32::new(0.0, 0.0));
        self.sample_buf_in.fill(0.0);
        self.sample_in_count = 0;
        self.sample_buf_out.fill(0.0);
        self.sample_out_pos = 0;
        self.sample_out_count = 0;
    }

    /// Processes exactly one block of `partition_len` samples in-place. Zero allocations.
    pub fn process_partition_block(&mut self, in_block: &[f32], out_block: &mut [f32]) {
        assert_eq!(in_block.len(), self.partition_len);
        assert_eq!(out_block.len(), self.partition_len);

        // 1. Build 2L time domain buffer: [prev_input (L), in_block (L)]
        self.time_buf_in[..self.partition_len].copy_from_slice(&self.prev_input);
        self.time_buf_in[self.partition_len..].copy_from_slice(in_block);
        self.prev_input.copy_from_slice(in_block);

        // 2. Forward FFT of time input buffer
        self.fft_forward
            .process_with_scratch(
                &mut self.time_buf_in,
                &mut self.curr_input_freq,
                &mut self.forward_scratch,
            )
            .expect("FFT forward failed");

        // 3. Store in circular frequency history buffer
        let hist_start = self.history_idx * self.spectrum_len;
        let hist_end = hist_start + self.spectrum_len;
        self.x_freq_history[hist_start..hist_end].copy_from_slice(&self.curr_input_freq);

        // 4. Frequency-domain accumulation across all partitions
        self.accum_freq.fill(Complex32::new(0.0, 0.0));

        for p in 0..self.num_partitions {
            // circular offset: current is history_idx, previous is (history_idx - p)
            let read_slot = (self.history_idx + self.num_partitions - p) % self.num_partitions;
            let x_slice = &self.x_freq_history[read_slot * self.spectrum_len..(read_slot + 1) * self.spectrum_len];
            let h_slice = &self.h_freq[p * self.spectrum_len..(p + 1) * self.spectrum_len];

            for k in 0..self.spectrum_len {
                self.accum_freq[k] += x_slice[k] * h_slice[k];
            }
        }

        // Advance history pointer
        self.history_idx = (self.history_idx + 1) % self.num_partitions;

        // 5. Inverse FFT back to time domain
        self.fft_inverse
            .process_with_scratch(
                &mut self.accum_freq,
                &mut self.time_buf_out,
                &mut self.inverse_scratch,
            )
            .expect("FFT inverse failed");

        // 6. Overlap-save: extract the second half [L .. 2L] and apply scale
        let scale = self.fft_scale;
        for i in 0..self.partition_len {
            out_block[i] = self.time_buf_out[self.partition_len + i] * scale;
        }
    }

    /// Process a single sample through internal FIFO bridging (allocation-free).
    #[inline(always)]
    pub fn process_sample(&mut self, sample: f32) -> f32 {
        self.sample_buf_in[self.sample_in_count] = sample;
        self.sample_in_count += 1;

        if self.sample_in_count == self.partition_len {
            let l = self.partition_len;
            
            // Overlap-save partition processing
            self.time_buf_in[..l].copy_from_slice(&self.prev_input);
            self.time_buf_in[l..].copy_from_slice(&self.sample_buf_in);
            self.prev_input.copy_from_slice(&self.sample_buf_in);

            self.fft_forward
                .process_with_scratch(
                    &mut self.time_buf_in,
                    &mut self.curr_input_freq,
                    &mut self.forward_scratch,
                )
                .expect("FFT forward failed");

            let hist_start = self.history_idx * self.spectrum_len;
            let hist_end = hist_start + self.spectrum_len;
            self.x_freq_history[hist_start..hist_end].copy_from_slice(&self.curr_input_freq);

            self.accum_freq.fill(Complex32::new(0.0, 0.0));

            for p in 0..self.num_partitions {
                let read_slot = (self.history_idx + self.num_partitions - p) % self.num_partitions;
                let x_slice = &self.x_freq_history[read_slot * self.spectrum_len..(read_slot + 1) * self.spectrum_len];
                let h_slice = &self.h_freq[p * self.spectrum_len..(p + 1) * self.spectrum_len];

                for k in 0..self.spectrum_len {
                    self.accum_freq[k] += x_slice[k] * h_slice[k];
                }
            }

            self.history_idx = (self.history_idx + 1) % self.num_partitions;

            self.fft_inverse
                .process_with_scratch(
                    &mut self.accum_freq,
                    &mut self.time_buf_out,
                    &mut self.inverse_scratch,
                )
                .expect("FFT inverse failed");

            let scale = self.fft_scale;
            for i in 0..l {
                self.sample_buf_out[i] = self.time_buf_out[l + i] * scale;
            }

            self.sample_in_count = 0;
            self.sample_out_pos = 0;
            self.sample_out_count = l;
        }

        if self.sample_out_count > 0 {
            let out_sample = self.sample_buf_out[self.sample_out_pos];
            self.sample_out_pos += 1;
            self.sample_out_count -= 1;
            out_sample
        } else {
            0.0
        }
    }

    /// Process an arbitrary block in-place with zero allocations.
    pub fn process_block(&mut self, buffer: &mut [f32]) {
        for s in buffer.iter_mut() {
            *s = self.process_sample(*s);
        }
    }
}
