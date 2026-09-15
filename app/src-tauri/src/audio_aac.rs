//! AAC encoding for exported clips. MP4 can't carry the cameras' G.711 audio, so exports
//! re-encode it: PCM is upsampled to 48 kHz and encoded to AAC-LC with the operating
//! system's encoder (Media Foundation on Windows), which avoids bundling an encoder and
//! its licensing questions. Other platforms export video only for now.

/// Sample rate of the encoded AAC.
pub const OUTPUT_RATE: u32 = 48_000;
/// AAC-LC frames carry 1024 samples.
pub const FRAME_SAMPLES: u32 = 1024;

/// One encoded AAC access unit (raw, no ADTS header).
#[derive(Debug, Clone)]
pub struct AacFrame {
    pub data: Vec<u8>,
}

/// Upsamples mono PCM by an integer factor with a windowed-sinc low-pass FIR, keeping
/// filter state between calls so chunk boundaries are seamless.
pub struct Upsampler {
    factor: usize,
    taps: Vec<f32>,
    history: Vec<f32>,
}

impl Upsampler {
    /// `None` when `input_rate` doesn't divide 48 kHz.
    pub fn new(input_rate: u32) -> Option<Self> {
        if input_rate == 0 || !OUTPUT_RATE.is_multiple_of(input_rate) {
            return None;
        }
        let factor = (OUTPUT_RATE / input_rate) as usize;
        let taps_per_phase = 16;
        let len = factor * taps_per_phase;
        // Cut off at 90% of the input Nyquist frequency, relative to the output rate.
        let cutoff = 0.45 / factor as f64;
        let center = (len - 1) as f64 / 2.0;
        let taps: Vec<f32> = (0..len)
            .map(|i| {
                let x = i as f64 - center;
                let sinc = if x == 0.0 {
                    2.0 * cutoff
                } else {
                    (2.0 * std::f64::consts::PI * cutoff * x).sin() / (std::f64::consts::PI * x)
                };
                let window = 0.42
                    - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / (len - 1) as f64).cos()
                    + 0.08 * (4.0 * std::f64::consts::PI * i as f64 / (len - 1) as f64).cos();
                (sinc * window * factor as f64) as f32
            })
            .collect();
        Some(Self {
            factor,
            history: vec![0.0; taps_per_phase],
            taps,
        })
    }

    #[cfg(test)]
    pub fn factor(&self) -> usize {
        self.factor
    }

    pub fn process(&mut self, input: &[i16], out: &mut Vec<i16>) {
        let taps_per_phase = self.history.len();
        for &sample in input {
            self.history.rotate_left(1);
            *self.history.last_mut().expect("history") = f32::from(sample);
            for phase in 0..self.factor {
                // Output sample `phase` uses every `factor`-th tap starting at `phase`.
                let mut acc = 0.0f32;
                for (k, &h) in self.history.iter().rev().enumerate() {
                    let tap = phase + k * self.factor;
                    if tap < self.taps.len() {
                        acc += h * self.taps[tap];
                    }
                }
                debug_assert!(taps_per_phase * self.factor == self.taps.len());
                out.push(acc.round().clamp(i16::MIN as f32, i16::MAX as f32) as i16);
            }
        }
    }
}

#[cfg(windows)]
pub use windows_mf::AacEncoder;

#[cfg(not(windows))]
pub struct AacEncoder;

#[cfg(not(windows))]
impl AacEncoder {
    pub fn new() -> Result<Self, String> {
        Err("AAC encoding is only available on Windows for now".into())
    }
    pub fn audio_specific_config(&self) -> &[u8] {
        &[]
    }
    pub fn encode(&mut self, _pcm_48k: &[i16], _out: &mut Vec<AacFrame>) -> Result<(), String> {
        Ok(())
    }
    pub fn finish(&mut self, _out: &mut Vec<AacFrame>) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(windows)]
mod windows_mf {
    use std::mem::ManuallyDrop;

    use windows::Win32::Media::MediaFoundation::*;
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
    use windows::core::Interface;

    use super::{AacFrame, OUTPUT_RATE};

    /// AAC-LC encoder backed by the Media Foundation transform that ships with Windows.
    /// Input: mono signed 16-bit PCM at 48 kHz.
    pub struct AacEncoder {
        transform: IMFTransform,
        config: Vec<u8>,
        output_size: u32,
        samples_in: u64,
    }

    // The transform is only used from the thread that owns the encoder.
    unsafe impl Send for AacEncoder {}

