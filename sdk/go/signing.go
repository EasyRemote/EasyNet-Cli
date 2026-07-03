package easynet

import (
	"encoding/json"
	"fmt"
	"strings"
)

// SignerPolicy describes who is allowed to attach a signature.
type SignerPolicy struct {
	mode            string
	signerID        string
	policyRef       string
	expiresAtUnixMS int64
}

func (p SignerPolicy) Mode() string {
	return p.mode
}

func (p SignerPolicy) SignerID() string {
	return p.signerID
}

func (p SignerPolicy) PolicyRef() string {
	return p.policyRef
}

func (p SignerPolicy) ExpiresAtUnixMS() int64 {
	return p.expiresAtUnixMS
}

// SigningMaterial is daemon/Axon-owned canonical material projected to SDK DTOs.
type SigningMaterial struct {
	algorithm            string
	canonicalBytesBase64 string
	argsDigestHex        string
	descriptorRef        string
	nonceBase64          string
	signedFields         []string
	expiresAtUnixMS      int64
	signerPolicy         *SignerPolicy
}

func (m SigningMaterial) Algorithm() string {
	return m.algorithm
}

func (m SigningMaterial) CanonicalBytesBase64() string {
	return m.canonicalBytesBase64
}

func (m SigningMaterial) ArgsDigestHex() string {
	return m.argsDigestHex
}

func (m SigningMaterial) DescriptorRef() string {
	return m.descriptorRef
}

func (m SigningMaterial) NonceBase64() string {
	return m.nonceBase64
}

func (m SigningMaterial) SignedFields() []string {
	return append([]string(nil), m.signedFields...)
}

func (m SigningMaterial) ExpiresAtUnixMS() int64 {
	return m.expiresAtUnixMS
}

func (m SigningMaterial) SignerPolicy() *SignerPolicy {
	if m.signerPolicy == nil {
		return nil
	}
	value := *m.signerPolicy
	return &value
}

// PreparedInvocation is immutable canonical signing material, not executable.
type PreparedInvocation struct {
	preparedID       string
	requestID        string
	descriptorRef    string
	descriptorHash   string
	schemaHash       string
	canonicalHashHex string
	expiresAtUnixMS  int64
	tuple            InvocationDraft
	signingMaterial  SigningMaterial
}

// NewPreparedInvocationFromJSON decodes daemon/Axon prepared signing material.
func NewPreparedInvocationFromJSON(raw []byte) (PreparedInvocation, error) {
	var fields map[string]json.RawMessage
	if err := json.Unmarshal(raw, &fields); err != nil {
		return PreparedInvocation{}, invalidInvocation(fmt.Sprintf("decode prepared invocation JSON: %v", err), err)
	}
	if preparedSubmitReady(fields) {
		return PreparedInvocation{}, invalidInvocation("PreparedInvocation must not be submit-ready", nil)
	}
	tuple, err := requiredDraft(fields, "tuple")
	if err != nil {
		return PreparedInvocation{}, err
	}
	material, err := requiredSigningMaterial(fields, "signing_material", tuple.DescriptorRef())
	if err != nil {
		return PreparedInvocation{}, err
	}
	prepared := PreparedInvocation{
		preparedID:       optionalPreparedString(fields, "prepared_id"),
		requestID:        optionalPreparedString(fields, "request_id"),
		descriptorRef:    optionalPreparedString(fields, "descriptor_ref"),
		descriptorHash:   optionalPreparedString(fields, "descriptor_hash_hex"),
		schemaHash:       optionalPreparedString(fields, "schema_hash_hex"),
		canonicalHashHex: optionalPreparedString(fields, "canonical_hash_hex"),
		expiresAtUnixMS:  optionalPreparedInt64(fields, "expires_at_unix_ms"),
		tuple:            tuple,
		signingMaterial:  material,
	}
	if prepared.preparedID == "" && prepared.requestID == "" {
		return PreparedInvocation{}, invalidInvocation("prepared_id or request_id is required", nil)
	}
	if prepared.descriptorRef == "" {
		prepared.descriptorRef = material.DescriptorRef()
	}
	if prepared.expiresAtUnixMS == 0 {
		prepared.expiresAtUnixMS = material.ExpiresAtUnixMS()
	}
	if prepared.descriptorRef == "" {
		return PreparedInvocation{}, invalidInvocation("descriptor_ref is required", nil)
	}
	return prepared, nil
}

