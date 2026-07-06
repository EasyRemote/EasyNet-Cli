package easynet

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"testing"
)

func TestWrapperRuntimeTransportBuildsFilesPutInvocation(t *testing.T) {
	identityTransport := newWrapperRuntimeIdentityTransport()
	identity, err := NewIdentityClient(identityTransport)
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	runtime, err := NewRuntimeClient(&compatibilityRuntimeInvokeTransport{outputJSON: wrapperRuntimeFilePutRawJSON})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeWrapperClient(runtime, identity)
	if err != nil {
		t.Fatalf("NewRuntimeWrapperClient: %v", err)
	}

	draft, err := client.BuildFileTransferInvocation(context.Background(), wrapperFilesPutRequest())
	if err != nil {
		t.Fatalf("BuildFileTransferInvocation: %v", err)
	}
	if draft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.alice.files.put@1.0.0" {
		t.Fatalf("descriptor ref = %q", draft.DescriptorRef())
	}
	args := draft.JSONArgs().(map[string]any)
	if args["filename"] != "brief.txt" ||
		args["bytes_b64"] != base64.StdEncoding.EncodeToString([]byte("hello")) ||
		args["content_type"] != "text/plain" {
		t.Fatalf("files.put args not normalized: %#v", args)
	}
	if _, ok := args["wrapper_kind"]; ok {
		t.Fatalf("wrapper-only field leaked into files.put args: %#v", args)
	}
	metadata := draft.Metadata()
	if metadata["profile"] != wrappersProfile ||
		metadata["system_ability"] != "alice.files.put" ||
		metadata["carrier_owner"] != "daemon_sdk" {
		t.Fatalf("metadata not normalized: %#v", metadata)
	}
	if len(identityTransport.seenBuildURA) != 1 || identityTransport.seenBuildURA[0]["ability_name"] != "alice.files.put" {
		t.Fatalf("ability URA was not delegated through identity client: %#v", identityTransport.seenBuildURA)
	}
}

func TestWrapperRuntimeTransportInvokesAndProjectsFilesPutOutput(t *testing.T) {
	identity, err := NewIdentityClient(newWrapperRuntimeIdentityTransport())
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	runtimeTransport := &compatibilityRuntimeInvokeTransport{outputJSON: wrapperRuntimeFilePutRawJSON}
	runtime, err := NewRuntimeClient(runtimeTransport)
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeWrapperClient(runtime, identity)
	if err != nil {
		t.Fatalf("NewRuntimeWrapperClient: %v", err)
	}

	record, err := client.TransferFile(context.Background(), wrapperFilesPutRequest())
	if err != nil {
		t.Fatalf("TransferFile: %v", err)
	}
	if record.FileRef != "easynet:///r/example/resource/alice.files/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" {
		t.Fatalf("file_ref = %q", record.FileRef)
	}
	if record.ContentHash == nil || *record.ContentHash != "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" {
		t.Fatalf("content_hash = %#v", record.ContentHash)
	}
	if record.SizeBytes == nil || *record.SizeBytes != 5 || record.ContentType != "text/plain" {
		t.Fatalf("record facts not projected: %#v", record)
	}
	args := runtimeTransport.seenDraft["args"].(map[string]any)
	if len(args) != 3 {
		t.Fatalf("files.put runtime args = %#v", args)
	}
}

func TestWrapperRuntimeTransportMapsTerminalFailure(t *testing.T) {
	identity, err := NewIdentityClient(newWrapperRuntimeIdentityTransport())
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	runtime, err := NewRuntimeClient(&compatibilityRuntimeInvokeTransport{fail: true})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeWrapperClient(runtime, identity)
	if err != nil {
		t.Fatalf("NewRuntimeWrapperClient: %v", err)
	}

	_, err = client.TransferFile(context.Background(), wrapperFilesPutRequest())
	if err == nil {
		t.Fatal("TransferFile succeeded, want failure")
	}
	if !IsCode(err, ErrAdmissionDenied) {
		t.Fatalf("error code = %v, want %s", err, ErrAdmissionDenied)
	}
}

