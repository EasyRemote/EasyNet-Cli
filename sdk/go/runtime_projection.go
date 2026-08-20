package easynet

import (
	"encoding/json"
	"fmt"
)

func rejectUnknownRuntimeProjectionFields(raw []byte, projection string, allowedFields ...string) error {
	var fields map[string]json.RawMessage
	if err := json.Unmarshal(raw, &fields); err != nil {
		return invalidRuntimePayload(fmt.Sprintf("decode %s JSON: %v", projection, err), err)
	}
	allowed := make(map[string]struct{}, len(allowedFields))
	for _, field := range allowedFields {
		allowed[field] = struct{}{}
	}
	for field := range fields {
		if _, ok := allowed[field]; !ok {
			return invalidRuntimePayload(projection+" contains noncanonical field "+field, nil)
		}
	}
	return nil
}
