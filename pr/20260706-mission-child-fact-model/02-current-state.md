Current state:
- Go and Python expose Mission run/run-file/track/cancel/events carrier builders and status/event projections.
- Go and Python expose SDK-owned MissionPlan EAL rendering and basic child Invocation conformance.
- Child receipt anchoring already requires a parent receipt URA and matching child receipt facts.

Gap:
- Child Invocation status entries can be accepted with missing execution facts.
- Plan validation only checks step presence and ability equality; it does not prove that observed steps are complete daemon child Invocation projections.
- Shared conformance does not name the complete child fact model explicitly.
