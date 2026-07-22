package easynet

import (
	"bytes"
	"crypto/ed25519"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"strings"
	"time"
)

const (
	// ManagedSigningDefaultPageLimit bounds every managed-signing List call.
	ManagedSigningDefaultPageLimit uint32 = 16
	// ManagedSigningMaxPageLimit is the largest page the SDK will request or
	// accept from the daemon key service.
	ManagedSigningMaxPageLimit uint32 = 16

	managedSigningAutoPaginationMaxPages = 1024
	managedSigningAutoPaginationMaxItems = 16_384
	managedSigningMaxCursorBytes         = 4096
)

// ManagedSigningStatus is the provider-owned lifecycle state of a rotatable
// signing key. Revoked is terminal; only Active keys can sign.
type ManagedSigningStatus string

const (
	ManagedSigningStatusActive  ManagedSigningStatus = "active"
	ManagedSigningStatusRetired ManagedSigningStatus = "retired"
	ManagedSigningStatusRevoked ManagedSigningStatus = "revoked"
)

func (s ManagedSigningStatus) valid() bool {
	switch s {
	case ManagedSigningStatusActive, ManagedSigningStatusRetired, ManagedSigningStatusRevoked:
		return true
	default:
		return false
	}
}

// ManagedSigningKey is the public projection of a daemon-custodied signing
// key. It deliberately has no private-key or persistence fields.
type ManagedSigningKey struct {
	KeyID           string               `json:"key_id"`
	Purpose         string               `json:"purpose"`
	PublicKey       ed25519.PublicKey    `json:"public_key"`
	Status          ManagedSigningStatus `json:"status"`
	RotationEpoch   uint64               `json:"rotation_epoch"`
	BoundSubjectURA string               `json:"bound_subject_ura,omitempty"`
	SignerPolicyRef string               `json:"signer_policy_ref,omitempty"`
	RotatedFrom     string               `json:"rotated_from,omitempty"`
	CreatedUnixMS   int64                `json:"created_unix_ms"`
	ExpiresUnixMS   *int64               `json:"expires_unix_ms,omitempty"`
	RevokedUnixMS   *int64               `json:"revoked_unix_ms,omitempty"`
}

// SigningPublicKey returns a defensive copy of the public verification key.
func (k ManagedSigningKey) SigningPublicKey() ed25519.PublicKey {
	return append(ed25519.PublicKey(nil), k.PublicKey...)
}

// ManagedSigningCreateRequest describes policy metadata for a key generated
// inside daemon custody.
type ManagedSigningCreateRequest struct {
	Purpose         string
	BoundSubjectURA string
}

// ManagedSigningKeyFilter selects public key projections without exposing
// storage or custody details. Empty fields mean no filter.
type ManagedSigningKeyFilter struct {
	Purpose string
	Status  ManagedSigningStatus
}

// ManagedSigningPageOptions selects one bounded inventory page. Cursor is an
// opaque daemon token; the SDK deliberately does not parse daemon URAs or
// cursor internals.
type ManagedSigningPageOptions struct {
	Limit  uint32
	Cursor string
}

// ManagedSigningKeyPage is one bounded page of public key projections.
type ManagedSigningKeyPage struct {
	Keys       []ManagedSigningKey
	NextCursor string
}

// ManagedSigningPeer is the public trust projection for one peer runtime.
type ManagedSigningPeer struct {
	PeerURA        string            `json:"peer_ura"`
	Fingerprint    []byte            `json:"fingerprint"`
	PublicKey      ed25519.PublicKey `json:"public_key"`
	ViaHubURA      string            `json:"via_hub_ura,omitempty"`
	AddedUnixMS    int64             `json:"added_unix_ms"`
	LastSeenUnixMS int64             `json:"last_seen_unix_ms"`
}

// ManagedSigningPeerRegistration contains only a peer's public trust
// material. Local private keys never participate in peer registration.
type ManagedSigningPeerRegistration struct {
	PeerURA   string
	PublicKey ed25519.PublicKey
	ViaHubURA string
}

// ManagedSigningPeerPage is one bounded page of peer trust projections.
type ManagedSigningPeerPage struct {
	Peers      []ManagedSigningPeer
	NextCursor string
}

// ManagedSigningClientOptions configures the daemon-local provider endpoint.
// SocketPath is required: product runtime discovery and directory policy
// belong to the daemon, not to this generic SDK facade.
type ManagedSigningClientOptions struct {
	SocketPath string
	Timeout    time.Duration
}

// ManagedSigningClient is a provider-backed facade over the daemon's
// rotatable signing-key domain.
type ManagedSigningClient struct {
	service daemonKeyServiceClient
}

// ManagedSigner is a key-bound canonical signing capability. It retains the
// complete validated public authority projection and verifies every
// daemon-produced signature.
type ManagedSigner struct {
	client     *ManagedSigningClient
	projection ManagedSigningKey
}

var _ CanonicalSigner = (*ManagedSigner)(nil)

