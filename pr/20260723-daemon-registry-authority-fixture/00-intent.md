# Intent

Remove ambient local-credentials dependency from daemon registry test assembly.

`RegistryDaemonBuildConfig::new` resolves production local Device authority
from the environment. Several tests and real-invoke fixtures called it and then
overwrote `authority_context`, which is too late: construction has already
touched local credentials and can panic in an unpaired checkout.

This slice introduces an explicit daemon registry build constructor for
pre-resolved authority fixtures and migrates test callers that already know the
authority context they need.
