//! Lists the exclusive-mode formats each output device accepts (diagnostics for F7.1).
fn main() {
    #[cfg(windows)]
    {
        use miu_music_lib::audio::exclusive::SampleKind;
        use wasapi::*;
        let _ = initialize_mta();
        let en = DeviceEnumerator::new().unwrap();
        for dev in &en.get_device_collection(&Direction::Render).unwrap() {
            let dev = dev.unwrap();
            let client = dev.get_iaudioclient().unwrap();
            println!("{}  (mix: {:?} Hz)", dev.get_friendlyname().unwrap_or_default(), client.get_mixformat().map(|f| f.get_samplespersec()));
            for rate in [44_100u32, 48_000, 88_200, 96_000, 176_400, 192_000] {
                let kinds: Vec<_> = SampleKind::ALL
                    .into_iter()
                    .filter(|k| {
                        let f = WaveFormat::new(k.store_bits() as usize, k.valid_bits() as usize, &SampleType::Int, rate as usize, 2, None);
                        client.is_supported_exclusive_with_quirks(&f).is_ok()
                    })
                    .collect();
                println!("  {rate:>6}: {kinds:?}");
            }
        }
    }
}