func NewManagedSigningClient(options ManagedSigningClientOptions) (*ManagedSigningClient, error) {
	service, err := newDaemonKeyServiceClient(options.SocketPath, options.Timeout)
	if err != nil {
		return nil, err
	}
	return &ManagedSigningClient{service: service}, nil
}

// Signer resolves one active key into a narrow canonical-signing capability.
// The capability retains no private material and verifies daemon signatures
// against the resolved public projection before returning them.
func (c *ManagedSigningClient) Signer(keyID string) (*ManagedSigner, error) {
	projection, err := c.PublicProjection(keyID)
	if err != nil {
		return nil, err
	}
	if projection.Status != ManagedSigningStatusActive {
		return nil, daemonKeyServiceRejected(
			"lifecycle",
			fmt.Sprintf("managed signing key %q is not active", projection.KeyID),
		)
	}
	if projection.BoundSubjectURA == "" || projection.SignerPolicyRef == "" {
		return nil, daemonKeyServiceRejected(
			"policy",
			fmt.Sprintf("managed signing key %q is not bound to a signing subject", projection.KeyID),
		)
	}
	return &ManagedSigner{
		client:     c,
		projection: cloneManagedSigningKey(projection),
	}, nil
}

// KeyID returns the immutable daemon inventory identifier bound to the signer.
func (s *ManagedSigner) KeyID() string {
	if s == nil {
		return ""
	}
	return s.projection.KeyID
}

// Projection returns a defensive copy of the complete validated public
// authority projection bound to this signer.
func (s *ManagedSigner) Projection() ManagedSigningKey {
	if s == nil {
		return ManagedSigningKey{}
	}
	return cloneManagedSigningKey(s.projection)
}

func (s *ManagedSigner) SignCanonical(canonicalBytes []byte) ([]byte, error) {
	if s == nil || s.client == nil || s.projection.KeyID == "" ||
		len(s.projection.PublicKey) != ed25519.PublicKeySize {
		return nil, invalidDaemonKeyServiceInput("managed signer is not initialized")
	}
	signature, err := s.client.signWithProjection(s.projection, canonicalBytes)
	if err != nil {
		return nil, err
	}
	if !ed25519.Verify(s.projection.PublicKey, canonicalBytes, signature) {
		return nil, invalidDaemonKeyServicePayload(
			"daemon key service returned a signature that does not verify against the bound key projection",
			nil,
		)
	}
	return signature, nil
}

func (s *ManagedSigner) SigningPublicKey() (ed25519.PublicKey, error) {
	if s == nil || len(s.projection.PublicKey) != ed25519.PublicKeySize {
		return nil, invalidDaemonKeyServiceInput("managed signer is not initialized")
	}
	return s.projection.SigningPublicKey(), nil
}

func (c *ManagedSigningClient) Create(request ManagedSigningCreateRequest) (ManagedSigningKey, error) {
	if err := requireManagedSigningClient(c); err != nil {
		return ManagedSigningKey{}, err
	}
	purpose, err := managedSigningRequiredText("purpose", request.Purpose)
	if err != nil {
		return ManagedSigningKey{}, err
	}
	payload := map[string]any{"method": "inventory.create", "purpose": purpose}
	if request.BoundSubjectURA != "" {
		subject, err := managedSigningRequiredText("bound subject URA", request.BoundSubjectURA)
		if err != nil {
			return ManagedSigningKey{}, err
		}
		payload["bound_subject"] = subject
	}
	response, err := c.service.call(payload)
	if err != nil {
		return ManagedSigningKey{}, err
	}
	created, err := decodeManagedSigningKeyResponse(response)
	if err != nil {
		return ManagedSigningKey{}, err
	}
	if created.Purpose != purpose || created.BoundSubjectURA != strings.TrimSpace(request.BoundSubjectURA) ||
		created.Status != ManagedSigningStatusActive || created.RotationEpoch != 0 || created.RotatedFrom != "" {
		return ManagedSigningKey{}, invalidDaemonKeyServicePayload(
			"daemon key service violated managed signing create postconditions",
			nil,
		)
	}
	return created, nil
}

func (c *ManagedSigningClient) List(filter ManagedSigningKeyFilter) ([]ManagedSigningKey, error) {
	if err := requireManagedSigningClient(c); err != nil {
		return nil, err
	}
	all := make([]ManagedSigningKey, 0)
	seenKeys := make(map[string]struct{})
	seenCursors := make(map[string]struct{})
	cursor := ""
	for pageIndex := 0; pageIndex < managedSigningAutoPaginationMaxPages; pageIndex++ {
		page, err := c.ListPage(filter, ManagedSigningPageOptions{
			Limit:  ManagedSigningDefaultPageLimit,
			Cursor: cursor,
		})
		if err != nil {
			return nil, err
		}
		if len(all)+len(page.Keys) > managedSigningAutoPaginationMaxItems {
			return nil, invalidDaemonKeyServicePayload("managed signing list exceeded the bounded auto-pagination item limit", nil)
		}
		for _, key := range page.Keys {
			if _, exists := seenKeys[key.KeyID]; exists {
				return nil, invalidDaemonKeyServicePayload(fmt.Sprintf("daemon key service returned duplicate key ID %q across pages", key.KeyID), nil)
			}
			seenKeys[key.KeyID] = struct{}{}
			all = append(all, key)
		}
		if page.NextCursor == "" {
			return all, nil
		}
		if _, exists := seenCursors[page.NextCursor]; exists {
			return nil, invalidDaemonKeyServicePayload("daemon key service repeated a managed signing key cursor", nil)
		}
		seenCursors[page.NextCursor] = struct{}{}
		cursor = page.NextCursor
	}
	return nil, invalidDaemonKeyServicePayload("managed signing list exceeded the bounded auto-pagination page limit", nil)
}

