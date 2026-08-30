# Intent

Complete the lifecycle of target-tmp archives uploaded for `ability.deploy`.
Once the daemon has parsed and canonicalized the root `ability.json`, the
single-use archive must be removed before registry mutation so repeated local
or remote deployment does not leak files into the Device tmp plane.
