# Architecture

`host_stream` retains one public executor interface with a Unix implementation
and a typed non-Unix unsupported result because its manifest contract names a
Unix domain socket. Agent purge remains unsupported on non-Unix platforms; its
open-handle identity check is therefore bounded to the Unix path that creates
the handle.
