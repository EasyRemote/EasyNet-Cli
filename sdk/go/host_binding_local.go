package easynet

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"strings"
)

type HostBindingDescriptorRefCanonicalizer func(context.Context, string) (string, error)

// LocalHostBindingTransport is a pure SDK Host Binding codec/hash transport.
// It is used by product hosts that need local host-stream frame semantics
// without binding daemon or Axon internals.
type LocalHostBindingTransport struct {
	canonicalizer HostBindingDescriptorRefCanonicalizer
}

func NewLocalHostBindingTransport(canonicalizer HostBindingDescriptorRefCanonicalizer) *LocalHostBindingTransport {
	return &LocalHostBindingTransport{canonicalizer: canonicalizer}
}

func NewLocalHostBindingClient(canonicalizer HostBindingDescriptorRefCanonicalizer) (*HostBindingClient, error) {
	return NewHostBindingClient(NewLocalHostBindingTransport(canonicalizer))
}

func NewIdentityHostBindingDescriptorRefCanonicalizer(identity *IdentityClient) HostBindingDescriptorRefCanonicalizer {
	return func(ctx context.Context, descriptorRef string) (string, error) {
		if identity == nil {
			return "", invalidProfileClient(hostBindingProfile, "identity client is required for descriptor_ref canonicalization")
		}
		return identity.CanonicalAbilityDescriptorRef(ctx, descriptorRef, "")
	}
}

func (t *LocalHostBindingTransport) BuildHostStreamBinding(ctx context.Context, requestJSON []byte) ([]byte, error) {
	var req HostStreamBindingRequest
	if err := json.Unmarshal(requestJSON, &req); err != nil {
		return nil, invalidProfilePayload(hostBindingProfile, fmt.Sprintf("decode host stream binding request: %v", err), err)
	}
	descriptorRef, err := t.canonicalDescriptorRef(ctx, req.DescriptorRef)
	if err != nil {
		return nil, err
	}
	req.DescriptorRef = descriptorRef
	if err := validateHostStreamBindingRequest(req); err != nil {
		return nil, err
	}
	cleanup := copyMap(req.Cleanup)
	if cleanup == nil {
		cleanup = map[string]any{}
	}
	readiness := copyMap(req.Readiness)
	if readiness == nil {
		readiness = map[string]any{
			"state":          "declared",
			"checked":        false,
			"endpoint_ready": nil,
		}
	}
	metadata := copyMap(req.Metadata)
	if metadata == nil {
		metadata = map[string]any{}
	}
	metadata["profile"] = hostBindingProfile
	metadata["frame_schema"] = hostStreamFrameSchema
	metadata["hash_algorithm"] = hostStreamHashAlgorithm
	return json.Marshal(HostStreamBinding{
		BindingID:     req.BindingID,
		DescriptorRef: req.DescriptorRef,
		Endpoint:      req.Endpoint,
		FrameSchema:   req.FrameSchema,
		Cleanup:       cleanup,
		TimeoutMS:     req.TimeoutMS,
		Readiness:     readiness,
		Lifecycle: map[string]any{
			"endpoint_owner":       "product_host",
			"process_owner":        "product_host",
			"frame_contract_owner": "daemon_sdk",
		},
		Metadata: metadata,
	})
}

func (t *LocalHostBindingTransport) DecodeRequest(ctx context.Context, envelopeJSON []byte) ([]byte, error) {
	var envelope HostStreamEnvelope
	if err := json.Unmarshal(envelopeJSON, &envelope); err != nil {
		return nil, invalidProfilePayload(hostBindingProfile, fmt.Sprintf("decode host stream envelope: %v", err), err)
	}
	if envelope.Request.Fn == "" || envelope.Request.CallID == "" || envelope.Request.Caller == "" {
		return nil, invalidProfilePayload(hostBindingProfile, "host stream envelope request is incomplete", nil)
	}
	return json.Marshal(HostStreamRequest{
		Function: envelope.Request.Fn,
		Args:     envelope.Request.Args,
		CallID:   envelope.Request.CallID,
		Caller:   envelope.Request.Caller,
		Metadata: map[string]any{
			"wire":                 "host_stream_request_v1",
			"frame_contract_owner": "daemon_sdk",
		},
	})
}

