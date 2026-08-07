//! Inspect a live call's SDP offer and compose a codec string that FreeSWITCH will honour.
//!
//! For each answered channel this example:
//! - reads `switch_r_sdp` via the typed `ChannelVariable`/`HeaderLookup` path (never
//!   `header_str()` -- a typo in a raw header name is a silent `None`, not a compile error),
//! - parses it with `SdpCodecs::parse` and prints what the peer offered,
//! - builds the audio codec string from the offer, appends a mandatory backup list,
//!   deduplicates, and drops entries no loaded implementation would accept,
//! - sets `absolute_codec_string` and `rtp_force_audio_fmtp` on the channel via ESL.
//!
//! Run with the `sdp` feature enabled:
//!   cargo run --example sdp_codec_string --features sdp
//!
//! Connection (all optional, these are the defaults):
//!   ESL_HOST=localhost  ESL_PORT=8021  ESL_PASSWORD=ClueCon

use freeswitch_esl_tokio::sdp::{
    default_rate, CodecImplementation, CodecString, CodecStringOptions, SdpCodecEntry, SdpCodecs,
    SdpWarning,
};
use freeswitch_esl_tokio::{
    ChannelVariable, EslClient, EslError, EslEventType, EventFormat, EventSubscription,
    HeaderLookup, UuidSetVar, DEFAULT_ESL_PASSWORD, DEFAULT_ESL_PORT,
};
use tracing::{error, info, warn};

// Codecs a regulated interface requires to always be present, appended after the
// offer rather than merged into it. This is the point of composing over merging:
// the peer's own order and qualifiers (its `PCMU@8000h@20i` beats a bare `PCMU`
// appended below it) are never displaced, only backfilled if genuinely missing.
const MANDATORY_BACKUP_CODECS: &str = "PCMU,PCMA";

// A caller-supplied inventory of what this switch has loaded. No ESL API exposes
// the real loaded-implementation table, so retain_available() only ever knows
// what you tell it here -- keep this in sync with the box's modules.conf.
//
// No "t38" entry on purpose. audio_codec_string() below emits the literal "t38" for
// any image/t38 m-line in the offer, but FreeSWITCH has no codec interface by that
// name -- it's just the string generate_m() writes for a T.38 section, never looked
// up like a codec. Left out of this inventory, a T.38 offer is silently dropped by
// retain_available() below, with only a generic "not in the loaded-implementation
// inventory" warning -- exactly the t38 trap documented in
// docs/codec-string-format.md. A caller that forwards T.38 sections must add
// CodecImplementation::new("t38") here to keep it, understanding that this
// misrepresents the switch's real codec table in order to preserve the entry.
fn loaded_implementations() -> Vec<CodecImplementation> {
    vec![
        CodecImplementation::new("PCMU"),
        CodecImplementation::new("PCMA"),
        CodecImplementation::new("G722"),
        CodecImplementation::new("opus"),
    ]
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let host = std::env::var("ESL_HOST").unwrap_or_else(|_| "localhost".to_string());
    let port: u16 = std::env::var("ESL_PORT")
        .ok()
        .and_then(|p| {
            p.parse()
                .ok()
        })
        .unwrap_or(DEFAULT_ESL_PORT);
    let password =
        std::env::var("ESL_PASSWORD").unwrap_or_else(|_| DEFAULT_ESL_PASSWORD.to_string());

    let (client, mut events) = match EslClient::connect(&host, port, &password).await {
        Ok(pair) => {
            info!("Connected to {}:{}", host, port);
            pair
        }
        Err(EslError::Io(e)) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
            error!("Connection refused at {}:{}", host, port);
            return Err(e.into());
        }
        Err(e) => return Err(e.into()),
    };

    // EventSubscription is the typed builder for subscriptions. It bundles the event
    // format, typed event names, and optional subclass filters into a single call to
    // apply_subscription(). Never construct "event plain CHANNEL_ANSWER" by hand.
    let subscription =
        EventSubscription::new(EventFormat::Plain).event(EslEventType::ChannelAnswer);
    client
        .apply_subscription(&subscription)
        .await?;
    info!("Subscribed to CHANNEL_ANSWER");

    // Parse the backup list once; the same CodecString is appended to every offer.
    // expect() is safe here: MANDATORY_BACKUP_CODECS is a compile-time constant whose
    // syntax was verified against the grammar in docs/codec-string-format.md.
    let backup: CodecString = MANDATORY_BACKUP_CODECS
        .parse()
        .expect("MANDATORY_BACKUP_CODECS is a valid codec string constant");
    let inventory = loaded_implementations();

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

        let uuid = match event.unique_id() {
            Some(u) => u.to_string(),
            None => continue,
        };

        // variable() is HeaderLookup's typed channel-variable accessor. It prepends
        // "variable_" internally and returns None cleanly when the header is absent.
        // Using the ChannelVariable enum rather than a raw string means a typo is a
        // compile error, not a silent None at runtime.
        let sdp_str = match event.variable(ChannelVariable::SwitchRSdp) {
            Some(s) => s.to_string(),
            None => {
                // uuid is a FreeSWITCH UUID (ASCII hex + dashes), but slice indexing
                // panics on a non-ASCII char boundary. Use char-safe truncation.
                let short: String = uuid
                    .chars()
                    .take(8)
                    .collect();
                warn!(
                    "{}: no switch_r_sdp -- call was not an INVITE with body SDP",
                    short
                );
                continue;
            }
        };

        // event is dropped here; its string data was already copied into sdp_str.
        handle_answered_channel(&client, &uuid, &sdp_str, &backup, &inventory).await;
    }

    client
        .disconnect()
        .await?;
    Ok(())
}

