# Invariants

1. EasyNet routable identities and addresses are URAs.
2. A transport endpoint string may be operator-visible, but SDK/API names must
   use URA for EasyNet identities.
3. Unknown retired aliases remain rejected; the SDK must not add compatibility
   fields for retired address spellings.
4. HTTP framework request-target calls and historical design documents are outside
   this implementation cleanup.
5. No behavior change is allowed beyond symbol/comment/error terminology.
6. A value returned by a semantic `*_ura` factory is a URA even when it is passed
   to a dereference helper; local names must preserve that ownership language.
