//! Channel state tracker — reference example for ESL channel lifecycle monitoring.
//!
//! Tracks all active channels as flat data maps storing event headers and
//! uuid_dump variables. Typed state accessors (`channel_state()`, `call_state()`,
//! etc.) parse on demand from the stored headers — no separate fields to sync.
//!
//! Bootstrap flow: subscribe → `show channels as json` → fake CHANNEL_CREATE
//! events → bgapi uuid_dump per channel. Single code path for bootstrap and
//! live events.
//!
//! uuid_dump uses bgapi so it doesn't block event processing — results arrive
//! as BACKGROUND_JOB events matched by Job-UUID.
//!
//! Usage: RUST_LOG=info cargo run --example channel_tracker [-- [host[:port]] [password]]

use std::collections::HashMap;
use std::fmt::Display;

use freeswitch_esl_tokio::{
    CallDirection, CallState, ChannelState, EslClient, EslError, EslEvent, EslEventType,
    EventFormat, DEFAULT_ESL_PORT,
};
use percent_encoding::percent_decode_str;
use tracing::{debug, error, info, warn};

fn short_uuid(uuid: &str) -> &str {
    &uuid[..8.min(uuid.len())]
}

fn display_or<T: Display>(opt: Option<T>) -> String {
    match opt {
        Some(v) => v.to_string(),
        None => "-".into(),
    }
}

/// Mapping from `show channels as json` field names to ESL event header names.
/// Used to build fake CHANNEL_CREATE events from bootstrap data so that
/// bootstrap and live events share the same processing path.
const DB_TO_EVENT: &[(&str, &str)] = &[
    ("uuid", "Unique-ID"),
    ("name", "Channel-Name"),
    ("state", "Channel-State"),
    ("callstate", "Channel-Call-State"),
    ("direction", "Call-Direction"),
    ("cid_name", "Caller-Caller-ID-Name"),
    ("cid_num", "Caller-Caller-ID-Number"),
    ("initial_cid_name", "Caller-Orig-Caller-ID-Name"),
    ("initial_cid_num", "Caller-Orig-Caller-ID-Number"),
    ("callee_name", "Caller-Callee-ID-Name"),
    ("callee_num", "Caller-Callee-ID-Number"),
    ("dest", "Caller-Destination-Number"),
    ("call_uuid", "Channel-Call-UUID"),
];

/// Build a fake CHANNEL_CREATE event from a `show channels as json` row,
/// mapping DB field names to event header names. Feeds through the normal
/// process_event path — no separate constructor to keep in sync.
fn fake_channel_create(row: &serde_json::Value) -> Option<EslEvent> {
    row.get("uuid")?
        .as_str()?;

    let mut event = EslEvent::with_type(EslEventType::ChannelCreate);
    for (json_key, header_name) in DB_TO_EVENT {
        if let Some(val) = row
            .get(json_key)
            .and_then(|v| v.as_str())
        {
            if !val.is_empty() {
                event.set_header(*header_name, val);
            }
        }
    }
    Some(event)
}

/// Flat data map — all event headers and uuid_dump variables accumulated over
/// the channel's lifetime. Typed state is parsed on demand from stored headers.
struct TrackedChannel {
    data: HashMap<String, String>,
}

impl TrackedChannel {
    fn new() -> Self {
        Self {
            data: HashMap::new(),
        }
    }

    fn channel_state(&self) -> Option<ChannelState> {
        self.get("Channel-State")?
            .parse()
            .ok()
    }

    fn call_state(&self) -> Option<CallState> {
        self.get("Channel-Call-State")?
            .parse()
            .ok()
    }

    fn call_direction(&self) -> Option<CallDirection> {
        self.get("Call-Direction")?
            .parse()
            .ok()
    }

    fn hangup_cause(&self) -> Option<&str> {
        self.get("Hangup-Cause")
    }

    /// Merge all event headers into the data map.
    fn update_from_event(&mut self, event: &EslEvent) {
        for (key, value) in event.headers() {
            self.data
                .insert(key.clone(), value.clone());
        }
    }

