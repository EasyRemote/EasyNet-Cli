package easynet

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"
)

// CompanionDesiredState is the operator-desired desktop companion state.
type CompanionDesiredState string

const (
	CompanionDesiredEnabled  CompanionDesiredState = "enabled"
	CompanionDesiredDisabled CompanionDesiredState = "disabled"
)

// CompanionSupervisorState is the OS user-session supervisor state.
type CompanionSupervisorState string

const (
	CompanionSupervisorUnsupportedPlatform CompanionSupervisorState = "unsupported_platform"
	CompanionSupervisorUnsupportedSession  CompanionSupervisorState = "unsupported_session"
	CompanionSupervisorNotInstalled        CompanionSupervisorState = "not_installed"
	CompanionSupervisorInstalledDisabled   CompanionSupervisorState = "installed_disabled"
	CompanionSupervisorInstalledEnabled    CompanionSupervisorState = "installed_enabled"
	CompanionSupervisorInstallError        CompanionSupervisorState = "install_error"
	CompanionSupervisorEnableError         CompanionSupervisorState = "enable_error"
	CompanionSupervisorDisableError        CompanionSupervisorState = "disable_error"
)

// CompanionObservedState is process or heartbeat state observed independently
// from supervisor metadata.
type CompanionObservedState string

const (
	CompanionObservedUnknown         CompanionObservedState = "unknown"
	CompanionObservedNotRunning      CompanionObservedState = "not_running"
	CompanionObservedStarting        CompanionObservedState = "starting"
	CompanionObservedRunning         CompanionObservedState = "running"
	CompanionObservedStale           CompanionObservedState = "stale"
	CompanionObservedExited          CompanionObservedState = "exited"
	CompanionObservedVersionMismatch CompanionObservedState = "version_mismatch"
	CompanionObservedHealthError     CompanionObservedState = "health_error"
)

// CompanionProjectedState is the operator-facing state derived from desired,
// supervisor, and observed facts.
type CompanionProjectedState string

const (
	CompanionProjectedDisabled            CompanionProjectedState = "disabled"
	CompanionProjectedUnsupportedPlatform CompanionProjectedState = "unsupported_platform"
	CompanionProjectedUnsupportedSession  CompanionProjectedState = "unsupported_session"
	CompanionProjectedNotInstalled        CompanionProjectedState = "not_installed"
	CompanionProjectedInstalledDisabled   CompanionProjectedState = "installed_disabled"
	CompanionProjectedReadyStopped        CompanionProjectedState = "ready_stopped"
	CompanionProjectedStarting            CompanionProjectedState = "starting"
	CompanionProjectedRunning             CompanionProjectedState = "running"
	CompanionProjectedStale               CompanionProjectedState = "stale"
	CompanionProjectedError               CompanionProjectedState = "error"
)

// DesktopCompanionStatus is the shared SDK/control-plane companion status DTO.
type DesktopCompanionStatus struct {
	Profile         string                   `json:"profile,omitempty"`
	Kind            string                   `json:"kind,omitempty"`
	PackageID       string                   `json:"package_id"`
	PackageVersion  string                   `json:"package_version"`
	DisplayName     string                   `json:"display_name"`
	Platform        string                   `json:"platform"`
	DesiredState    CompanionDesiredState    `json:"desired_state"`
	SupervisorState CompanionSupervisorState `json:"supervisor_state"`
	ObservedState   CompanionObservedState   `json:"observed_state"`
	ProjectedState  CompanionProjectedState  `json:"projected_state"`
	BootPolicy      string                   `json:"boot_policy"`
	StopPolicy      string                   `json:"stop_policy"`
	Health          string                   `json:"health"`
	PID             *uint64                  `json:"pid,omitempty"`
	Version         string                   `json:"version,omitempty"`
	LastSeenUnixMS  *uint64                  `json:"last_seen_unix_ms,omitempty"`
	LaunchMethod    string                   `json:"launch_method,omitempty"`
	Error           map[string]any           `json:"error,omitempty"`
	Metadata        map[string]any           `json:"metadata,omitempty"`
}