func (t *LocalHostBindingTransport) EncodeItem(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := hostBindingObject(requestJSON, "host stream item request")
	if err != nil {
		return nil, err
	}
	seq, err := hostBindingRequiredUint64(request, "seq")
	if err != nil {
		return nil, err
	}
	return json.Marshal(HostStreamFrame{
		FrameType: "item",
		Seq:       &seq,
		Value:     request["value"],
	})
}

func (t *LocalHostBindingTransport) EncodeError(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := hostBindingObject(requestJSON, "host stream error request")
	if err != nil {
		return nil, err
	}
	errorValue, ok := request["error"].(map[string]any)
	if !ok || errorValue == nil {
		return nil, invalidProfilePayload(hostBindingProfile, "error is required", nil)
	}
	return json.Marshal(map[string]any{
		"frame_type":  "error",
		"seq":         nil,
		"value":       nil,
		"error":       errorValue,
		"terminal":    nil,
		"output_hash": nil,
	})
}

func (t *LocalHostBindingTransport) EncodeTerminal(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := hostBindingObject(requestJSON, "host stream terminal request")
	if err != nil {
		return nil, err
	}
	summaryValue, ok := request["summary"].(map[string]any)
	if !ok || summaryValue == nil {
		return nil, invalidProfilePayload(hostBindingProfile, "summary is required", nil)
	}
	outputHash, _ := summaryValue["output_hash"].(string)
	frames, err := hostBindingRequiredInt64(summaryValue, "frames")
	if err != nil {
		return nil, err
	}
	metadata, _ := summaryValue["metadata"].(map[string]any)
	summary := HostStreamTerminalSummary{
		OutputHash: outputHash,
		Frames:     frames,
		Metadata:   copyMap(metadata),
	}
	if summary.OutputHash == "" || summary.Frames < 0 {
		return nil, invalidProfilePayload(hostBindingProfile, "terminal output_hash and frames are required", nil)
	}
	seq := uint64(summary.Frames)
	return json.Marshal(HostStreamFrame{
		FrameType:  "terminal",
		Seq:        &seq,
		Terminal:   &summary,
		OutputHash: &summary.OutputHash,
	})
}

func (t *LocalHostBindingTransport) FoldOutputHash(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := hostBindingObject(requestJSON, "host stream hash fold request")
	if err != nil {
		return nil, err
	}
	stateValue, ok := request["state"].(map[string]any)
	if !ok || stateValue == nil {
		return nil, invalidProfilePayload(hostBindingProfile, "state is required", nil)
	}
	stateJSON, err := json.Marshal(stateValue)
	if err != nil {
		return nil, invalidProfilePayload(hostBindingProfile, fmt.Sprintf("encode host stream hash state: %v", err), err)
	}
	state, err := NewHostStreamHashStateFromJSON(stateJSON)
	if err != nil {
		return nil, err
	}
	seq, err := hostBindingRequiredUint64(request, "seq")
	if err != nil {
		return nil, err
	}
	if err := validateHostStreamHashFold(state, seq); err != nil {
		return nil, err
	}
	canonicalJSON, err := hostBindingCanonicalJSON(request["value"])
	if err != nil {
		return nil, err
	}
	outputHash, err := hostBindingFoldHash(state.OutputHash, seq, canonicalJSON)
	if err != nil {
		return nil, err
	}
	return json.Marshal(HostStreamHashState{
		Algorithm:     hostStreamHashAlgorithm,
		OutputHash:    outputHash,
		Frames:        state.Frames + 1,
		LastSeq:       &seq,
		CanonicalJSON: canonicalJSON,
	})
}

func (t *LocalHostBindingTransport) Close(context.Context) error {
	return nil
}

func (t *LocalHostBindingTransport) canonicalDescriptorRef(ctx context.Context, descriptorRef string) (string, error) {
	if t == nil || t.canonicalizer == nil {
		return "", invalidProfilePayload(hostBindingProfile, "descriptor_ref canonicalizer is required", nil)
	}
	canonical, err := t.canonicalizer(ctx, descriptorRef)
	if err != nil {
		return "", err
	}
	if canonical == "" {
		return "", invalidProfilePayload(hostBindingProfile, "descriptor_ref canonicalizer returned empty value", nil)
	}
	return canonical, nil
}

