package easynet

import (
	"encoding/base64"
	"encoding/json"
	"fmt"
)

func invocationDraftArgumentBytes(draft InvocationDraft) ([]byte, error) {
	if draft.ArgumentsBase64() != "" {
		decoded, err := base64.StdEncoding.Strict().DecodeString(draft.ArgumentsBase64())
		if err != nil {
			return nil, invalidRuntimePayload(fmt.Sprintf("decode arguments_base64: %v", err), err)
		}
		return decoded, nil
	}
	raw, err := json.Marshal(draft.JSONArgs())
	if err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("encode args JSON: %v", err), err)
	}
	return raw, nil
}