// DesktopCompanionList is the shared local companion list DTO.
type DesktopCompanionList struct {
	Kind       string                   `json:"kind,omitempty"`
	Companions []DesktopCompanionStatus `json:"companions"`
}

// DesktopCompanionActionResult is the shared companion lifecycle result DTO.
type DesktopCompanionActionResult struct {
	Profile      string                  `json:"profile,omitempty"`
	Kind         string                  `json:"kind,omitempty"`
	PackageID    string                  `json:"package_id"`
	Action       string                  `json:"action"`
	Changed      bool                    `json:"changed"`
	StatusBefore *DesktopCompanionStatus `json:"status_before,omitempty"`
	StatusAfter  *DesktopCompanionStatus `json:"status_after,omitempty"`
	Error        map[string]any          `json:"error,omitempty"`
	Metadata     map[string]any          `json:"metadata,omitempty"`
}

// CompanionTransport is the optional daemon transport capability for local
// desktop companion lifecycle calls.
type CompanionTransport interface {
	CompanionList(ctx context.Context, handleID string) ([]byte, error)
	CompanionStatus(ctx context.Context, handleID string, packageID string, packageVersion string) ([]byte, error)
	CompanionEnable(ctx context.Context, handleID string, packageID string, packageVersion string) ([]byte, error)
	CompanionDisable(ctx context.Context, handleID string, packageID string, packageVersion string) ([]byte, error)
	CompanionStart(ctx context.Context, handleID string, packageID string, packageVersion string) ([]byte, error)
	CompanionStop(ctx context.Context, handleID string, packageID string, packageVersion string) ([]byte, error)
}

// CompanionList returns local desktop companion statuses for this daemon.
func (h *DaemonHandle) CompanionList(ctx context.Context) (DesktopCompanionList, error) {
	transport, err := h.requireCompanionTransport(ctx)
	if err != nil {
		return DesktopCompanionList{}, err
	}
	raw, err := transport.CompanionList(ctx, h.handleID)
	if err != nil {
		return DesktopCompanionList{}, wrapDaemonTransportError("desktop companion list failed", err)
	}
	return NewDesktopCompanionListFromJSON(raw)
}

// CompanionStatus returns one local desktop companion status.
func (h *DaemonHandle) CompanionStatus(ctx context.Context, packageID string, packageVersion string) (DesktopCompanionStatus, error) {
	transport, packageID, packageVersion, err := h.companionActionInput(ctx, packageID, packageVersion)
	if err != nil {
		return DesktopCompanionStatus{}, err
	}
	raw, err := transport.CompanionStatus(ctx, h.handleID, packageID, packageVersion)
	if err != nil {
		return DesktopCompanionStatus{}, wrapDaemonTransportError("desktop companion status failed", err)
	}
	return NewDesktopCompanionStatusFromJSON(raw)
}

// CompanionEnable enables one local desktop companion.
func (h *DaemonHandle) CompanionEnable(ctx context.Context, packageID string, packageVersion string) (DesktopCompanionActionResult, error) {
	return h.companionAction(ctx, "enable", packageID, packageVersion)
}

// CompanionDisable disables one local desktop companion.
func (h *DaemonHandle) CompanionDisable(ctx context.Context, packageID string, packageVersion string) (DesktopCompanionActionResult, error) {
	return h.companionAction(ctx, "disable", packageID, packageVersion)
}

// CompanionStart starts one local desktop companion.
func (h *DaemonHandle) CompanionStart(ctx context.Context, packageID string, packageVersion string) (DesktopCompanionActionResult, error) {
	return h.companionAction(ctx, "start", packageID, packageVersion)
}

// CompanionStop stops one local desktop companion.
func (h *DaemonHandle) CompanionStop(ctx context.Context, packageID string, packageVersion string) (DesktopCompanionActionResult, error) {
	return h.companionAction(ctx, "stop", packageID, packageVersion)
}

