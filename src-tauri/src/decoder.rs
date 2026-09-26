use std::fs::File;
use std::time::Duration;
use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::{Decoder, DecoderOptions};
use symphonia::core::conv::FromSample;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use rodio::Source;
use symphonia::core::units::Time;


pub struct SymphoniaSource {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    sample_rate: u32,
    channels: u16,
    total_duration: Option<Duration>,

    // 当前解码出来的交错 PCM（f32）
    buffer: Vec<f32>,
    buffer_pos: usize,
}

fn append_buffer(buffer: &mut Vec<f32>, buf: AudioBufferRef) {
    let channels = buf.spec().channels.count();
    let frames = buf.frames();

    buffer.clear();
    buffer.reserve(frames * channels);

    macro_rules! push_interleaved {
        ($b:expr) => {
            for frame in 0..frames {
                for ch in 0..channels {
                    buffer.push(f32::from_sample($b.chan(ch)[frame]));
                }
            }
        };
    }

    match buf {
        AudioBufferRef::U8(b) => push_interleaved!(b),
        AudioBufferRef::U16(b) => push_interleaved!(b),
        AudioBufferRef::U24(b) => push_interleaved!(b),
        AudioBufferRef::U32(b) => push_interleaved!(b),
        AudioBufferRef::S8(b) => push_interleaved!(b),
        AudioBufferRef::S16(b) => push_interleaved!(b),
        AudioBufferRef::S24(b) => push_interleaved!(b),
        AudioBufferRef::S32(b) => push_interleaved!(b),
        AudioBufferRef::F32(b) => push_interleaved!(b),
        AudioBufferRef::F64(b) => push_interleaved!(b),
    }
}

impl SymphoniaSource {
    pub fn open(path: &str) -> Result<Self, String> {
        let file = File::open(path).map_err(|e| format!("打开文件失败: {}", e))?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());

        let mut hint = Hint::new();
        if let Some(ext) = std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
        {
            hint.with_extension(ext);
        }

        let probed = symphonia::default::get_probe()
            .format(
                &hint,
                mss,
                &FormatOptions::default(),
                &MetadataOptions::default(),
            )
            .map_err(|e| format!("无法识别格式: {}", e))?;

        let format = probed.format;

        let track = format
            .default_track()
            .ok_or_else(|| "找不到音频轨道".to_string())?;
        let track_id = track.id;

        let decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .map_err(|e| format!("创建解码器失败: {}", e))?;

        let sample_rate = track.codec_params.sample_rate.unwrap_or(44100);
        let channels = track
            .codec_params
            .channels
            .map(|c| c.count() as u16)
            .unwrap_or(2);

        let total_duration = track
            .codec_params
            .n_frames
            .map(|n| Duration::from_secs_f64(n as f64 / sample_rate as f64));

        Ok(Self {
            format,
            decoder,
            track_id,
            sample_rate,
            channels,
            total_duration,
            buffer: Vec::new(),
            buffer_pos: 0,
        })
    }

    /// 解码下一帧，把交错 PCM 追加进 buffer。成功返回 true。
    fn decode_next_packet(&mut self) -> bool {
        loop {
            let packet = match self.format.next_packet() {
                Ok(p) => p,
                Err(symphonia::core::errors::Error::IoError(ref e))
                    if e.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    return false; // 正常结束
                }
                Err(e) => {
                    eprintln!("next_packet 错误: {}", e);
                    return false;
                }
            };

            if packet.track_id() != self.track_id {
                continue;
            }

            match self.decoder.decode(&packet) {
                Ok(audio_buf) => {
                    append_buffer(&mut self.buffer, audio_buf);
                    self.buffer_pos = 0;
                    return true;
                }
                Err(symphonia::core::errors::Error::DecodeError(_)) => {
                    // 单帧错误，跳过继续
                    continue;
                }
                Err(e) => {
                    eprintln!("解码错误: {}", e);
                    return false;
                }
            }
        }
    }

    
}

impl Iterator for SymphoniaSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.buffer_pos < self.buffer.len() {
            let s = self.buffer[self.buffer_pos];
            self.buffer_pos += 1;
            return Some(s);
        }

        if self.decode_next_packet() && self.buffer_pos < self.buffer.len() {
            let s = self.buffer[self.buffer_pos];
            self.buffer_pos += 1;
            return Some(s);
        }

        None
    }
}

impl Source for SymphoniaSource {
    fn current_frame_len(&self) -> Option<usize> {
        Some(self.buffer.len() - self.buffer_pos)
    }

    fn channels(&self) -> u16 {
        self.channels
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        self.total_duration
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), rodio::source::SeekError> {
        let total = pos.as_secs_f64();
        let time = Time {
            seconds: total.floor() as u64,
            frac: total.fract(),
        };
    
        self.format
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time,
                    track_id: Some(self.track_id),
                },
            )
            .map_err(|e| {
                eprintln!("symphonia seek 失败: {}", e);
                rodio::source::SeekError::NotSupported {
                    underlying_source: "SymphoniaSource",
                }
            })?;
    
        self.decoder.reset();
        self.buffer.clear();
        self.buffer_pos = 0;
    
        Ok(())
    }
}