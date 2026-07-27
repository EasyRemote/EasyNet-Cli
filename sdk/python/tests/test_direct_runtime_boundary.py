import ast
import inspect

import easynet_sdk.providers.runtime.direct as direct_runtime


_TUPLE_WIRE_CONSTRUCTORS = {
    "_invoke_pb2": {
        "EnvelopeOpen",
        "InvokeRequest",
        "InvokeServerStreamRequest",
    },
    "_types_pb2": {
        "AgentIdentity",
        "CallerSignature",
        "CausalContext",
        "Empty",
        "Envelope",
        "MerkleRoot",
        "ReceiptList",
        "ReceiptRef",
        "SubjectIdentity",
    },
}


class _TupleWireConstructionVisitor(ast.NodeVisitor):
    def __init__(self) -> None:
        self.class_stack: list[str] = []
        self.owners: dict[str, list[str | None]] = {}

    def visit_ClassDef(self, node: ast.ClassDef) -> None:
        self.class_stack.append(node.name)
        self.generic_visit(node)
        self.class_stack.pop()

    def visit_Call(self, node: ast.Call) -> None:
        if isinstance(node.func, ast.Attribute) and isinstance(
            node.func.value, ast.Name
        ):
            module = node.func.value.id
            constructor = node.func.attr
            if constructor in _TUPLE_WIRE_CONSTRUCTORS.get(module, set()):
                owner = self.class_stack[-1] if self.class_stack else None
                self.owners.setdefault(f"{module}.{constructor}", []).append(owner)
        self.generic_visit(node)


def _source() -> str:
    return inspect.getsource(direct_runtime)


def test_only_axon_descriptor_projector_constructs_tuple_wire_carriers() -> None:
    visitor = _TupleWireConstructionVisitor()
    visitor.visit(ast.parse(_source()))

    expected = {
        f"{module}.{constructor}"
        for module, constructors in _TUPLE_WIRE_CONSTRUCTORS.items()
        for constructor in constructors
    }
    assert set(visitor.owners) == expected
    assert all(
        owners and set(owners) == {"_AxonGrpcInvocation"}
        for owners in visitor.owners.values()
    )


def test_direct_runtime_has_no_subject_rewrite_or_legacy_receipt_projector() -> None:
    source = _source()

    for retired in (
        "_descriptor_bound_subject_ura",
        "descriptor_bound_resource_subject_ura",
        "_direct_invocation_fields",
        "def _receipt(",
        "replace(draft",
        'else b""',
        "carrier-v1",
        '"unsupported_frame"',
    ):
        assert retired not in source


def test_direct_runtime_delegates_tuple_and_receipt_validation_to_axon() -> None:
    source = _source()

    assert "_AxonDescriptorBoundEnvelope(" in source
    assert "_AxonDescriptorBoundInvocationRequest(" in source
    assert "_axon_invocation_receipt_from_json(canonical)" in source


def test_direct_dispatch_has_an_explicit_signed_request_boundary() -> None:
    source = _source()
    grpc_invocation_source = inspect.getsource(direct_runtime._AxonGrpcInvocation)

    assert "class _AxonDescriptorBoundDraft:" in source
    assert "class _AxonGrpcInvocation:" in source
    assert "request: _AxonDescriptorBoundInvocationRequest" in source
    assert "direct runtime dispatch requires caller_signature" in source
    assert "if self.signature is None" not in grpc_invocation_source
