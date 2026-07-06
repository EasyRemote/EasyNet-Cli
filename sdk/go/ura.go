package easynet

import (
	"fmt"
	"net/url"
	"strings"
)

const URAScheme = "easynet:///r/"

type URAKind string

const (
	URAKindUnknown  URAKind = "unknown"
	URAKindUser     URAKind = "user"
	URAKindDevice   URAKind = "device"
	URAKindAgent    URAKind = "agent"
	URAKindAbility  URAKind = "ability"
	URAKindHub      URAKind = "hub"
	URAKindResource URAKind = "resource"
)

type ResourceNamespace string

const (
	ResourceNamespaceFS      ResourceNamespace = "fs"
	ResourceNamespaceProcess ResourceNamespace = "process"
	ResourceNamespacePTY     ResourceNamespace = "pty"
	ResourceNamespaceShell   ResourceNamespace = "shell"
	ResourceNamespaceHTTP    ResourceNamespace = "http"
)

var validResourceNamespaces = map[ResourceNamespace]struct{}{
	ResourceNamespaceFS:      {},
	ResourceNamespaceProcess: {},
	ResourceNamespacePTY:     {},
	ResourceNamespaceShell:   {},
	ResourceNamespaceHTTP:    {},
}

func IsResourceNamespace(namespace string) bool {
	_, ok := validResourceNamespaces[ResourceNamespace(namespace)]
	return ok
}

// Ura is the Go SDK value object for canonical runtime URAs.
type Ura struct {
	raw string
}

type ParsedURA struct {
	Raw               string
	Realm             string
	Kind              URAKind
	UserID            string
	DeviceID          string
	AgentID           string
	AbilityID         string
	AbilityOwner      AbilityOwner
	AbilityNamespace  string
	AbilityLocalName  string
	OwnerID           string
	ResourceNamespace ResourceNamespace
	Path              string
}

func ParseURA(raw string) (Ura, error) {
	if _, err := ParseURAParts(raw); err != nil {
		return Ura{}, err
	}
	return Ura{raw: raw}, nil
}

func (u Ura) String() string { return u.raw }

func (u Ura) Parts() ParsedURA {
	parts, err := ParseURAParts(u.raw)
	if err != nil {
		panic("Ura stores only validated canonical addresses")
	}
	return parts
}

func (u Ura) Kind() URAKind { return u.Parts().Kind }

func (u Ura) AbilityName() string {
	parts := u.Parts()
	if parts.Kind != URAKindAbility {
		return ""
	}
	return parts.AbilityID
}

func (u Ura) PublicAbilityName(registeredName string) string {
	registeredName = strings.TrimSpace(registeredName)
	if registeredName == "" {
		return ""
	}
	return registeredName
}

func (u Ura) PublicAbilityNameForOwner(ownerURA string) string {
	name, ok := PublicAbilityNameFromAbilityURA(ownerURA, u.raw)
	if !ok {
		return ""
	}
	return name
}

func AbilityNameFromURA(raw string) string {
	u, err := ParseURA(raw)
	if err != nil {
		return ""
	}
	return u.AbilityName()
}

func PublicAbilityNameForOwner(ownerURA, registeredName string) string {
	if _, err := ParseURA(ownerURA); err != nil {
		return ""
	}
	return Ura{}.PublicAbilityName(registeredName)
}

func OwnerAbilityURA(ownerURA, abilityName string) string {
	ownerURA = strings.TrimSpace(ownerURA)
	abilityName = strings.TrimSpace(abilityName)
	if ownerURA == "" || abilityName == "" {
		return ""
	}
	owner, err := ParseURAParts(ownerURA)
	if err != nil {
		return ""
	}
	switch owner.Kind {
	case URAKindAgent:
		if owner.UserID == "" || owner.AgentID == "" {
			return ""
		}
		return AbilityURA(owner.Realm, owner.UserID, owner.AgentID, abilityName)
	case URAKindHub:
		if !strings.Contains(abilityName, ".") || strings.HasPrefix(abilityName, "01HUB.") {
			return ""
		}
		return fmt.Sprintf("%s%s/ability/hub.%s", URAScheme, owner.Realm, abilityName)
	case URAKindDevice:
		if owner.DeviceID == "" {
			return ""
		}
		return fmt.Sprintf("%s%s/ability/device.%s.%s", URAScheme, owner.Realm, owner.DeviceID, abilityName)
	default:
		return ""
	}
}

