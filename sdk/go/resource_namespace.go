package easynet

import (
	"strings"

	axonsdk "axon.run/sdk/go/axon"
)

var runtimeResourceNamespaces = map[ResourceNamespace]struct{}{
	ResourceNamespaceFS:      {},
	ResourceNamespaceProcess: {},
	ResourceNamespacePTY:     {},
	ResourceNamespaceShell:   {},
	ResourceNamespaceHTTP:    {},
}

func isRuntimeResourceNamespace(namespace string) bool {
	_, ok := runtimeResourceNamespaces[ResourceNamespace(namespace)]
	return ok
}

// runtimeResourceURA projects an SDK runtime resource namespace into Axon's
// generic opaque resource path. Axon owns URA grammar; the SDK owns the finite
// runtime namespace vocabulary exposed by ResourceNamespace.
func runtimeResourceURA(realm, userID, namespace, path string) string {
	if !isRuntimeResourceNamespace(namespace) {
		return ""
	}
	clean := strings.TrimPrefix(path, "/")
	resourcePath := namespace
	if clean != "" {
		resourcePath += "/" + clean
	}
	return axonsdk.ResourceDotURA(realm, userID, resourcePath)
}

func projectRuntimeResourcePath(
	kind axonsdk.URAKind,
	path string,
) (ResourceNamespace, string) {
	if kind != axonsdk.URAKindResource {
		return "", path
	}
	namespace, remainder, hasPath := strings.Cut(path, "/")
	if !isRuntimeResourceNamespace(namespace) {
		return "", path
	}
	if !hasPath {
		return ResourceNamespace(namespace), ""
	}
	return ResourceNamespace(namespace), remainder
}
