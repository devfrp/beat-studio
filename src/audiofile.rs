// Reading sound files (WAV, MP3, FLAC, OGG) for sampler channels, and MP3
// export through the system's LAME library (libmp3lame.so.0), loaded when
// needed so the studio still starts without it.

use std::ffi::{c_int, c_void};
use std::fs::File;
use std::path::Path;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::engine::SampleData;

/// File endings offered when picking a sound.
pub const AUDIO_EXTENSIONS: [&str; 6] = ["wav", "mp3", "flac", "ogg", "aif", "aiff"];
/// Longest sound a sampler channel loads, so a whole album cannot fill the memory.
const MAX_SECONDS: f32 = 600.0;

pub fn is_audio(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| AUDIO_EXTENSIONS.contains(&e.to_lowercase().as_str()))
}

/// Decodes a whole file to stereo frames at its own rate.
pub fn decode(path: &Path) -> Result<SampleData, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|_| "this sound format is not supported".to_string())?;
    let mut format = probed.format;
    let track = format.default_track().ok_or("the file has no sound")?;
    let track_id = track.id;
    let rate = track.codec_params.sample_rate.ok_or("unknown sample rate")? as f32;
    let mut decoder =
        symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default()).map_err(|_| "this sound format is not supported")?;
    let mut frames: Vec<[f32; 2]> = Vec::new();
    let max = (MAX_SECONDS * rate) as usize;
    while let Ok(packet) = format.next_packet() {
        if packet.track_id() != track_id {
            continue;
        }
        let Ok(decoded) = decoder.decode(&packet) else { continue };
        let spec = *decoded.spec();
        let chans = spec.channels.count().max(1);
        let mut buf = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        buf.copy_interleaved_ref(decoded);
        for f in buf.samples().chunks(chans) {
            frames.push(if chans == 1 { [f[0], f[0]] } else { [f[0], f[1]] });
        }
        if frames.len() >= max {
            frames.truncate(max);
            break;
        }
    }
    if frames.is_empty() {
        return Err("the file has no sound".into());
    }
    Ok(SampleData { rate, frames })
}

// ---------------------------------------------------------------- MP3 export

extern "C" {
    fn dlopen(filename: *const std::ffi::c_char, flag: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const std::ffi::c_char) -> *mut c_void;
}

type LameInit = unsafe extern "C" fn() -> *mut c_void;
type LameSetInt = unsafe extern "C" fn(*mut c_void, c_int) -> c_int;
type LameInitParams = unsafe extern "C" fn(*mut c_void) -> c_int;
type LameEncode = unsafe extern "C" fn(*mut c_void, *const f32, c_int, *mut u8, c_int) -> c_int;
type LameFlush = unsafe extern "C" fn(*mut c_void, *mut u8, c_int) -> c_int;
type LameClose = unsafe extern "C" fn(*mut c_void) -> c_int;

pub const MP3_KBPS: i32 = 192;

/// True when the LAME library is installed.
pub fn mp3_available() -> bool {
    unsafe { !dlopen(c"libmp3lame.so.0".as_ptr(), 2).is_null() }
}

/// Encodes interleaved stereo samples to MP3 bytes.
pub fn mp3_bytes(samples: &[f32], rate: u32) -> Result<Vec<u8>, String> {
    unsafe {
        let lib = dlopen(c"libmp3lame.so.0".as_ptr(), 2);
        if lib.is_null() {
            return Err("MP3 export needs the LAME library (package \"lame\")".into());
        }
        let sym = |name: &std::ffi::CStr| {
            let p = dlsym(lib, name.as_ptr());
            if p.is_null() { Err(format!("LAME is missing {}", name.to_string_lossy())) } else { Ok(p) }
        };
        let init: LameInit = std::mem::transmute(sym(c"lame_init")?);
        let set_rate: LameSetInt = std::mem::transmute(sym(c"lame_set_in_samplerate")?);
        let set_chans: LameSetInt = std::mem::transmute(sym(c"lame_set_num_channels")?);
        let set_brate: LameSetInt = std::mem::transmute(sym(c"lame_set_brate")?);
        let set_quality: LameSetInt = std::mem::transmute(sym(c"lame_set_quality")?);
        let init_params: LameInitParams = std::mem::transmute(sym(c"lame_init_params")?);
        let encode: LameEncode = std::mem::transmute(sym(c"lame_encode_buffer_interleaved_ieee_float")?);
        let flush: LameFlush = std::mem::transmute(sym(c"lame_encode_flush")?);
        let close: LameClose = std::mem::transmute(sym(c"lame_close")?);

        let gf = init();
        if gf.is_null() {
            return Err("LAME could not start".into());
        }
        set_rate(gf, rate as c_int);
        set_chans(gf, 2);
        set_brate(gf, MP3_KBPS);
        set_quality(gf, 2);
        if init_params(gf) < 0 {
            close(gf);
            return Err("LAME refused the settings".into());
        }
        let mut out = Vec::new();
        let clipped: Vec<f32> = samples.iter().map(|x| x.clamp(-1.0, 1.0)).collect();
        for chunk in clipped.chunks(8192 * 2) {
            let frames = chunk.len() / 2;
            let mut buf = vec![0u8; frames * 5 / 4 + 7200];
            let n = encode(gf, chunk.as_ptr(), frames as c_int, buf.as_mut_ptr(), buf.len() as c_int);
            if n < 0 {
                close(gf);
                return Err(format!("MP3 encoding failed ({n})"));
            }
            out.extend_from_slice(&buf[..n as usize]);
        }
        let mut buf = vec![0u8; 7200];
        let n = flush(gf, buf.as_mut_ptr(), buf.len() as c_int);
        if n > 0 {
            out.extend_from_slice(&buf[..n as usize]);
        }
        close(gf);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file::wav_bytes;

    #[test]
    fn decodes_wav_and_encodes_mp3() {
        let dir = std::env::temp_dir().join(format!("beatstudio-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let samples: Vec<f32> = (0..44100 * 2).map(|i| ((i / 2) as f32 * 0.03).sin() * 0.5).collect();
        let path = dir.join("tone.wav");
        std::fs::write(&path, wav_bytes(&samples, 44100)).unwrap();
        let s = decode(&path).unwrap();
        assert_eq!(s.rate, 44100.0);
        assert_eq!(s.frames.len(), 44100);
        assert!((s.frames[100][0] - samples[200]).abs() < 1e-3);
        assert!(is_audio(&path) && !is_audio(Path::new("a.mid")));
        if mp3_available() {
            let mp3 = mp3_bytes(&samples, 44100).unwrap();
            assert!(mp3.len() > 1000);
            let mp3_path = dir.join("tone.mp3");
            std::fs::write(&mp3_path, &mp3).unwrap();
            let back = decode(&mp3_path).unwrap();
            assert!(back.frames.len() > 40000);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
