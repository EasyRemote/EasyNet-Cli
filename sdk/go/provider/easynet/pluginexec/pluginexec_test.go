package pluginexec

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"strings"
	"testing"
)

func TestServeIOWritesResultFrame(t *testing.T) {
	var output bytes.Buffer
	err := ServeIO(
		context.Background(),
		bytes.NewBufferString(testFrame()+"\n"),
		&output,
		func(_ context.Context, invocation SidecarInvocation) (any, error) {
			if invocation.CallID != "call-1" {
				t.Fatalf("call id = %q", invocation.CallID)
			}
			if invocation.Args["message"] != "hello" {
				t.Fatalf("args = %#v", invocation.Args)
			}
			return map[string]any{
				"ok":        true,
				"message":   invocation.Args["message"],
				"nonce_len": len(invocation.InvocationNonce),
			}, nil
		},
	)
	if err != nil {
		t.Fatalf("ServeIO: %v", err)
	}
	var decoded map[string]any
	if err := json.Unmarshal(output.Bytes(), &decoded); err != nil {
		t.Fatalf("decode output: %v", err)
	}
	if decoded["type"] != "result" || decoded["call_id"] != "call-1" {
		t.Fatalf("unexpected response: %#v", decoded)
	}
	value := decoded["value"].(map[string]any)
	if value["message"] != "hello" || value["nonce_len"] != float64(4) {
		t.Fatalf("unexpected value: %#v", value)
	}
}

func TestServeIOWritesErrorFrameForHandlerFailure(t *testing.T) {
	var output bytes.Buffer
	err := ServeIO(
		context.Background(),
		bytes.NewBufferString(testFrame()+"\n"),
		&output,
		func(context.Context, SidecarInvocation) (any, error) {
			return nil, errors.New("boom")
		},
	)
	if err != nil {
		t.Fatalf("ServeIO: %v", err)
	}
	var decoded map[string]any
	if err := json.Unmarshal(output.Bytes(), &decoded); err != nil {
		t.Fatalf("decode output: %v", err)
	}
	if decoded["type"] != "error" || decoded["call_id"] != "call-1" || decoded["message"] != "boom" {
		t.Fatalf("unexpected response: %#v", decoded)
	}
}

func TestServeIOWritesErrorFrameForProtocolFailure(t *testing.T) {
	var output bytes.Buffer
	err := ServeIO(
		context.Background(),
		bytes.NewBufferString(`{"type":"stream_open","call_id":"call-1","invocation":{}}`+"\n"),
		&output,
		func(context.Context, SidecarInvocation) (any, error) {
			t.Fatal("handler must not run for invalid frame")
			return nil, nil
		},
	)
	if err != nil {
		t.Fatalf("ServeIO: %v", err)
	}
	var decoded map[string]any
	if err := json.Unmarshal(output.Bytes(), &decoded); err != nil {
		t.Fatalf("decode output: %v", err)
	}
	if decoded["type"] != "error" || decoded["call_id"] != "call-1" {
		t.Fatalf("unexpected response: %#v", decoded)
	}
}

func TestServeIORejectsRetiredTupleAliases(t *testing.T) {
	var output bytes.Buffer
	frame := `{"type":"invoke","call_id":"call-1","invocation":{"caller_ura":"easynet:///r/hub/user/alice","caller":"easynet:///r/hub/user/bob","callee_ura":"easynet:///r/hub/device/provider","ability_ura":"demo.echo","subject_ura":"easynet:///r/hub/resource/demo","invocation_nonce":[1,2,3,4],"args":{}}}`
	err := ServeIO(
		context.Background(),
		bytes.NewBufferString(frame+"\n"),
		&output,
		func(context.Context, SidecarInvocation) (any, error) {
			t.Fatal("handler must not run for retired aliases")
			return nil, nil
		},
	)
	if err != nil {
		t.Fatalf("ServeIO: %v", err)
	}
	var decoded map[string]any
	if err := json.Unmarshal(output.Bytes(), &decoded); err != nil {
		t.Fatalf("decode output: %v", err)
	}
	if decoded["type"] != "error" || decoded["call_id"] != "call-1" {
		t.Fatalf("unexpected response: %#v", decoded)
	}
	if message, _ := decoded["message"].(string); !strings.Contains(message, "retired") {
		t.Fatalf("unexpected error message: %#v", decoded)
	}
}

func TestServeIORejectsUnknownInvocationFields(t *testing.T) {
	var output bytes.Buffer
	frame := `{"type":"invoke","call_id":"call-1","invocation":{"caller_ura":"easynet:///r/hub/user/alice","callee_ura":"easynet:///r/hub/device/provider","ability_ura":"demo.echo","subject_ura":"easynet:///r/hub/resource/demo","invocation_nonce":[1,2,3,4],"descriptor_ref":"legacy-provider-leak","args":{}}}`
	err := ServeIO(
		context.Background(),
		bytes.NewBufferString(frame+"\n"),
		&output,
		func(context.Context, SidecarInvocation) (any, error) {
			t.Fatal("handler must not run for unknown invocation fields")
			return nil, nil
		},
	)
	if err != nil {
		t.Fatalf("ServeIO: %v", err)
	}
	var decoded map[string]any
	if err := json.Unmarshal(output.Bytes(), &decoded); err != nil {
		t.Fatalf("decode output: %v", err)
	}
	if decoded["type"] != "error" || decoded["call_id"] != "call-1" {
		t.Fatalf("unexpected response: %#v", decoded)
	}
	if message, _ := decoded["message"].(string); !strings.Contains(message, "canonical invocation frame") {
		t.Fatalf("unexpected error message: %#v", decoded)
	}
}

func testFrame() string {
	return `{"type":"invoke","call_id":"call-1","invocation":{"caller_ura":"easynet:///r/hub/user/alice","callee_ura":"easynet:///r/hub/device/provider","ability_ura":"demo.echo","subject_ura":"easynet:///r/hub/resource/demo","invocation_nonce":[1,2,3,4],"causal_context":{"root":true},"args":{"message":"hello"}}}`
}
