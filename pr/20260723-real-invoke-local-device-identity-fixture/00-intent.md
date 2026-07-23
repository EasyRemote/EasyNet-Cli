# Intent

Remove ambient empty-HOME local identity assumptions from real-invoke device and
filesystem tests.

Local device operations now correctly require a provisioned Device identity.
Tests that exercise those operations must use an explicit joined-device fixture
instead of depending on empty HOME behavior or developer machine credentials.
