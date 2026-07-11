package easynet

import (
	"strings"

	axonsdk "easynet.run/axon/sdk/go/easynet"
)

// URA helpers are Go SDK facades over Axon-owned grammar.
//
// Product code should prefer the Addressing provider. These package-level
// helpers keep pure value-object ergonomics, while parse/build semantics remain
// delegated to Axon rather than maintained as a second grammar here.

const URAScheme = axonsdk.URAScheme

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

type AbilityOwnerKind string

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

const (
	AbilityOwnerHub    AbilityOwnerKind = "hub"
	AbilityOwnerAgent  AbilityOwnerKind = "agent"
	AbilityOwnerDevice AbilityOwnerKind = "device"
)

func IsResourceNamespace(namespace string) bool {
	return axonsdk.IsResourceNamespace(namespace)
}

func ParseURA(raw string) (Ura, error) {
	parsed, err := axonsdk.ParseURA(raw)
	if err != nil {
		return Ura{}, err
	}
	return Ura{raw: parsed.String()}, nil
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
	return axonsdk.AbilityNameFromURA(raw)
}

func PublicAbilityNameForOwner(ownerURA, registeredName string) string {
	return axonsdk.PublicAbilityNameForOwner(ownerURA, registeredName)
}

func OwnerAbilityURA(ownerURA, abilityName string) string {
	return axonsdk.OwnerAbilityURA(ownerURA, abilityName)
}

func PublicAbilityNameFromAbilityURA(ownerURA, abilityURA string) (string, bool) {
	return axonsdk.PublicAbilityNameFromAbilityURA(ownerURA, abilityURA)
}

func UserURA(realm, userID string) string {
	return axonsdk.UserURA(realm, userID)
}

func DeviceURA(realm, deviceID string) string {
	return axonsdk.DeviceURA(realm, deviceID)
}

func AgentURA(realm, userID, agentID string) string {
	return axonsdk.AgentURA(realm, userID, agentID)
}

func DeviceAgentURA(realm, deviceID, agentID string) string {
	return axonsdk.DeviceAgentURA(realm, deviceID, agentID)
}

func AbilityURA(realm, userID, agentID, abilityID string) string {
	return axonsdk.AbilityURA(realm, userID, agentID, abilityID)
}

func HubURA(realm string) string {
	return axonsdk.HubURA(realm)
}

func HubAbilityURA(realm, abilityName string) string {
	return axonsdk.HubAbilityURA(realm, abilityName)
}

func ResourceDotURA(realm, ownerID, path string) string {
	return axonsdk.ResourceDotURA(realm, ownerID, path)
}

func ResourceURA(realm, userID, namespace, path string) string {
	return axonsdk.ResourceURA(realm, userID, namespace, path)
}

func FilesResourceURA(realm, username, sha256Hex string) string {
	return axonsdk.FilesResourceURA(realm, username, sha256Hex)
}

func APIKeyResourceURA(realm, token string) string {
	return axonsdk.APIKeyResourceURA(realm, token)
}

func PagesResourceURA(realm, username, project, path string) string {
	return axonsdk.PagesResourceURA(realm, username, project, path)
}

func AgentSkillResourceURA(realm, username, agentID, skillName string) string {
	return axonsdk.AgentSkillResourceURA(realm, username, agentID, skillName)
}

func AgentSkillFileResourceURA(realm, username, agentID, skillName, relPath string) string {
	return axonsdk.AgentSkillFileResourceURA(realm, username, agentID, skillName, relPath)
}

func RealmUserPrefix(realm string) string {
	return axonsdk.RealmUserPrefix(realm)
}

func RealmDevicePrefix(realm string) string {
	return axonsdk.RealmDevicePrefix(realm)
}

func RealmAgentPrefix(realm string) string {
	return axonsdk.RealmAgentPrefix(realm)
}

func UserAgentPrefix(realm, userID string) string {
	return axonsdk.UserAgentPrefix(realm, userID)
}

func RealmAbilityPrefix(realm string) string {
	return axonsdk.RealmAbilityPrefix(realm)
}

func RealmResourcePrefix(realm string) string {
	return axonsdk.RealmResourcePrefix(realm)
}

func DeviceNodeIDInRealm(raw, realm string) (string, bool) {
	return axonsdk.DeviceNodeIDInRealm(raw, realm)
}

func DisplayID(raw string) string {
	return axonsdk.DisplayID(raw)
}

func ParseAbilityTail(tail string) (ParsedAbility, error) {
	ability, err := axonsdk.ParseAbilityTail(tail)
	if err != nil {
		return ParsedAbility{}, err
	}
	return parsedAbilityFromAxon(ability), nil
}

func DeviceAbilityURA(realm, deviceID, namespace, localName string) string {
	return axonsdk.DeviceAbilityURA(realm, deviceID, namespace, localName)
}

func ParseURAParts(raw string) (ParsedURA, error) {
	parts, err := axonsdk.ParseURAParts(raw)
	if err != nil {
		return ParsedURA{}, err
	}
	return parsedURAFromAxon(parts), nil
}

func parsedURAFromAxon(parts axonsdk.ParsedURA) ParsedURA {
	return ParsedURA{
		Raw:               parts.Raw,
		Realm:             parts.Realm,
		Kind:              URAKind(string(parts.Kind)),
		UserID:            parts.UserID,
		DeviceID:          parts.DeviceID,
		AgentID:           parts.AgentID,
		AbilityID:         parts.AbilityID,
		AbilityOwner:      abilityOwnerFromAxon(parts.AbilityOwner),
		AbilityNamespace:  parts.AbilityNamespace,
		AbilityLocalName:  parts.AbilityLocalName,
		OwnerID:           parts.OwnerID,
		ResourceNamespace: ResourceNamespace(string(parts.ResourceNamespace)),
		Path:              parts.Path,
	}
}

func parsedAbilityFromAxon(ability axonsdk.ParsedAbility) ParsedAbility {
	return ParsedAbility{
		Owner:     abilityOwnerFromAxon(ability.Owner),
		Namespace: ability.Namespace,
		LocalName: ability.LocalName,
	}
}

func abilityOwnerFromAxon(owner axonsdk.AbilityOwner) AbilityOwner {
	return AbilityOwner{
		Kind:     AbilityOwnerKind(string(owner.Kind)),
		UserID:   owner.UserID,
		AgentID:  owner.AgentID,
		DeviceID: owner.DeviceID,
	}
}
