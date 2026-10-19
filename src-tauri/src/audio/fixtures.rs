//! Generated PCM files for audio tests (no real music in the repo).

use std::path::{Path, PathBuf};

/// Value of frame `i`, channel `c`: a distinct, non-zero ramp so positions can be checked exactly.
pub fn ramp(i: usize, c: usize) -> f32 {
    let v = ((i % 1000) as f32 / 1000.0) * 0.5 + 0.1;
    if c == 0 { v } else { -v }
}

/// Little-endian PCM WAV, 16 or 24 bit.
pub fn wav(dir: &Path, name: &str, rate: u32, channels: u16, bits: u16, frames: usize) -> PathBuf {
    let bytes_per = (bits / 8) as usize;
    let mut data = Vec::with_capacity(frames * channels as usize * bytes_per);
    for i in 0..frames {
        for c in 0..channels as usize {
            let v = ramp(i, c);
            if bits == 16 {
                data.extend(((v * 32767.0).round() as i16).to_le_bytes());
            } else {
                let s = (v * 8_388_607.0).round() as i32;
                data.extend(&s.to_le_bytes()[..3]);
            }
        }
    }
    let mut v = b"RIFF".to_vec();
    v.extend((36 + data.len() as u32).to_le_bytes());
    v.extend(b"WAVEfmt ");
    v.extend(16u32.to_le_bytes());
    v.extend(1u16.to_le_bytes()); // PCM
    v.extend(channels.to_le_bytes());
    v.extend(rate.to_le_bytes());
    v.extend((rate * channels as u32 * bytes_per as u32).to_le_bytes());
    v.extend((channels * bytes_per as u16).to_le_bytes());
    v.extend(bits.to_le_bytes());
    v.extend(b"data");
    v.extend((data.len() as u32).to_le_bytes());
    v.extend(data);
    let p = dir.join(name);
    std::fs::write(&p, v).unwrap();
    p
}

/// 80-bit IEEE extended, as AIFF stores the sample rate.
fn extended(x: f64) -> [u8; 10] {
    let exp = x.log2().floor() as i32;
    let mantissa = (x / 2f64.powi(exp) * 2f64.powi(63)) as u64;
    let mut out = [0u8; 10];
    out[..2].copy_from_slice(&((exp + 16383) as u16).to_be_bytes());
    out[2..].copy_from_slice(&mantissa.to_be_bytes());
    out
}

/// Big-endian 16-bit PCM AIFF.
pub fn aiff(dir: &Path, name: &str, rate: u32, channels: u16, frames: usize) -> PathBuf {
    let mut data = Vec::new();
    for i in 0..frames {
        for c in 0..channels as usize {
            data.extend(((ramp(i, c) * 32767.0).round() as i16).to_be_bytes());
        }
    }
    let mut comm = channels.to_be_bytes().to_vec();
    comm.extend((frames as u32).to_be_bytes());
    comm.extend(16u16.to_be_bytes());
    comm.extend(extended(rate as f64));
    let mut ssnd = vec![0u8; 8]; // offset + block size
    ssnd.extend(data);
    let mut body = b"AIFF".to_vec();
    body.extend(b"COMM");
    body.extend((comm.len() as u32).to_be_bytes());
    body.extend(comm);
    body.extend(b"SSND");
    body.extend((ssnd.len() as u32).to_be_bytes());
    body.extend(ssnd);
    let mut v = b"FORM".to_vec();
    v.extend((body.len() as u32).to_be_bytes());
    v.extend(body);
    let p = dir.join(name);
    std::fs::write(&p, v).unwrap();
    p
}
