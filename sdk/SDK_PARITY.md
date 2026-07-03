# SDK Parity

Parity is measured by behavior and public state transitions, not by identical
method spelling.

## Language Tiers

| Language | Tier | Primary consumer | Current status |
| --- | --- | --- | --- |
| Rust | P0 | native SDK core and FFI implementation | partial Runtime Core |
| C ABI | P0 | language binding projection | partial ABI v4 Runtime Core |
| Go | P0 | EasyNet backend/Hub | placeholder |
| Python | P0 | EasyRemote | placeholder |
| Node/TypeScript | P1 | desktop tools and extensions | placeholder |
| Java/JVM | P1 | enterprise and Android-adjacent integrations | placeholder |
| Swift | P1 | macOS/iOS-adjacent clients | placeholder |

## Capability Matrix

| Capability | Rust | C ABI | Go | Python | Node | Java | Swift |
| --- | --- | --- | --- | --- | --- | --- | --- |
| ABI/version discovery | partial | partial | gap | gap | gap | gap | gap |
| daemon start/attach/discover/stop/detach | partial | partial | gap | gap | gap | gap | gap |
| runtime health | partial | partial | gap | gap | gap | gap | gap |
| complete invocation draft | partial | builder handles partial | gap | gap | gap | gap | gap |
| prepare/sign/submit | partial | handle observation partial | gap | gap | gap | gap | gap |
| unary invoke | partial | partial | gap | gap | gap | gap | gap |
| stream | existing dispatch | existing dispatch | gap | gap | gap | gap | gap |
| bidi | existing dispatch | existing dispatch | gap | gap | gap | gap | gap |
| directory + identity | gap | gap | gap | gap | gap | gap | gap |
| receipt | summary only | summary only | gap | gap | gap | gap | gap |
| publication | gap | gap | gap | gap | gap | gap | gap |
| host binding | gap | gap | gap | gap | gap | gap | gap |
| mission | gap | gap | gap | gap | gap | gap | gap |
| admin + gateway | gap | gap | gap | gap | gap | gap | gap |
| events | gap | gap | gap | gap | gap | gap | gap |
| surface | gap | gap | gap | gap | gap | gap | gap |
| compatibility | gap | gap | gap | gap | gap | gap | gap |
| conformance runner | scaffold | scaffold | gap | gap | gap | gap | gap |

## Known Gaps

- C ABI now exposes invocation builder handles and submitted InvocationHandle
  await/cancel/events/free handles for unary submit; live event streaming and
  receipt fetch/verify remain incomplete.
- Directory, identity, receipt fetch/verify, publication, host binding,
  mission, admin/gateway, events, surface, compatibility, and convenience
  wrappers are schema/conformance scaffolds only.
- Go and Python packages need real Runtime Core facades before backend or
  EasyRemote cutover.
- Stream and bidi terminal events need schema-backed close/cancel/terminal
  parity before P1 language release.

## Stability Levels

| Level | Meaning |
| --- | --- |
| scaffold | files, schemas, and conformance case names exist |
| partial | code exists for part of the object family and is covered by narrow tests |
| profile-ready | all public methods for the profile pass conformance in one language |
| language-stable | all declared profiles pass conformance for that language |
| cutover-ready | product import bans and route/facade smokes pass |

No current language is `language-stable`.
