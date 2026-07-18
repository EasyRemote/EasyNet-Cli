package easynet

import (
	"strings"

	axonsdk "axon.run/sdk/go/axon"
)

var productResourceNamespaces = map[ResourceNamespace]struct{}{
	ResourceNamespaceFS:      {},
	ResourceNamespaceProcess: {},
	ResourceNamespacePTY:     {},
	ResourceNamespaceShell:   {},
	ResourceNamespaceHTTP:    {},
}

func isProductResourceNamespace(namespace string) bool {
	_, ok := productResourceNamespaces[ResourceNamespace(namespace)]
	return ok
}

// productResourceURA projects EasyNet's provider namespace into Axon's
// generic opaque resource path. Axon owns URA grammar; the product owns the
// finite namespace vocabulary.
func productResourceURA(realm, userID, namespace, path string) string {
	if !isProductResourceNamespace(namespace) {
		return ""
	}
	clean := strings.TrimPrefix(path, "/")
	resourcePath := namespace
	if clean != "" {
		resourcePath += "/" + clean
	}
	return axonsdk.ResourceDotURA(realm, userID, resourcePath)
}

func projectProductResourcePath(
	kind axonsdk.URAKind,
	path string,
) (ResourceNamespace, string) {
	if kind != axonsdk.URAKindResource {
		return "", path
	}
	namespace, remainder, hasPath := strings.Cut(path, "/")
	if !isProductResourceNamespace(namespace) {
		return "", path
	}
	if !hasPath {
		return ResourceNamespace(namespace), ""
	}
	return ResourceNamespace(namespace), remainder
}
