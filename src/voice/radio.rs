//! WAV in/out and a pit-radio effect for the coach's voice.
//!
//! The effect is what a team radio does to a voice: band-limit it to roughly 300 Hz–3.2 kHz, push
//! the presence range, squash the dynamics, drive it a little into saturation, and sit it on a bed
//! of hiss that keys up with a click and closes with a squelch tail. Besides sounding the part, the
//! narrow band hides most of the artefacts a synthetic voice has.

use anyhow::{bail, Context, Result};

/// Mono audio, samples in -1.0..=1.0.
#[derive(Debug, Clone)]
pub struct Wav {
    pub sample_rate: u32,
    pub samples: Vec<f32>,
}

/// Reads a RIFF WAV (16-bit PCM or 32-bit float, any channel count, downmixed to mono).
pub fn read_wav(bytes: &[u8]) -> Result<Wav> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        bail!("not a WAV file");
    }
    let u16_at = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]);
    let u32_at = |i: usize| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);

    let mut format = None;
    let mut pos = 12;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        // Streamed WAVs (espeak --stdout) can leave the data size at 0 or u32::MAX: read to the end.
        let declared = u32_at(pos + 4) as usize;
        let body = pos + 8;
        let len = if declared == 0 || body + declared > bytes.len() { bytes.len() - body } else { declared };
        match id {
            b"fmt " => {
                if len < 16 {
                    bail!("WAV fmt chunk too short");
                }
                // (format tag, channels, sample rate, bits per sample)
                format = Some((u16_at(body), u16_at(body + 2), u32_at(body + 4), u16_at(body + 14)));
            }
            b"data" => {
                let (tag, channels, sample_rate, bits) = format.context("WAV data before fmt chunk")?;
                let channels = channels.max(1) as usize;
                let data = &bytes[body..body + len];
                let interleaved: Vec<f32> = match (tag, bits) {
                    (1 | 0xFFFE, 16) => data.as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes(*c) as f32 / 32768.0).collect(),
                    (3 | 0xFFFE, 32) => data.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect(),
                    _ => bail!("unsupported WAV format (tag {tag}, {bits}-bit)"),
                };
                let samples = interleaved
                    .chunks_exact(channels)
                    .map(|frame| frame.iter().sum::<f32>() / channels as f32)
                    .collect();
                return Ok(Wav { sample_rate, samples });
            }
            _ => {}
        }
        pos = body + len + (len & 1);
    }
    bail!("WAV has no data chunk")
}

