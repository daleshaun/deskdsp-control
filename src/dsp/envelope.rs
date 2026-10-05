//! Fast/slow envelope follower used for dynamics detection (ported from desk-mic-forge).

#[derive(Debug, Clone)]
pub struct EnvelopeFollower {
    sample_rate: f32,
    attack_ms: f32,
    release_ms: f32,
    attack_coeff: f32,
    release_coeff: f32,
    envelope: f32,
}

impl EnvelopeFollower {
    pub fn new(sample_rate: f32, attack_ms: f32, release_ms: f32) -> Self {
        let mut ef = Self {
            sample_rate,
            attack_ms,
            release_ms,
            attack_coeff: 0.0,
            release_coeff: 0.0,
            envelope: 0.0,
        };
        ef.recalculate();
        ef
    }

    pub fn set_times(&mut self, attack_ms: f32, release_ms: f32) {
        self.attack_ms = attack_ms.max(0.01);
        self.release_ms = release_ms.max(0.1);
        self.recalculate();
    }

    fn recalculate(&mut self) {
        self.attack_coeff = Self::time_to_coeff(self.attack_ms, self.sample_rate);
        self.release_coeff = Self::time_to_coeff(self.release_ms, self.sample_rate);
    }

    fn time_to_coeff(time_ms: f32, sample_rate: f32) -> f32 {
        if time_ms <= 0.0 {
            1.0
        } else {
            1.0 - (-1.0 / (0.001 * time_ms * sample_rate)).exp()
        }
    }

    pub fn reset(&mut self) {
        self.envelope = 0.0;
    }

    #[inline(always)]
    pub fn process(&mut self, input: f32) -> f32 {
        let rectified = input.abs();
        let coeff = if rectified > self.envelope {
            self.attack_coeff
        } else {
            self.release_coeff
        };
        self.envelope += coeff * (rectified - self.envelope);
        self.envelope
    }

    #[inline(always)]
    pub fn get_value(&self) -> f32 {
        self.envelope
    }
}