func preparedSubmitReady(fields map[string]json.RawMessage) bool {
	raw, ok := fields["submit_ready"]
	if !ok {
		return false
	}
	var value bool
	return json.Unmarshal(raw, &value) == nil && value
}

func (p PreparedInvocation) PreparedID() string {
	return p.preparedID
}

func (p PreparedInvocation) RequestID() string {
	return p.requestID
}

func (p PreparedInvocation) DescriptorRef() string {
	return p.descriptorRef
}

func (p PreparedInvocation) DescriptorHash() string {
	return p.descriptorHash
}

func (p PreparedInvocation) SchemaHash() string {
	return p.schemaHash
}

func (p PreparedInvocation) CanonicalHashHex() string {
	return p.canonicalHashHex
}

func (p PreparedInvocation) ExpiresAtUnixMS() int64 {
	return p.expiresAtUnixMS
}

func (p PreparedInvocation) Tuple() InvocationDraft {
	return p.tuple
}

func (p PreparedInvocation) SigningMaterial() SigningMaterial {
	return p.signingMaterial
}

func (p PreparedInvocation) SubmitReady() bool {
	return false
}

// SignWithCallerSignature attaches externally produced signature material.
func (p PreparedInvocation) SignWithCallerSignature(signature InvocationSignature) (SignedInvocation, error) {
	if strings.TrimSpace(signature.Algorithm) == "" {
		return SignedInvocation{}, invalidInvocation("signature.algorithm is required", nil)
	}
	if strings.TrimSpace(signature.SignatureBase64) == "" {
		return SignedInvocation{}, invalidInvocation("signature.signature_base64 is required", nil)
	}
	signerID := signature.KeyIDHint
	if policy := p.signingMaterial.SignerPolicy(); policy != nil && policy.SignerID() != "" {
		signerID = policy.SignerID()
	}
	if signerID == "" {
		signerID = signature.SignerPublicKeyBase64
	}
	if strings.TrimSpace(signerID) == "" {
		return SignedInvocation{}, invalidInvocation("signer id is required", nil)
	}
	return SignedInvocation{
		prepared:  p,
		signature: signature,
		signerID:  signerID,
		policy:    p.signingMaterial.SignerPolicy(),
	}, nil
}

// SignedInvocation is the immutable submit-ready pre-runtime envelope.
type SignedInvocation struct {
	prepared  PreparedInvocation
	signature InvocationSignature
	signerID  string
	policy    *SignerPolicy
}

func (s SignedInvocation) Prepared() PreparedInvocation {
	return s.prepared
}

func (s SignedInvocation) Signature() InvocationSignature {
	return s.signature
}

func (s SignedInvocation) SignerID() string {
	return s.signerID
}

func (s SignedInvocation) Policy() *SignerPolicy {
	if s.policy == nil {
		return nil
	}
	value := *s.policy
	return &value
}

func (s SignedInvocation) SubmitReady() bool {
	return strings.TrimSpace(s.signerID) != "" &&
		strings.TrimSpace(s.signature.Algorithm) != "" &&
		strings.TrimSpace(s.signature.SignatureBase64) != "" &&
		strings.TrimSpace(s.prepared.DescriptorRef()) != "" &&
		strings.TrimSpace(s.prepared.SigningMaterial().CanonicalBytesBase64()) != ""
}

// MarshalJSON emits the daemon signed-invocation envelope shape.
func (s SignedInvocation) MarshalJSON() ([]byte, error) {
	if !s.SubmitReady() {
		return nil, invalidInvocation("signed invocation is not submit-ready", nil)
	}
	obj := map[string]any{
		"signer_id": s.signerID,
		"prepared": map[string]any{
			"prepared_id":            s.prepared.PreparedID(),
			"request_id":             s.prepared.RequestID(),
			"descriptor_ref":         s.prepared.DescriptorRef(),
			"canonical_hash_hex":     s.prepared.CanonicalHashHex(),
			"expires_at_unix_ms":     s.prepared.ExpiresAtUnixMS(),
			"canonical_bytes_base64": s.prepared.SigningMaterial().CanonicalBytesBase64(),
		},
		"signature": s.signature,
	}
	if s.policy != nil {
		obj["policy"] = map[string]any{
			"mode":               s.policy.Mode(),
			"signer_id":          s.policy.SignerID(),
			"policy_ref":         s.policy.PolicyRef(),
			"expires_at_unix_ms": s.policy.ExpiresAtUnixMS(),
		}
	}
	return json.Marshal(obj)
}

