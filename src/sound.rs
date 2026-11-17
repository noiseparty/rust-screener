//! Alert sound: the embedded WAV, scaled to the volume setting and played asynchronously.

#[cfg_attr(not(windows), allow(dead_code))]
static ALERT: &[u8] = include_bytes!("../assets/alert.wav");

/// Copy of `wav` with every 16-bit PCM sample multiplied by `volume / 100` (clamped to i16).
/// `None` unless it is a RIFF/WAVE with a PCM 16-bit `fmt ` chunk and a `data` chunk.
#[cfg_attr(not(windows), allow(dead_code))]
fn scaled_wav(wav: &[u8], volume: u8) -> Option<Vec<u8>> {
    if wav.len() < 12 || &wav[0..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        return None;
    }
    let (mut pcm16, mut data) = (false, None);
    let mut pos = 12;
    while pos + 8 <= wav.len() {
        let id = &wav[pos..pos + 4];
        let size = u32::from_le_bytes(wav[pos + 4..pos + 8].try_into().ok()?) as usize;
        let start = pos + 8;
        let end = start.checked_add(size).filter(|&e| e <= wav.len())?;
        match id {
            b"fmt " if size >= 16 => {
                let format = u16::from_le_bytes([wav[start], wav[start + 1]]);
                let bits = u16::from_le_bytes([wav[start + 14], wav[start + 15]]);
                pcm16 = format == 1 && bits == 16;
            }
            b"data" => data = Some(start..end),
            _ => {}
        }
        // Chunks are word-aligned: an odd size is followed by one pad byte.
        pos = end + (size & 1);
    }
    let data = data.filter(|_| pcm16)?;
    let mut out = wav.to_vec();
    for s in out[data].chunks_exact_mut(2) {
        let v = i16::from_le_bytes([s[0], s[1]]) as i32 * volume as i32 / 100;
        s.copy_from_slice(&(v.clamp(i16::MIN as i32, i16::MAX as i32) as i16).to_le_bytes());
    }
    Some(out)
}

/// The buffer the current async sound plays from; PlaySoundW reads it while playing.
#[cfg(windows)]
static PLAYING: std::sync::Mutex<Option<Vec<u8>>> = std::sync::Mutex::new(None);

/// Plays the alert at `volume` percent without blocking. No-op when muted or at volume 0.
/// The previous sound is stopped before its buffer is replaced, so PlaySoundW never reads freed memory.
#[cfg(windows)]
pub fn play_alert(volume: u8, muted: bool) {
    use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
    if muted || volume == 0 {
        return;
    }
    let Some(wav) = scaled_wav(ALERT, volume) else { return };
    let mut slot = PLAYING.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: a null sound with no flags stops this process's current sound; nothing is read.
    unsafe { PlaySoundW(std::ptr::null(), std::ptr::null_mut(), 0) };
    let buf = slot.insert(wav);
    // SAFETY: with SND_MEMORY the pointer is a complete WAV image; it lives in PLAYING until the
    // next call stops this sound (above) before dropping it.
    unsafe {
        PlaySoundW(buf.as_ptr().cast(), std::ptr::null_mut(), SND_MEMORY | SND_ASYNC | SND_NODEFAULT)
    };
}

#[cfg(not(windows))]
pub fn play_alert(_volume: u8, _muted: bool) {}

#[cfg(test)]
mod tests {
    use super::scaled_wav;

    fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut c = id.to_vec();
        c.extend_from_slice(&(body.len() as u32).to_le_bytes());
        c.extend_from_slice(body);
        if body.len() % 2 == 1 {
            c.push(0);
        }
        c
    }

    /// Mono 8000 Hz WAV with an odd-sized LIST chunk before `data`.
    fn wav(format: u16, bits: u16, samples: &[i16]) -> Vec<u8> {
        let mut fmt = Vec::new();
        fmt.extend_from_slice(&format.to_le_bytes());
        fmt.extend_from_slice(&1u16.to_le_bytes());
        fmt.extend_from_slice(&8000u32.to_le_bytes());
        fmt.extend_from_slice(&16000u32.to_le_bytes());
        fmt.extend_from_slice(&2u16.to_le_bytes());
        fmt.extend_from_slice(&bits.to_le_bytes());
        let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut body = b"WAVE".to_vec();
        body.extend(chunk(b"fmt ", &fmt));
        body.extend(chunk(b"LIST", b"odd"));
        body.extend(chunk(b"data", &data));
        chunk(b"RIFF", &body)
    }

    fn samples(w: &[u8]) -> Vec<i16> {
        let d = w.len() - 8; // data is the last chunk: 4 samples * 2 bytes
        w[d..].chunks_exact(2).map(|s| i16::from_le_bytes([s[0], s[1]])).collect()
    }

    #[test]
    fn scales_samples() {
        let w = wav(1, 16, &[1000, -1000, 32767, -32768]);
        assert_eq!(samples(&scaled_wav(&w, 50).unwrap()), [500, -500, 16383, -16384]);
        assert_eq!(samples(&scaled_wav(&w, 0).unwrap()), [0, 0, 0, 0]);
        assert_eq!(scaled_wav(&w, 100).unwrap(), w);
        assert_eq!(samples(&scaled_wav(&w, 200).unwrap()), [2000, -2000, 32767, -32768]);
    }

    #[test]
    fn rejects_non_pcm16_and_garbage() {
        assert!(scaled_wav(&wav(3, 16, &[1, 2, 3, 4]), 50).is_none());
        assert!(scaled_wav(&wav(1, 8, &[1, 2, 3, 4]), 50).is_none());
        assert!(scaled_wav(b"not a wav file at all", 50).is_none());
        assert!(scaled_wav(&[], 50).is_none());
        let mut truncated = wav(1, 16, &[1, 2, 3, 4]);
        truncated.truncate(truncated.len() - 3);
        assert!(scaled_wav(&truncated, 50).is_none());
    }

    #[test]
    fn embedded_alert_parses() {
        assert!(scaled_wav(super::ALERT, 50).is_some());
    }
}
