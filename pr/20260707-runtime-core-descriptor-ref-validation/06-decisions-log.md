# Decisions Log

## 2026-07-07

- Kept descriptor-ref canonical construction in identity/profile helpers; the Runtime Core builder only rejects malformed descriptor-bound drafts.
- Used a Python seam because the Python builder cannot require a daemon identity transport without coupling draft construction to process liveness.
