// Recording from a microphone or line input. The input stream writes into its
// own buffer (not the project, so it never waits for the project lock); the
// interface starts and stops the capture with playback.

use std::sync::{Arc, Mutex};

/// Longest take kept, in seconds.
const MAX_SECONDS: f32 = 1800.0;

#[derive(Default)]
pub struct Recorder {
    pub active: bool,
    pub rate: f32,
    pub frames: Vec<[f32; 2]>,
    /// Loudest recent level, for the input meter.
    pub peak: f32,
}

pub type SharedRecorder = Arc<Mutex<Recorder>>;

pub fn input_devices() -> Vec<String> {
    use cpal::traits::{DeviceTrait, HostTrait};
    let host = cpal::default_host();
    let Ok(devices) = host.input_devices() else { return Vec::new() };
    devices.filter_map(|d| d.name().ok()).collect()
}

fn push(rec: &SharedRecorder, data: impl Iterator<Item = f32>, chans: usize) {
    let Ok(mut r) = rec.lock() else { return };
    let max = (MAX_SECONDS * r.rate) as usize;
    let mut peak = r.peak * 0.9;
    let mut frame = [0.0f32; 2];
    let active = r.active;
    for (i, x) in data.enumerate() {
        let c = i % chans;
        if c < 2 {
            frame[c] = x;
        }
        peak = peak.max(x.abs());
        if c == chans - 1 && active && r.frames.len() < max {
            r.frames.push(if chans == 1 { [frame[0], frame[0]] } else { frame });
        }
    }
    r.peak = peak;
}

/// Opens an input (the named one, else the default).
pub fn start_input(rec: SharedRecorder, device: Option<&str>) -> Option<cpal::Stream> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let host = cpal::default_host();
    let named = device.and_then(|name| host.input_devices().ok()?.find(|d| d.name().ok().as_deref() == Some(name)));
    let device = named.or_else(|| host.default_input_device())?;
    let supported = device.default_input_config().ok()?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let chans = config.channels as usize;
    if let Ok(mut r) = rec.lock() {
        r.rate = config.sample_rate.0 as f32;
    }
    let err = |e| eprintln!("input error: {e}");
    let stream = match format {
        cpal::SampleFormat::F32 => {
            device.build_input_stream(&config, move |data: &[f32], _| push(&rec, data.iter().copied(), chans), err, None)
        }
        cpal::SampleFormat::I16 => device.build_input_stream(
            &config,
            move |data: &[i16], _| push(&rec, data.iter().map(|&x| x as f32 / i16::MAX as f32), chans),
            err,
            None,
        ),
        cpal::SampleFormat::I32 => device.build_input_stream(
            &config,
            move |data: &[i32], _| push(&rec, data.iter().map(|&x| x as f32 / i32::MAX as f32), chans),
            err,
            None,
        ),
        _ => return None,
    }
    .ok()?;
    stream.play().ok()?;
    Some(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_frames_only_while_active() {
        let rec: SharedRecorder = Arc::new(Mutex::new(Recorder { rate: 48000.0, ..Default::default() }));
        push(&rec, [0.1, 0.2, 0.3, 0.4].into_iter(), 2);
        assert!(rec.lock().unwrap().frames.is_empty());
        rec.lock().unwrap().active = true;
        push(&rec, [0.1, 0.2, 0.3, 0.4].into_iter(), 2);
        push(&rec, [0.5].into_iter(), 1);
        let r = rec.lock().unwrap();
        assert_eq!(r.frames, vec![[0.1, 0.2], [0.3, 0.4], [0.5, 0.5]]);
        assert!(r.peak > 0.4);
    }
}
