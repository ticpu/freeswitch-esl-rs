//! Options controlling which qualifiers are emitted when converting an [`SdpCodec`]
//! to a [`CodecStringEntry`].

use crate::sdp::{codec::SdpCodec, codec_string::CodecStringEntry, error::CodecStringError};

/// Controls which qualifiers appear in a [`CodecStringEntry`] built from an [`SdpCodec`].
///
/// Audio defaults emit rate, ptime, bitrate, and channels but **not** fmtp (see
/// `docs/codec-string-format.md` — audio fmtp in the codec string does not reach a
/// generated offer unless the leg has a bridged partner). Video entries emit only name
/// and fmtp, no numeric qualifiers.
///
/// This deliberately deviates from FreeSWITCH's own `add_audio_codec`, which emits
/// `@<n>c` only when channels > 1 and never emits `@<n>b` and `@<n>c` together — the two
/// share one format buffer, so one overwrites the other (`switch_core_media.c:13530-13536`,
/// see `docs/codec-string-format.md`). Audio defaults here emit `@1c` on every mono codec
/// and can emit both qualifiers at once. This is behaviourally neutral for matching and
/// dedup — an absent channel count normalizes to `1` either way — but it is not a
/// byte-for-byte port of upstream's emission quirk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodecStringOptions {
    emit_rate: bool,
    /// Turning ptime off for G.722 is a foot-gun: a rate without a ptime selects
    /// whichever implementation is first in FreeSWITCH's list rather than the 20 ms
    /// default, which may produce non-standard 10 ms packetization. Leave this on.
    emit_ptime: bool,
    emit_bitrate: bool,
    emit_channels: bool,
    /// Audio fmtp does not reach a generated offer unless the leg has a bridged
    /// partner at INVITE time, so emitting it by default would look correct in tests
    /// but be a no-op on a live originate. Disabled by default for audio.
    emit_fmtp: bool,
}

impl Default for CodecStringOptions {
    /// Audio-mode defaults: rate, ptime, bitrate, channels on; fmtp off.
    fn default() -> Self {
        Self {
            emit_rate: true,
            emit_ptime: true,
            emit_bitrate: true,
            emit_channels: true,
            emit_fmtp: false,
        }
    }
}

impl CodecStringOptions {
    /// Default options for audio: rate, ptime, bitrate, channels on; fmtp off.
    pub fn audio() -> Self {
        Self::default()
    }

    /// Options for video: only name and fmtp; no numeric qualifiers.
    ///
    /// Matches FreeSWITCH's video path in `add_audio_codec`.
    pub fn video() -> Self {
        Self {
            emit_rate: false,
            emit_ptime: false,
            emit_bitrate: false,
            emit_channels: false,
            emit_fmtp: true,
        }
    }

    /// Emit or suppress the rate qualifier (`@<n>h`).
    pub fn with_rate(mut self, emit: bool) -> Self {
        self.emit_rate = emit;
        self
    }

    /// Emit or suppress the ptime qualifier (`@<n>i`).
    ///
    /// # Warning
    ///
    /// For G.722, suppressing ptime while keeping rate causes FreeSWITCH to select
    /// whichever implementation is first in its list rather than the 20 ms default,
    /// potentially producing non-standard packetization. Leave this on.
    pub fn with_ptime(mut self, emit: bool) -> Self {
        self.emit_ptime = emit;
        self
    }

    /// Emit or suppress the bitrate qualifier (`@<n>b`).
    pub fn with_bitrate(mut self, emit: bool) -> Self {
        self.emit_bitrate = emit;
        self
    }

    /// Emit or suppress the channel count qualifier (`@<n>c`).
    pub fn with_channels(mut self, emit: bool) -> Self {
        self.emit_channels = emit;
        self
    }

    /// Emit or suppress the fmtp (`~fmtp` segment).
    pub fn with_fmtp(mut self, emit: bool) -> Self {
        self.emit_fmtp = emit;
        self
    }

    /// Whether rate will be emitted.
    pub fn emits_rate(&self) -> bool {
        self.emit_rate
    }

    /// Whether ptime will be emitted.
    pub fn emits_ptime(&self) -> bool {
        self.emit_ptime
    }

    /// Whether bitrate will be emitted.
    pub fn emits_bitrate(&self) -> bool {
        self.emit_bitrate
    }

    /// Whether channels will be emitted.
    pub fn emits_channels(&self) -> bool {
        self.emit_channels
    }

    /// Whether fmtp will be emitted.
    pub fn emits_fmtp(&self) -> bool {
        self.emit_fmtp
    }