    /// Parse uuid_dump response body (Key: Value lines, percent-encoded values)
    /// and merge into the data map.
    fn update_from_dump(&mut self, body: &str) {
        for line in body.lines() {
            if let Some((key, value)) = line.split_once(": ") {
                let decoded = percent_decode_str(value)
                    .decode_utf8_lossy()
                    .into_owned();
                self.data
                    .insert(key.to_string(), decoded);
            }
        }
    }

    fn get(&self, key: &str) -> Option<&str> {
        self.data
            .get(key)
            .map(|s| s.as_str())
    }

    fn var(&self, name: &str) -> Option<&str> {
        self.data
            .get(&format!("variable_{}", name))
            .map(|s| s.as_str())
    }

    fn format_fields(&self) -> (String, String, String, &str, &str) {
        (
            display_or(self.channel_state()),
            display_or(self.call_state()),
            display_or(self.call_direction()),
            self.get("Caller-Caller-ID-Number")
                .unwrap_or("-"),
            self.get("Channel-Name")
                .unwrap_or("-"),
        )
    }
}

struct ChannelTracker {
    channels: HashMap<String, TrackedChannel>,
    /// Maps bgapi Job-UUID → channel UUID for pending uuid_dump results.
    pending_dumps: HashMap<String, String>,
}

impl ChannelTracker {
    fn new() -> Self {
        Self {
            channels: HashMap::new(),
            pending_dumps: HashMap::new(),
        }
    }

    /// Bootstrap from `show channels as json` — builds fake CHANNEL_CREATE
    /// events and feeds them through the normal process_event path.
    /// Returns UUIDs of bootstrapped channels (for uuid_dump follow-up).
    fn bootstrap(&mut self, body: &str) -> Vec<String> {
        let json: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => {
                warn!("Failed to parse show channels JSON: {}", e);
                return Vec::new();
            }
        };

        let rows = match json
            .get("rows")
            .and_then(|v| v.as_array())
        {
            Some(rows) => rows,
            None => {
                info!("No active channels at bootstrap");
                return Vec::new();
            }
        };

