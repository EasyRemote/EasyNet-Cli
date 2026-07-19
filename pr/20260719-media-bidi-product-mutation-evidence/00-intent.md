# Intent

Strengthen the Docker media/bidi product E2E from broad success assertions into
auditable mutation evidence for product-operation cardinality and terminal
receipt ownership.

The product requirement is not merely that stream and bidi calls return data.
The test must prove that each product operation creates exactly one provider
ledger record, that each record is bound to the expected descriptor tuple, and
that each Axon receipt chain is verified with exactly one completed terminal
receipt.
