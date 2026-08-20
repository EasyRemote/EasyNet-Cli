Architecture
============

Root abstraction
----------------

`ReceiptSemantics` tells downstream receipt and lifecycle code whether an
ability is an ordinary operational invocation or a governed state transition.
That is not a neutral zero value.

Boundary decision
-----------------

Remove the trait-level default from `ReceiptSemantics`. Keep the explicit
`ReceiptSemantics::Operational` assignment inside `AbilityDescriptor::new`,
where the constructor owns the descriptor creation contract.

Layering
--------

The core runtime descriptor model owns receipt semantics. SDKs and product
facades consume the emitted fact; they must not infer operational semantics from
missing data.
