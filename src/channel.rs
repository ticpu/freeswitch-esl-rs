//! Channel-related data types extracted from ESL event headers.

use crate::event::EslEvent;

/// Channel timing data from FreeSWITCH's `switch_channel_timetable_t`.
///
/// Timestamps are epoch microseconds (`i64`). A value of `0` means the
/// corresponding event never occurred (e.g., `hungup == Some(0)` means
/// the channel has not hung up yet). `None` means the header was absent
/// or unparseable.
///
/// Extracted from ESL event headers using a prefix (typically `"Caller"`
/// or `"Other-Leg"`). The wire header format is `{prefix}-{suffix}`,
/// e.g. `Caller-Channel-Created-Time`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ChannelTimetable {
    /// When the caller profile was created.
    pub profile_created: Option<i64>,
    /// When the channel was created.
    pub created: Option<i64>,
    /// When the channel was answered.
    pub answered: Option<i64>,
    /// When early media (183) was received.
    pub progress: Option<i64>,
    /// When media-bearing early media arrived.
    pub progress_media: Option<i64>,
    /// When the channel hung up.
    pub hungup: Option<i64>,
    /// When the channel was transferred.
    pub transferred: Option<i64>,
    /// When the channel was resurrected.
    pub resurrected: Option<i64>,
    /// When the channel was bridged.
    pub bridged: Option<i64>,
    /// Timestamp of the last hold event.
    pub last_hold: Option<i64>,
    /// Accumulated hold time in microseconds.
    pub hold_accum: Option<i64>,
}

impl ChannelTimetable {
    /// Extract a timetable from event headers with the given prefix.
    ///
    /// Returns `None` if no timestamp headers with this prefix are present
    /// or parseable. Common prefixes: `"Caller"`, `"Other-Leg"`.
    pub fn from_event(event: &EslEvent, prefix: &str) -> Option<Self> {
        let mut tt = Self::default();
        let mut found = false;

        macro_rules! field {
            ($field:ident, $suffix:literal) => {
                let header = format!("{}-{}", prefix, $suffix);
                if let Some(v) = event
                    .header(&header)
                    .and_then(|s| {
                        s.parse()
                            .ok()
                    })
                {
                    tt.$field = Some(v);
                    found = true;
                }
            };
        }

        field!(profile_created, "Profile-Created-Time");
        field!(created, "Channel-Created-Time");
        field!(answered, "Channel-Answered-Time");
        field!(progress, "Channel-Progress-Time");
        field!(progress_media, "Channel-Progress-Media-Time");
        field!(hungup, "Channel-Hangup-Time");
        field!(transferred, "Channel-Transfer-Time");
        field!(resurrected, "Channel-Resurrect-Time");
        field!(bridged, "Channel-Bridged-Time");
        field!(last_hold, "Channel-Last-Hold");
        field!(hold_accum, "Channel-Hold-Accum");

        if found {
            Some(tt)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caller_timetable_all_fields() {
        let mut event = EslEvent::new();
        event.set_header("Caller-Profile-Created-Time", "1700000000000000");
        event.set_header("Caller-Channel-Created-Time", "1700000001000000");
        event.set_header("Caller-Channel-Answered-Time", "1700000005000000");
        event.set_header("Caller-Channel-Progress-Time", "1700000002000000");
        event.set_header("Caller-Channel-Progress-Media-Time", "1700000003000000");
        event.set_header("Caller-Channel-Hangup-Time", "0");
        event.set_header("Caller-Channel-Transfer-Time", "0");
        event.set_header("Caller-Channel-Resurrect-Time", "0");
        event.set_header("Caller-Channel-Bridged-Time", "1700000006000000");
        event.set_header("Caller-Channel-Last-Hold", "0");
        event.set_header("Caller-Channel-Hold-Accum", "0");

        let tt = event
            .caller_timetable()
            .expect("should parse");
        assert_eq!(tt.profile_created, Some(1700000000000000));
        assert_eq!(tt.created, Some(1700000001000000));
        assert_eq!(tt.answered, Some(1700000005000000));
        assert_eq!(tt.progress, Some(1700000002000000));
        assert_eq!(tt.progress_media, Some(1700000003000000));
        assert_eq!(tt.hungup, Some(0));
        assert_eq!(tt.transferred, Some(0));
        assert_eq!(tt.resurrected, Some(0));
        assert_eq!(tt.bridged, Some(1700000006000000));
        assert_eq!(tt.last_hold, Some(0));
        assert_eq!(tt.hold_accum, Some(0));
    }

    #[test]
    fn other_leg_timetable() {
        let mut event = EslEvent::new();
        event.set_header("Other-Leg-Profile-Created-Time", "1700000000000000");
        event.set_header("Other-Leg-Channel-Created-Time", "1700000001000000");
        event.set_header("Other-Leg-Channel-Answered-Time", "1700000005000000");
        event.set_header("Other-Leg-Channel-Progress-Time", "0");
        event.set_header("Other-Leg-Channel-Progress-Media-Time", "0");
        event.set_header("Other-Leg-Channel-Hangup-Time", "0");
        event.set_header("Other-Leg-Channel-Transfer-Time", "0");
        event.set_header("Other-Leg-Channel-Resurrect-Time", "0");
        event.set_header("Other-Leg-Channel-Bridged-Time", "1700000006000000");
        event.set_header("Other-Leg-Channel-Last-Hold", "0");
        event.set_header("Other-Leg-Channel-Hold-Accum", "0");

        let tt = event
            .other_leg_timetable()
            .expect("should parse");
        assert_eq!(tt.created, Some(1700000001000000));
        assert_eq!(tt.bridged, Some(1700000006000000));
    }

    #[test]
    fn timetable_no_headers() {
        let event = EslEvent::new();
        assert!(event
            .caller_timetable()
            .is_none());
        assert!(event
            .other_leg_timetable()
            .is_none());
    }

    #[test]
    fn timetable_partial_headers() {
        let mut event = EslEvent::new();
        event.set_header("Caller-Channel-Created-Time", "1700000001000000");

        let tt = event
            .caller_timetable()
            .expect("at least one field parsed");
        assert_eq!(tt.created, Some(1700000001000000));
        assert_eq!(tt.answered, None);
        assert_eq!(tt.profile_created, None);
    }

    #[test]
    fn timetable_invalid_value_only() {
        let mut event = EslEvent::new();
        event.set_header("Caller-Channel-Created-Time", "not_a_number");

        assert!(event
            .caller_timetable()
            .is_none());
    }

    #[test]
    fn timetable_zero_preserved() {
        let mut event = EslEvent::new();
        event.set_header("Caller-Channel-Hangup-Time", "0");

        let tt = event
            .caller_timetable()
            .expect("should parse");
        assert_eq!(tt.hungup, Some(0));
    }

    #[test]
    fn timetable_custom_prefix() {
        let mut event = EslEvent::new();
        event.set_header("Channel-Channel-Created-Time", "1700000001000000");

        let tt = event
            .timetable("Channel")
            .expect("custom prefix should work");
        assert_eq!(tt.created, Some(1700000001000000));
    }
}