func PublicAbilityNameFromAbilityURA(ownerURA, abilityURA string) (string, bool) {
	owner, ownerErr := ParseURA(ownerURA)
	ability, abilityErr := ParseURA(abilityURA)
	if ownerErr != nil || abilityErr != nil {
		return "", false
	}
	ownerParts := owner.Parts()
	abilityParts := ability.Parts()
	if abilityParts.Kind != URAKindAbility {
		return "", false
	}
	switch ownerParts.Kind {
	case URAKindAgent:
		if ownerParts.Realm == abilityParts.Realm &&
			abilityParts.AbilityOwner.Kind == AbilityOwnerAgent &&
			ownerParts.UserID == abilityParts.AbilityOwner.UserID &&
			ownerParts.AgentID == abilityParts.AbilityOwner.AgentID {
			return abilityParts.AbilityID, true
		}
	case URAKindHub:
		if ownerParts.Realm == abilityParts.Realm &&
			abilityParts.AbilityOwner.Kind == AbilityOwnerHub {
			return ability.AbilityName(), true
		}
	case URAKindDevice:
		if ownerParts.Realm == abilityParts.Realm &&
			abilityParts.AbilityOwner.Kind == AbilityOwnerDevice &&
			ownerParts.DeviceID == abilityParts.AbilityOwner.DeviceID {
			return abilityParts.AbilityID, true
		}
	}
	return "", false
}

func UserURA(realm, userID string) string {
	return Ura{raw: fmt.Sprintf("%s%s/user/%s", URAScheme, realm, userID)}.String()
}

func DeviceURA(realm, deviceID string) string {
	return Ura{raw: fmt.Sprintf("%s%s/device/%s", URAScheme, realm, deviceID)}.String()
}

func AgentURA(realm, userID, agentID string) string {
	return Ura{raw: fmt.Sprintf("%s%s/agent/%s.%s", URAScheme, realm, userID, agentID)}.String()
}

func AbilityURA(realm, userID, agentID, abilityID string) string {
	return Ura{raw: fmt.Sprintf("%s%s/ability/%s.%s.%s", URAScheme, realm, userID, agentID, abilityID)}.String()
}

func HubURA(realm string) string {
	return Ura{raw: fmt.Sprintf("%s%s/hub", URAScheme, realm)}.String()
}

func HubAbilityURA(realm, abilityName string) string {
	if realm == "" || abilityName == "" {
		return ""
	}
	tail := abilityName
	if strings.HasPrefix(tail, "01HUB.") {
		return ""
	}
	if !strings.HasPrefix(tail, "hub.") {
		if !strings.Contains(tail, ".") {
			return ""
		}
		tail = "hub." + tail
	}
	raw := fmt.Sprintf("%s%s/ability/%s", URAScheme, realm, tail)
	if _, err := ParseURA(raw); err != nil {
		return ""
	}
	return raw
}

func ResourceDotURA(realm, ownerID, path string) string {
	clean := strings.TrimPrefix(path, "/")
	if clean == "" {
		return fmt.Sprintf("%s%s/resource/%s", URAScheme, realm, ownerID)
	}
	return fmt.Sprintf("%s%s/resource/%s/%s", URAScheme, realm, ownerID, clean)
}

func ResourceURA(realm, userID, namespace, path string) string {
	if !IsResourceNamespace(namespace) {
		return ""
	}
	clean := strings.TrimPrefix(path, "/")
	return fmt.Sprintf("%s%s/resource/%s/%s/%s", URAScheme, realm, userID, namespace, clean)
}

func FilesResourceURA(realm, username, sha256Hex string) string {
	return ResourceDotURA(realm, username+".files", sha256Hex)
}

func APIKeyResourceURA(realm, token string) string {
	return fmt.Sprintf("%s%s/resource/api_key.%s", URAScheme, realm, token)
}

func PagesResourceURA(realm, username, project, path string) string {
	if path != "" && !strings.HasPrefix(path, "/") {
		path = "/" + path
	}
	return fmt.Sprintf("%s%s/resource/%s.%s%s", URAScheme, realm, username, project, path)
}

func AgentSkillResourceURA(realm, username, agentID, skillName string) string {
	if realm == "" || username == "" || agentID == "" || skillName == "" {
		return ""
	}
	raw := ResourceDotURA(
		url.PathEscape(realm),
		"agent."+url.PathEscape(username)+"."+url.PathEscape(agentID),
		"skill/"+url.PathEscape(skillName),
	)
	if _, err := ParseURA(raw); err != nil {
		return ""
	}
	return raw
}

