# Java/Swift Mission Seam Plan

## Goal
Implement the SPEC-backed Mission carrier/status seam for Java and Swift without adding product-specific Mission execution logic.

## Scope
- Extend mission/carrier_status required_for coverage to Java and Swift.
- Add generic Mission carrier request DTOs, daemon-backed transport facade, status/event projections, and Runtime Core event stream adapter.
- Keep mission/plan_child_invocation scoped to existing Go/Python plan helper coverage unless the SPEC later requires Java/Swift plan rendering.

## Non-Goals
- No EasyNet/EasyRemote-specific Mission abstractions.
- No SDK-owned Mission execution, scheduler, retry policy, or receipt fabrication.
- No legacy aliases or URI naming.
