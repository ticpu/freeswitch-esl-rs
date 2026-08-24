# Evaluating feature and bug requests

Read this before answering an incoming `FEATURE-REQUEST-*.md` or `BUG-*.md`.
Those files are in `.git/info/exclude` and never committed; the answer replaces
the request in the same file, so the file ends up holding the decision rather
than the ask.

## Is it already fixed?

Check before evaluating anything else. A request written against a released
version may describe something the tip already handles — search `git log` for
the symbol it names, not for its prose.

Do not answer "which published versions are affected" from git tags. Tags here
do not cover every published version and some are not ancestors of `master`, so
the tag graph cannot decide it. Unpack the published `.crate` artifacts and
compare the function itself.

Yanking a published version is never done without asking; say the range and let
the maintainer decide.

## Verify the evidence, do not inherit it

A request's cited line numbers are from whatever checkout its author had. Grep
`$FREESWITCH_SOURCE` for the literal string or the symbol and rebuild the site
list yourself — every claim about which events carry a header has so far been
undercounted, and a doc comment written from the request's table inherits the
error. If a request names sites in one module, grep the rest of the tree too:
core sometimes synthesises the same event a module does.

Reading `$FREESWITCH_SOURCE` at a cited line answers a different question than
the pinned reference does — the checkout is not parked on the pin. See the
source-ref rules in `CLAUDE.md`.

Distinguish an emitting site from a lookalike. A string can appear as an
xml-binding params key, a command argument, or an XML/JSON dump field without
ever reaching the wire as an event header.

## Does the string belong in this crate?

For any request adding a variant to a wire-name enum (`EventHeader`,
`ChannelVariable`, `SofiaVariable`, `EslEventType`, `SofiaEventSubclass`),
establish where the string comes from:

- **FreeSWITCH core or a bundled module emits it on the wire** — it belongs
  here.
- **A deployment's dialplan or config sets it** (`set foo=…` ahead of an
  application) — it belongs in the consumer's own enum, built with the same
  macro over the `VariableName` trait; `typed-wrapper-design.md` shows the
  shape. This crate models what the switch ships, not what an install
  configures. A consumer's pre-commit hook forbidding raw header strings is not
  an argument for admitting one; that hook is the consumer's, and its own enum
  satisfies it.
- **Only a dump, a command argument or a params event carries it** — not a wire
  header or channel variable, so not a variant.

A new `SofiaVariable` costs more than the variant: the match naming the SIP
header behind each one takes no catch-all arm, so the crate does not build
until the addition is classified. Weigh that when the request is for a variable
rather than a header, and read the rationale for the missing arm before adding
one.

## Does the shape belong here?

Requests usually propose a shape along with the ask. Judge it against the
crate's own rules rather than accepting it:

- Where the switch spells one logical field more than one way, each spelling is
  its own variant and a `HeaderLookup` accessor may union them. Never fold the
  spellings in the store — `case_alias_key()` excludes underscore keys on
  purpose, and folding would alias every dash/underscore pair crate-wide.
- A union accessor is not a forbidden silent fallback when the alternatives are
  mutually exclusive on the wire and neither is an error; `unique_id()` is the
  precedent. It becomes one the moment it hides a failure.
- Do not ask the caller to pick a variant by event family unless the mapping is
  exceptionless. Check for the exception before designing around the rule — one
  event spelling a field the other way turns a documented rule into a silent
  miss.
- A new public type to report which of several equivalent keys matched is
  over-modelling when the caller can already derive it and the typed variants
  expose it.

## Semver

`#[non_exhaustive]` plus tail append plus additive trait default methods is a
minor bump. Widening an existing default method's behaviour is not a break, but
it changes results for every implementor and belongs in the release changelog
explicitly.

Anything that cannot ship additively goes in `docs/next-major.md`, which is for
breaking changes only — an additive request does not belong there just because
it is deferred.

## Naming and doc comments

Signature and casing conventions are in `CLAUDE.md`. What triage adds:

Name a variant after what distinguishes it, not after a category both
candidates share. A name a reader can pick absent-mindedly is the failure the
request is usually reporting.

Doc comments name the emitting event family, never a site list — a family
survives an upstream refactor and a line-numbered site list does not. Where
this crate already has vocabulary for the family (`SofiaEventSubclass` group
constants), use it.

State when a header travels alone. Headers that usually appear together are not
a block, and documenting them as one produces a consumer that reads a field
that is absent.

## The answer

Say which claims held and which did not, give a ranked recommendation with the
naming and doc wording you would actually use, name the regression test, and
list what the request missed. A regression test earns its place here by failing
loudly on a later "tidy these into one" refactor — the failure mode being
reported is silent, so the test is what makes it audible.

Record a design decision in `docs/design-rationale.md` only once the shape is
built, and only for the standing constraint a later change could violate — not
for the request, the correction, or the site tables.
