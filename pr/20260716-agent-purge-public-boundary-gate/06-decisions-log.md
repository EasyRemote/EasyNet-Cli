# Decisions Log

- Gate the public-boundary invariant instead of adding handler fallback or
  compatibility inputs. The runtime behavior is already converged; the open risk
  is future regression of descriptor/handler ownership.
