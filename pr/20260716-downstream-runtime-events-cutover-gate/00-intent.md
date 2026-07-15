# Intent

Close the downstream guard for the runtime-events route ownership cutover.

Runtime events expose provider-neutral stream and cursor semantics through the
SDK. Product route catalogs remain Backend adapter policy, and EasyNet provider
route names remain in the explicit EasyNet provider ABI. This slice prevents the
Backend events adapter from re-importing SDK-core or provider route catalog
types after the canonical runtime-events package stopped exporting route
lowering.
