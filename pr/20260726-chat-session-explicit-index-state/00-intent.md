# Intent

Remove the hidden missing-file fallback from chat session index persistence.

`SessionIndex` is a durable product read model. The storage reader must report
whether the index was loaded or missing; fresh-agent empty state is a caller
policy, not a serde/storage fact.