// ListPage returns one bounded page of public managed-signing projections.
func (c *ManagedSigningClient) ListPage(
	filter ManagedSigningKeyFilter,
	options ManagedSigningPageOptions,
) (ManagedSigningKeyPage, error) {
	if err := requireManagedSigningClient(c); err != nil {
		return ManagedSigningKeyPage{}, err
	}
	limit, cursor, err := normalizeManagedSigningPageOptions(options)
	if err != nil {
		return ManagedSigningKeyPage{}, err
	}
	payload := map[string]any{"method": "inventory.list", "limit": limit}
	if cursor != "" {
		payload["cursor"] = cursor
	}
	if filter.Purpose != "" {
		purpose, err := managedSigningRequiredText("purpose filter", filter.Purpose)
		if err != nil {
			return ManagedSigningKeyPage{}, err
		}
		payload["purpose"] = purpose
	}
	if filter.Status != "" {
		if !filter.Status.valid() {
			return ManagedSigningKeyPage{}, invalidDaemonKeyServiceInput(fmt.Sprintf("unsupported managed signing status %q", filter.Status))
		}
		payload["status"] = filter.Status
	}
	response, err := c.service.call(payload)
	if err != nil {
		return ManagedSigningKeyPage{}, err
	}
	page, err := decodeManagedSigningKeysPageResponse(response, limit, cursor)
	if err != nil {
		return ManagedSigningKeyPage{}, err
	}
	for _, key := range page.Keys {
		if filter.Purpose != "" && key.Purpose != strings.TrimSpace(filter.Purpose) {
			return ManagedSigningKeyPage{}, invalidDaemonKeyServicePayload("daemon key service returned a key outside the requested purpose filter", nil)
		}
		if filter.Status != "" && key.Status != filter.Status {
			return ManagedSigningKeyPage{}, invalidDaemonKeyServicePayload("daemon key service returned a key outside the requested status filter", nil)
		}
	}
	return page, nil
}

// PublicProjection resolves one key's complete public policy projection.
func (c *ManagedSigningClient) PublicProjection(keyID string) (ManagedSigningKey, error) {
	if err := requireManagedSigningClient(c); err != nil {
		return ManagedSigningKey{}, err
	}
	keyID, err := managedSigningRequiredText("key ID", keyID)
	if err != nil {
		return ManagedSigningKey{}, err
	}
	response, err := c.service.call(map[string]any{
		"method": "inventory.public_key",
		"key_id": keyID,
	})
	if err != nil {
		return ManagedSigningKey{}, err
	}
	projection, err := decodeManagedSigningKeyResponse(response)
	if err != nil {
		return ManagedSigningKey{}, err
	}
	if projection.KeyID != keyID {
		return ManagedSigningKey{}, invalidDaemonKeyServicePayload("daemon key service returned a public projection for a different key ID", nil)
	}
	return projection, nil
}

// Sign requests a signature over caller-supplied canonical bytes. The daemon
// enforces active/unexpired lifecycle state and immutable subject policy.
func (c *ManagedSigningClient) Sign(keyID string, canonicalBytes []byte) ([]byte, error) {
	keyID, err := validateManagedSigningRequest(keyID, canonicalBytes)
	if err != nil {
		return nil, err
	}
	signer, err := c.Signer(keyID)
	if err != nil {
		return nil, err
	}
	return signer.SignCanonical(canonicalBytes)
}

// signWithProjection is the sole inventory.sign protocol operation. It is
// intentionally unexported and is reachable only through a validated,
// key-bound ManagedSigner.
func (c *ManagedSigningClient) signWithProjection(projection ManagedSigningKey, canonicalBytes []byte) ([]byte, error) {
	if err := requireManagedSigningClient(c); err != nil {
		return nil, err
	}
	keyID, err := validateManagedSigningRequest(projection.KeyID, canonicalBytes)
	if err != nil {
		return nil, err
	}
	if projection.BoundSubjectURA == "" || projection.SignerPolicyRef == "" {
		return nil, invalidDaemonKeyServiceInput("managed signer requires a bound subject and signer policy reference")
	}
	response, err := c.service.call(map[string]any{
		"method":              "inventory.sign",
		"key_id":              keyID,
		"expected_purpose":    projection.Purpose,
		"subject_ura":         projection.BoundSubjectURA,
		"signer_policy_ref":   projection.SignerPolicyRef,
		"canonical_bytes_b64": base64.StdEncoding.EncodeToString(canonicalBytes),
	})
	if err != nil {
		return nil, err
	}
	if err := requireDaemonKeyServiceResult(response, "signature", "signature_b64"); err != nil {
		return nil, err
	}
	encoded, err := daemonKeyServiceResponseString(response, "signature_b64")
	if err != nil {
		return nil, err
	}
	signature, err := decodeCanonicalDaemonKeyServiceBase64(encoded, ed25519.SignatureSize, "Ed25519 signature")
	if err != nil {
		return nil, invalidDaemonKeyServicePayload("daemon key service returned an invalid Ed25519 signature", err)
	}
	return signature, nil
}

