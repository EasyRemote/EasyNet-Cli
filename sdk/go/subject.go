package easynet

import (
	"fmt"
	"strings"
)

const sdkURASchemeRoot = "easynet:///r/"

// DescriptorBoundResourceSubjectURA returns the canonical resource URA shape
// used when a backend business subject must be represented as a
// descriptor-bound Invocation entity subject.
func DescriptorBoundResourceSubjectURA(realm string, ownerID string, path string) (string, error) {
	realm = strings.TrimSpace(realm)
	ownerID = strings.TrimSpace(ownerID)
	path = strings.TrimSpace(strings.TrimPrefix(path, "/"))
	if realm == "" {
		return "", invalidProfilePayload(directoryIdentityProfile, "descriptor-bound subject realm is required", nil)
	}
	if ownerID == "" {
		return "", invalidProfilePayload(directoryIdentityProfile, "descriptor-bound subject owner_id is required", nil)
	}
	if path == "" {
		return "", invalidProfilePayload(directoryIdentityProfile, "descriptor-bound subject path is required", nil)
	}
	if strings.Contains(realm, "/") || strings.Contains(ownerID, "/") {
		return "", invalidProfilePayload(directoryIdentityProfile, "descriptor-bound subject realm and owner_id must be URA path segments", nil)
	}
	if hasEmptyPathSegment(path) {
		return "", invalidProfilePayload(directoryIdentityProfile, "descriptor-bound subject path must not contain empty segments", nil)
	}
	return fmt.Sprintf("%s%s/resource/%s/%s", sdkURASchemeRoot, realm, ownerID, path), nil
}

func hasEmptyPathSegment(value string) bool {
	for _, segment := range strings.Split(value, "/") {
		if segment == "" {
			return true
		}
	}
	return false
}
