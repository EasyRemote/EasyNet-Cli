// Package directorycore defines the provider-neutral Directory core model.
package directorycore

type ResolveKind string

const (
	ResolveRoute             ResolveKind = "RESOLVE_TYPE_ROUTE"
	ResolveListing           ResolveKind = "RESOLVE_TYPE_DIRECTORY_LISTING"
	ResolveCanonicalIdentity ResolveKind = "RESOLVE_TYPE_CANONICAL_IDENTITY"
	ResolveOwner             ResolveKind = "RESOLVE_TYPE_OWNER"
)
