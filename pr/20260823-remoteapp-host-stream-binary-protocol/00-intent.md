# Intent

Fix the RemoteApp cross-device native host-stream smoke so the deployed native
EasyNet ability declares the same binary resident-host protocol that
EasyRemote's `HostServer` actually implements.

This is a product data-plane correctness fix: RemoteApp/window/video streams
cannot treat protocol mismatch as an acceptable truncation mode.
