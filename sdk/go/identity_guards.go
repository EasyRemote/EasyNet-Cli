package easynet

import "strings"

const allZeroPrincipalID = "00000000-0000-0000-0000-000000000000"

func containsAllZeroPrincipal(value string) bool {
	clean := strings.ToLower(strings.TrimSpace(value))
	return strings.Contains(clean, allZeroPrincipalID)
}