func AgentSkillFileResourceURA(realm, username, agentID, skillName, relPath string) string {
	base := AgentSkillResourceURA(realm, username, agentID, skillName)
	if base == "" {
		return ""
	}
	relPath = strings.TrimPrefix(relPath, "/")
	if relPath == "" {
		return base
	}
	parts := strings.Split(relPath, "/")
	for i := range parts {
		parts[i] = url.PathEscape(parts[i])
	}
	raw := base + "/file/" + strings.Join(parts, "/")
	if _, err := ParseURA(raw); err != nil {
		return ""
	}
	return raw
}

func RealmUserPrefix(realm string) string   { return fmt.Sprintf("%s%s/user/", URAScheme, realm) }
func RealmDevicePrefix(realm string) string { return fmt.Sprintf("%s%s/device/", URAScheme, realm) }
func RealmAgentPrefix(realm string) string  { return fmt.Sprintf("%s%s/agent/", URAScheme, realm) }

func UserAgentPrefix(realm, userID string) string {
	return fmt.Sprintf("%s%s/agent/%s.", URAScheme, realm, userID)
}

func RealmAbilityPrefix(realm string) string  { return fmt.Sprintf("%s%s/ability/", URAScheme, realm) }
func RealmResourcePrefix(realm string) string { return fmt.Sprintf("%s%s/resource/", URAScheme, realm) }

func DeviceNodeIDInRealm(raw, realm string) (string, bool) {
	if raw == "" || realm == "" {
		return "", false
	}
	parts, err := ParseURAParts(raw)
	if err != nil || parts.Kind != URAKindDevice || parts.Realm != realm || parts.DeviceID == "" {
		return "", false
	}
	return parts.DeviceID, true
}

func DisplayID(raw string) string {
	parts, err := ParseURAParts(raw)
	if err != nil {
		return raw
	}
	switch parts.Kind {
	case URAKindDevice:
		return parts.DeviceID
	case URAKindUser:
		return parts.UserID
	case URAKindAgent:
		return parts.UserID + "." + parts.AgentID
	case URAKindAbility:
		switch parts.AbilityOwner.Kind {
		case AbilityOwnerHub:
			return "hub." + parts.AbilityID
		case AbilityOwnerDevice:
			return "device." + parts.AbilityOwner.DeviceID + "." + parts.AbilityID
		case AbilityOwnerAgent:
			return parts.AbilityOwner.UserID + "." + parts.AbilityOwner.AgentID + "." + parts.AbilityID
		default:
			return parts.UserID + "." + parts.AgentID + "." + parts.AbilityID
		}
	case URAKindHub:
		return "hub"
	case URAKindResource:
		if parts.ResourceNamespace == "" {
			if parts.Path == "" {
				return parts.UserID
			}
			return parts.UserID + "/" + parts.Path
		}
		return parts.UserID + "/" + string(parts.ResourceNamespace) + "/" + parts.Path
	default:
		return raw
	}
}

type AbilityOwnerKind string

const (
	AbilityOwnerHub    AbilityOwnerKind = "hub"
	AbilityOwnerAgent  AbilityOwnerKind = "agent"
	AbilityOwnerDevice AbilityOwnerKind = "device"
)

type AbilityOwner struct {
	Kind     AbilityOwnerKind
	UserID   string
	AgentID  string
	DeviceID string
}

type ParsedAbility struct {
	Owner     AbilityOwner
	Namespace string
	LocalName string
}

func ParseAbilityTail(tail string) (ParsedAbility, error) {
	if tail == "" || strings.Contains(tail, "/") {
		return ParsedAbility{}, fmt.Errorf("ability tail must be a single non-empty path segment")
	}
	var owner AbilityOwner
	var rest string
	switch {
	case strings.HasPrefix(tail, "hub."):
		owner = AbilityOwner{Kind: AbilityOwnerHub}
		rest = strings.TrimPrefix(tail, "hub.")
	case strings.HasPrefix(tail, "device."):
		after := strings.TrimPrefix(tail, "device.")
		deviceID, r, ok := strings.Cut(after, ".")
		if !ok {
			deviceID, r = after, ""
		}
		if deviceID == "" {
			return ParsedAbility{}, fmt.Errorf("device owner requires a <device-id> segment")
		}
		owner = AbilityOwner{Kind: AbilityOwnerDevice, DeviceID: deviceID}
		rest = r
	default:
		userID, afterUser, ok := strings.Cut(tail, ".")
		if !ok {
			return ParsedAbility{}, fmt.Errorf("agent ability tail must be <user-id>.<agent-id>.<rest>")
		}
		agentID, r, ok := strings.Cut(afterUser, ".")
		if !ok {
			agentID, r = afterUser, ""
		}
		if userID == "" || agentID == "" {
			return ParsedAbility{}, fmt.Errorf("agent ability tail must be <user-id>.<agent-id>.<rest>")
		}
		if userID == "hub" || userID == "device" {
			return ParsedAbility{}, fmt.Errorf("agent owner token %q is reserved", userID)
		}
		owner = AbilityOwner{Kind: AbilityOwnerAgent, UserID: userID, AgentID: agentID}
		rest = r
	}
	namespace, localName, ok := strings.Cut(rest, ".")
	if !ok {
		namespace, localName = "", rest
	}
	if localName == "" {
		return ParsedAbility{}, fmt.Errorf("ability tail missing local name")
	}
	return ParsedAbility{Owner: owner, Namespace: namespace, LocalName: localName}, nil
}

