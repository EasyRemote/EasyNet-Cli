package easynet

import (
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"strings"
)

const signingProfile = "signing"

// SignerHandle is a provider-authorized signer reference, never key material.
type SignerHandle struct {
	Profile   string         `json:"profile"`
	SignerID  string         `json:"signer_id"`
	OwnerURA  string         `json:"owner_ura"`
	KeyID     string         `json:"key_id"`
	Algorithm string         `json:"algorithm"`
	Policy    map[string]any `json:"policy"`
	Metadata  map[string]any `json:"metadata"`
}

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
	canonicalHashHex     string
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

// CanonicalHashHex returns the SDK-validated SHA-256 commitment supplied with
// the canonical signing bytes.
func (m SigningMaterial) CanonicalHashHex() string {
	return m.canonicalHashHex
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
	return decodePreparedInvocation(raw, true)
}

// signingMaterialFromPrepareJSON decodes a stateless prepare projection. Such
// projections intentionally contain no native prepared capability.
func signingMaterialFromPrepareJSON(raw []byte) (SigningMaterial, error) {
	prepared, err := decodePreparedInvocation(raw, false)
	if err != nil {
		return SigningMaterial{}, err
	}
	return prepared.SigningMaterial(), nil
}

func decodePreparedInvocation(raw []byte, requirePreparedID bool) (PreparedInvocation, error) {
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
	material, err := requiredSigningMaterial(fields, "signing_material")
	if err != nil {
		return PreparedInvocation{}, err
	}
	if material.DescriptorRef() != tuple.DescriptorRef() {
		return PreparedInvocation{}, invalidInvocation("signing_material.descriptor_ref must match tuple descriptor_ref", nil)
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
	if requirePreparedID && prepared.preparedID == "" {
		return PreparedInvocation{}, invalidInvocation("prepared_id is required", nil)
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
	canonicalHashHex, err := validatedCanonicalMaterialHash(
		prepared.signingMaterial.CanonicalBytesBase64(),
		prepared.canonicalHashHex,
	)
	if err != nil {
		return PreparedInvocation{}, err
	}
	prepared.canonicalHashHex = canonicalHashHex
	prepared.signingMaterial.canonicalHashHex = canonicalHashHex
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
	signature = normalizeInvocationSignatureMaterial(signature)
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

// SignatureProvider produces caller signatures over daemon/Axon signing material.
type SignatureProvider interface {
	Sign(material SigningMaterial, handle SignerHandle) (InvocationSignature, error)
}

// StaticSignatureProvider adapts already-produced signatures.
type StaticSignatureProvider struct {
	signature InvocationSignature
}

func NewStaticSignatureProvider(signature InvocationSignature) StaticSignatureProvider {
	return StaticSignatureProvider{signature: signature}
}

func (p StaticSignatureProvider) Sign(material SigningMaterial, handle SignerHandle) (InvocationSignature, error) {
	_ = material
	_ = handle
	return p.signature, nil
}

// Signer binds a daemon-authorized handle to a concrete signature provider.
type Signer struct {
	handle   SignerHandle
	provider SignatureProvider
}

func NewSigner(handle SignerHandle, provider SignatureProvider) (Signer, error) {
	if provider == nil {
		return Signer{}, invalidInvocation("signature provider is required", nil)
	}
	if err := validateSignerHandle(handle); err != nil {
		return Signer{}, err
	}
	return Signer{handle: handle, provider: provider}, nil
}

func NewSignerFromSignature(handle SignerHandle, signature InvocationSignature) (Signer, error) {
	return NewSigner(handle, NewStaticSignatureProvider(signature))
}

func (s Signer) Handle() SignerHandle {
	return s.handle
}

func (s Signer) Sign(prepared PreparedInvocation) (SignedInvocation, error) {
	if err := validateSignerHandle(s.handle); err != nil {
		return SignedInvocation{}, err
	}
	if s.provider == nil {
		return SignedInvocation{}, invalidInvocation("signature provider is required", nil)
	}
	signature, err := s.provider.Sign(prepared.SigningMaterial(), s.handle)
	if err != nil {
		return SignedInvocation{}, err
	}
	return s.SignWithSignature(prepared, signature)
}

func (s Signer) SignWithSignature(prepared PreparedInvocation, signature InvocationSignature) (SignedInvocation, error) {
	if err := validateSignerHandle(s.handle); err != nil {
		return SignedInvocation{}, err
	}
	if err := validatePreparedPolicy(prepared, s.handle); err != nil {
		return SignedInvocation{}, err
	}
	normalized, err := normalizeSignature(s.handle, signature)
	if err != nil {
		return SignedInvocation{}, err
	}
	signed, err := prepared.SignWithCallerSignature(normalized)
	if err != nil {
		return SignedInvocation{}, err
	}
	if signed.SignerID() != s.handle.SignerID {
		return SignedInvocation{}, invalidInvocation("signed invocation signer does not match handle", nil)
	}
	return signed, nil
}

// signInvocationDraft attaches a caller signature to a complete Invocation
// draft without consuming or mutating the input. Existing caller signatures are
// preserved; this keeps browser/user pre-signed Invocations from being
// re-signed by a backend or host process.
func (s Signer) signInvocationDraft(draft InvocationDraft) (InvocationDraft, error) {
	if draft.CallerSignature() != nil {
		return draft, nil
	}
	if err := validateSignerHandle(s.handle); err != nil {
		return InvocationDraft{}, err
	}
	if draft.CallerURA() != s.handle.OwnerURA {
		return InvocationDraft{}, invalidInvocation("signer handle owner_ura must match invocation caller_ura", nil)
	}
	if s.provider == nil {
		return InvocationDraft{}, invalidInvocation("signature provider is required", nil)
	}
	material, err := signingMaterialForInvocationDraft(draft)
	if err != nil {
		return InvocationDraft{}, err
	}
	signature, err := s.provider.Sign(material, s.handle)
	if err != nil {
		return InvocationDraft{}, err
	}
	normalized, err := normalizeSignature(s.handle, signature)
	if err != nil {
		return InvocationDraft{}, err
	}
	signed := draft
	signed.callerSignature = &normalized
	return signed, nil
}

// signingMaterialForInvocationDraft projects SDK Invocation DTO fields into the
// canonical material callers sign. The byte layout is delegated to the Axon
// canonical facade; this helper owns only DTO validation and projection.
func signingMaterialForInvocationDraft(draft InvocationDraft) (SigningMaterial, error) {
	bound, err := descriptorBoundInvocationDraft(draft)
	if err != nil {
		return SigningMaterial{}, err
	}
	canonical, err := bound.CanonicalBytes()
	if err != nil {
		return SigningMaterial{}, err
	}
	args := bound.Payload()
	digest := sha256.Sum256(args)
	canonicalHash := sha256.Sum256(canonical)
	return SigningMaterial{
		algorithm:            "ed25519",
		canonicalBytesBase64: base64.StdEncoding.EncodeToString(canonical),
		canonicalHashHex:     hex.EncodeToString(canonicalHash[:]),
		argsDigestHex:        hex.EncodeToString(digest[:]),
		descriptorRef:        draft.DescriptorRef(),
		nonceBase64:          draft.NonceBase64(),
		signedFields: []string{
			"caller_ura",
			"callee_ura",
			"subject_ura",
			"descriptor_ref",
			"args_digest",
			"nonce_base64",
			"causal_context",
		},
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
			"tuple":                  s.prepared.Tuple(),
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

func requiredSigningMaterial(fields map[string]json.RawMessage, name string) (SigningMaterial, error) {
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
	if strings.TrimSpace(material.canonicalBytesBase64) == "" {
		return SigningMaterial{}, invalidInvocation("signing_material.canonical_bytes_base64 is required", nil)
	}
	if _, err := decodeCanonicalBytesBase64(material.canonicalBytesBase64); err != nil {
		return SigningMaterial{}, err
	}
	if strings.TrimSpace(material.argsDigestHex) == "" {
		return SigningMaterial{}, invalidInvocation("signing_material.args_digest_hex is required", nil)
	}
	if strings.TrimSpace(material.descriptorRef) == "" {
		return SigningMaterial{}, invalidInvocation("signing_material.descriptor_ref is required", nil)
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

func validatedCanonicalMaterialHash(
	canonicalBytesBase64 string,
	canonicalHashHex string,
) (string, error) {
	if canonicalHashHex == "" {
		return "", nil
	}
	canonicalHash, err := normalizeSHA256Hex(canonicalHashHex, "canonical_hash_hex")
	if err != nil {
		return "", err
	}
	canonicalBytes, err := decodeCanonicalBytesBase64(canonicalBytesBase64)
	if err != nil {
		return "", err
	}
	actual := sha256.Sum256(canonicalBytes)
	if hex.EncodeToString(actual[:]) != canonicalHash {
		return "", invalidInvocation("canonical_hash_hex does not match canonical_bytes_base64", nil)
	}
	return canonicalHash, nil
}

func normalizeSHA256Hex(value string, fieldName string) (string, error) {
	raw := strings.TrimPrefix(value, "sha256:")
	if len(raw) != 64 {
		return "", invalidInvocation(fmt.Sprintf("%s must be a sha256 hex digest", fieldName), nil)
	}
	if _, err := hex.DecodeString(raw); err != nil {
		return "", invalidInvocation(fmt.Sprintf("%s must be hex", fieldName), err)
	}
	return strings.ToLower(raw), nil
}

func decodeCanonicalBytesBase64(value string) ([]byte, error) {
	return decodeBase64Field(value, "canonical_bytes_base64")
}

func decodeBase64Field(value string, fieldName string) ([]byte, error) {
	if strings.TrimSpace(value) == "" {
		return nil, invalidInvocation(fmt.Sprintf("%s is required", fieldName), nil)
	}
	decoded, err := base64.StdEncoding.DecodeString(value)
	if err != nil {
		return nil, invalidInvocation(fmt.Sprintf("%s must be base64", fieldName), err)
	}
	return decoded, nil
}

func validateSignerHandle(handle SignerHandle) error {
	if strings.TrimSpace(handle.Profile) != signingProfile {
		return invalidInvocation("signer handle profile must be signing", nil)
	}
	if strings.TrimSpace(handle.SignerID) == "" {
		return invalidInvocation("signer handle signer_id is required", nil)
	}
	if strings.TrimSpace(handle.KeyID) == "" {
		return invalidInvocation("signer handle key_id is required", nil)
	}
	if strings.TrimSpace(handle.OwnerURA) == "" {
		return invalidInvocation("signer handle owner_ura is required", nil)
	}
	if handle.Policy == nil {
		return invalidInvocation("signer handle policy is required", nil)
	}
	if handle.Metadata == nil {
		return invalidInvocation("signer handle metadata is required", nil)
	}
	if strings.ToLower(strings.TrimSpace(handle.Algorithm)) != "ed25519" {
		return invalidInvocation("signer handle algorithm must be ed25519", nil)
	}
	if !isDaemonSignerSource(handle.Metadata["source"]) {
		return invalidInvocation("signer handle source must be daemon key inventory", nil)
	}
	mode, ok := handle.Policy["mode"].(string)
	if !ok || !isDaemonSignerMode(mode) {
		return invalidInvocation("signer handle policy mode is not supported", nil)
	}
	usageString, ok := handle.Policy["usage"].(string)
	if !ok || !isInvocationSigningUsage(usageString) {
		return invalidInvocation("signer handle policy usage is not supported", nil)
	}
	if policySignerID, ok := handle.Policy["signer_id"].(string); ok && strings.TrimSpace(policySignerID) != "" && policySignerID != handle.SignerID {
		return invalidInvocation("signer handle policy.signer_id must match signer_id", nil)
	}
	policyRef, ok := handle.Policy["policy_ref"].(string)
	if !ok || strings.TrimSpace(policyRef) == "" {
		return invalidInvocation("signer handle policy_ref is required", nil)
	}
	inventoryOwnerURA, ok := handle.Policy["inventory_owner_ura"].(string)
	if !ok || strings.TrimSpace(inventoryOwnerURA) == "" {
		return invalidInvocation("signer handle inventory_owner_ura is required", nil)
	}
	if inventoryOwnerURA != handle.OwnerURA {
		return invalidInvocation("signer handle inventory_owner_ura must match owner_ura", nil)
	}
	keyState, ok := handle.Policy["key_state"].(string)
	if !ok || strings.TrimSpace(keyState) != "active" {
		return invalidInvocation("signer handle key_state must be active", nil)
	}
	if metadataPolicyRef, ok := handle.Metadata["policy_ref"].(string); ok && strings.TrimSpace(metadataPolicyRef) != "" && metadataPolicyRef != policyRef {
		return invalidInvocation("signer handle metadata policy_ref must match policy.policy_ref", nil)
	}
	return nil
}

func isDaemonSignerSource(value any) bool {
	source, ok := value.(string)
	if !ok {
		return false
	}
	switch strings.TrimSpace(source) {
	case "daemon_keyring", "daemon_key_inventory", "identity.list_user_pubkeys", "identity.signer", "daemon.identity.signer":
		return true
	default:
		return false
	}
}

func isDaemonSignerMode(value string) bool {
	switch strings.TrimSpace(value) {
	case "local_daemon_signing":
		return true
	default:
		return false
	}
}

func isInvocationSigningUsage(value string) bool {
	switch strings.TrimSpace(value) {
	case "invocation.sign":
		return true
	default:
		return false
	}
}

func validatePreparedPolicy(prepared PreparedInvocation, handle SignerHandle) error {
	policy := prepared.SigningMaterial().SignerPolicy()
	if policy == nil {
		return nil
	}
	if policy.SignerID() != "" && policy.SignerID() != handle.SignerID {
		return invalidInvocation("prepared signer policy does not match signer handle", nil)
	}
	if policy.Mode() == "" {
		return nil
	}
	handleMode, ok := handle.Policy["mode"].(string)
	if ok && handleMode != "" && policy.Mode() != handleMode {
		return invalidInvocation("prepared signer policy mode does not match handle", nil)
	}
	return nil
}

func normalizeSignature(handle SignerHandle, signature InvocationSignature) (InvocationSignature, error) {
	handleAlgorithm := strings.ToLower(strings.TrimSpace(handle.Algorithm))
	algorithm := strings.ToLower(strings.TrimSpace(signature.Algorithm))
	if algorithm == "" {
		algorithm = handleAlgorithm
	}
	if algorithm == "" {
		return InvocationSignature{}, invalidInvocation("signature.algorithm is required", nil)
	}
	if handleAlgorithm != "" && algorithm != handleAlgorithm {
		return InvocationSignature{}, invalidInvocation("signature algorithm does not match signer handle", nil)
	}
	keyIDHint := signature.KeyIDHint
	if keyIDHint == "" {
		keyIDHint = handle.SignerID
	}
	if keyIDHint != handle.SignerID && keyIDHint != handle.KeyID {
		return InvocationSignature{}, invalidInvocation("signature key_id_hint does not match signer handle", nil)
	}
	return InvocationSignature{
		Algorithm:             algorithm,
		SignatureBase64:       signature.SignatureBase64,
		KeyIDHint:             handle.SignerID,
		SignerPublicKeyBase64: signature.SignerPublicKeyBase64,
	}, nil
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