/// 16-bit PCM mono WAV.
pub fn write_wav(wav: &Wav) -> Vec<u8> {
    let data_len = (wav.samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&wav.sample_rate.to_le_bytes());
    out.extend_from_slice(&(wav.sample_rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in &wav.samples {
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes());
    }
    out
}

/// RBJ cookbook biquad, direct form I.
#[derive(Clone, Copy)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Biquad {
    fn new(b: [f64; 3], a: [f64; 3]) -> Self {
        let n = |v: f64| (v / a[0]) as f32;
        Biquad { b0: n(b[0]), b1: n(b[1]), b2: n(b[2]), a1: n(a[1]), a2: n(a[2]), x1: 0.0, x2: 0.0, y1: 0.0, y2: 0.0 }
    }

    fn highpass(sr: f64, f0: f64, q: f64) -> Self {
        let (cos, alpha) = Self::omega(sr, f0, q);
        Self::new([(1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0], [1.0 + alpha, -2.0 * cos, 1.0 - alpha])
    }

    fn lowpass(sr: f64, f0: f64, q: f64) -> Self {
        let (cos, alpha) = Self::omega(sr, f0, q);
        Self::new([(1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0], [1.0 + alpha, -2.0 * cos, 1.0 - alpha])
    }

    fn peak(sr: f64, f0: f64, q: f64, gain_db: f64) -> Self {
        let (cos, alpha) = Self::omega(sr, f0, q);
        let a = 10f64.powf(gain_db / 40.0);
        Self::new([1.0 + alpha * a, -2.0 * cos, 1.0 - alpha * a], [1.0 + alpha / a, -2.0 * cos, 1.0 - alpha / a])
    }

    fn omega(sr: f64, f0: f64, q: f64) -> (f64, f64) {
        // Keep the corner below Nyquist so low sample rates (espeak's 22.05 kHz) stay stable.
        let w = 2.0 * std::f64::consts::PI * f0.min(sr * 0.45) / sr;
        (w.cos(), w.sin() / (2.0 * q))
    }

    fn run(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2 - self.a1 * self.y1 - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// The radio's speaker and mic: a 4th-order 300 Hz–3.2 kHz band with a presence lift.
struct RadioBand(Vec<Biquad>);

impl RadioBand {
    fn new(sr: f64) -> Self {
        let q = std::f64::consts::FRAC_1_SQRT_2;
        RadioBand(vec![
            Biquad::highpass(sr, 300.0, q),
            Biquad::highpass(sr, 300.0, q),
            Biquad::lowpass(sr, 3200.0, q),
            Biquad::lowpass(sr, 3200.0, q),
            Biquad::peak(sr, 1800.0, 1.0, 4.0),
        ])
    }

    fn run(&mut self, x: f32) -> f32 {
        self.0.iter_mut().fold(x, |v, f| f.run(v))
    }
}

/// Deterministic white noise (xorshift32), so the same text always renders the same audio.
struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

fn normalize(samples: &mut [f32], peak: f32) {
    let max = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    if max > 1e-6 {
        samples.iter_mut().for_each(|s| *s *= peak / max);
    }
}

/// Feed-forward compressor on a peak envelope, levels in dB.
fn compress(samples: &mut [f32], sr: f32, threshold_db: f32, ratio: f32, attack_s: f32, release_s: f32) {
    let attack = (-1.0 / (attack_s * sr)).exp();
    let release = (-1.0 / (release_s * sr)).exp();
    let mut env = 0.0f32;
    for s in samples.iter_mut() {
        let level = s.abs();
        let coeff = if level > env { attack } else { release };
        env = coeff * env + (1.0 - coeff) * level;
        let env_db = 20.0 * env.max(1e-6).log10();
        let over = env_db - threshold_db;
        if over > 0.0 {
            *s *= 10f32.powf(-over * (1.0 - 1.0 / ratio) / 20.0);
        }
    }
}

/// Where a clip sits in a transmission. A message spoken sentence by sentence is several clips
/// played back to back: only the first keys up and only the last closes the squelch, so together
/// they sound like one transmission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transmission {
    pub first: bool,
    pub last: bool,
}

/// Pause after a sentence that isn't the last, filled with hiss so the next clip butts onto it.
pub const SENTENCE_GAP_S: f32 = 0.25;

/// Voice through a team radio: key-up click, hiss under the voice, squelch tail at the end.
///
/// Levels are fixed rather than normalised per clip, so the hiss stays the same level from one
/// sentence to the next.
pub fn radio_filter(input: &Wav, part: Transmission) -> Wav {
    let sr = input.sample_rate as f32;
    let secs = |s: f32| (s * sr) as usize;
    // Vary the seed per clip so consecutive sentences don't repeat the same hiss.
    let mut noise = Noise(0x9E37_79B9 ^ (input.samples.len() as u32).wrapping_mul(2_654_435_761) | 1);

    let voice = &input.samples;
    let lead = if part.first { secs(0.12) } else { 0 };
    let tail = if part.last { secs(0.16) } else { secs(SENTENCE_GAP_S) };
    let total = lead + voice.len() + tail;
    let mut out = vec![0.0f32; total];

    // Voice: band-limit, compress hard, then soft-clip for a little grit.
    let mut band = RadioBand::new(sr as f64);
    let drive = 1.8f32;
    let norm = drive.tanh();
    let mut shaped: Vec<f32> = voice.iter().map(|&x| band.run(x)).collect();
    normalize(&mut shaped, 0.9);
    compress(&mut shaped, sr, -20.0, 4.0, 0.004, 0.08);
    normalize(&mut shaped, 0.9);
    for (i, x) in shaped.iter().enumerate() {
        out[lead + i] = (drive * x).tanh() / norm * 0.82;
    }

    // Hiss for the whole transmission, through its own copy of the band so it sounds like the
    // radio. Faded in and out only where the transmission really starts and ends.
    let mut hiss_band = RadioBand::new(sr as f64);
    let fade = secs(0.01).max(1);
    for (i, s) in out.iter_mut().enumerate() {
        let from_start = if part.first { i as f32 / fade as f32 } else { 1.0 };
        let to_end = if part.last { (total - 1 - i) as f32 / fade as f32 } else { 1.0 };
        *s += hiss_band.run(noise.next()) * 0.018 * from_start.min(to_end).min(1.0);
    }

    // Key-up click: a few milliseconds of decaying noise right at the start.
    if part.first {
        let click = secs(0.006).max(1);
        let at = secs(0.02).min(total.saturating_sub(click));
        let mut click_band = RadioBand::new(sr as f64);
        for i in 0..click.min(total - at) {
            let decay = 1.0 - i as f32 / click as f32;
            out[at + i] += click_band.run(noise.next()) * 0.5 * decay * decay;
        }
    }

    // Squelch tail: a burst of louder static that dies away after the voice.
    if part.last {
        let mut tail_band = RadioBand::new(sr as f64);
        let tail_start = lead + voice.len() + secs(0.03);
        let tail_len = secs(0.11).max(1);
        for i in 0..tail_len {
            let at = tail_start + i;
            if at >= total {
                break;
            }
            let t = i as f32 / tail_len as f32;
            out[at] += tail_band.run(noise.next()) * 0.22 * (1.0 - t).powi(2);
        }
    }

    // Safety net only: the fixed levels above stay under this almost always.
    let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    if peak > 0.89 {
        normalize(&mut out, 0.89);
    }
    Wav { sample_rate: input.sample_rate, samples: out }
}

/// Scales a clip by `volume` (a multiplier). Above 1.0 it is made louder without hard clipping: a
/// tanh soft-limiter with small-signal gain of about `volume` whose loudest sample lands on 0.97.
pub fn apply_volume(wav: &mut Wav, volume: f32) {
    const CEILING: f32 = 0.97;
    let peak = wav.samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    if peak <= 0.0 || !volume.is_finite() {
        return;
    }
    if volume <= 1.0 || volume * peak <= CEILING {
        for s in &mut wav.samples {
            *s *= volume;
        }
        return;
    }
    let g = volume / CEILING;
    let norm = CEILING / (g * peak).tanh();
    for s in &mut wav.samples {
        *s = norm * (g * *s).tanh();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(wav: &Wav) -> f32 {
        (wav.samples.iter().map(|s| s * s).sum::<f32>() / wav.samples.len() as f32).sqrt()
    }

    fn peak(wav: &Wav) -> f32 {
        wav.samples.iter().fold(0.0f32, |m, s| m.max(s.abs()))
    }

    #[test]
    fn volume_one_is_a_no_op() {
        let wav = sine(24_000, 440.0, 0.1);
        let mut out = wav.clone();
        apply_volume(&mut out, 1.0);
        assert!(wav.samples.iter().zip(&out.samples).all(|(a, b)| (a - b).abs() < 1e-6));
    }

    #[test]
    fn volume_below_one_scales() {
        let wav = sine(24_000, 440.0, 0.1);
        let mut out = wav.clone();
        apply_volume(&mut out, 0.5);
        assert!(wav.samples.iter().zip(&out.samples).all(|(a, b)| (a * 0.5 - b).abs() < 1e-6));
    }

    #[test]
    fn volume_above_one_is_louder_without_clipping() {
        let wav = sine(24_000, 440.0, 0.1);
        let mut out = wav.clone();
        apply_volume(&mut out, 2.0);
        assert!(peak(&out) <= 0.97 + 1e-6);
        assert!(rms(&out) > rms(&wav) * 1.6);
        // A clip already near full scale gets denser, never past the ceiling.
        let mut hot = wav.clone();
        hot.samples.iter_mut().for_each(|s| *s *= 1.78);
        apply_volume(&mut hot, 3.0);
        assert!(peak(&hot) <= 0.97 + 1e-6);
    }

    fn sine(sr: u32, freq: f32, secs: f32) -> Wav {
        let n = (sr as f32 * secs) as usize;
        Wav {
            sample_rate: sr,
            samples: (0..n).map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / sr as f32).sin() * 0.5).collect(),
        }
    }

    fn band_rms(sr: u32, freq: f32) -> f32 {
        let mut band = RadioBand::new(sr as f64);
        let wav = sine(sr, freq, 0.5);
        let out: Vec<f32> = wav.samples.iter().map(|&x| band.run(x)).collect();
        let settled = &out[out.len() / 2..];
        (settled.iter().map(|s| s * s).sum::<f32>() / settled.len() as f32).sqrt()
    }

    #[test]
    fn wav_round_trip() {
        let wav = sine(24_000, 440.0, 0.1);
        let back = read_wav(&write_wav(&wav)).unwrap();
        assert_eq!(back.sample_rate, 24_000);
        assert_eq!(back.samples.len(), wav.samples.len());
        assert!(wav.samples.iter().zip(&back.samples).all(|(a, b)| (a - b).abs() < 1e-3));
    }

    #[test]
    fn reads_stereo_float_with_unknown_data_size() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF\0\0\0\0WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&3u16.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&22_050u32.to_le_bytes());
        bytes.extend_from_slice(&(22_050u32 * 8).to_le_bytes());
        bytes.extend_from_slice(&8u16.to_le_bytes());
        bytes.extend_from_slice(&32u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());
        for frame in [[0.5f32, 0.1], [-0.2, -0.4]] {
            for s in frame {
                bytes.extend_from_slice(&s.to_le_bytes());
            }
        }
        let wav = read_wav(&bytes).unwrap();
        assert_eq!(wav.sample_rate, 22_050);
        assert_eq!(wav.samples.len(), 2);
        assert!((wav.samples[0] - 0.3).abs() < 1e-6 && (wav.samples[1] + 0.3).abs() < 1e-6);
    }

    #[test]
    fn band_keeps_voice_and_cuts_the_rest() {
        for sr in [22_050, 24_000] {
            let mid = band_rms(sr, 1000.0);
            assert!(mid > 0.3, "1 kHz should pass ({mid})");
            assert!(band_rms(sr, 80.0) < mid * 0.1, "80 Hz should be cut");
            assert!(band_rms(sr, 9000.0) < mid * 0.1, "9 kHz should be cut");
        }
    }

    #[test]
    fn radio_output_is_padded_and_never_clips() {
        let wav = sine(24_000, 700.0, 0.4);
        let out = radio_filter(&wav, Transmission { first: true, last: true });
        assert!(out.samples.len() > wav.samples.len());
        assert!(out.samples.iter().all(|s| s.abs() <= 0.9));
        assert!(out.samples.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn sentence_parts_butt_together() {
        let wav = sine(24_000, 700.0, 0.4);
        let first = radio_filter(&wav, Transmission { first: true, last: false });
        let middle = radio_filter(&wav, Transmission { first: false, last: false });
        let last = radio_filter(&wav, Transmission { first: false, last: true });
        let gap = (24_000.0 * SENTENCE_GAP_S) as usize;
        // No lead-in after the first part; every part but the last ends in the sentence pause.
        assert_eq!(middle.samples.len(), wav.samples.len() + gap);
        assert_eq!(first.samples.len(), (24_000.0 * 0.12) as usize + wav.samples.len() + gap);
        // Hiss runs right up to the joins instead of fading to silence there.
        assert!(middle.samples[0].abs() > 0.0 && first.samples.last().unwrap().abs() > 0.0);
        assert!(last.samples.last().unwrap().abs() < 0.01);
    }
}