func (c *ManagedSigningClient) Rotate(keyID string) (ManagedSigningKey, error) {
	if err := requireManagedSigningClient(c); err != nil {
		return ManagedSigningKey{}, err
	}
	keyID, err := managedSigningRequiredText("key ID", keyID)
	if err != nil {
		return ManagedSigningKey{}, err
	}
	response, err := c.service.call(map[string]any{"method": "inventory.rotate", "key_id": keyID})
	if err != nil {
		return ManagedSigningKey{}, err
	}
	rotated, err := decodeManagedSigningKeyResponse(response)
	if err != nil {
		return ManagedSigningKey{}, err
	}
	if rotated.KeyID == keyID || rotated.RotatedFrom != keyID ||
		rotated.Status != ManagedSigningStatusActive || rotated.RotationEpoch == 0 {
		return ManagedSigningKey{}, invalidDaemonKeyServicePayload(
			"daemon key service violated managed signing rotate postconditions",
			nil,
		)
	}
	return rotated, nil
}

// Revoke moves an active or retired key to its terminal state and returns the
// daemon-issued transition timestamp.
func (c *ManagedSigningClient) Revoke(keyID string) (int64, error) {
	if err := requireManagedSigningClient(c); err != nil {
		return 0, err
	}
	keyID, err := managedSigningRequiredText("key ID", keyID)
	if err != nil {
		return 0, err
	}
	response, err := c.service.call(map[string]any{"method": "inventory.revoke", "key_id": keyID})
	if err != nil {
		return 0, err
	}
	if err := requireDaemonKeyServiceResult(response, "inventory_revoked", "revoked_unix_ms"); err != nil {
		return 0, err
	}
	var revokedUnixMS int64
	if err := decodeDaemonKeyServiceField(response, "revoked_unix_ms", &revokedUnixMS); err != nil {
		return 0, err
	}
	if revokedUnixMS <= 0 {
		return 0, invalidDaemonKeyServicePayload("daemon key service returned an invalid revocation timestamp", nil)
	}
	return revokedUnixMS, nil
}

func (c *ManagedSigningClient) SetExpiry(keyID string, expiresUnixMS int64) error {
	if err := requireManagedSigningClient(c); err != nil {
		return err
	}
	keyID, err := managedSigningRequiredText("key ID", keyID)
	if err != nil {
		return err
	}
	if expiresUnixMS <= 0 {
		return invalidDaemonKeyServiceInput("managed signing expiry must be a positive Unix millisecond timestamp")
	}
	response, err := c.service.call(map[string]any{
		"method":          "inventory.set_expiry",
		"key_id":          keyID,
		"expires_unix_ms": expiresUnixMS,
	})
	if err != nil {
		return err
	}
	return requireDaemonKeyServiceResult(response, "ok")
}

func (c *ManagedSigningClient) BindSubject(keyID, subjectURA string) error {
	if err := requireManagedSigningClient(c); err != nil {
		return err
	}
	keyID, err := managedSigningRequiredText("key ID", keyID)
	if err != nil {
		return err
	}
	subjectURA, err = managedSigningRequiredText("subject URA", subjectURA)
	if err != nil {
		return err
	}
	response, err := c.service.call(map[string]any{
		"method":      "inventory.bind_subject",
		"key_id":      keyID,
		"subject_ura": subjectURA,
	})
	if err != nil {
		return err
	}
	return requireDaemonKeyServiceResult(response, "ok")
}

// AddPeer creates or refreshes one public trust projection. The returned flag
// is true only when the daemon inserted a new peer.
func (c *ManagedSigningClient) AddPeer(registration ManagedSigningPeerRegistration) (bool, error) {
	if err := requireManagedSigningClient(c); err != nil {
		return false, err
	}
	peerURA, err := managedSigningRequiredText("peer URA", registration.PeerURA)
	if err != nil {
		return false, err
	}
	if len(registration.PublicKey) != ed25519.PublicKeySize {
		return false, invalidDaemonKeyServiceInput("managed signing peer public key must be 32 bytes")
	}
	payload := map[string]any{
		"method":         "inventory.peer_add",
		"peer_ura":       peerURA,
		"public_key_b64": base64.StdEncoding.EncodeToString(registration.PublicKey),
	}
	if registration.ViaHubURA != "" {
		viaHubURA, err := managedSigningRequiredText("peer via-hub URA", registration.ViaHubURA)
		if err != nil {
			return false, err
		}
		payload["via_hub"] = viaHubURA
	}
	response, err := c.service.call(payload)
	if err != nil {
		return false, err
	}
	if err := requireDaemonKeyServiceResult(response, "inventory_peer_added", "added"); err != nil {
		return false, err
	}
	var added bool
	if err := decodeDaemonKeyServiceField(response, "added", &added); err != nil {
		return false, err
	}
	return added, nil
}

