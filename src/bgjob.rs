//! Background job tracking for [`bgapi`](crate::EslClient::bgapi) commands.
//!
//! Every `bgapi` call returns a Job-UUID immediately; the actual result arrives
//! later as a [`BackgroundJob`](crate::EslEventType::BackgroundJob) event.
//!
//! Matching those events by Job-UUID is a correctness requirement, not
//! tidiness. `BACKGROUND_JOB` is fired on the global event bus, so every client
//! receives every other client's job results; a loop that acts on whichever one
//! arrives will act on results it never issued. Issuing one command at a time
//! does not help — an operator's `fs_cli -x 'bgapi ...'` lands in your stream
//! too. `BgJobTracker` eliminates the boilerplate of maintaining a pending-jobs
//! `HashMap` and doing that matching in the event loop.
//!
//! # Without context
//!
//! ```rust,no_run
//! # async fn example(client: &freeswitch_esl_tokio::EslClient, mut events: freeswitch_esl_tokio::EslEventStream) -> Result<(), freeswitch_esl_tokio::EslError> {
//! use freeswitch_esl_tokio::BgJobTracker;
//!
//! let mut bg = BgJobTracker::new();
//! bg.send(&client, "status").await?;
//!
//! while let Some(Ok(event)) = events.recv().await {
//!     if let Some(((), result)) = bg.try_complete(&event) {
//!         println!("{}", result.parse_body()?);
//!         continue;
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # With context
//!
//! Attach caller-defined context to each job. The context is returned alongside
//! the result, enabling dispatch without a separate tracking map:
//!
//! ```rust,no_run
//! # async fn example(client: &freeswitch_esl_tokio::EslClient, mut events: freeswitch_esl_tokio::EslEventStream) -> Result<(), freeswitch_esl_tokio::EslError> {
//! use freeswitch_esl_tokio::BgJobTracker;
//!
//! let mut bg: BgJobTracker<String> = BgJobTracker::new();
//! let uuid = "abc123".to_string();
//! bg.bgapi(&client, &format!("uuid_dump {uuid}"), uuid.clone()).await?;
//!
//! while let Some(Ok(event)) = events.recv().await {
//!     if let Some((channel_uuid, result)) = bg.try_complete(&event) {
//!         println!("dump for {}: {:?}", channel_uuid, result.body());
//!         continue;
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Stale job cleanup
//!
//! If a `BACKGROUND_JOB` event is lost (subscription misconfiguration, network
//! issue, FreeSWITCH bug), entries remain in the tracker indefinitely. Long-lived
//! trackers **must** call [`BgJobTracker::retain`] periodically to evict stale
//! entries. Store a timestamp in the context type and sweep on a timer — see
//! [`retain`](BgJobTracker::retain) for an example.
//!
//! Monitor [`BgJobTracker::pending_count`] to detect unexpected growth.

use std::collections::HashMap;
use std::fmt;

use crate::command::parse_api_body;
use crate::connection::EslClient;
use crate::error::{EslError, EslResult};
use freeswitch_types::event::{EslEvent, EslEventType};
use freeswitch_types::lookup::HeaderLookup;

/// Tracks pending `bgapi` commands and correlates
/// [`BackgroundJob`](EslEventType::BackgroundJob) events to their originating
/// call.
///
/// The type parameter `C` is user-defined context attached to each pending job.
/// Use `()` (the default) when no context is needed.
pub struct BgJobTracker<C = ()> {
    pending: HashMap<String, C>,
}

impl<C: fmt::Debug> fmt::Debug for BgJobTracker<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BgJobTracker")
            .field("pending", &self.pending)
            .finish()
    }
}

impl<C> Default for BgJobTracker<C> {
    fn default() -> Self {
        Self {
            pending: HashMap::new(),
        }
    }
}

