# Invariants — RemoteApp Route Visibility Gate

- Route semantics stay in the RemoteApp daemon transport projection.
- The frontend consumes `transportRouteState`/`productionReadiness.routeState`;
  it does not invent a separate network classifier.
- UI evidence does not replace real direct/STUN/TURN/EasyNet relay E2E reports.
- The product matrix remains incomplete until real network path evidence exists.