    /// Convert an [`SdpCodec`] into a [`CodecStringEntry`] honouring these options.
    ///
    /// Returns `Err` if the codec name or fmtp value would produce an invalid entry
    /// (wire-injection, `@` in fmtp, dotted fmtp without a module prefix).
    pub fn sdp_codec_to_entry(
        &self,
        codec: &SdpCodec,
    ) -> Result<CodecStringEntry, CodecStringError> {
        let mut entry = CodecStringEntry::new(codec.name())?;

        if self.emit_fmtp {
            if let Some(fmtp) = codec.fmtp() {
                // Dotted fmtp without a module prefix is rejected by with_fmtp.
                // Callers that need dotted fmtp must set a module on the entry
                // before calling sdp_codec_to_entry, or use the builder directly.
                entry = entry.with_fmtp(fmtp)?;
            }
        }

        Ok(self.apply_qualifiers(entry, codec))
    }

    /// Apply the infallible numeric qualifiers (rate/ptime/bitrate/channels) per these
    /// options.
    ///
    /// Split out from [`sdp_codec_to_entry`](Self::sdp_codec_to_entry) so
    /// `SdpCodecs::audio_codec_string`/`video_codec_string` can recover from a failed
    /// fmtp embedding (clear it, keep the entry, warn) without re-deriving this logic.
    pub(crate) fn apply_qualifiers(
        &self,
        mut entry: CodecStringEntry,
        codec: &SdpCodec,
    ) -> CodecStringEntry {
        if self.emit_rate {
            entry = entry.with_rate(codec.clock_rate());
        }

        if self.emit_ptime {
            if let Some(p) = codec.ptime() {
                entry = entry.with_ptime(p);
            }
        }

        if self.emit_bitrate {
            if let Some(b) = codec.bitrate() {
                entry = entry.with_bitrate(b);
            }
        }

        if self.emit_channels {
            if let Some(c) = codec.channels() {
                // codec.channels() is Option<u8> from a=rtpmap; widen to u32 for the
                // codec-string field which uses the same width as C atoi into uint32_t.
                entry = entry.with_channels(u32::from(c));
            }
        }

        entry
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sdp::codec::{SdpDirection, SdpMediaType};

    fn make_audio_codec(name: &str, rate: u32, ptime: u32) -> SdpCodec {
        SdpCodec::new(
            SdpMediaType::Audio,
            0,
            name,
            rate,
            Some(1),
            None,
            Some(ptime),
            None,
            Some(64000),
            SdpDirection::SendRecv,
            false,
        )
    }

    fn make_video_codec(name: &str) -> SdpCodec {
        SdpCodec::new(
            SdpMediaType::Video,
            99,
            name,
            90000,
            None,
            Some("profile-level-id=42e01f".to_string()),
            None,
            None,
            None,
            SdpDirection::SendRecv,
            true,
        )
    }

    // --- defaults ---

    #[test]
    fn audio_defaults_emit_rate_ptime_bitrate_channels_not_fmtp() {
        let opts = CodecStringOptions::audio();
        assert!(opts.emits_rate());
        assert!(opts.emits_ptime());
        assert!(opts.emits_bitrate());
        assert!(opts.emits_channels());
        assert!(!opts.emits_fmtp(), "audio fmtp must be off by default");
    }

    #[test]
    fn video_defaults_emit_only_fmtp() {
        let opts = CodecStringOptions::video();
        assert!(!opts.emits_rate());
        assert!(!opts.emits_ptime());
        assert!(!opts.emits_bitrate());
        assert!(!opts.emits_channels());
        assert!(opts.emits_fmtp());
    }

    // --- sdp_codec_to_entry ---

    #[test]
    fn audio_entry_has_rate_ptime_bitrate_no_fmtp() {
        let codec = make_audio_codec("PCMU", 8000, 20);
        let entry = CodecStringOptions::audio()
            .sdp_codec_to_entry(&codec)
            .unwrap();
        assert_eq!(entry.name(), "PCMU");
        assert_eq!(entry.rate(), Some(8000));
        assert_eq!(entry.ptime(), Some(20));
        assert_eq!(entry.bitrate(), Some(64000));
        assert_eq!(entry.channels(), Some(1));
        assert!(
            entry
                .fmtp()
                .is_none(),
            "audio fmtp must not be emitted by default"
        );
    }

    #[test]
    fn video_entry_emits_no_qualifiers() {
        let codec = make_video_codec("H264");
        let entry = CodecStringOptions::video()
            .sdp_codec_to_entry(&codec)
            .unwrap();
        assert_eq!(entry.name(), "H264");
        assert!(
            entry
                .rate()
                .is_none(),
            "video must not emit rate"
        );
        assert!(
            entry
                .ptime()
                .is_none(),
            "video must not emit ptime"
        );
        assert!(
            entry
                .bitrate()
                .is_none(),
            "video must not emit bitrate"
        );
        assert!(
            entry
                .channels()
                .is_none(),
            "video must not emit channels"
        );
        assert_eq!(
            entry.fmtp(),
            Some("profile-level-id=42e01f"),
            "video must emit fmtp"
        );
    }

    // --- pure-data: Clone, Debug, PartialEq ---

    #[test]
    fn codec_string_options_clone_and_eq() {
        let a = CodecStringOptions::audio();
        let b = a.clone();
        assert_eq!(a, b);
        let c = CodecStringOptions::video();
        assert_ne!(a, c);
    }
}
