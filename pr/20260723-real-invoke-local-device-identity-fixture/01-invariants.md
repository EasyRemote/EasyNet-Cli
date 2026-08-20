# Invariants

1. Production local identity resolution remains strict: no synthetic Device
   identity is introduced.
2. Tests that need local device ownership explicitly seed joined credentials.
3. Tests that intentionally validate empty HOME behavior keep the empty HOME
   fixture.
4. Filesystem ResourceRefs are minted only after local Device identity is
   provisioned.
5. Public ability names and response contracts remain unchanged.
