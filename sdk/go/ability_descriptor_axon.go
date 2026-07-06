package easynet

import axonsdk "easynet.run/axon/sdk/go/easynet"

// ParseAbilityDescriptorRef projects a DescriptorRef through Axon's canonical
// parser. SDK paths that can reach a runtime should use ProjectAbilityDescriptorRef
// or IdentityClient.ProjectDescriptorRef so daemon profile transports remain the
// runtime boundary.
func ParseAbilityDescriptorRef(raw string) (AbilityDescriptorRef, error) {
	ref, err := axonsdk.ParseAbilityDescriptorRef(raw)
	if err != nil {
		return AbilityDescriptorRef{}, err
	}
	return AbilityDescriptorRef{Raw: ref.Raw, AbilityURA: ref.AbilityURA, Version: ref.Version}, nil
}
