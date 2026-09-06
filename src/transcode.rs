//! decode source audio to pcm and re-encode as flac or wav

use std::path::Path;

use anyhow::{Context, Result};
use flacenc::component::BitRepr;
use flacenc::error::Verify;

pub fn to_flac(src: &Path, dst: &Path) -> Result<()> {
    let (chs, rate, pcm) = decode(src)?;
    let samples: Vec<i32> = pcm.into_iter().map(i32::from).collect();
    let config = flacenc::config::Encoder::default()
        .into_verified()
        .map_err(|(_, e)| anyhow::anyhow!("flac config: {e}"))?;
    let source =
        flacenc::source::MemSource::from_samples(&samples, chs as usize, 16usize, rate as usize);
    let stream = flacenc::encode_with_fixed_block_size(&config, source, config.block_size)
        .map_err(|e| anyhow::anyhow!("flac encode: {e}"))?;
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream
        .write(&mut sink)
        .map_err(|e| anyhow::anyhow!("flac stream: {e}"))?;
    std::fs::write(dst, sink.as_slice()).context("write flac")
}

pub fn to_wav(src: &Path, dst: &Path) -> Result<()> {
    let (chs, rate, pcm) = decode(src)?;
    let mut data = Vec::with_capacity(pcm.len() * 2);
    for s in pcm {
        data.extend_from_slice(&s.to_le_bytes());
    }
    let block_align = chs as u32 * 2;
    let mut out = Vec::with_capacity(44 + data.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((36 + data.len()) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&chs.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * block_align).to_le_bytes());
    out.extend_from_slice(&(block_align as u16).to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    std::fs::write(dst, out).context("write wav")
}

fn decode(src: &Path) -> Result<(u16, u32, Vec<i16>)> {
    let file = std::fs::File::open(src).context("open source")?;
    let mss = symphonia::core::io::MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = symphonia::core::formats::probe::Hint::new();
    if let Some(e) = src.extension().and_then(|e| e.to_str()) {
        hint.with_extension(e);
    }
    let mut format = symphonia::default::get_probe()
        .probe(&hint, mss, Default::default(), Default::default())
        .context("probe source")?;
    let track = format
        .default_track(symphonia::core::formats::TrackType::Audio)
        .context("no audio track")?;
    let track_id = track.id;
    let params = track
        .codec_params
        .as_ref()
        .context("codec params")?
        .audio()
        .context("audio params")?;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(params, &Default::default())
        .context("audio decoder")?;
    let mut pcm: Vec<i16> = Vec::new();
    let mut rate = 0u32;
    let mut chs = 0u16;
    use symphonia::core::errors::Error;
    loop {
        let pkt = match format.next_packet() {
            Ok(Some(pkt)) => pkt,
            Ok(None) => break,
            Err(Error::ResetRequired) => break,
            Err(e) => return Err(e).context("read packet"),
        };
        if pkt.track_id != track_id {
            continue;
        }
        let buf = match decoder.decode(&pkt) {
            Ok(buf) => buf,
            Err(Error::IoError(_)) | Err(Error::DecodeError(_)) => continue,
            Err(e) => return Err(e).context("decode packet"),
        };
        let spec = buf.spec().clone();
        if rate == 0 {
            rate = spec.rate();
            chs = spec.channels().count() as u16;
        }
        let mut bytes: Vec<u8> = Vec::new();
        buf.copy_bytes_to_vec_interleaved_as::<i16>(&mut bytes);
        for c in bytes.as_chunks::<2>().0 {
            pcm.push(i16::from_le_bytes([c[0], c[1]]));
        }
    }
    if pcm.is_empty() {
        anyhow::bail!("no audio frames decoded from {:?}", src);
    }
    Ok((chs, rate, pcm))
}
