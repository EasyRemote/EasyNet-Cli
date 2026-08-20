# Decisions

## DEC-1: Use `@runtime/sdk`

Use `@runtime/sdk` for the private Node package root. It matches the Java
`run.runtime` namespace and the Swift `RuntimeSDK` product without introducing
EasyNet, EasyRemote, daemon, provider, or Axon product branding at the canonical
SDK root.