    fn err(context: &str, e: windows::core::Error) -> String {
        format!("{context}: {e}")
    }

    impl AacEncoder {
        pub fn new() -> Result<Self, String> {
            // SAFETY: plain Media Foundation / COM calls with valid arguments; every
            // returned interface is owned by the `windows` crate wrappers.
            unsafe {
                // Ignore "already initialized with another mode": MF still works.
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET).map_err(|e| err("MFStartup", e))?;

                let input = MFT_REGISTER_TYPE_INFO {
                    guidMajorType: MFMediaType_Audio,
                    guidSubtype: MFAudioFormat_PCM,
                };
                let output = MFT_REGISTER_TYPE_INFO {
                    guidMajorType: MFMediaType_Audio,
                    guidSubtype: MFAudioFormat_AAC,
                };
                let mut activates: *mut Option<IMFActivate> = std::ptr::null_mut();
                let mut count = 0u32;
                MFTEnumEx(
                    MFT_CATEGORY_AUDIO_ENCODER,
                    MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_SORTANDFILTER,
                    Some(&input),
                    Some(&output),
                    &mut activates,
                    &mut count,
                )
                .map_err(|e| err("MFTEnumEx", e))?;
                if count == 0 || activates.is_null() {
                    return Err(
                        "no AAC encoder is installed (Windows N without the Media Feature Pack?)"
                            .into(),
                    );
                }
                let list = std::slice::from_raw_parts_mut(activates, count as usize);
                let activate = list[0].take().ok_or("empty encoder entry")?;
                for entry in list.iter_mut() {
                    drop(entry.take());
                }
                windows::Win32::System::Com::CoTaskMemFree(Some(activates as *const _));
                let transform: IMFTransform = activate
                    .ActivateObject()
                    .map_err(|e| err("ActivateObject", e))?;

                let out_type = MFCreateMediaType().map_err(|e| err("MFCreateMediaType", e))?;
                out_type
                    .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)
                    .map_err(|e| err("type", e))?;
                out_type
                    .SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC)
                    .map_err(|e| err("type", e))?;
                out_type
                    .SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)
                    .map_err(|e| err("type", e))?;
                out_type
                    .SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, OUTPUT_RATE)
                    .map_err(|e| err("type", e))?;
                out_type
                    .SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 1)
                    .map_err(|e| err("type", e))?;
                // 96 kbit/s, the lowest bitrate the encoder offers; plenty for voice.
                out_type
                    .SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, 12_000)
                    .map_err(|e| err("type", e))?;
                out_type
                    .SetUINT32(&MF_MT_AAC_PAYLOAD_TYPE, 0)
                    .map_err(|e| err("type", e))?;
                transform
                    .SetOutputType(0, &out_type, 0)
                    .map_err(|e| err("SetOutputType", e))?;

                let in_type = MFCreateMediaType().map_err(|e| err("MFCreateMediaType", e))?;
                in_type
                    .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)
                    .map_err(|e| err("type", e))?;
                in_type
                    .SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)
                    .map_err(|e| err("type", e))?;
                in_type
                    .SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)
                    .map_err(|e| err("type", e))?;
                in_type
                    .SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, OUTPUT_RATE)
                    .map_err(|e| err("type", e))?;
                in_type
                    .SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 1)
                    .map_err(|e| err("type", e))?;
                in_type
                    .SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, 2)
                    .map_err(|e| err("type", e))?;
                in_type
                    .SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, OUTPUT_RATE * 2)
                    .map_err(|e| err("type", e))?;
                transform
                    .SetInputType(0, &in_type, 0)
                    .map_err(|e| err("SetInputType", e))?;

                // MF_MT_USER_DATA holds HEAACWAVEINFO's extra fields (12 bytes) followed
                // by the AudioSpecificConfig that MP4's esds box needs.
                let current = transform
                    .GetOutputCurrentType(0)
                    .map_err(|e| err("GetOutputCurrentType", e))?;
                let size = current
                    .GetBlobSize(&MF_MT_USER_DATA)
                    .map_err(|e| err("user data", e))?;
                let mut blob = vec![0u8; size as usize];
                current
                    .GetBlob(&MF_MT_USER_DATA, &mut blob, None)
                    .map_err(|e| err("user data", e))?;
                let config = blob.get(12..).map(<[u8]>::to_vec).unwrap_or_default();
                if config.is_empty() {
                    return Err("encoder returned no AudioSpecificConfig".into());
                }

                let info = transform
                    .GetOutputStreamInfo(0)
                    .map_err(|e| err("GetOutputStreamInfo", e))?;
                transform
                    .ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)
                    .map_err(|e| err("begin streaming", e))?;
                transform
                    .ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)
                    .map_err(|e| err("start of stream", e))?;

                Ok(Self {
                    transform,
                    config,
                    output_size: info.cbSize.max(8192),
                    samples_in: 0,
                })
            }
        }

        /// The AudioSpecificConfig for the MP4 `esds` box.
        pub fn audio_specific_config(&self) -> &[u8] {
            &self.config
        }

        /// Feeds 48 kHz mono PCM; encoded frames are appended to `out`.
        pub fn encode(&mut self, pcm: &[i16], out: &mut Vec<AacFrame>) -> Result<(), String> {
            if pcm.is_empty() {
                return Ok(());
            }
            // SAFETY: the buffer is locked for exactly its allocated length.
            unsafe {
                let bytes = (pcm.len() * 2) as u32;
                let buffer =
                    MFCreateMemoryBuffer(bytes).map_err(|e| err("MFCreateMemoryBuffer", e))?;
                let mut ptr = std::ptr::null_mut();
                buffer
                    .Lock(&mut ptr, None, None)
                    .map_err(|e| err("Lock", e))?;
                std::ptr::copy_nonoverlapping(pcm.as_ptr().cast::<u8>(), ptr, bytes as usize);
                buffer.Unlock().map_err(|e| err("Unlock", e))?;
                buffer
                    .SetCurrentLength(bytes)
                    .map_err(|e| err("SetCurrentLength", e))?;

                let sample = MFCreateSample().map_err(|e| err("MFCreateSample", e))?;
                sample.AddBuffer(&buffer).map_err(|e| err("AddBuffer", e))?;
                // Media Foundation time is in 100 ns units.
                let time = (self.samples_in as i64 * 10_000_000) / i64::from(OUTPUT_RATE);
                let duration = (pcm.len() as i64 * 10_000_000) / i64::from(OUTPUT_RATE);
                sample
                    .SetSampleTime(time)
                    .map_err(|e| err("SetSampleTime", e))?;
                sample
                    .SetSampleDuration(duration)
                    .map_err(|e| err("SetSampleDuration", e))?;
                self.samples_in += pcm.len() as u64;

                loop {
                    match self.transform.ProcessInput(0, &sample, 0) {
                        Ok(()) => break,
                        // The encoder wants its pending output collected first.
                        Err(e) if e.code() == MF_E_NOTACCEPTING => self.drain_output(out)?,
                        Err(e) => return Err(err("ProcessInput", e)),
                    }
                }
            }
            self.drain_output(out)
        }

        /// Flushes the encoder's buffered audio.
        pub fn finish(&mut self, out: &mut Vec<AacFrame>) -> Result<(), String> {
            // SAFETY: plain message call.
            unsafe {
                self.transform
                    .ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0)
                    .map_err(|e| err("end of stream", e))?;
                self.transform
                    .ProcessMessage(MFT_MESSAGE_COMMAND_DRAIN, 0)
                    .map_err(|e| err("drain", e))?;
            }
            self.drain_output(out)
        }

        fn drain_output(&mut self, out: &mut Vec<AacFrame>) -> Result<(), String> {
            loop {
                // SAFETY: we allocate the output sample, as the MS AAC encoder expects,
                // and release it through ManuallyDrop::into_inner below.
                unsafe {
                    let buffer = MFCreateMemoryBuffer(self.output_size)
                        .map_err(|e| err("MFCreateMemoryBuffer", e))?;
                    let sample = MFCreateSample().map_err(|e| err("MFCreateSample", e))?;
                    sample.AddBuffer(&buffer).map_err(|e| err("AddBuffer", e))?;
                    let mut data = [MFT_OUTPUT_DATA_BUFFER {
                        dwStreamID: 0,
                        pSample: ManuallyDrop::new(Some(sample.clone())),
                        dwStatus: 0,
                        pEvents: ManuallyDrop::new(None),
                    }];
                    let mut status = 0u32;
                    let result = self.transform.ProcessOutput(0, &mut data, &mut status);
                    let [entry] = data;
                    drop(ManuallyDrop::into_inner(entry.pSample));
                    drop(ManuallyDrop::into_inner(entry.pEvents));
                    match result {
                        Ok(()) => {
                            let contiguous = sample
                                .ConvertToContiguousBuffer()
                                .map_err(|e| err("ConvertToContiguousBuffer", e))?;
                            let mut ptr = std::ptr::null_mut();
                            let mut len = 0u32;
                            contiguous
                                .Lock(&mut ptr, None, Some(&mut len))
                                .map_err(|e| err("Lock", e))?;
                            let bytes = std::slice::from_raw_parts(ptr, len as usize).to_vec();
                            contiguous.Unlock().map_err(|e| err("Unlock", e))?;
                            if !bytes.is_empty() {
                                out.push(AacFrame { data: bytes });
                            }
                        }
                        Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(()),
                        Err(e) => return Err(err("ProcessOutput", e)),
                    }
                }
            }
        }
    }

    impl Drop for AacEncoder {
        fn drop(&mut self) {
            // SAFETY: balanced with MFStartup in `new`.
            unsafe {
                let _ = self.transform.cast::<IMFTransform>();
                let _ = MFShutdown();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: u32, freq: f64, seconds: f64) -> Vec<i16> {
        (0..(rate as f64 * seconds) as usize)
            .map(|i| {
                ((i as f64 * freq * 2.0 * std::f64::consts::PI / rate as f64).sin() * 12_000.0)
                    as i16
            })
            .collect()
    }

    #[test]
    fn upsampler_keeps_the_tone_and_length() {
        let mut up = Upsampler::new(8_000).unwrap();
        assert_eq!(up.factor(), 6);
        let input = sine(8_000, 440.0, 0.5);
        let mut out = Vec::new();
        // Chunked processing must match one-shot processing.
        for chunk in input.chunks(97) {
            up.process(chunk, &mut out);
        }
        assert_eq!(out.len(), input.len() * 6);
        let mut up2 = Upsampler::new(8_000).unwrap();
        let mut once = Vec::new();
        up2.process(&input, &mut once);
        assert_eq!(out, once);

        // Past the filter's start-up, the amplitude is preserved (~12000).
        let peak = out[2000..].iter().map(|s| s.unsigned_abs()).max().unwrap();
        assert!((10_500..13_500).contains(&peak), "peak {peak}");
        assert!(Upsampler::new(11_025).is_none());
    }

    #[cfg(windows)]
    #[test]
    fn media_foundation_encodes_aac() {
        let mut encoder = match AacEncoder::new() {
            Ok(encoder) => encoder,
            // Windows N editions without the Media Feature Pack have no AAC encoder.
            Err(e) if e.contains("no AAC encoder") => return,
            Err(e) => panic!("{e}"),
        };
        let config = encoder.audio_specific_config().to_vec();
        // AAC-LC (object type 2), 48 kHz (index 3), mono: 0x11 0x88.
        assert_eq!(&config[..2], &[0x11, 0x88]);

        let pcm = sine(48_000, 440.0, 1.0);
        let mut frames = Vec::new();
        for chunk in pcm.chunks(4800) {
            encoder.encode(chunk, &mut frames).unwrap();
        }
        encoder.finish(&mut frames).unwrap();
        // One second at 48 kHz is ~47 frames of 1024 samples.
        assert!((40..=50).contains(&frames.len()), "{} frames", frames.len());
        assert!(
            frames
                .iter()
                .all(|f| !f.data.is_empty() && f.data.len() < 2048)
        );

        // If ffprobe is around, check that a decoder accepts the stream (as ADTS).
        let path = std::env::temp_dir().join("backsight-aac-test.aac");
        let mut adts = Vec::new();
        for frame in &frames {
            let len = frame.data.len() + 7;
            // AAC-LC, 48 kHz (index 3), mono.
            adts.extend_from_slice(&[
                0xFF,
                0xF1,
                (1 << 6) | (3 << 2),
                (1 << 6) | ((len >> 11) as u8 & 0x03),
                (len >> 3) as u8,
                ((len as u8 & 0x07) << 5) | 0x1F,
                0xFC,
            ]);
            adts.extend_from_slice(&frame.data);
        }
        std::fs::write(&path, &adts).unwrap();
        if let Ok(output) = std::process::Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "stream=codec_name,sample_rate,channels",
                "-of",
                "csv=p=0",
            ])
            .arg(&path)
            .output()
        {
            let text = String::from_utf8_lossy(&output.stdout);
            assert!(text.contains("aac,48000,1"), "ffprobe said {text:?}");
        }
        let _ = std::fs::remove_file(&path);
    }
}
