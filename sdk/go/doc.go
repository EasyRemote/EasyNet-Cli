// Package easynet provides the Go binding for the canonical runtime SDK.
//
// The package is product-neutral. Downstream applications own their workflows,
// DTOs, account policy, HTTP routes and UI projections outside this module;
// they lower those workflows through the generic runtime concepts exported here.
//
// Public APIs expose runtime-host lifecycle, Addressing, Invocation, signing,
// PrincipalLifecycle, Directory, receipts, runtime events, administration,
// health and typed errors. They do not expose Axon SDK packages, generated
// protobufs, C ABI handles, runtime-host internals or product profile clients.
package easynet
