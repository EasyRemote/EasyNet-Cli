# Decisions Log

## 2026-07-26

- Treat direct receipt provider use as a supported SDK path that must enforce canonical history admission itself.
- Do not rely on product/session wrappers as the only guard; that preserves a bypass for products that correctly compose the provider directly.
- Keep local receipt DTO validation before history admission so malformed filters report as receipt-shape errors, while descriptor resolution and invocation remain behind the admission guard.
- Update architecture gates to treat Python `_receipt_history_admission.py` as the shared canonical guard and reject reintroduced session-local duplicate validators.
