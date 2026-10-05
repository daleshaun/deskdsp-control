//! Musical Scale Pitch Quantizer for Vocal Tuning.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Note {
    C = 0,
    CSharp = 1,
    D = 2,
    DSharp = 3,
    E = 4,
    F = 5,
    FSharp = 6,
    G = 7,
    GSharp = 8,
    A = 9,
    ASharp = 10,
    B = 11,
}

impl Note {
    pub fn name(self) -> &'static str {
        match self {
            Note::C => "C",
            Note::CSharp => "C#",
            Note::D => "D",
            Note::DSharp => "D#",
            Note::E => "E",
            Note::F => "F",
            Note::FSharp => "F#",
            Note::G => "G",
            Note::GSharp => "G#",
            Note::A => "A",
            Note::ASharp => "A#",
            Note::B => "B",
        }
    }

    pub fn from_semitone(semitone: u32) -> Self {
        match semitone % 12 {
            0 => Note::C,
            1 => Note::CSharp,
            2 => Note::D,
            3 => Note::DSharp,
            4 => Note::E,
            5 => Note::F,
            6 => Note::FSharp,
            7 => Note::G,
            8 => Note::GSharp,
            9 => Note::A,
            10 => Note::ASharp,
            _ => Note::B,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scale {
    Chromatic,
    Major,
    NaturalMinor,
    HarmonicMinor,
    MajorPentatonic,
    MinorPentatonic,
}

impl Scale {
    pub fn mask(self) -> [bool; 12] {
        match self {
            Scale::Chromatic => [true; 12],
            // Major: R, 2, 3, 4, 5, 6, 7 (0, 2, 4, 5, 7, 9, 11)
            Scale::Major => [true, false, true, false, true, true, false, true, false, true, false, true],
            // Natural Minor: R, 2, b3, 4, 5, b6, b7 (0, 2, 3, 5, 7, 8, 10)
            Scale::NaturalMinor => [true, false, true, true, false, true, false, true, true, false, true, false],
            // Harmonic Minor: R, 2, b3, 4, 5, b6, 7 (0, 2, 3, 5, 7, 8, 11)
            Scale::HarmonicMinor => [true, false, true, true, false, true, false, true, true, false, false, true],
            // Major Pentatonic: R, 2, 3, 5, 6 (0, 2, 4, 7, 9)
            Scale::MajorPentatonic => [true, false, true, false, true, false, false, true, false, true, false, false],
            // Minor Pentatonic: R, b3, 4, 5, b7 (0, 3, 5, 7, 10)
            Scale::MinorPentatonic => [true, false, false, true, false, true, false, true, false, false, true, false],
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScaleQuantizer {
    pub root: Note,
    pub scale: Scale,
    pub last_target_midi: Option<i32>,
}

impl ScaleQuantizer {
    pub fn new(root: Note, scale: Scale) -> Self {
        Self { root, scale, last_target_midi: None }
    }

    #[inline(always)]
    pub fn freq_to_midi(freq: f32) -> f32 {
        69.0 + 12.0 * (freq / 440.0).log2()
    }

    #[inline(always)]
    pub fn midi_to_freq(midi: f32) -> f32 {
        440.0 * 2.0_f32.powf((midi - 69.0) / 12.0)
    }

    /// Quantizes input frequency with hysteresis to prevent note flutter near scale boundaries.
    pub fn quantize_with_hysteresis(&mut self, freq: f32) -> (f32, &'static str, f32) {
        if freq <= 10.0 {
            return (freq, "--", 0.0);
        }

        let midi = Self::freq_to_midi(freq);
        let rounded_midi = midi.round() as i32;
        let scale_mask = self.scale.mask();
        let root_offset = self.root as i32;

        let mut best_midi = rounded_midi;
        let mut min_distance = 999.0_f32;

        for offset in -6..=6 {
            let candidate_midi = rounded_midi + offset;
            let note_in_scale = (candidate_midi - root_offset).rem_euclid(12) as usize;
            if scale_mask[note_in_scale] {
                let dist = (candidate_midi as f32 - midi).abs();
                if dist < min_distance {
                    min_distance = dist;
                    best_midi = candidate_midi;
                }
            }
        }

        // Hysteresis: stick with the previous note unless the new candidate is > 20 cents closer
        if let Some(prev) = self.last_target_midi {
            let prev_in_scale = (prev - root_offset).rem_euclid(12) as usize;
            if scale_mask[prev_in_scale] {
                let prev_dist = (prev as f32 - midi).abs();
                if prev_dist < 0.70 && prev_dist <= min_distance + 0.20 {
                    best_midi = prev;
                }
            }
        }

        self.last_target_midi = Some(best_midi);
        let target_freq = Self::midi_to_freq(best_midi as f32);
        let cents = 1200.0 * (freq / target_freq).log2();
        let note = Note::from_semitone(best_midi.rem_euclid(12) as u32);

        (target_freq, note.name(), cents)
    }

    /// Quantizes input frequency to the closest note in the selected scale.
    /// Returns (target_freq_hz, note_name, cents_deviation).
    pub fn quantize(&self, freq: f32) -> (f32, &'static str, f32) {
        let mut clone = self.clone();
        clone.last_target_midi = None;
        clone.quantize_with_hysteresis(freq)
    }
}