func requiredDraft(fields map[string]json.RawMessage, name string) (InvocationDraft, error) {
	raw, ok := fields[name]
	if !ok {
		return InvocationDraft{}, invalidInvocation(fmt.Sprintf("%s is required", name), nil)
	}
	return NewInvocationDraftFromJSON(raw)
}

func requiredSigningMaterial(fields map[string]json.RawMessage, name string, fallbackDescriptorRef string) (SigningMaterial, error) {
	raw, ok := fields[name]
	if !ok {
		return SigningMaterial{}, invalidInvocation(fmt.Sprintf("%s is required", name), nil)
	}
	var materialFields map[string]json.RawMessage
	if err := json.Unmarshal(raw, &materialFields); err != nil {
		return SigningMaterial{}, invalidInvocation(fmt.Sprintf("%s must be an object", name), err)
	}
	material := SigningMaterial{
		algorithm:            optionalPreparedString(materialFields, "algorithm"),
		canonicalBytesBase64: optionalPreparedString(materialFields, "canonical_bytes_base64"),
		argsDigestHex:        optionalPreparedString(materialFields, "args_digest_hex"),
		descriptorRef:        optionalPreparedString(materialFields, "descriptor_ref"),
		nonceBase64:          optionalPreparedString(materialFields, "nonce_base64"),
		signedFields:         optionalStringSlice(materialFields, "signed_fields"),
		expiresAtUnixMS:      optionalPreparedInt64(materialFields, "expires_at_unix_ms"),
		signerPolicy:         optionalSignerPolicy(materialFields, "signer_policy"),
	}
	if material.descriptorRef == "" {
		material.descriptorRef = fallbackDescriptorRef
	}
	if strings.TrimSpace(material.canonicalBytesBase64) == "" {
		return SigningMaterial{}, invalidInvocation("signing_material.canonical_bytes_base64 is required", nil)
	}
	if strings.TrimSpace(material.argsDigestHex) == "" {
		return SigningMaterial{}, invalidInvocation("signing_material.args_digest_hex is required", nil)
	}
	if material.expiresAtUnixMS == 0 {
		return SigningMaterial{}, invalidInvocation("signing_material.expires_at_unix_ms is required", nil)
	}
	return material, nil
}

func optionalSignerPolicy(fields map[string]json.RawMessage, name string) *SignerPolicy {
	raw, ok := fields[name]
	if !ok || string(raw) == "null" {
		return nil
	}
	var value map[string]json.RawMessage
	if err := json.Unmarshal(raw, &value); err != nil {
		return nil
	}
	return &SignerPolicy{
		mode:            optionalPreparedString(value, "mode"),
		signerID:        optionalPreparedString(value, "signer_id"),
		policyRef:       optionalPreparedString(value, "policy_ref"),
		expiresAtUnixMS: optionalPreparedInt64(value, "expires_at_unix_ms"),
	}
}

func optionalPreparedString(fields map[string]json.RawMessage, name string) string {
	raw, ok := fields[name]
	if !ok || string(raw) == "null" {
		return ""
	}
	var value string
	if err := json.Unmarshal(raw, &value); err != nil {
		return ""
	}
	return value
}

func optionalPreparedInt64(fields map[string]json.RawMessage, name string) int64 {
	raw, ok := fields[name]
	if !ok || string(raw) == "null" {
		return 0
	}
	var value int64
	if err := json.Unmarshal(raw, &value); err != nil {
		return 0
	}
	return value
}

func optionalStringSlice(fields map[string]json.RawMessage, name string) []string {
	raw, ok := fields[name]
	if !ok || string(raw) == "null" {
		return nil
	}
	var value []string
	if err := json.Unmarshal(raw, &value); err != nil {
		return nil
	}
	return append([]string(nil), value...)
}