        let mut uuids = Vec::new();
        for row in rows {
            if let Some(event) = fake_channel_create(row) {
                if let Some(uuid) = event.unique_id() {
                    uuids.push(uuid.to_string());
                }
                self.process_event(&event);
            }
        }
        info!("Bootstrap loaded {} channels", uuids.len());
        uuids
    }

    fn apply_dump(&mut self, uuid: &str, body: &str) {
        if let Some(ch) = self
            .channels
            .get_mut(uuid)
        {
            ch.update_from_dump(body);
            debug!("uuid_dump applied for {}", short_uuid(uuid));
        }
    }

    fn handle_background_job(&mut self, event: &EslEvent) {
        let job_uuid = match event.job_uuid() {
            Some(j) => j,
            None => return,
        };
        if let Some(channel_uuid) = self
            .pending_dumps
            .remove(job_uuid)
        {
            if let Some(body) = event.body() {
                self.apply_dump(&channel_uuid, body);
            }
        }
    }

    fn process_event(&mut self, event: &EslEvent) {
        let event_type = match event.event_type() {
            Some(t) => t,
            None => return,
        };
        let uuid = match event.unique_id() {
            Some(u) => u.to_string(),
            None => return,
        };

        match event_type {
            EslEventType::ChannelCreate => {
                let mut ch = TrackedChannel::new();
                ch.update_from_event(event);
                self.channels
                    .insert(uuid.clone(), ch);
            }
            EslEventType::ChannelDestroy => {
                if let Some(ch) = self
                    .channels
                    .get_mut(&uuid)
                {
                    ch.update_from_event(event);
                    info!(
                        "{} {} cause={} name={}",
                        event_type,
                        short_uuid(&uuid),
                        ch.hangup_cause()
                            .unwrap_or("UNKNOWN"),
                        ch.get("Channel-Name")
                            .unwrap_or("-"),
                    );
                    self.channels
                        .remove(&uuid);
                } else {
                    info!("{} {} (untracked)", event_type, short_uuid(&uuid));
                }
                return;
            }
            EslEventType::ChannelHangup | EslEventType::ChannelHangupComplete => {
                self.update_channel(&uuid, event);
                let cause = self
                    .channels
                    .get(&uuid)
                    .and_then(|ch| ch.hangup_cause())
                    .unwrap_or("-");
                info!("{} {} cause={}", event_type, short_uuid(&uuid), cause);
                return;
            }
            EslEventType::ChannelUnbridge => {
                if let Some(ch) = self
                    .channels
                    .get_mut(&uuid)
                {
                    ch.update_from_event(event);
                    ch.data
                        .remove("Other-Leg-Unique-ID");
                }
            }
            _ => {
                self.update_channel(&uuid, event);
            }
        }
        self.print_channel_event(&uuid, event_type);
    }

    fn update_channel(&mut self, uuid: &str, event: &EslEvent) {
        if let Some(ch) = self
            .channels
            .get_mut(uuid)
        {
            ch.update_from_event(event);
        }
    }

    fn print_channel_event(&self, uuid: &str, event_type: EslEventType) {
        if let Some(ch) = self
            .channels
            .get(uuid)
        {
            let (state, call_state, dir, cid, name) = ch.format_fields();
            info!(
                "{:<9} {} state={} callstate={} dir={} cid={} name={}",
                event_type,
                short_uuid(uuid),
                state,
                call_state,
                dir,
                cid,
                name,
            );
        } else {
            info!("{:<9} {} (untracked)", event_type, short_uuid(uuid));
        }
    }

    fn print_summary(&self) {
        if self
            .channels
            .is_empty()
        {
            info!("--- No active channels ---");
            return;
        }
        info!(
            "--- {} active channel(s) ---",
            self.channels
                .len()
        );
        println!(
            "{:<36}  {:<14} {:<10} {:<8} {:<16} {:<16} NAME",
            "UUID", "STATE", "CALLSTATE", "DIR", "CID-NUM", "DEST",
        );
        let mut sorted: Vec<(&str, &TrackedChannel)> = self
            .channels
            .iter()
            .map(|(k, v)| (k.as_str(), v))
            .collect();
        sorted.sort_by_key(|(uuid, _)| *uuid);
        for (uuid, ch) in sorted {
            let (state, call_state, dir, cid, name) = ch.format_fields();
            let dest = ch
                .get("Caller-Destination-Number")
                .unwrap_or("-");
            let mut flags = String::new();
            if ch.call_state() == Some(CallState::Held) {
                flags.push_str("[HELD]");
            }
            if ch
                .var("rtp_secure_media_confirmed")
                .is_some()
            {
                flags.push_str("[SEC]");
            }
            if let Some(other) = ch.get("Other-Leg-Unique-ID") {
                flags.push_str(&format!("[B:{}]", short_uuid(other)));
            }
            if let Some(call_id) = ch.var("sip_call_id") {
                flags.push_str(&format!("[SIP:{}]", &call_id[..16.min(call_id.len())]));
            }
            println!(
                "{:<36}  {:<14} {:<10} {:<8} {:<16} {:<16} {}{}",
                uuid,
                state,
                call_state,
                dir,
                cid,
                dest,
                name,
                if flags.is_empty() {
                    String::new()
                } else {
                    format!(" {}", flags)
                },
            );
        }
    }
}

