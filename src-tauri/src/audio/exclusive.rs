//! Exclusive (bit-perfect) output (F7.1): the device runs at the file's own rate and receives
//! integer samples, bypassing the Windows mixer. This file holds the pure parts: picking a format
//! the device accepts and turning samples into the bytes it wants.

/// Integer sample layouts exclusive-mode devices accept, widest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleKind {
    I32,
    /// 24 valid bits in a 32-bit container.
    I24In32,
    /// Packed 3-byte samples.
    I24,
    I16,
}

impl SampleKind {
    pub const ALL: [SampleKind; 4] = [SampleKind::I32, SampleKind::I24In32, SampleKind::I24, SampleKind::I16];

    /// Bits per sample in memory.
    pub fn store_bits(self) -> u32 {
        match self {
            SampleKind::I32 | SampleKind::I24In32 => 32,
            SampleKind::I24 => 24,
            SampleKind::I16 => 16,
        }
    }

    /// Bits that carry the signal.
    pub fn valid_bits(self) -> u32 {
        match self {
            SampleKind::I32 => 32,
            SampleKind::I24In32 | SampleKind::I24 => 24,
            SampleKind::I16 => 16,
        }
    }

    /// True when a source with `bits` bit depth passes through without losing any.
    pub fn keeps(self, bits: Option<u32>) -> bool {
        bits.is_none_or(|b| b <= self.valid_bits())
    }
}

/// Append `samples` (-1.0..=1.0) to `out` as little-endian integers of `kind`, clipping at full scale.
pub fn encode(samples: &[f32], kind: SampleKind, out: &mut Vec<u8>) {
    let scaled = |s: f32, bits: u32| {
        let full = (1i64 << (bits - 1)) as f64;
        (s as f64 * full).round().clamp(-full, full - 1.0) as i64
    };
    out.reserve(samples.len() * kind.store_bits() as usize / 8);
    for &s in samples {
        match kind {
            SampleKind::I16 => out.extend_from_slice(&(scaled(s, 16) as i16).to_le_bytes()),
            SampleKind::I24 => out.extend_from_slice(&(scaled(s, 24) as i32).to_le_bytes()[..3]),
            SampleKind::I24In32 => out.extend_from_slice(&((scaled(s, 24) as i32) << 8).to_le_bytes()),
            SampleKind::I32 => out.extend_from_slice(&(scaled(s, 32) as i32).to_le_bytes()),
        }
    }
}

const RATES: [u32; 8] = [44_100, 48_000, 88_200, 96_000, 176_400, 192_000, 352_800, 384_000];

/// The rate and sample layout to open the device with for a file at `source_rate`: the file's own
/// rate when accepted (no resampling), else the nearest rate of the same family (44.1 or 48 kHz),
/// preferring higher; within a rate, the widest layout. `supports(rate, kind)` asks the device.
pub fn choose_format(source_rate: u32, supports: impl Fn(u32, SampleKind) -> bool) -> Option<(u32, SampleKind)> {
    let family = |r: u32| r % 44_100 == 0;
    let mut rates: Vec<u32> = RATES.iter().copied().filter(|&r| r != source_rate).collect();
    rates.sort_by_key(|&r| {
        let distance = ((r as f64 / source_rate as f64).log2().abs() * 100.0).round() as u32;
        (family(r) != family(source_rate), distance, r < source_rate)
    });
    std::iter::once(source_rate)
        .chain(rates)
        .find_map(|r| SampleKind::ALL.into_iter().find(|&k| supports(r, k)).map(|k| (r, k)))
}

/// The device side: a thread that holds the endpoint in exclusive mode and keeps its buffer full.
#[cfg(windows)]
pub mod device {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{self, Sender};
    use std::sync::Arc;
    use std::thread::JoinHandle;
    use std::time::Duration;

    use wasapi::{calculate_period_100ns, DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat};

    use super::{choose_format, encode, SampleKind};

    /// Fills interleaved f32 frames; runs on the device thread.
    pub type Render = Box<dyn FnMut(&mut [f32]) + Send>;

    pub struct Opened {
        pub rate: u32,
        pub kind: SampleKind,
        pub channels: usize,
        pub name: String,
    }

    /// Holds the device; dropping it stops playback and gives the device back to Windows.
    pub struct ExclusiveOut {
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
    }

    impl Drop for ExclusiveOut {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }

    fn wave(rate: u32, kind: SampleKind, channels: usize) -> WaveFormat {
        WaveFormat::new(kind.store_bits() as usize, kind.valid_bits() as usize, &SampleType::Int, rate as usize, channels, None)
    }

    /// The format `open` would pick on the device named `name` (or the default) for a file at
    /// `source_rate`, without taking the device. Diagnostics and machine tests.
    pub fn accepted_format(name: Option<&str>, source_rate: u32, channels: usize) -> Option<(u32, SampleKind)> {
        let _ = wasapi::initialize_mta();
        let en = DeviceEnumerator::new().ok()?;
        let dev = match name {
            Some(n) => en.get_device_collection(&Direction::Render).and_then(|c| c.get_device_with_name(n)),
            None => en.get_default_device(&Direction::Render),
        }
        .ok()?;
        let probe = dev.get_iaudioclient().ok()?;
        choose_format(source_rate, |r, k| probe.is_supported_exclusive_with_quirks(&wave(r, k, channels)).is_ok())
    }

    /// Take the device named `name` (or the default) for a file at `source_rate`. Once the format
    /// is agreed, send the renderer on the returned channel; the device starts when it arrives.
    pub fn open(name: Option<String>, source_rate: u32, channels: usize) -> Result<(ExclusiveOut, Opened, Sender<Render>), String> {
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = mpsc::channel::<Result<Opened, String>>();
        let (give_tx, give_rx) = mpsc::channel::<Render>();
        let thread_stop = stop.clone();
        let thread = std::thread::Builder::new()
            .name("audio-exclusive".into())
            .spawn(move || {
                let _ = wasapi::initialize_mta();
                let e = |x: wasapi::WasapiError| x.to_string();
                let setup = (|| {
                    let en = DeviceEnumerator::new().map_err(e)?;
                    let dev = match &name {
                        Some(n) => en.get_device_collection(&Direction::Render).and_then(|c| c.get_device_with_name(n)),
                        None => en.get_default_device(&Direction::Render),
                    }
                    .or_else(|_| en.get_default_device(&Direction::Render))
                    .map_err(e)?;
                    let probe = dev.get_iaudioclient().map_err(e)?;
                    let (rate, kind) = choose_format(source_rate, |r, k| probe.is_supported_exclusive_with_quirks(&wave(r, k, channels)).is_ok())
                        .ok_or("the device accepts no exclusive-mode format")?;
                    let format = probe.is_supported_exclusive_with_quirks(&wave(rate, kind, channels)).map_err(e)?;
                    let (default_period, _) = probe.get_device_period().map_err(e)?;
                    // 128-byte alignment keeps Intel HDA devices happy.
                    let period = probe.calculate_aligned_period_near(default_period, Some(128), &format).map_err(e)?;
                    let mode = |p: i64| StreamMode::PollingExclusive { period_hns: p, buffer_duration_hns: 8 * p };
                    let mut client = dev.get_iaudioclient().map_err(e)?;
                    let mut used = period;
                    if client.initialize_client(&format, &Direction::Render, &mode(period)).is_err() {
                        // Usually "buffer size not aligned": retry with the size the device proposes.
                        let frames = client.get_buffer_size().map_err(e)?;
                        used = calculate_period_100ns(frames as i64, rate as i64);
                        client = dev.get_iaudioclient().map_err(e)?;
                        client.initialize_client(&format, &Direction::Render, &mode(used)).map_err(e)?;
                    }
                    let render = client.get_audiorenderclient().map_err(e)?;
                    let opened = Opened { rate, kind, channels, name: dev.get_friendlyname().unwrap_or_default() };
                    Ok::<_, String>((client, render, opened, used))
                })();
                let (client, render_client, opened, period) = match setup {
                    Ok(v) => v,
                    Err(err) => {
                        let _ = ready_tx.send(Err(err));
                        return;
                    }
                };
                let kind = opened.kind;
                let _ = ready_tx.send(Ok(opened));
                let Ok(mut render) = give_rx.recv() else { return };
                let mut samples = Vec::new();
                let mut bytes = Vec::new();
                let mut fill = || -> Result<(), String> {
                    let frames = client.get_available_space_in_frames().map_err(e)? as usize;
                    if frames > 0 {
                        samples.clear();
                        samples.resize(frames * channels, 0.0);
                        render(&mut samples);
                        bytes.clear();
                        encode(&samples, kind, &mut bytes);
                        render_client.write_to_device(frames, &bytes, None).map_err(e)?;
                    }
                    Ok(())
                };
                let started = fill().and_then(|_| client.start_stream().map_err(e));
                if let Err(err) = started {
                    log::warn!("exclusive output did not start: {err}");
                    return;
                }
                let nap = Duration::from_micros((period as u64 / 10).clamp(2_000, 20_000));
                while !thread_stop.load(Ordering::Acquire) {
                    std::thread::sleep(nap);
                    if let Err(err) = fill() {
                        log::warn!("exclusive output stopped: {err}");
                        break;
                    }
                }
                let _ = client.stop_stream();
            })
            .map_err(|e| e.to_string())?;
        let out = ExclusiveOut { stop, thread: Some(thread) };
        match ready_rx.recv() {
            Ok(Ok(opened)) => Ok((out, opened, give_tx)),
            Ok(Err(err)) => Err(err),
            Err(_) => Err("exclusive output thread stopped".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a decoder produces for a `bits`-bit integer sample.
    fn decoded(v: i32, bits: u32) -> f32 {
        v as f32 / (1u64 << (bits - 1)) as f32
    }

    #[test]
    fn sixteen_bit_samples_survive_the_round_trip_exactly() {
        let src: Vec<i32> = vec![0, 1, -1, 12_345, -32_768, 32_767];
        let f: Vec<f32> = src.iter().map(|&v| decoded(v, 16)).collect();
        let mut out = Vec::new();
        encode(&f, SampleKind::I16, &mut out);
        let back: Vec<i32> = out.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as i32).collect();
        assert_eq!(back, src);
    }

    #[test]
    fn twenty_four_bit_samples_survive_in_every_container() {
        let src: Vec<i32> = vec![0, 1, -1, 4_000_001, -8_388_608, 8_388_607];
        let f: Vec<f32> = src.iter().map(|&v| decoded(v, 24)).collect();
        for kind in [SampleKind::I24, SampleKind::I24In32, SampleKind::I32] {
            let mut out = Vec::new();
            encode(&f, kind, &mut out);
            let back: Vec<i32> = match kind {
                SampleKind::I24 => out.chunks_exact(3).map(|b| i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8).collect(),
                _ => out.chunks_exact(4).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]) >> 8).collect(),
            };
            assert_eq!(back, src, "{kind:?}");
        }
    }

    #[test]
    fn full_scale_and_beyond_clip_instead_of_wrapping() {
        let mut out = Vec::new();
        encode(&[1.0, 1.5, -1.5], SampleKind::I16, &mut out);
        let back: Vec<i16> = out.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
        assert_eq!(back, [32_767, 32_767, -32_768]);
        let mut out = Vec::new();
        encode(&[1.0], SampleKind::I32, &mut out);
        assert_eq!(i32::from_le_bytes([out[0], out[1], out[2], out[3]]), i32::MAX);
    }

    #[test]
    fn sixteen_bit_containers_are_not_bit_perfect_for_hi_res() {
        assert!(SampleKind::I16.keeps(Some(16)));
        assert!(!SampleKind::I16.keeps(Some(24)));
        assert!(SampleKind::I24In32.keeps(Some(24)));
        assert!(SampleKind::I32.keeps(None), "lossy files have no bit depth to lose");
    }

    #[test]
    fn the_files_own_rate_is_used_when_the_device_takes_it() {
        let got = choose_format(96_000, |_, _| true);
        assert_eq!(got, Some((96_000, SampleKind::I32)));
    }

    #[test]
    fn the_widest_integer_format_the_device_takes_wins() {
        let got = choose_format(44_100, |_, k| matches!(k, SampleKind::I24 | SampleKind::I16));
        assert_eq!(got, Some((44_100, SampleKind::I24)));
    }

    #[test]
    fn an_unsupported_rate_falls_back_to_the_same_family() {
        // 88.2 kHz on a DAC that only does 44.1 / 48 / 96: 44.1 is an exact halving.
        let got = choose_format(88_200, |r, _| [44_100, 48_000, 96_000].contains(&r));
        assert_eq!(got.map(|g| g.0), Some(44_100));
        let got = choose_format(192_000, |r, _| [44_100, 48_000, 96_000].contains(&r));
        assert_eq!(got.map(|g| g.0), Some(96_000));
    }

    #[test]
    fn nothing_accepted_means_no_exclusive_mode() {
        assert_eq!(choose_format(44_100, |_, _| false), None);
    }
}
