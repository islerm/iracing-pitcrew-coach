//! Plays a WAV out loud on this PC.

use anyhow::Result;

#[cfg(windows)]
#[link(name = "winmm")]
extern "system" {
    fn PlaySoundW(sound: *const u16, module: *const std::ffi::c_void, flags: u32) -> i32;
}

/// Plays WAV bytes and blocks until they finish.
#[cfg(windows)]
pub fn play_wav(bytes: &[u8]) -> Result<()> {
    const SND_SYNC: u32 = 0x0;
    const SND_NODEFAULT: u32 = 0x2;
    const SND_MEMORY: u32 = 0x4;
    // SAFETY: with SND_MEMORY the pointer is read as an in-memory WAV image; `bytes` stays
    // alive for the whole call because SND_SYNC doesn't return until playback has ended.
    let ok = unsafe { PlaySoundW(bytes.as_ptr() as *const u16, std::ptr::null(), SND_MEMORY | SND_SYNC | SND_NODEFAULT) };
    if ok == 0 {
        anyhow::bail!("PlaySound failed");
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn play_wav(_bytes: &[u8]) -> Result<()> {
    use std::sync::Once;
    static WARNED: Once = Once::new();
    WARNED.call_once(|| eprintln!("Playing audio on the PC is only supported on Windows."));
    Ok(())
}
