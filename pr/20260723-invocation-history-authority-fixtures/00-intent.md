# Intent

Remove ambient authority construction from invocation history governance tests.

`invocation.history.*` is a canonical receipt/history surface. Its registration
tests should prove descriptor publication and dispatchability under an explicit
governance authority instead of depending on process-local Device credentials.
