# Intent

Remove the remaining ambient runtime catalog constructors from
`real_invoke_tests.rs`.

The real-invoke suite is a broad executable test harness. Its shared runtime
catalog helper must bind LocalRuntime to an explicit combined authority context,
and local tests should reuse that helper instead of constructing ad hoc catalogs.