func DeviceAbilityURA(realm, deviceID, namespace, localName string) string {
	tail := "device." + deviceID
	if namespace != "" {
		tail += "." + namespace
	}
	tail += "." + localName
	return fmt.Sprintf("%s%s/ability/%s", URAScheme, realm, tail)
}

func ParseURAParts(raw string) (ParsedURA, error) {
	rest, ok := strings.CutPrefix(raw, URAScheme)
	if !ok {
		return ParsedURA{}, fmt.Errorf("URA must start with %s", URAScheme)
	}
	realm, afterRealm, ok := strings.Cut(rest, "/")
	if !ok || realm == "" {
		return ParsedURA{}, fmt.Errorf("URA missing realm segment")
	}
	role, tail, ok := strings.Cut(afterRealm, "/")
	if !ok {
		role, tail = afterRealm, ""
	}
	out := ParsedURA{Raw: raw, Realm: realm, Kind: URAKind(role)}
	switch role {
	case "user":
		if tail == "" || strings.Contains(tail, "/") || strings.Contains(tail, ".") {
			return ParsedURA{}, fmt.Errorf("user URA requires one user-id segment")
		}
		out.UserID = tail
	case "device":
		if tail == "" || strings.Contains(tail, "/") || strings.Contains(tail, ".") {
			return ParsedURA{}, fmt.Errorf("device URA requires one device-id segment")
		}
		out.DeviceID = tail
	case "agent":
		userID, agentID, ok := strings.Cut(tail, ".")
		if !ok || userID == "" || agentID == "" || strings.Contains(agentID, ".") || strings.Contains(tail, "/") {
			return ParsedURA{}, fmt.Errorf("agent URA tail must be <user-id>.<agent-id>")
		}
		out.UserID, out.AgentID = userID, agentID
	case "ability":
		ability, err := ParseAbilityTail(tail)
		if err != nil {
			return ParsedURA{}, fmt.Errorf("ability URA tail invalid: %w", err)
		}
		out.AbilityOwner = ability.Owner
		out.AbilityNamespace = ability.Namespace
		out.AbilityLocalName = ability.LocalName
		abilityID := ability.LocalName
		if ability.Namespace != "" {
			abilityID = ability.Namespace + "." + ability.LocalName
		}
		out.AbilityID = abilityID
		switch ability.Owner.Kind {
		case AbilityOwnerHub:
			out.UserID = "hub"
			out.AgentID = ability.Namespace
		case AbilityOwnerDevice:
			out.UserID = "device"
			out.DeviceID = ability.Owner.DeviceID
			out.AgentID = ability.Owner.DeviceID
		case AbilityOwnerAgent:
			out.UserID = ability.Owner.UserID
			out.AgentID = ability.Owner.AgentID
		default:
			return ParsedURA{}, fmt.Errorf("ability URA owner kind %q is unknown", ability.Owner.Kind)
		}
	case "hub":
		if tail != "" {
			return ParsedURA{}, fmt.Errorf("hub URA must use /hub without a tail")
		}
		out.Kind = URAKindHub
	case "resource":
		owner, path, _ := strings.Cut(tail, "/")
		if owner == "" {
			return ParsedURA{}, fmt.Errorf("resource URA requires owner segment")
		}
		out.OwnerID, out.UserID, out.Path = owner, owner, path
		if !strings.ContainsRune(owner, '.') {
			namespace, resourcePath, ok := strings.Cut(path, "/")
			if !ok || namespace == "" {
				return ParsedURA{}, fmt.Errorf("resource URA requires <namespace>/<path> for user-owned resources")
			}
			if !IsResourceNamespace(namespace) {
				return ParsedURA{}, fmt.Errorf("resource URA namespace %q is unknown", namespace)
			}
			out.ResourceNamespace = ResourceNamespace(namespace)
			out.Path = resourcePath
		}
	default:
		return ParsedURA{}, fmt.Errorf("unknown URA role %q", role)
	}
	return out, nil
}
