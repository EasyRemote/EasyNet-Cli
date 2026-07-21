package easynet

import (
	"strings"

	axonsdk "axon.run/sdk/go/axon"
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

type URA struct {
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
	return isProductResourceNamespace(namespace)
}

func ParseURA(raw string) (URA, error) {
	parsed, err := axonsdk.ParseURA(raw)
	if err != nil {
		return URA{}, err
	}
	return URA{raw: parsed.String()}, nil
}

func (u URA) String() string { return u.raw }

func (u URA) Parts() ParsedURA {
	parts, err := ParseURAParts(u.raw)
	if err != nil {
		panic("URA stores only validated canonical addresses")
	}
	return parts
}

func (u URA) Kind() URAKind { return u.Parts().Kind }

func (u URA) AbilityName() string {
	parts := u.Parts()
	if parts.Kind != URAKindAbility {
		return ""
	}
	return parts.AbilityID
}

func (u URA) PublicAbilityName(registeredName string) string {
	registeredName = strings.TrimSpace(registeredName)
	if registeredName == "" {
		return ""
	}
	return registeredName
}

func (u URA) PublicAbilityNameForOwner(ownerURA string) string {
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
	return axonsdk.AuthorityURA(realm)
}

func HubAbilityURA(realm, abilityName string) string {
	return axonsdk.AuthorityAbilityURA(realm, abilityName)
}

func ResourceDotURA(realm, ownerID, path string) string {
	return axonsdk.ResourceDotURA(realm, ownerID, path)
}

func ResourceURA(realm, userID, namespace, path string) string {
	return productResourceURA(realm, userID, namespace, path)
}

// RuntimeResourceURA builds a resource rooted at a runtime-owned identifier.
// ownerID and resourceID form the opaque dot-owner token interpreted by Axon.
func RuntimeResourceURA(realm, ownerID, resourceID, path string) string {
	owner := ownerID + "." + resourceID
	if path == "/" {
		return axonsdk.ResourceDotURA(realm, owner, "") + "/"
	}
	return axonsdk.ResourceDotURA(realm, owner, path)
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
	if raw == "" || realm == "" {
		return "", false
	}
	parts, err := axonsdk.ParseURAParts(raw)
	if err != nil || parts.Realm != realm || parts.Kind != axonsdk.URAKindDevice {
		return "", false
	}
	return parts.DeviceID, parts.DeviceID != ""
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
	resourceNamespace, resourcePath := projectProductResourcePath(parts.Kind, parts.Path)
	kind := URAKind(string(parts.Kind))
	if parts.Kind == axonsdk.URAKindAuthority {
		kind = URAKindHub
	}
	return ParsedURA{
		Raw:               parts.Raw,
		Realm:             parts.Realm,
		Kind:              kind,
		UserID:            parts.UserID,
		DeviceID:          parts.DeviceID,
		AgentID:           parts.AgentID,
		AbilityID:         parts.AbilityID,
		AbilityOwner:      abilityOwnerFromAxon(parts.AbilityOwner),
		AbilityNamespace:  parts.AbilityNamespace,
		AbilityLocalName:  parts.AbilityLocalName,
		OwnerID:           parts.OwnerID,
		ResourceNamespace: resourceNamespace,
		Path:              resourcePath,
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
	kind := AbilityOwnerKind(string(owner.Kind))
	if owner.Kind == axonsdk.AbilityOwnerAuthority {
		kind = AbilityOwnerHub
	}
	return AbilityOwner{
		Kind:     kind,
		UserID:   owner.UserID,
		AgentID:  owner.AgentID,
		DeviceID: owner.DeviceID,
	}
}