func TestWrapperRuntimeTransportOpensTerminalStreamAndBidi(t *testing.T) {
	identity, err := NewIdentityClient(newWrapperRuntimeIdentityTransport())
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	var seenStreamDraft map[string]any
	var seenBidiDraft map[string]any
	var seenStreams []map[string]any
	runtime, err := NewRuntimeClient(RuntimeTransportFunc{
		OpenStreamFunc: func(ctx context.Context, draftJSON []byte) (StreamTransport, []byte, error) {
			if err := json.Unmarshal(draftJSON, &seenStreamDraft); err != nil {
				t.Fatalf("stream draft JSON: %v", err)
			}
			return &memoryStreamTransport{events: []string{
				`{"sequence":1,"event":"terminal","state":"Completed","terminal":true}`,
			}}, []byte(`{"stream_id":"wrapper-stream-1","state":"Opening","max_buffered_events":4}`), nil
		},
		OpenBidiFunc: func(ctx context.Context, draftJSON []byte, streamsJSON []byte) (BidiTransport, []byte, error) {
			if err := json.Unmarshal(draftJSON, &seenBidiDraft); err != nil {
				t.Fatalf("bidi draft JSON: %v", err)
			}
			if err := json.Unmarshal(streamsJSON, &seenStreams); err != nil {
				t.Fatalf("streams JSON: %v", err)
			}
			return &memoryBidiTransport{}, []byte(`{"session_id":"wrapper-bidi-1","state":"Open","max_buffered_frames":4}`), nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeWrapperClient(runtime, identity)
	if err != nil {
		t.Fatalf("NewRuntimeWrapperClient: %v", err)
	}

	assertStream := func(label string, open func() (*StreamHandle, error), wantDescriptor string) {
		t.Helper()
		stream, err := open()
		if err != nil {
			t.Fatalf("%s: %v", label, err)
		}
		if stream.StreamID() != "wrapper-stream-1" {
			t.Fatalf("%s stream id = %q", label, stream.StreamID())
		}
		if seenStreamDraft["descriptor_ref"] != wantDescriptor {
			t.Fatalf("%s descriptor = %#v", label, seenStreamDraft["descriptor_ref"])
		}
	}
	assertBidi := func(label string, open func([]BidiStreamDescriptor) (*BidiSession, error), wantDescriptor string) {
		t.Helper()
		session, err := open([]BidiStreamDescriptor{
			{StreamID: 1, ContentType: "application/json", Ordering: "ordered"},
		})
		if err != nil {
			t.Fatalf("%s: %v", label, err)
		}
		if session.SessionID() != "wrapper-bidi-1" {
			t.Fatalf("%s session id = %q", label, session.SessionID())
		}
		if seenBidiDraft["descriptor_ref"] != wantDescriptor {
			t.Fatalf("%s descriptor = %#v", label, seenBidiDraft["descriptor_ref"])
		}
	}

	assertStream("terminal stream", func() (*StreamHandle, error) {
		return client.OpenTerminalSessionStream(context.Background(), wrapperTerminalStartRequest())
	}, "easynet:///r/example/ability/device.dev-a.wrapper.terminal.start@1.0.0")
	assertBidi("terminal bidi", func(streams []BidiStreamDescriptor) (*BidiSession, error) {
		return client.OpenTerminalSessionBidi(context.Background(), wrapperTerminalStartRequest(), streams)
	}, "easynet:///r/example/ability/device.dev-a.wrapper.terminal.start@1.0.0")
	assertStream("remote desktop stream", func() (*StreamHandle, error) {
		return client.OpenRemoteDesktopSessionStream(context.Background(), wrapperRemoteDesktopStartRequest())
	}, "easynet:///r/example/ability/device.dev-a.wrapper.remote_desktop.start@1.0.0")
	assertBidi("remote desktop bidi", func(streams []BidiStreamDescriptor) (*BidiSession, error) {
		return client.OpenRemoteDesktopSessionBidi(context.Background(), wrapperRemoteDesktopStartRequest(), streams)
	}, "easynet:///r/example/ability/device.dev-a.wrapper.remote_desktop.start@1.0.0")
	assertStream("browser stream", func() (*StreamHandle, error) {
		return client.OpenBrowserSessionStream(context.Background(), wrapperBrowserStartRequest())
	}, "easynet:///r/example/ability/device.dev-a.wrapper.browser.start@1.0.0")
	assertBidi("browser bidi", func(streams []BidiStreamDescriptor) (*BidiSession, error) {
		return client.OpenBrowserSessionBidi(context.Background(), wrapperBrowserStartRequest(), streams)
	}, "easynet:///r/example/ability/device.dev-a.wrapper.browser.start@1.0.0")
	assertStream("media stream", func() (*StreamHandle, error) {
		return client.OpenMediaSessionStream(context.Background(), wrapperMediaStartRequest())
	}, "easynet:///r/example/ability/device.dev-a.wrapper.media.start@1.0.0")
	assertBidi("media bidi", func(streams []BidiStreamDescriptor) (*BidiSession, error) {
		return client.OpenMediaSessionBidi(context.Background(), wrapperMediaStartRequest(), streams)
	}, "easynet:///r/example/ability/device.dev-a.wrapper.media.start@1.0.0")
	if len(seenStreams) != 1 || seenStreams[0]["stream_id"] != float64(1) {
		t.Fatalf("bidi streams not forwarded: %#v", seenStreams)
	}
}

func wrapperFilesPutRequest() WrapperFileTransferRequest {
	size := int64(5)
	return WrapperFileTransferRequest{
		WrapperCarrierBase: wrapperBaseForTest(),
		WrapperFileRecordRequest: WrapperFileRecordRequest{
			OwnerURA:    "easynet:///r/example/user/alice",
			ContentType: "text/plain",
			SizeBytes:   &size,
			Metadata:    map[string]any{"route": "context_upload"},
		},
		Operation:   "put",
		AbilityName: "alice.files.put",
		Filename:    "brief.txt",
		BytesBase64: base64.StdEncoding.EncodeToString([]byte("hello")),
	}
}

func newWrapperRuntimeIdentityTransport() *compatibilityRuntimeIdentityTransport {
	return &compatibilityRuntimeIdentityTransport{
		abilityByName: map[string]string{
			wrapperAbilityFileTransfer:       "easynet:///r/example/ability/device.dev-a.wrapper.file.transfer",
			wrapperAbilityTerminalStart:      "easynet:///r/example/ability/device.dev-a.wrapper.terminal.start",
			wrapperAbilityRemoteDesktopStart: "easynet:///r/example/ability/device.dev-a.wrapper.remote_desktop.start",
			wrapperAbilityBrowserStart:       "easynet:///r/example/ability/device.dev-a.wrapper.browser.start",
			wrapperAbilityMediaStart:         "easynet:///r/example/ability/device.dev-a.wrapper.media.start",
			"alice.files.put":                "easynet:///r/example/ability/device.dev-a.alice.files.put",
		},
		descriptorByAbility: map[string]string{
			"easynet:///r/example/ability/device.dev-a.wrapper.file.transfer":        "easynet:///r/example/ability/device.dev-a.wrapper.file.transfer@1.0.0",
			"easynet:///r/example/ability/device.dev-a.wrapper.terminal.start":       "easynet:///r/example/ability/device.dev-a.wrapper.terminal.start@1.0.0",
			"easynet:///r/example/ability/device.dev-a.wrapper.remote_desktop.start": "easynet:///r/example/ability/device.dev-a.wrapper.remote_desktop.start@1.0.0",
			"easynet:///r/example/ability/device.dev-a.wrapper.browser.start":        "easynet:///r/example/ability/device.dev-a.wrapper.browser.start@1.0.0",
			"easynet:///r/example/ability/device.dev-a.wrapper.media.start":          "easynet:///r/example/ability/device.dev-a.wrapper.media.start@1.0.0",
			"easynet:///r/example/ability/device.dev-a.alice.files.put":              "easynet:///r/example/ability/device.dev-a.alice.files.put@1.0.0",
		},
		descriptorProjection: identityDescriptorProjectionJSON,
	}
}

const wrapperRuntimeFilePutRawJSON = `{
	"ura":"easynet:///r/example/resource/alice.files/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
	"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
	"content_type":"text/plain",
	"size":5,
	"filename":"brief.txt"
}`