async fn handle_answered_channel(
    client: &EslClient,
    uuid: &str,
    sdp_str: &str,
    backup: &CodecString,
    inventory: &[CodecImplementation],
) {
    // uuid is a FreeSWITCH UUID (ASCII hex + dashes), but slice indexing panics on
    // a non-ASCII char boundary. Collect the first 8 chars instead.
    let short: String = uuid
        .chars()
        .take(8)
        .collect();

    // SdpCodecs::parse fails only on structural breakage: missing v=, malformed
    // a=rtpmap, non-numeric payload type. Unknown codecs with no a=rtpmap go to
    // unmapped(), so the parse succeeds even for exotic carrier SDPs.
    let parsed = match SdpCodecs::parse(sdp_str) {
        Ok(p) => p,
        Err(e) => {
            warn!("{}: SDP parse error: {}", short, e);
            return;
        }
    };

    // Print a table of what the peer offered in the order it offered them.
    println!("{}: offered codecs (SDP offer order):", short);
    println!(
        "  {:<14} {:>3}  {:>8}  {:>3}  {:>5}  fmtp",
        "name", "pt", "rate", "ch", "ptime"
    );
    for entry in parsed.entries() {
        match entry {
            SdpCodecEntry::Rtp(c) => {
                println!(
                    "  {:<14} {:>3}  {:>8}  {:>3}  {:>5}  {}",
                    c.name(),
                    c.payload_type(),
                    c.clock_rate(),
                    c.channels()
                        .map_or_else(|| "-".to_string(), |n| n.to_string()),
                    c.ptime()
                        .map_or_else(|| "-".to_string(), |p| p.to_string()),
                    c.fmtp()
                        .unwrap_or(""),
                );
            }
            SdpCodecEntry::T38 => println!("  t38 (image/udptl)"),
            // SdpCodecEntry is #[non_exhaustive]; future variants land here.
            _ => {}
        }
    }

    // Negotiated through FreeSWITCH's smh->mparams->te and cng_pt paths, never through
    // the codec string, so these never reach entries() or the string built below. This
    // is what the offer said, not what the switch picked: it keeps one of each per
    // session and never reads their fmtp. Display renders the a=rtpmap they came from.
    for payload in parsed.non_codec_payloads() {
        println!("{}: {}", short, payload);
    }

    // unmapped(): payload types in the m= format list with no a=rtpmap and no
    // RFC 3551 static-table entry. Surfaced as data rather than a parse error so
    // the caller can distinguish "never offered" from "offered but unresolvable".
    for u in parsed.unmapped() {
        warn!("{}: {}", short, u);
    }

    // warnings(): recoverable parse oddities, e.g. an unparseable a=ptime value.
    // The parser records these and continues, matching FreeSWITCH's atoi tolerance.
    for w in parsed.warnings() {
        warn!("{}: SDP warning: {}", short, w);
    }

    // Build the audio codec string from the offer. Lenient mode: an SDP crossing
    // the network is not something we control, so an unrepresentable fmtp gets
    // cleared (with a warning) rather than failing the whole call.
    let mut build_warnings: Vec<SdpWarning> = Vec::new();
    let mut codec_string =
        match parsed.audio_codec_string(&CodecStringOptions::audio(), Some(&mut build_warnings)) {
            Ok(cs) => cs,
            Err(e) => {
                warn!("{}: audio_codec_string error: {}", short, e);
                return;
            }
        };
    for w in &build_warnings {
        warn!("{}: codec-string warning: {}", short, w);
    }
    println!("{}: from offer: {}", short, codec_string);

    // Append the mandatory backup list. This is composition, not a merge: nothing
    // from the offer is removed or reordered, the backup entries just land at the
    // tail as a floor. Concatenation order is the policy -- see dedup() below.
    codec_string.extend_from(backup);
    println!("{}: with backup appended: {}", short, codec_string);

    // dedup() uses FreeSWITCH's own key (name/rate/ptime/channels/fmtp, normalized
    // the way switch_loadable_module_get_codecs_sorted does). A bare "PCMU" and
    // "PCMU@8000h@20i" collapse to one entry because they normalize to the same
    // key. The FIRST occurrence wins -- which is exactly why the offer was
    // concatenated ahead of the backup list: the peer's qualifiers survive, the
    // backup entry for that name is silently absorbed rather than duplicated.
    codec_string.dedup();
    println!("{}: after dedup: {}", short, codec_string);

    // retain_available() is a caller-supplied check: no ESL API exposes the loaded
    // codec implementation table, so `inventory` is this deployment's own list.
    // An entry can still survive here (its name/modname matches) and yet be
    // dropped later by the switch if none of its numeric *qualifiers* match a
    // loaded implementation. codec_string.qualified() also yields fmtp-only
    // entries, which can't be dropped this way but do change codec behaviour.
    let removed = codec_string.retain_available(inventory);
    for entry in &removed {
        warn!(
            "{}: {} not in the loaded-implementation inventory, dropped",
            short, entry
        );
    }
    for entry in codec_string.qualified() {
        info!(
            "{}: {} carries a qualifier or fmtp worth double-checking",
            short, entry
        );
    }
    println!("{}: final codec string: {}", short, codec_string);

    // Set absolute_codec_string. This variable takes precedence over codec_string
    // and ep_codec_string and takes effect on the next SDP offer this leg generates.
    //
    // Delivery path: uuid_setvar splits its argument string on spaces and honours
    // single-quote grouping. AMR format-parameter strings routinely contain "; "
    // with a space (e.g. "octet-align=1; mode-set=0,1,2"), so an unquoted value
    // would be truncated at the first space. Single-quoting prevents that split.
    // UuidSetVar's Display does not add quotes itself -- it is a bare wire builder --
    // so the quoting is this caller's job, same as it would be for any other consumer
    // of the typed command.
    let quoted = format!("'{}'", escape_for_setvar(&codec_string.to_string()));
    let cmd = UuidSetVar::new(
        uuid,
        ChannelVariable::AbsoluteCodecString.to_string(),
        quoted,
    );
    match client
        .api(&cmd.to_string())
        .await
    {
        Ok(resp) => {
            if let Err(e) = resp.into_result() {
                warn!(
                    "{}: uuid_setvar {} failed: {}",
                    short,
                    ChannelVariable::AbsoluteCodecString,
                    e
                );
            } else {
                info!(
                    "{}: {} set: {}",
                    short,
                    ChannelVariable::AbsoluteCodecString,
                    codec_string,
                );
            }
        }
        Err(e) => warn!(
            "{}: api error setting {}: {}",
            short,
            ChannelVariable::AbsoluteCodecString,
            e
        ),
    }

    // rtp_force_audio_fmtp: the offer's a=fmtp for whichever codec ended up first
    // in the final string. Audio fmtp notation in a codec string does not reach
    // the generated a=fmtp line unless this leg already has a bridged partner at
    // INVITE time (FreeSWITCH's partner-dependent SDP branch); on an unpartnered
    // leg this variable is the only thing that actually applies it.
    //
    // fmtp_for() takes an explicit clock rate rather than assuming one, because
    // "first codec in the offer" is routinely not the first codec in the final
    // string. When the surviving entry carries no explicit @rate qualifier,
    // default_rate() (freeswitch_esl_tokio::sdp -- the same table dedup() itself
    // normalizes against) gives the rate FreeSWITCH would assume at match time.
    if let Some(first) = codec_string
        .entries()
        .first()
    {
        let rate = first
            .rate()
            .unwrap_or_else(|| default_rate(first.name()));
        if let Some(fmtp) = parsed.fmtp_for(first.name(), rate) {
            // uuid_setvar splits on spaces and processes \ escapes even inside single
            // quotes (cleanup_separated_string). Escape ' and \
            // in the raw peer fmtp before wrapping in single quotes: unescaped '
            // toggles quote state (corrupting the argument boundary) and unescaped \
            // consumes the next char. \n is line-split out of wire headers so it
            // cannot appear here.
            let safe_fmtp = format!("'{}'", escape_for_setvar(fmtp));
            let fmtp_cmd = UuidSetVar::new(
                uuid,
                ChannelVariable::RtpForceAudioFmtp.to_string(),
                safe_fmtp,
            );
            match client
                .api(&fmtp_cmd.to_string())
                .await
            {
                Ok(resp) => {
                    if let Err(e) = resp.into_result() {
                        warn!(
                            "{}: uuid_setvar {} failed: {}",
                            short,
                            ChannelVariable::RtpForceAudioFmtp,
                            e
                        );
                    } else {
                        info!(
                            "{}: {} set: {}",
                            short,
                            ChannelVariable::RtpForceAudioFmtp,
                            fmtp,
                        );
                    }
                }
                Err(e) => warn!(
                    "{}: api error setting {}: {}",
                    short,
                    ChannelVariable::RtpForceAudioFmtp,
                    e
                ),
            }
        }
    }

    // If you were originating a brand new call with this codec string instead of
    // live-updating an answered one, the delivery path -- and its escaping rule --
    // is completely different. An inline `{var=value}` block is parsed by
    // switch_separate_string on a bare comma, not by uuid_setvar's space/quote
    // rules, so every comma inside the value must be backslash-escaped by hand:
    //
    //   originate {absolute_codec_string=PCMU\,PCMA\,G722}sofia/gateway/gw1/1234 &park()
    //
    // Drop the backslashes and the brace parser still succeeds -- it just silently
    // stores only "PCMU" as the value, because the unescaped commas end the
    // variable assignment early. There is no error, no warning, just a channel
    // that negotiates one codec instead of three. This is why Originate's own
    // Variables builder (see examples/originate_examples.rs) escapes commas for
    // you rather than leaving it to string formatting.
    let correct_inline = format!(
        "{{{}={}}}",
        ChannelVariable::AbsoluteCodecString,
        codec_string
            .to_string()
            .replace(',', "\\,")
    );
    let broken_inline = format!(
        "{{{}={}}}",
        ChannelVariable::AbsoluteCodecString,
        codec_string
    );
    println!(
        "{}: correct inline originate form: {}",
        short, correct_inline
    );
    println!(
        "{}: WITHOUT escaping (only the first codec survives): {}",
        short, broken_inline
    );
}

/// Escape `'` and `\` in a value before wrapping it in single quotes for `uuid_setvar`.
///
/// `uuid_setvar` uses `cleanup_separated_string` with a space delimiter.
/// That function processes `\` escapes even inside a `'...'` region:
/// - `\'` prevents the `'` from toggling the quoting state (argument boundary corruption).
/// - `\\` prevents the `\` from consuming the next character.
fn escape_for_setvar(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\'' => out.push_str("\\'"),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out
}