/// Request a uuid_dump via bgapi (non-blocking). The result arrives as a
/// BACKGROUND_JOB event and is matched by Job-UUID in the event loop.
async fn request_dump(client: &EslClient, tracker: &mut ChannelTracker, uuid: &str) {
    match client
        .bgapi(&format!("uuid_dump {}", uuid))
        .await
    {
        Ok(response) => {
            if let Some(job_uuid) = response.job_uuid() {
                tracker
                    .pending_dumps
                    .insert(job_uuid.to_string(), uuid.to_string());
            }
        }
        Err(e) => {
            debug!("bgapi uuid_dump {} failed: {}", short_uuid(uuid), e);
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let args: Vec<String> = std::env::args().collect();
    let (host, port) = match args
        .get(1)
        .map(|s| s.as_str())
    {
        Some(hp) if hp.contains(':') => {
            let (h, p) = hp
                .split_once(':')
                .unwrap();
            (
                h.to_string(),
                p.parse::<u16>()
                    .expect("invalid port"),
            )
        }
        Some(h) => (h.to_string(), DEFAULT_ESL_PORT),
        None => ("localhost".to_string(), DEFAULT_ESL_PORT),
    };
    let password = args
        .get(2)
        .map(|s| s.as_str())
        .unwrap_or("ClueCon");

    let (client, mut events) = match EslClient::connect(&host, port, password).await {
        Ok(pair) => {
            info!("Connected to FreeSWITCH at {}:{}", host, port);
            pair
        }
        Err(EslError::Io(e)) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
            error!(
                "Connection refused — is FreeSWITCH running on {}:{}?",
                host, port,
            );
            return Err(e.into());
        }
        Err(e) => {
            error!("Failed to connect: {}", e);
            return Err(e.into());
        }
    };

    client
        .subscribe_events(
            EventFormat::Plain,
            &[
                EslEventType::ChannelCreate,
                EslEventType::ChannelDestroy,
                EslEventType::ChannelState,
                EslEventType::ChannelCallstate,
                EslEventType::ChannelAnswer,
                EslEventType::ChannelHangup,
                EslEventType::ChannelHangupComplete,
                EslEventType::ChannelExecute,
                EslEventType::ChannelExecuteComplete,
                EslEventType::ChannelHold,
                EslEventType::ChannelUnhold,
                EslEventType::ChannelBridge,
                EslEventType::ChannelUnbridge,
                EslEventType::ChannelProgress,
                EslEventType::ChannelProgressMedia,
                EslEventType::ChannelOutgoing,
                EslEventType::ChannelPark,
                EslEventType::ChannelUnpark,
                EslEventType::ChannelApplication,
                EslEventType::ChannelOriginate,
                EslEventType::ChannelUuid,
                EslEventType::CallSecure,
                EslEventType::CallUpdate,
                EslEventType::BackgroundJob,
                EslEventType::Heartbeat,
            ],
        )
        .await?;

    info!("Subscribed to channel events + heartbeat");

    let mut tracker = ChannelTracker::new();

    // Bootstrap: show channels → fake events → bgapi uuid_dump per channel.
    // Subscribe first so we don't miss channels created during bootstrap.
    // Dump results arrive as BACKGROUND_JOB events in the event loop.
    match client
        .api("show channels as json")
        .await
    {
        Ok(response) => {
            if let Some(body) = response.body() {
                let uuids = tracker.bootstrap(body);
                for uuid in &uuids {
                    request_dump(&client, &mut tracker, uuid).await;
                }
            }
        }
        Err(e) => warn!("Failed to bootstrap channels: {}", e),
    }

    info!("Listening for events... Press Ctrl+C to exit");

    while let Some(result) = events
        .recv()
        .await
    {
        let event = match result {
            Ok(event) => event,
            Err(e) => {
                error!("Event error: {}", e);
                continue;
            }
        };

        match event.event_type() {
            Some(EslEventType::Heartbeat) => tracker.print_summary(),
            Some(EslEventType::BackgroundJob) => tracker.handle_background_job(&event),
            Some(EslEventType::ChannelCreate) => {
                let uuid = event
                    .unique_id()
                    .map(|s| s.to_string());
                tracker.process_event(&event);
                if let Some(uuid) = uuid {
                    request_dump(&client, &mut tracker, &uuid).await;
                }
            }
            _ => tracker.process_event(&event),
        }
    }

    info!("Connection closed");
    client
        .disconnect()
        .await?;

    Ok(())
}
