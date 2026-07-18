package easynet

import axonsdk "axon.run/sdk/go/axon"

// ParseAbilityDescriptorRef projects a DescriptorRef through Axon's canonical
// parser. SDK paths that can reach a Runtime should use
// ProjectAbilityDescriptorRef through the Addressing seam.
func ParseAbilityDescriptorRef(raw string) (AbilityDescriptorRef, error) {
	ref, err := axonsdk.ParseAbilityDescriptorRef(raw)
	if err != nil {
		return AbilityDescriptorRef{}, err
	}
	return AbilityDescriptorRef{Raw: ref.Raw, AbilityURA: ref.AbilityURA, Version: ref.Version}, nil
}
