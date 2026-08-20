# Loopback bowout: what survives, and what lies

mod_loopback places a channel pair between a caller and the dialplan, then removes
itself from the media path once it can connect the two real channels directly. The leg
that leaves is said to *resign*, or *bow out*. It emits `CHANNEL_HANGUP_COMPLETE` for a
call that is still up.

This is the doc for the part that bites: after a resignation, several things that look
like they describe the channel emitting them actually describe the channel that left.
Which ones depends on *how* it resigned.

## Channel names

A loopback pair is named from the destination, with a leg suffix appended:

```
loopback/9199/test        dialled  ->  loopback/9199-a  and  loopback/9199-b
loopback/app=bridge:x     dialled  ->  loopback/bridge-a and loopback/bridge-b
```

The context and dialplan segments are stripped before the name is built, and the
`app=` form is reduced to the application token alone. The suffix is lowercase `-a` or
`-b` and is always present, so the extension itself may end in `-a` without ambiguity:
strip exactly one suffix.

The A leg is the one an `originate` returns; the B leg is routed into the dialplan.
Each carries the partner's UUID in `other_loopback_leg_uuid`.

## Two resignation paths, and they are not interchangeable

**Execute-time.** Armed with `loopback_bowout_on_execute`, or triggered by an
application flagged `SAF_NO_LOOPBACK`. The leg finds a real channel through
`find_non_loopback_bridge`, hands its remaining work over with
`switch_channel_caller_extension_masquerade`, and hangs up. The marker value is
`bowout`.

**Frame-count.** Fires once both legs are bridged to non-loopback channels and
answered, unless vetoed by `loopback_bowout` being false. The two real channels are
spliced with `switch_ivr_uuid_bridge` and both loopback legs are marked. The marker
value is `bridge`.

Both mean the same thing to a consumer — the leg is gone, the call is not. Branch on
the token to log, never to decide whether a resignation happened. A third token would
still be one.

The paths differ in what they leave behind, and that difference is the rest of this
doc: **only the execute-time path masquerades**, so only it moves state onto the
survivor.

## Variable scope

Three hops, each carrying or dropping different things. Getting one confused for
another is the usual cause of "the variable was set but the far end never saw it".

### 1. Originate to both legs

mod_loopback replays the whole variable event onto the B leg, so anything set in the
originate lands on both legs. `{}` and `[]` do not scope a loopback pair — neither form
can address one leg. Per-leg isolation comes only from nesting dial strings.

Caller ID lands on the B leg. Never read caller ID off the A leg, whose outbound
profile is synthetic.

### 2. Resigning leg to survivor — execute-time path only

`switch_channel_caller_extension_masquerade` copies the resigning leg's remaining
applications **and every one of its channel variables** onto the target, skipping only
names listed in `attended_transfer_no_copy`. That list is the sole lever for keeping a
variable off the survivor.

Everything the loopback leg carried is therefore on the real channel afterwards,
including the variables mod_loopback itself sets to describe the resignation. The
marker is set *before* the masquerade runs, so the survivor inherits it.

The frame-count path masquerades nothing; its markers stay on the loopback legs.

This asymmetry is why no variable test identifies a loopback channel. On one path the
variables are truthful and on the other they describe a channel that no longer exists,
and the variables themselves cannot tell you which path you are on.

### 3. Survivor to the far end

Surviving is not the same as reaching. A bridged leg inherits only **exported**
variables, so a value that survived the masquerade still appears nowhere in the
outbound INVITE unless it was exported or placed in the bridge dial string.

A header that must reach the far switch therefore needs the export even though the
variable is demonstrably present locally — the two facts look contradictory in a trace
and are not.

## The caller profile rides along too

On the execute-time path the resigning leg's caller profile is cloned onto the
survivor. Every `Caller-*` event header is generated from the caller profile, so after
a bowout the surviving real channel reports the *loopback leg's* channel name, unique
ID and source under those headers.

Consequences worth stating plainly:

- `Caller-Channel-Name` on a real channel can read `loopback/...`.
- `Caller-Unique-ID` can name a leg that is gone, so correlating on it attributes a
  live channel's events to a dead one.
- `Caller-Source` names mod_loopback, which is why it does not answer "which module
  owns this channel" either.

The channel's own `Channel-Name` is not part of the caller profile and stays truthful.

## What is safe to key on

- **Resignation happened:** the presence of the marker variable, never its value.
- **This channel is a loopback:** the channel's own name. Not a variable, not any
  `Caller-*` field.
- **Which channel continues the call:** `loopback_bowout_other_uuid`. Not `${uuid}`,
  which on the survivor still reads the resigned leg's identifier.

The first two are only conclusive together. A marker on a channel whose name is not a
loopback name means the survivor inherited it, and tearing that channel down ends a
live call.

## Ordering guarantees nothing

On the execute-time path the resignation arrives **before** `CHANNEL_BRIDGE`, and may
arrive before the originate's own `+OK` — the reply travels on a different thread from
the session. On the frame-count path the bridge comes first.

So any rule of the form "the bowout happens after X" is wrong on one of the two paths.
Re-anchor when the marker arrives, and accept a late `+OK` afterwards.

## Direction is positional

The masquerade targets whatever `find_non_loopback_bridge` reaches, which depends on
where the loopback leg sits rather than on what the caller intended.

- Under a target extension the caller placed, it reaches the intended leg, and the
  callback loses a channel pair from the media path.
- Dialled as the far side of a bridge, it reaches the *other* party's leg instead,
  seizing a channel the caller never meant it to touch.

So arm the execute-time trigger only on a leg carrying your own target extension.
Left off entirely, the frame-count path still splices the real legs shortly after
answer.

## Related

- [originate-loopback-yaml.md](originate-loopback-yaml.md) — worked YAML examples for
  originating into a loopback, including a bowout.
- [live-test-switch.md](live-test-switch.md) — what the shared test switch provides and
  the session cost of a loopback pair.
