pub struct G711;

impl G711 {
    pub fn linear_to_ulaw(sample: i16) -> u8 {
        let mut sample = sample;
        let sign = if sample < 0 {
            sample = -sample;
            0
        } else {
            0x80
        };
        if sample > 32635 { sample = 32635; }
        sample += 128 + 4;
        let mut exponent = 7;
        let mut exp_mask = 0x4000;
        while (sample & exp_mask) == 0 && exponent > 0 {
            exponent -= 1;
            exp_mask >>= 1;
        }
        let mantissa = (sample >> (exponent + 3)) & 0x0F;
        let ulaw = sign | (exponent << 4) | (mantissa as i32);
        !(ulaw as u8)
    }

    pub fn ulaw_to_linear(ulaw: u8) -> i16 {
        let ulaw = !ulaw;
        let sign = (ulaw & 0x80) != 0;
        let exponent = (ulaw >> 4) & 0x07;
        let mantissa = ulaw & 0x0F;
        let mut sample = ((mantissa as i16) << 3) + 132;
        sample <<= exponent;
        sample -= 132;
        if sign { sample } else { -sample }
    }
}

pub struct Resampler {
    source_rate: f32,
    target_rate: f32,
    phase: f32,
}

impl Resampler {
    pub fn new(source_rate: f32, target_rate: f32) -> Self {
        Self {
            source_rate,
            target_rate,
            phase: 0.0,
        }
    }

    pub fn resample(&mut self, input: &[f32], output: &mut [f32]) -> (usize, usize) {
        let ratio = self.source_rate / self.target_rate;
        let mut in_idx = 0;
        let mut out_idx = 0;

        while out_idx < output.len() && (in_idx as f32 + self.phase) < input.len() as f32 {
            let current_in = in_idx as f32 + self.phase;
            let i = current_in.floor() as usize;
            let frac = current_in - current_in.floor();

            let next_i = if i + 1 < input.len() { i + 1 } else { i };

            // Linear interpolation
            output[out_idx] = input[i] * (1.0 - frac) + input[next_i] * frac;

            out_idx += 1;
            self.phase += ratio;
            while self.phase >= 1.0 {
                self.phase -= 1.0;
                in_idx += 1;
            }
        }
        (in_idx, out_idx)
    }
}