impl<C> BgJobTracker<C> {
    /// Create an empty tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Send a `bgapi` command and track its Job-UUID with the given context.
    ///
    /// Returns the Job-UUID string on success. The context is stored and
    /// returned by [`try_complete`](Self::try_complete) when the matching
    /// `BACKGROUND_JOB` event arrives.
    ///
    /// # Errors
    ///
    /// Returns [`EslError::ProtocolError`] if the response is missing the
    /// `Job-UUID` header. In this case the command has already been sent to
    /// FreeSWITCH but cannot be tracked, and `ctx` is lost. A missing
    /// `Job-UUID` is a protocol violation; consider disconnecting and
    /// reconnecting from a known-good state.
    pub async fn bgapi(&mut self, client: &EslClient, command: &str, ctx: C) -> EslResult<String> {
        let resp = client
            .bgapi(command)
            .await?;
        let job_uuid = resp
            .job_uuid()
            .ok_or_else(|| EslError::ProtocolError {
                message: "bgapi response missing Job-UUID header".into(),
            })?
            .to_string();
        self.pending
            .insert(job_uuid.clone(), ctx);
        Ok(job_uuid)
    }

    /// Register an already-sent `bgapi` by its Job-UUID.
    ///
    /// Use this when you send via [`EslClient::bgapi`] directly and want to
    /// track the result through this tracker.
    pub fn track(&mut self, job_uuid: String, ctx: C) {
        self.pending
            .insert(job_uuid, ctx);
    }

    /// Check if an event completes a pending background job.
    ///
    /// If the event is a [`BackgroundJob`](EslEventType::BackgroundJob) whose
    /// Job-UUID matches a tracked job, removes it from the tracker and returns
    /// the caller's context alongside a [`BgJobResult`] wrapper.
    ///
    /// `None` means "not one of yours" — another client's job result, or not a
    /// `BACKGROUND_JOB` at all. It never means your job failed, so treat it as a
    /// no-op and keep waiting; a job whose result never arrives (the connection
    /// that issued it was replaced) is reclaimed by [`retain`](Self::retain), not
    /// by an event.
    pub fn try_complete<'e>(&mut self, event: &'e EslEvent) -> Option<(C, BgJobResult<'e>)> {
        if !event.is_event_type(EslEventType::BackgroundJob) {
            return None;
        }
        let job_uuid = event.job_uuid()?;
        let ctx = self
            .pending
            .remove(job_uuid)?;
        Some((ctx, BgJobResult(event)))
    }

    /// Number of jobs awaiting completion.
    pub fn pending_count(&self) -> usize {
        self.pending
            .len()
    }

    /// Whether a specific Job-UUID is being tracked.
    pub fn is_pending(&self, job_uuid: &str) -> bool {
        self.pending
            .contains_key(job_uuid)
    }

    /// Stop tracking a job without completing it. Returns the context if the
    /// job was pending.
    pub fn cancel(&mut self, job_uuid: &str) -> Option<C> {
        self.pending
            .remove(job_uuid)
    }

    /// Iterate over pending Job-UUIDs (no specific order).
    pub fn pending_jobs(&self) -> impl Iterator<Item = &str> {
        self.pending
            .keys()
            .map(|s| s.as_str())
    }

    /// Retain only jobs matching the predicate, removing the rest.
    ///
    /// Long-lived trackers **must** call this periodically to evict stale
    /// entries whose `BACKGROUND_JOB` events were lost. Store a timestamp
    /// in the context type and sweep on a timer:
    ///
    /// ```rust,no_run
    /// use std::time::{Duration, Instant};
    /// use freeswitch_esl_tokio::BgJobTracker;
    ///
    /// let mut bg: BgJobTracker<Instant> = BgJobTracker::new();
    /// // ... track jobs with Instant::now() as context ...
    ///
    /// // Periodic sweep (e.g. every 60s in your event loop):
    /// let max_age = Duration::from_secs(300);
    /// bg.retain(|_uuid, submitted_at| submitted_at.elapsed() < max_age);
    ///
    /// // Monitor for unexpected growth:
    /// if bg.pending_count() > 100 {
    ///     eprintln!("warning: {} bgapi jobs still pending", bg.pending_count());
    /// }
    /// ```
    pub fn retain(&mut self, mut f: impl FnMut(&str, &mut C) -> bool) {
        self.pending
            .retain(|uuid, ctx| f(uuid, ctx));
    }
}

