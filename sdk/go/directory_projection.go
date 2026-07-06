package easynet

import (
	"encoding/json"
	"fmt"
	"math"
	"strconv"
)

// DirectoryReadModelEnumLookup resolves a daemon directory read-model enum
// ordinal into the stable presentation name exposed by SDK DTOs.
type DirectoryReadModelEnumLookup func(int) (string, bool)

// DirectoryReadModelEnumProjector normalizes schemaless directory read-model
// enum values without leaking protocol SDK enum packages to product repos.
type DirectoryReadModelEnumProjector struct {
	lookup DirectoryReadModelEnumLookup
}

func NewDirectoryReadModelEnumProjector(lookup DirectoryReadModelEnumLookup) DirectoryReadModelEnumProjector {
	return DirectoryReadModelEnumProjector{lookup: lookup}
}

func (p DirectoryReadModelEnumProjector) Normalize(value any) string {
	if value == nil {
		return ""
	}
	switch typed := value.(type) {
	case string:
		return typed
	case json.Number:
		return p.normalizeJSONNumber(typed)
	case float64:
		return p.normalizeFloat64(typed)
	case float32:
		return p.normalizeFloat64(float64(typed))
	case int:
		return p.normalizeOrdinal(typed)
	case int8:
		return p.normalizeSignedOrdinal(int64(typed))
	case int16:
		return p.normalizeSignedOrdinal(int64(typed))
	case int32:
		return p.normalizeSignedOrdinal(int64(typed))
	case int64:
		return p.normalizeSignedOrdinal(typed)
	case uint:
		return p.normalizeUnsignedOrdinal(uint64(typed))
	case uint8:
		return p.normalizeUnsignedOrdinal(uint64(typed))
	case uint16:
		return p.normalizeUnsignedOrdinal(uint64(typed))
	case uint32:
		return p.normalizeUnsignedOrdinal(uint64(typed))
	case uint64:
		return p.normalizeUnsignedOrdinal(typed)
	default:
		return fmt.Sprint(value)
	}
}

func (p DirectoryReadModelEnumProjector) normalizeJSONNumber(value json.Number) string {
	if ordinal, err := value.Int64(); err == nil {
		return p.normalizeSignedOrdinal(ordinal)
	}
	if asFloat, err := value.Float64(); err == nil {
		return p.normalizeFloat64(asFloat)
	}
	return value.String()
}

func (p DirectoryReadModelEnumProjector) normalizeFloat64(value float64) string {
	if math.IsNaN(value) || math.IsInf(value, 0) || math.Trunc(value) != value {
		return strconv.FormatFloat(value, 'f', -1, 64)
	}
	if value > float64(maxDirectoryReadModelEnumInt()) || value < float64(minDirectoryReadModelEnumInt()) {
		return strconv.FormatFloat(value, 'f', -1, 64)
	}
	return p.normalizeOrdinal(int(value))
}

func (p DirectoryReadModelEnumProjector) normalizeSignedOrdinal(value int64) string {
	if value > int64(maxDirectoryReadModelEnumInt()) || value < int64(minDirectoryReadModelEnumInt()) {
		return strconv.FormatInt(value, 10)
	}
	return p.normalizeOrdinal(int(value))
}

func (p DirectoryReadModelEnumProjector) normalizeUnsignedOrdinal(value uint64) string {
	if value > uint64(maxDirectoryReadModelEnumInt()) {
		return strconv.FormatUint(value, 10)
	}
	return p.normalizeOrdinal(int(value))
}

func (p DirectoryReadModelEnumProjector) normalizeOrdinal(value int) string {
	if p.lookup != nil {
		if name, ok := p.lookup(value); ok {
			return name
		}
	}
	return strconv.Itoa(value)
}

func maxDirectoryReadModelEnumInt() int {
	return int(^uint(0) >> 1)
}

func minDirectoryReadModelEnumInt() int {
	return -maxDirectoryReadModelEnumInt() - 1
}

func NormalizeDirectoryReadModelEnum(value any, lookup DirectoryReadModelEnumLookup) string {
	return NewDirectoryReadModelEnumProjector(lookup).Normalize(value)
}

func NormalizeDirectoryNodeState(value any) string {
	return NormalizeDirectoryReadModelEnum(value, directoryNodeStateName)
}

func NormalizeDirectoryTrustLevel(value any) string {
	return NormalizeDirectoryReadModelEnum(value, directoryTrustLevelName)
}

func directoryNodeStateName(value int) (string, bool) {
	switch value {
	case 0:
		return "UNSPECIFIED", true
	case 1:
		return "JOINING", true
	case 2:
		return "PROBATION", true
	case 3:
		return "HEALTHY", true
	case 4:
		return "SUSPECT", true
	case 5:
		return "QUARANTINED", true
	case 6:
		return "DRAINING", true
	case 7:
		return "REMOVED", true
	default:
		return "", false
	}
}

func directoryTrustLevelName(value int) (string, bool) {
	switch value {
	case 0:
		return "UNSPECIFIED", true
	case 1:
		return "UNTRUSTED", true
	case 2:
		return "PROBATION", true
	case 3:
		return "STANDARD", true
	case 4:
		return "ELEVATED", true
	case 5:
		return "PRIVILEGED", true
	default:
		return "", false
	}
}
