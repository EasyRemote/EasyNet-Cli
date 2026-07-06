Risks:
- Over-tightening MissionStatus parsing could reject daemon fixtures that represent a planned but not submitted child step.
- Mitigation: only child_invocations entries are treated as daemon execution facts; planned-but-not-started steps should remain absent from child_invocations and appear as missing during plan conformance.
- Receipt validation must stay separate from receipt construction; the SDK must not fabricate missing receipt fields.