/// Convenience methods available only when no caller context is needed.
impl BgJobTracker<()> {
    /// Send a `bgapi` command and track it without context.
    ///
    /// Convenience for `bgapi(client, command, ())` when no caller context is
    /// needed.
    pub async fn send(&mut self, client: &EslClient, command: &str) -> EslResult<String> {
        self.bgapi(client, command, ())
            .await
    }
}

/// Convenience wrapper over a [`BackgroundJob`](EslEventType::BackgroundJob)
/// event returned by [`BgJobTracker::try_complete`].
///
/// Provides direct access to the job result body and delegates all
/// [`HeaderLookup`] accessors to the underlying event.
pub struct BgJobResult<'a>(&'a EslEvent);

impl fmt::Debug for BgJobResult<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BgJobResult")
            .field(
                "job_uuid",
                &self
                    .0
                    .job_uuid(),
            )
            .field(
                "body",
                &self
                    .0
                    .body(),
            )
            .finish()
    }
}

impl<'a> BgJobResult<'a> {
    /// The Job-UUID header from the event.
    pub fn job_uuid(&self) -> Option<&str> {
        self.0
            .job_uuid()
    }

    /// The raw result body (the output of the original `bgapi` command).
    pub fn body(&self) -> Option<&'a str> {
        self.0
            .body()
    }

    /// Parse the result body, handling `+OK`/`-ERR` prefixes.
    ///
    /// Delegates to [`parse_api_body`].
    pub fn parse_body(&self) -> EslResult<&'a str> {
        parse_api_body(
            self.0
                .body()
                .unwrap_or(""),
        )
    }

    /// The underlying [`EslEvent`] for direct access to headers not exposed
    /// by [`BgJobResult`] or [`HeaderLookup`].
    pub fn event(&self) -> &'a EslEvent {
        self.0
    }
}

impl freeswitch_types::sip_header::SipHeaderLookup for BgJobResult<'_> {
    fn sip_header_str(&self, name: &str) -> Option<&str> {
        self.0
            .header_str(name)
    }
}

