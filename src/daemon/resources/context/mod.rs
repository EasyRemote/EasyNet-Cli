// EasyNet CLI - daemon context services
// ======================================
//
// File: src/daemon/context/mod.rs
// Description: Daemon-owned background services for the local Context
//              surface.
//
// Context capture is local product policy. Persistence owns the file
// format; this module owns daemon process loops that feed it.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub mod clipboard_tracker;
