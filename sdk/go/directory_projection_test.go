package easynet

import (
	"encoding/json"
	"strconv"
	"testing"
)

func TestDirectoryReadModelEnumProjectorNormalizesWireShapes(t *testing.T) {
	projector := NewDirectoryReadModelEnumProjector(func(value int) (string, bool) {
		switch value {
		case 3:
			return "HEALTHY", true
		case 6:
			return "DRAINING", true
		default:
			return "", false
		}
	})

	cases := []struct {
		name  string
		value any
		want  string
	}{
		{"nil", nil, ""},
		{"string-passthrough", "HEALTHY", "HEALTHY"},
		{"float64-known", float64(6), "DRAINING"},
		{"float64-unknown", float64(99), "99"},
		{"int-known", 3, "HEALTHY"},
		{"int64-unknown", int64(77), "77"},
		{
			"uint64-out-of-int-range",
			uint64(maxDirectoryReadModelEnumInt()) + 1,
			strconv.FormatUint(uint64(maxDirectoryReadModelEnumInt())+1, 10),
		},
		{"json-number-known", json.Number("6"), "DRAINING"},
		{"json-number-float", json.Number("6.0"), "DRAINING"},
		{"json-number-non-integral", json.Number("6.5"), "6.5"},
		{"json-number-invalid", json.Number("not-a-number"), "not-a-number"},
		{"float64-non-integral", float64(6.5), "6.5"},
		{"unexpected-type", true, "true"},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := projector.Normalize(tc.value); got != tc.want {
				t.Fatalf("Normalize(%#v) = %q, want %q", tc.value, got, tc.want)
			}
		})
	}
}

func TestDirectoryNodeAndTrustProjectionNames(t *testing.T) {
	if got := NormalizeDirectoryNodeState(float64(6)); got != "DRAINING" {
		t.Fatalf("node state = %q, want DRAINING", got)
	}
	if got := NormalizeDirectoryTrustLevel(5); got != "PRIVILEGED" {
		t.Fatalf("trust level = %q, want PRIVILEGED", got)
	}
	if got := NormalizeDirectoryNodeState(404); got != "404" {
		t.Fatalf("unknown node state = %q, want 404", got)
	}
	if got := NormalizeDirectoryTrustLevel("ELEVATED"); got != "ELEVATED" {
		t.Fatalf("string trust level = %q, want ELEVATED", got)
	}
}
