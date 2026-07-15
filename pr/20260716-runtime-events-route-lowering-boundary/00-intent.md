# Intent

Converge the runtime-events SDK surface by removing provider route-lowering
objects from the provider-neutral Go runtime-events package.

`sdk/go/runtimeevents` owns event stream lifecycle semantics. Topic-to-ability
route catalogs, cursor projection modes, and subscription argument lowering are
provider concerns and must not be exported as canonical runtime SDK concepts.
