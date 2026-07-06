# DescriptorRef Projection Boundary

## Goal

Keep descriptor-ref grammar ownership outside Python Runtime Core. Descriptor
projection stays behind the Identity/Addressing facade so the Python SDK does
not become a second Axon parser.

## Non-goals

- Do not add product-specific descriptor names.
- Do not replace daemon/Axon canonical descriptor-ref projection.
- Do not introduce descriptor-ref aliases or legacy input forms.
- Do not make `InvocationBuilder` parse or split descriptor refs locally.

## Acceptance Criteria

- Go `InvocationBuilder.Build` validates descriptor refs through the existing Axon-delegated helper.
- Python `parse_ability_descriptor_ref` delegates to `AddressingClient.project_descriptor_ref` or the default Identity projection facade.
- Python `InvocationBuilder.build` validates tuple completeness without parsing descriptor-ref grammar.
- Import-boundary tests reject descriptor-ref split/count/partition logic in Python Runtime Core.
- Public DTO fields remain unchanged.
- Full Go and Python SDK tests continue to pass.