func hostBindingObject(raw []byte, label string) (map[string]any, error) {
	var payload map[string]any
	if err := json.Unmarshal(raw, &payload); err != nil {
		return nil, invalidProfilePayload(hostBindingProfile, fmt.Sprintf("decode %s: %v", label, err), err)
	}
	if payload == nil {
		return nil, invalidProfilePayload(hostBindingProfile, label+" must be a JSON object", nil)
	}
	return payload, nil
}

func hostBindingRequiredUint64(payload map[string]any, key string) (uint64, error) {
	value, ok := payload[key]
	if !ok {
		return 0, invalidProfilePayload(hostBindingProfile, key+" is required", nil)
	}
	switch typed := value.(type) {
	case float64:
		if typed < 0 || typed != float64(uint64(typed)) {
			return 0, invalidProfilePayload(hostBindingProfile, key+" must be a non-negative integer", nil)
		}
		return uint64(typed), nil
	case int:
		if typed < 0 {
			return 0, invalidProfilePayload(hostBindingProfile, key+" must be a non-negative integer", nil)
		}
		return uint64(typed), nil
	case uint64:
		return typed, nil
	default:
		return 0, invalidProfilePayload(hostBindingProfile, key+" must be a non-negative integer", nil)
	}
}

func hostBindingRequiredInt64(payload map[string]any, key string) (int64, error) {
	value, ok := payload[key]
	if !ok {
		return 0, invalidProfilePayload(hostBindingProfile, key+" is required", nil)
	}
	switch typed := value.(type) {
	case float64:
		if typed < 0 || typed != float64(int64(typed)) {
			return 0, invalidProfilePayload(hostBindingProfile, key+" must be a non-negative integer", nil)
		}
		return int64(typed), nil
	case int:
		if typed < 0 {
			return 0, invalidProfilePayload(hostBindingProfile, key+" must be a non-negative integer", nil)
		}
		return int64(typed), nil
	case int64:
		if typed < 0 {
			return 0, invalidProfilePayload(hostBindingProfile, key+" must be a non-negative integer", nil)
		}
		return typed, nil
	default:
		return 0, invalidProfilePayload(hostBindingProfile, key+" must be a non-negative integer", nil)
	}
}

func hostBindingCanonicalJSON(value any) (string, error) {
	var buf bytes.Buffer
	encoder := json.NewEncoder(&buf)
	encoder.SetEscapeHTML(false)
	if err := encoder.Encode(value); err != nil {
		return "", invalidProfilePayload(hostBindingProfile, fmt.Sprintf("host stream frame is not valid JSON: %v", err), err)
	}
	return strings.TrimSuffix(buf.String(), "\n"), nil
}

func hostBindingFoldHash(previousOutputHash string, seq uint64, canonicalJSON string) (string, error) {
	const prefix = "sha256:"
	if !strings.HasPrefix(previousOutputHash, prefix) {
		return "", invalidProfilePayload(hostBindingProfile, "previous output_hash must be sha256-prefixed", nil)
	}
	hexPart := strings.TrimPrefix(previousOutputHash, prefix)
	if len(hexPart) != 64 || strings.ToLower(hexPart) != hexPart {
		return "", invalidProfilePayload(hostBindingProfile, "previous output_hash must use sha256:<64 lowercase hex> form", nil)
	}
	previous, err := hex.DecodeString(hexPart)
	if err != nil {
		return "", invalidProfilePayload(hostBindingProfile, "previous output_hash is not hex", err)
	}
	var seqBytes [8]byte
	binary.BigEndian.PutUint64(seqBytes[:], seq)
	hasher := sha256.New()
	_, _ = hasher.Write(previous)
	_, _ = hasher.Write(seqBytes[:])
	_, _ = hasher.Write([]byte(canonicalJSON))
	return prefix + hex.EncodeToString(hasher.Sum(nil)), nil
}