func (h *DaemonHandle) companionAction(ctx context.Context, action string, packageID string, packageVersion string) (DesktopCompanionActionResult, error) {
	transport, packageID, packageVersion, err := h.companionActionInput(ctx, packageID, packageVersion)
	if err != nil {
		return DesktopCompanionActionResult{}, err
	}
	var raw []byte
	switch action {
	case "enable":
		raw, err = transport.CompanionEnable(ctx, h.handleID, packageID, packageVersion)
	case "disable":
		raw, err = transport.CompanionDisable(ctx, h.handleID, packageID, packageVersion)
	case "start":
		raw, err = transport.CompanionStart(ctx, h.handleID, packageID, packageVersion)
	case "stop":
		raw, err = transport.CompanionStop(ctx, h.handleID, packageID, packageVersion)
	default:
		return DesktopCompanionActionResult{}, invalidRuntimePayload("unsupported desktop companion action", nil)
	}
	if err != nil {
		return DesktopCompanionActionResult{}, wrapDaemonTransportError("desktop companion "+action+" failed", err)
	}
	return NewDesktopCompanionActionResultFromJSON(raw)
}

func (h *DaemonHandle) companionActionInput(ctx context.Context, packageID string, packageVersion string) (CompanionTransport, string, string, error) {
	transport, err := h.requireCompanionTransport(ctx)
	if err != nil {
		return nil, "", "", err
	}
	packageID = strings.TrimSpace(packageID)
	if packageID == "" {
		return nil, "", "", invalidRuntimePayload("package_id is required", nil)
	}
	return transport, packageID, strings.TrimSpace(packageVersion), nil
}

func (h *DaemonHandle) requireCompanionTransport(ctx context.Context) (CompanionTransport, error) {
	if err := h.requireAttached(); err != nil {
		return nil, err
	}
	if ctx == nil {
		return nil, invalidRuntimeClient("context is required")
	}
	transport, ok := h.transport.(CompanionTransport)
	if !ok {
		return nil, &SDKError{
			Code:      ErrNotImplemented,
			Stage:     "sdk",
			Retry:     RetryNever,
			Retryable: false,
			Message:   "daemon transport does not support desktop companion lifecycle",
		}
	}
	return transport, nil
}

func NewDesktopCompanionStatusFromJSON(raw []byte) (DesktopCompanionStatus, error) {
	var status DesktopCompanionStatus
	if err := json.Unmarshal(raw, &status); err != nil {
		return DesktopCompanionStatus{}, invalidRuntimePayload(fmt.Sprintf("decode desktop companion status JSON: %v", err), err)
	}
	if err := validateDesktopCompanionStatus(status); err != nil {
		return DesktopCompanionStatus{}, err
	}
	return status, nil
}

func NewDesktopCompanionListFromJSON(raw []byte) (DesktopCompanionList, error) {
	var list DesktopCompanionList
	if err := json.Unmarshal(raw, &list); err != nil {
		return DesktopCompanionList{}, invalidRuntimePayload(fmt.Sprintf("decode desktop companion list JSON: %v", err), err)
	}
	for _, status := range list.Companions {
		if err := validateDesktopCompanionStatus(status); err != nil {
			return DesktopCompanionList{}, err
		}
	}
	return list, nil
}

func NewDesktopCompanionActionResultFromJSON(raw []byte) (DesktopCompanionActionResult, error) {
	var result DesktopCompanionActionResult
	if err := json.Unmarshal(raw, &result); err != nil {
		return DesktopCompanionActionResult{}, invalidRuntimePayload(fmt.Sprintf("decode desktop companion action JSON: %v", err), err)
	}
	if result.PackageID == "" || result.Action == "" {
		return DesktopCompanionActionResult{}, invalidRuntimePayload("desktop companion action result is missing required fields", nil)
	}
	return result, nil
}

func validateDesktopCompanionStatus(status DesktopCompanionStatus) error {
	if status.PackageID == "" || status.PackageVersion == "" || status.DisplayName == "" {
		return invalidRuntimePayload("desktop companion status is missing required identity fields", nil)
	}
	if status.Platform == "" || status.DesiredState == "" || status.SupervisorState == "" || status.ObservedState == "" || status.ProjectedState == "" {
		return invalidRuntimePayload("desktop companion status is missing required state fields", nil)
	}
	return nil
}