func (c *ManagedSigningClient) ListPeers() ([]ManagedSigningPeer, error) {
	if err := requireManagedSigningClient(c); err != nil {
		return nil, err
	}
	all := make([]ManagedSigningPeer, 0)
	seenPeers := make(map[string]struct{})
	seenCursors := make(map[string]struct{})
	cursor := ""
	for pageIndex := 0; pageIndex < managedSigningAutoPaginationMaxPages; pageIndex++ {
		page, err := c.ListPeersPage(ManagedSigningPageOptions{
			Limit:  ManagedSigningDefaultPageLimit,
			Cursor: cursor,
		})
		if err != nil {
			return nil, err
		}
		if len(all)+len(page.Peers) > managedSigningAutoPaginationMaxItems {
			return nil, invalidDaemonKeyServicePayload("managed signing peer list exceeded the bounded auto-pagination item limit", nil)
		}
		for _, peer := range page.Peers {
			if _, exists := seenPeers[peer.PeerURA]; exists {
				return nil, invalidDaemonKeyServicePayload(fmt.Sprintf("daemon key service returned duplicate peer URA %q across pages", peer.PeerURA), nil)
			}
			seenPeers[peer.PeerURA] = struct{}{}
			all = append(all, peer)
		}
		if page.NextCursor == "" {
			return all, nil
		}
		if len(page.Peers) == 0 {
			return nil, invalidDaemonKeyServicePayload("daemon key service returned an empty peer page with a continuation cursor", nil)
		}
		if _, exists := seenCursors[page.NextCursor]; exists {
			return nil, invalidDaemonKeyServicePayload("daemon key service repeated a managed signing peer cursor", nil)
		}
		seenCursors[page.NextCursor] = struct{}{}
		cursor = page.NextCursor
	}
	return nil, invalidDaemonKeyServicePayload("managed signing peer list exceeded the bounded auto-pagination page limit", nil)
}

// ListPeersPage returns one bounded page of public peer projections.
func (c *ManagedSigningClient) ListPeersPage(options ManagedSigningPageOptions) (ManagedSigningPeerPage, error) {
	if err := requireManagedSigningClient(c); err != nil {
		return ManagedSigningPeerPage{}, err
	}
	limit, cursor, err := normalizeManagedSigningPageOptions(options)
	if err != nil {
		return ManagedSigningPeerPage{}, err
	}
	payload := map[string]any{"method": "inventory.peer_list", "limit": limit}
	if cursor != "" {
		payload["cursor"] = cursor
	}
	response, err := c.service.call(payload)
	if err != nil {
		return ManagedSigningPeerPage{}, err
	}
	return decodeManagedSigningPeersPageResponse(response, limit, cursor)
}

type managedSigningKeyWire struct {
	KeyID           string               `json:"key_id"`
	Purpose         string               `json:"purpose"`
	PublicKeyB64    string               `json:"public_key_b64"`
	Status          ManagedSigningStatus `json:"status"`
	RotationEpoch   *uint64              `json:"rotation_epoch"`
	BoundSubject    *string              `json:"bound_subject"`
	SignerPolicyRef *string              `json:"signer_policy_ref"`
	RotatedFrom     *string              `json:"rotated_from"`
	CreatedUnixMS   *int64               `json:"created_unix_ms"`
	ExpiresUnixMS   *int64               `json:"expires_unix_ms"`
	RevokedUnixMS   *int64               `json:"revoked_unix_ms"`
}

type managedSigningPeerWire struct {
	PeerURA        string  `json:"peer_ura"`
	FingerprintB64 string  `json:"fingerprint_b64"`
	PublicKeyB64   string  `json:"public_key_b64"`
	ViaHub         *string `json:"via_hub"`
	AddedUnixMS    *int64  `json:"added_unix_ms"`
	LastSeenUnixMS *int64  `json:"last_seen_unix_ms"`
}

func decodeManagedSigningKeyResponse(response map[string]json.RawMessage) (ManagedSigningKey, error) {
	if err := requireDaemonKeyServiceResult(response, "inventory_key", "entry"); err != nil {
		return ManagedSigningKey{}, err
	}
	var wire managedSigningKeyWire
	if err := decodeDaemonKeyServiceField(response, "entry", &wire); err != nil {
		return ManagedSigningKey{}, err
	}
	return projectManagedSigningKey(wire)
}

