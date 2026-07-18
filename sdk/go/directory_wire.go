package easynet

import (
	"encoding/json"
	"errors"
	"fmt"
	"strings"
)

// DirectoryEntry is the EasyNet product projection for one directory row.
// Axon owns invocation and forwarding semantics; the daemon product owns this
// directory record and its lifecycle.
type DirectoryEntry struct {
	AgentURA       string  `json:"agent_ura"`
	NodeID         string  `json:"node_id"`
	DisplayName    *string `json:"display_name"`
	Status         string  `json:"status"`
	OriginRealm    *string `json:"origin_realm"`
	HubEndpoint    *string `json:"hub_endpoint"`
	LastSeenUnixMS *int64  `json:"last_seen_unix_ms"`
}

type DirectorySigningAuthority struct {
	Kind    string `json:"kind"`
	HostURA string `json:"host_ura,omitempty"`
}

type DirectoryAgentSummary struct {
	AgentURA         string                    `json:"agent_ura"`
	SigningAuthority DirectorySigningAuthority `json:"signing_authority"`
	Status           string                    `json:"status"`
	AbilityCount     uint64                    `json:"ability_count"`
}

// DirectoryEvent is the closed product event union emitted by the daemon
// directory provider.
type DirectoryEvent struct {
	Type               string                     `json:"type"`
	Agents             []DirectoryAgentSummary    `json:"agents,omitempty"`
	SnapshotUnixMS     int64                      `json:"snapshot_unix_ms,omitempty"`
	AgentURA           string                     `json:"agent_ura,omitempty"`
	SigningAuthority   *DirectorySigningAuthority `json:"signing_authority,omitempty"`
	ReplacedPrior      *bool                      `json:"replaced_prior,omitempty"`
	WasActive          *bool                      `json:"was_active,omitempty"`
	Reason             string                     `json:"reason,omitempty"`
	OwnerURA           string                     `json:"owner_ura,omitempty"`
	HostDeviceURA      string                     `json:"host_device_ura,omitempty"`
	ProjectionRevision uint64                     `json:"projection_revision,omitempty"`
	ProjectionDigest   string                     `json:"projection_digest,omitempty"`
	AbilityCount       uint64                     `json:"ability_count,omitempty"`
	StaleCount         uint64                     `json:"stale_count,omitempty"`
	RemovedCount       uint64                     `json:"removed_count,omitempty"`
	LeaseExpiresUnixMS int64                      `json:"lease_expires_unix_ms,omitempty"`
	UnixMS             int64                      `json:"unix_ms,omitempty"`
}

func ParseDirectoryEntry(raw []byte) (DirectoryEntry, error) {
	var entry DirectoryEntry
	if err := json.Unmarshal(raw, &entry); err != nil {
		return DirectoryEntry{}, fmt.Errorf("directory entry: decode JSON: %w", err)
	}
	if err := entry.Validate(); err != nil {
		return DirectoryEntry{}, err
	}
	return entry, nil
}

func (entry DirectoryEntry) Validate() error {
	if strings.TrimSpace(entry.AgentURA) == "" {
		return errors.New("directory entry: agent_ura is required")
	}
	if strings.TrimSpace(entry.NodeID) == "" {
		return errors.New("directory entry: node_id is required")
	}
	if strings.TrimSpace(entry.Status) == "" {
		return errors.New("directory entry: status is required")
	}
	return nil
}

func (entry DirectoryEntry) CanonicalJSON() ([]byte, error) {
	if err := entry.Validate(); err != nil {
		return nil, err
	}
	return marshalDirectoryObject(entry)
}

func ParseDirectoryEvent(raw []byte) (DirectoryEvent, error) {
	var event DirectoryEvent
	if err := json.Unmarshal(raw, &event); err != nil {
		return DirectoryEvent{}, fmt.Errorf("directory event: decode JSON: %w", err)
	}
	if err := event.Validate(); err != nil {
		return DirectoryEvent{}, err
	}
	return event, nil
}

func (event DirectoryEvent) Validate() error {
	switch event.Type {
	case "snapshot":
		if event.Agents == nil {
			return errors.New("directory event snapshot: agents is required")
		}
		for index, agent := range event.Agents {
			if err := agent.Validate(); err != nil {
				return fmt.Errorf("directory event snapshot: agents[%d]: %w", index, err)
			}
		}
	case "agent_advertised":
		if strings.TrimSpace(event.AgentURA) == "" {
			return errors.New("directory event agent_advertised: agent_ura is required")
		}
		if event.SigningAuthority == nil {
			return errors.New(
				"directory event agent_advertised: signing_authority is required",
			)
		}
		if err := event.SigningAuthority.Validate(); err != nil {
			return fmt.Errorf("directory event agent_advertised: %w", err)
		}
		if event.ReplacedPrior == nil {
			return errors.New(
				"directory event agent_advertised: replaced_prior is required",
			)
		}
	case "agent_revoked":
		if strings.TrimSpace(event.AgentURA) == "" || strings.TrimSpace(event.Reason) == "" {
			return errors.New(
				"directory event agent_revoked: agent_ura and reason are required",
			)
		}
		if event.WasActive == nil {
			return errors.New("directory event agent_revoked: was_active is required")
		}
	case "heartbeat":
		// unix_ms may be zero in deterministic tests.
	case "owner_projection_changed":
		if strings.TrimSpace(event.OwnerURA) == "" ||
			strings.TrimSpace(event.HostDeviceURA) == "" ||
			strings.TrimSpace(event.ProjectionDigest) == "" {
			return errors.New(
				"directory event owner_projection_changed: owner_ura, host_device_ura and projection_digest are required",
			)
		}
	default:
		return fmt.Errorf("directory event: unsupported type %q", event.Type)
	}
	return nil
}

func (summary DirectoryAgentSummary) Validate() error {
	if strings.TrimSpace(summary.AgentURA) == "" || strings.TrimSpace(summary.Status) == "" {
		return errors.New("agent_ura and status are required")
	}
	return summary.SigningAuthority.Validate()
}

func (authority DirectorySigningAuthority) Validate() error {
	switch authority.Kind {
	case "self_signed":
		if authority.HostURA != "" {
			return errors.New("self_signed authority cannot contain host_ura")
		}
	case "hosted_by":
		if strings.TrimSpace(authority.HostURA) == "" {
			return errors.New("hosted_by authority requires host_ura")
		}
	default:
		return fmt.Errorf("unsupported signing authority kind %q", authority.Kind)
	}
	return nil
}

func (event DirectoryEvent) CanonicalJSON() ([]byte, error) {
	if err := event.Validate(); err != nil {
		return nil, err
	}
	return marshalDirectoryObject(event)
}

func marshalDirectoryObject(value any) ([]byte, error) {
	raw, err := json.Marshal(value)
	if err != nil {
		return nil, err
	}
	var object map[string]any
	if err := json.Unmarshal(raw, &object); err != nil {
		return nil, err
	}
	return json.Marshal(object)
}
