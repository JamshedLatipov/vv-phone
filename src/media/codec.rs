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