func decodeManagedSigningKeysPageResponse(
	response map[string]json.RawMessage,
	limit uint32,
	requestCursor string,
) (ManagedSigningKeyPage, error) {
	if err := requireDaemonKeyServiceResult(response, "inventory_keys", "entries", "next_cursor"); err != nil {
		return ManagedSigningKeyPage{}, err
	}
	var wires []managedSigningKeyWire
	if err := decodeDaemonKeyServiceField(response, "entries", &wires); err != nil {
		return ManagedSigningKeyPage{}, err
	}
	if len(wires) > int(limit) {
		return ManagedSigningKeyPage{}, invalidDaemonKeyServicePayload("daemon key service returned a key page larger than the requested limit", nil)
	}
	keys := make([]ManagedSigningKey, 0, len(wires))
	seen := make(map[string]struct{}, len(wires))
	for _, wire := range wires {
		key, err := projectManagedSigningKey(wire)
		if err != nil {
			return ManagedSigningKeyPage{}, err
		}
		if _, exists := seen[key.KeyID]; exists {
			return ManagedSigningKeyPage{}, invalidDaemonKeyServicePayload(fmt.Sprintf("daemon key service returned duplicate key ID %q", key.KeyID), nil)
		}
		seen[key.KeyID] = struct{}{}
		keys = append(keys, key)
	}
	nextCursor, err := decodeManagedSigningNextCursor(response, requestCursor)
	if err != nil {
		return ManagedSigningKeyPage{}, err
	}
	return ManagedSigningKeyPage{Keys: keys, NextCursor: nextCursor}, nil
}

func projectManagedSigningKey(wire managedSigningKeyWire) (ManagedSigningKey, error) {
	keyID, err := managedSigningProjectionText("key_id", wire.KeyID)
	if err != nil {
		return ManagedSigningKey{}, err
	}
	purpose, err := managedSigningProjectionText("purpose", wire.Purpose)
	if err != nil {
		return ManagedSigningKey{}, err
	}
	if !wire.Status.valid() {
		return ManagedSigningKey{}, invalidDaemonKeyServicePayload(fmt.Sprintf("daemon key service returned unsupported managed signing status %q", wire.Status), nil)
	}
	publicKey, err := decodeCanonicalDaemonKeyServiceBase64(wire.PublicKeyB64, ed25519.PublicKeySize, "managed Ed25519 public key")
	if err != nil {
		return ManagedSigningKey{}, invalidDaemonKeyServicePayload("daemon key service returned an invalid managed Ed25519 public key", err)
	}
	if wire.RotationEpoch == nil {
		return ManagedSigningKey{}, invalidDaemonKeyServicePayload("daemon key service response missing managed signing rotation_epoch", nil)
	}
	if wire.CreatedUnixMS == nil || *wire.CreatedUnixMS <= 0 {
		return ManagedSigningKey{}, invalidDaemonKeyServicePayload("daemon key service returned an invalid managed signing created_unix_ms", nil)
	}
	boundSubjectURA, err := optionalManagedSigningText("bound_subject", wire.BoundSubject)
	if err != nil {
		return ManagedSigningKey{}, err
	}
	signerPolicyRef, err := optionalManagedSigningText("signer_policy_ref", wire.SignerPolicyRef)
	if err != nil {
		return ManagedSigningKey{}, err
	}
	rotatedFrom, err := optionalManagedSigningText("rotated_from", wire.RotatedFrom)
	if err != nil {
		return ManagedSigningKey{}, err
	}
	if *wire.RotationEpoch == 0 && rotatedFrom != "" {
		return ManagedSigningKey{}, invalidDaemonKeyServicePayload("generation-zero managed signing key cannot have rotated_from", nil)
	}
	if *wire.RotationEpoch > 0 && rotatedFrom == "" {
		return ManagedSigningKey{}, invalidDaemonKeyServicePayload("rotated managed signing key is missing rotated_from", nil)
	}
	if rotatedFrom == keyID {
		return ManagedSigningKey{}, invalidDaemonKeyServicePayload("managed signing key cannot rotate from itself", nil)
	}
	if wire.Status == ManagedSigningStatusRevoked {
		if wire.RevokedUnixMS == nil || *wire.RevokedUnixMS < *wire.CreatedUnixMS {
			return ManagedSigningKey{}, invalidDaemonKeyServicePayload("revoked managed signing key has an invalid revoked_unix_ms", nil)
		}
	} else if wire.RevokedUnixMS != nil {
		return ManagedSigningKey{}, invalidDaemonKeyServicePayload("non-revoked managed signing key contains revoked_unix_ms", nil)
	}
	if wire.ExpiresUnixMS != nil && *wire.ExpiresUnixMS <= 0 {
		return ManagedSigningKey{}, invalidDaemonKeyServicePayload("managed signing key contains an invalid expires_unix_ms", nil)
	}
	expectedPolicyRef := ""
	if boundSubjectURA != "" {
		expectedPolicyRef = canonicalManagedSignerPolicyRef(
			purpose,
			boundSubjectURA,
			keyID,
			ed25519.PublicKey(publicKey),
		)
	}
	if signerPolicyRef != expectedPolicyRef {
		return ManagedSigningKey{}, invalidDaemonKeyServicePayload("managed signing signer_policy_ref does not match its canonical subject/key projection", nil)
	}
	return ManagedSigningKey{
		KeyID:           keyID,
		Purpose:         purpose,
		PublicKey:       ed25519.PublicKey(publicKey),
		Status:          wire.Status,
		RotationEpoch:   *wire.RotationEpoch,
		BoundSubjectURA: boundSubjectURA,
		SignerPolicyRef: signerPolicyRef,
		RotatedFrom:     rotatedFrom,
		CreatedUnixMS:   *wire.CreatedUnixMS,
		ExpiresUnixMS:   cloneInt64Pointer(wire.ExpiresUnixMS),
		RevokedUnixMS:   cloneInt64Pointer(wire.RevokedUnixMS),
	}, nil
}