impl HeaderLookup for BgJobResult<'_> {
    fn header_str(&self, name: &str) -> Option<&str> {
        self.0
            .header_str(name)
    }

    fn variable_str(&self, name: &str) -> Option<&str> {
        self.0
            .variable_str(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use freeswitch_types::headers::EventHeader;

    fn bg_job_event(job_uuid: &str, body: &str) -> EslEvent {
        let mut event = EslEvent::with_type(EslEventType::BackgroundJob);
        event.set_header(EventHeader::JobUuid.as_str(), job_uuid);
        event.set_body(body);
        event
    }

    #[test]
    fn new_tracker_is_empty() {
        let bg = BgJobTracker::<String>::new();
        assert_eq!(bg.pending_count(), 0);
    }

    #[test]
    fn track_adds_pending() {
        let mut bg = BgJobTracker::new();
        bg.track("uuid-1".into(), "ctx-1".to_string());
        assert!(bg.is_pending("uuid-1"));
        assert_eq!(bg.pending_count(), 1);
    }

    #[test]
    fn try_complete_matches_background_job() {
        let mut bg = BgJobTracker::new();
        bg.track("uuid-1".into(), "channel-abc".to_string());

        let event = bg_job_event("uuid-1", "+OK done");
        let (ctx, result) = bg
            .try_complete(&event)
            .expect("should match");
        assert_eq!(ctx, "channel-abc");
        assert_eq!(result.job_uuid(), Some("uuid-1"));
        assert_eq!(result.body(), Some("+OK done"));
    }

    #[test]
    fn try_complete_ignores_non_background_job() {
        let mut bg = BgJobTracker::new();
        bg.track("uuid-1".into(), ());

        let event = EslEvent::with_type(EslEventType::ChannelCreate);
        assert!(bg
            .try_complete(&event)
            .is_none());
        assert_eq!(bg.pending_count(), 1);
    }

    #[test]
    fn try_complete_ignores_untracked_job() {
        let mut bg = BgJobTracker::<()>::new();
        let event = bg_job_event("uuid-unknown", "+OK");
        assert!(bg
            .try_complete(&event)
            .is_none());
    }

    #[test]
    fn try_complete_removes_from_pending() {
        let mut bg = BgJobTracker::new();
        bg.track("uuid-1".into(), ());

        let event = bg_job_event("uuid-1", "+OK");
        assert!(bg
            .try_complete(&event)
            .is_some());
        assert!(!bg.is_pending("uuid-1"));
        assert_eq!(bg.pending_count(), 0);
        // Second call returns None
        assert!(bg
            .try_complete(&event)
            .is_none());
    }

    #[test]
    fn try_complete_no_job_uuid_header() {
        let mut bg = BgJobTracker::new();
        bg.track("uuid-1".into(), ());

        // BackgroundJob event without Job-UUID header
        let event = EslEvent::with_type(EslEventType::BackgroundJob);
        assert!(bg
            .try_complete(&event)
            .is_none());
        assert_eq!(bg.pending_count(), 1);
    }

    #[test]
    fn cancel_returns_context() {
        let mut bg = BgJobTracker::new();
        bg.track("uuid-1".into(), "my-ctx".to_string());
        assert_eq!(bg.cancel("uuid-1"), Some("my-ctx".to_string()));
        assert!(!bg.is_pending("uuid-1"));
    }

    #[test]
    fn cancel_nonexistent_returns_none() {
        let mut bg = BgJobTracker::<String>::new();
        assert_eq!(bg.cancel("uuid-nope"), None);
    }

    #[test]
    fn bgjob_result_parse_body_ok() {
        let event = bg_job_event("uuid-1", "+OK Job-UUID: abc-123");
        let result = BgJobResult(&event);
        assert_eq!(
            result
                .parse_body()
                .unwrap(),
            "Job-UUID: abc-123"
        );
    }

    #[test]
    fn bgjob_result_parse_body_err() {
        let event = bg_job_event("uuid-1", "-ERR invalid command");
        let result = BgJobResult(&event);
        assert!(result
            .parse_body()
            .is_err());
    }

    #[test]
    fn bgjob_result_delegates_header_lookup() {
        let event = bg_job_event("uuid-1", "+OK");
        let result = BgJobResult(&event);
        // HeaderLookup::job_uuid() should work through delegation
        assert_eq!(result.job_uuid(), Some("uuid-1"));
    }

    #[test]
    fn bgjob_result_event_access() {
        let event = bg_job_event("uuid-1", "+OK data");
        let result = BgJobResult(&event);
        assert_eq!(
            result
                .event()
                .event_type(),
            Some(EslEventType::BackgroundJob)
        );
    }

    #[test]
    fn pending_jobs_iterates_uuids() {
        let mut bg = BgJobTracker::new();
        bg.track("uuid-a".into(), "ctx-a".to_string());
        bg.track("uuid-b".into(), "ctx-b".to_string());

        let mut uuids: Vec<&str> = bg
            .pending_jobs()
            .collect();
        uuids.sort();
        assert_eq!(uuids, vec!["uuid-a", "uuid-b"]);
    }

    #[test]
    fn pending_jobs_empty() {
        let bg = BgJobTracker::<()>::new();
        assert_eq!(
            bg.pending_jobs()
                .count(),
            0
        );
    }

    #[test]
    fn retain_filters_pending() {
        let mut bg = BgJobTracker::new();
        bg.track("keep-1".into(), 1u32);
        bg.track("drop-2".into(), 2u32);
        bg.track("keep-3".into(), 3u32);

        bg.retain(|uuid, _ctx| uuid.starts_with("keep"));

        assert_eq!(bg.pending_count(), 2);
        assert!(bg.is_pending("keep-1"));
        assert!(bg.is_pending("keep-3"));
        assert!(!bg.is_pending("drop-2"));
    }

    #[test]
    fn retain_can_mutate_context() {
        let mut bg = BgJobTracker::new();
        bg.track("uuid-1".into(), 0u32);

        bg.retain(|_, ctx| {
            *ctx += 1;
            true
        });

        let event = bg_job_event("uuid-1", "+OK");
        let (ctx, _) = bg
            .try_complete(&event)
            .expect("should match");
        assert_eq!(ctx, 1);
    }
}
