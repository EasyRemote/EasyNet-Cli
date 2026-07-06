package easynet

import (
	"encoding/json"
	"strings"
	"testing"
)

func TestInvokeRemoteFacadeMarshalsRequestThroughAxon(t *testing.T) {
	raw, err := MarshalInvokeRemoteUpRequest(InvokeRemoteUpRequest{
		SubjectDevice:       "easynet:///r/acme/device/node-1",
		SubjectURA:          "easynet:///r/acme/resource/fs/tmp",
		AbilityURA:          "easynet:///r/acme/ability/device.node-1.skill.list",
		Args:                JSONByteSlice([]byte(`{}`)),
		ArgsContentEnvelope: PlainInvokeRemoteContentEnvelope(),
	})
	if err != nil {
		t.Fatalf("MarshalInvokeRemoteUpRequest: %v", err)
	}
	var frame map[string]any
	if err := json.Unmarshal(raw, &frame); err != nil {
		t.Fatalf("decode request: %v", err)
	}
	if frame["type"] != InvokeRemoteRequestType {
		t.Fatalf("type = %v", frame["type"])
	}
	if _, ok := frame["ability"]; ok {
		t.Fatal("facade emitted legacy ability field")
	}
	if frame["ability_ura"] == "" {
		t.Fatal("facade omitted ability_ura")
	}
}

func TestInvokeRemoteFacadeRejectsOutOfRangePayloadByte(t *testing.T) {
	_, err := DecodeInvokeRemoteDown([]byte(`{"type":"result","payload":[300],"error":null}`))
	if err == nil {
		t.Fatal("DecodeInvokeRemoteDown accepted out-of-range payload byte")
	}
	if !strings.Contains(err.Error(), "out of byte range") {
		t.Fatalf("unexpected error: %v", err)
	}
}