func decodeManagedSigningPeersPageResponse(
	response map[string]json.RawMessage,
	limit uint32,
	requestCursor string,
) (ManagedSigningPeerPage, error) {
	if err := requireDaemonKeyServiceResult(response, "inventory_peers", "peers", "next_cursor"); err != nil {
		return ManagedSigningPeerPage{}, err
	}
	var wires []managedSigningPeerWire
	if err := decodeDaemonKeyServiceField(response, "peers", &wires); err != nil {
		return ManagedSigningPeerPage{}, err
	}
	if len(wires) > int(limit) {
		return ManagedSigningPeerPage{}, invalidDaemonKeyServicePayload("daemon key service returned a peer page larger than the requested limit", nil)
	}
	peers := make([]ManagedSigningPeer, 0, len(wires))
	seen := make(map[string]struct{}, len(wires))
	for _, wire := range wires {
		peerURA, err := managedSigningProjectionText("peer_ura", wire.PeerURA)
		if err != nil {
			return ManagedSigningPeerPage{}, err
		}
		if _, exists := seen[peerURA]; exists {
			return ManagedSigningPeerPage{}, invalidDaemonKeyServicePayload(fmt.Sprintf("daemon key service returned duplicate peer URA %q", peerURA), nil)
		}
		publicKey, err := decodeCanonicalDaemonKeyServiceBase64(wire.PublicKeyB64, ed25519.PublicKeySize, "peer Ed25519 public key")
		if err != nil {
			return ManagedSigningPeerPage{}, invalidDaemonKeyServicePayload("daemon key service returned an invalid peer Ed25519 public key", err)
		}
		fingerprint, err := decodeCanonicalDaemonKeyServiceBase64(wire.FingerprintB64, sha256.Size, "peer fingerprint")
		if err != nil {
			return ManagedSigningPeerPage{}, invalidDaemonKeyServicePayload("daemon key service returned an invalid peer fingerprint", err)
		}
		expectedFingerprint := sha256.Sum256(publicKey)
		if !bytes.Equal(fingerprint, expectedFingerprint[:]) {
			return ManagedSigningPeerPage{}, invalidDaemonKeyServicePayload("daemon key service returned a peer fingerprint that does not match SHA-256(public_key)", nil)
		}
		viaHubURA, err := optionalManagedSigningText("via_hub", wire.ViaHub)
		if err != nil {
			return ManagedSigningPeerPage{}, err
		}
		if wire.AddedUnixMS == nil || wire.LastSeenUnixMS == nil ||
			*wire.AddedUnixMS <= 0 || *wire.LastSeenUnixMS < *wire.AddedUnixMS {
			return ManagedSigningPeerPage{}, invalidDaemonKeyServicePayload("daemon key service returned invalid managed peer timestamps", nil)
		}
		seen[peerURA] = struct{}{}
		peers = append(peers, ManagedSigningPeer{
			PeerURA:        peerURA,
			Fingerprint:    append([]byte(nil), fingerprint...),
			PublicKey:      ed25519.PublicKey(publicKey),
			ViaHubURA:      viaHubURA,
			AddedUnixMS:    *wire.AddedUnixMS,
			LastSeenUnixMS: *wire.LastSeenUnixMS,
		})
	}
	nextCursor, err := decodeManagedSigningNextCursor(response, requestCursor)
	if err != nil {
		return ManagedSigningPeerPage{}, err
	}
	return ManagedSigningPeerPage{Peers: peers, NextCursor: nextCursor}, nil
}

func decodeDaemonKeyServiceField(response map[string]json.RawMessage, field string, target any) error {
	raw, ok := response[field]
	if !ok {
		return invalidDaemonKeyServicePayload(fmt.Sprintf("daemon key-service response missing %s", field), nil)
	}
	if err := decodeDaemonKeyServiceJSON(raw, target, true); err != nil {
		return invalidDaemonKeyServicePayload(fmt.Sprintf("daemon key-service response field %s is invalid", field), err)
	}
	return nil
}

func normalizeManagedSigningPageOptions(options ManagedSigningPageOptions) (uint32, string, error) {
	limit := options.Limit
	if limit == 0 {
		limit = ManagedSigningDefaultPageLimit
	}
	if limit > ManagedSigningMaxPageLimit {
		return 0, "", invalidDaemonKeyServiceInput(
			fmt.Sprintf("managed signing page limit must be at most %d", ManagedSigningMaxPageLimit),
		)
	}
	cursor := options.Cursor
	if cursor != "" && (strings.TrimSpace(cursor) == "" || strings.TrimSpace(cursor) != cursor) {
		return 0, "", invalidDaemonKeyServiceInput("managed signing cursor must be a non-empty canonical token")
	}
	if len(cursor) > managedSigningMaxCursorBytes {
		return 0, "", invalidDaemonKeyServiceInput("managed signing cursor exceeds 4096 bytes")
	}
	return limit, cursor, nil
}

func decodeManagedSigningNextCursor(
	response map[string]json.RawMessage,
	requestCursor string,
) (string, error) {
	var nextCursor *string
	if err := decodeDaemonKeyServiceField(response, "next_cursor", &nextCursor); err != nil {
		return "", err
	}
	if nextCursor == nil {
		return "", nil
	}
	normalized := strings.TrimSpace(*nextCursor)
	if normalized == "" || normalized != *nextCursor {
		return "", invalidDaemonKeyServicePayload("daemon key service returned an invalid continuation cursor", nil)
	}
	if len(*nextCursor) > managedSigningMaxCursorBytes {
		return "", invalidDaemonKeyServicePayload("daemon key service returned a continuation cursor exceeding 4096 bytes", nil)
	}
	if *nextCursor == requestCursor {
		return "", invalidDaemonKeyServicePayload("daemon key service returned a non-advancing continuation cursor", nil)
	}
	return *nextCursor, nil
}

func decodeCanonicalDaemonKeyServiceBase64(encoded string, expectedLength int, field string) ([]byte, error) {
	decoded, err := base64.StdEncoding.DecodeString(encoded)
	if err != nil {
		return nil, fmt.Errorf("decode %s: %w", field, err)
	}
	if len(decoded) != expectedLength {
		return nil, fmt.Errorf("%s length is %d, want %d", field, len(decoded), expectedLength)
	}
	if base64.StdEncoding.EncodeToString(decoded) != encoded {
		return nil, fmt.Errorf("%s is not canonical base64", field)
	}
	return decoded, nil
}

func canonicalManagedSignerPolicyRef(
	purpose string,
	subjectURA string,
	keyID string,
	publicKey ed25519.PublicKey,
) string {
	digest := sha256.New()
	components := []string{
		"canonical-runtime.managed-signing.policy",
		"v2",
		purpose,
		subjectURA,
		keyID,
		base64.StdEncoding.EncodeToString(publicKey),
	}
	for _, component := range components {
		_, _ = digest.Write([]byte(component))
		_, _ = digest.Write([]byte{0})
	}
	return fmt.Sprintf("managed-signing:v2:sha256:%x", digest.Sum(nil)[:16])
}

func validateManagedSigningRequest(keyID string, canonicalBytes []byte) (string, error) {
	keyID, err := managedSigningRequiredText("key ID", keyID)
	if err != nil {
		return "", err
	}
	if len(canonicalBytes) == 0 {
		return "", invalidDaemonKeyServiceInput("canonical bytes are required for managed signing")
	}
	if len(canonicalBytes) > daemonKeyServiceMaxCanonicalSigningBytes {
		return "", invalidDaemonKeyServiceInput("canonical bytes exceed the 64 MiB runtime signing limit")
	}
	return keyID, nil
}

func requireManagedSigningClient(client *ManagedSigningClient) error {
	if client == nil || client.service.socketPath == "" {
		return invalidDaemonKeyServiceInput("managed signing client is required")
	}
	return nil
}

func managedSigningRequiredText(field, value string) (string, error) {
	normalized := strings.TrimSpace(value)
	if normalized == "" {
		return "", invalidDaemonKeyServiceInput(fmt.Sprintf("managed signing %s is required", field))
	}
	return normalized, nil
}

func managedSigningProjectionText(field, value string) (string, error) {
	normalized := strings.TrimSpace(value)
	if normalized == "" || normalized != value {
		return "", invalidDaemonKeyServicePayload(fmt.Sprintf("daemon key service returned an invalid managed signing %s", field), nil)
	}
	return value, nil
}

func optionalManagedSigningText(field string, value *string) (string, error) {
	if value == nil {
		return "", nil
	}
	normalized := strings.TrimSpace(*value)
	if normalized == "" || normalized != *value {
		return "", invalidDaemonKeyServicePayload(fmt.Sprintf("daemon key service returned an invalid managed signing %s", field), nil)
	}
	return *value, nil
}

func cloneInt64Pointer(value *int64) *int64 {
	if value == nil {
		return nil
	}
	copy := *value
	return &copy
}

func cloneManagedSigningKey(key ManagedSigningKey) ManagedSigningKey {
	key.PublicKey = key.SigningPublicKey()
	key.ExpiresUnixMS = cloneInt64Pointer(key.ExpiresUnixMS)
	key.RevokedUnixMS = cloneInt64Pointer(key.RevokedUnixMS)
	return key
}
