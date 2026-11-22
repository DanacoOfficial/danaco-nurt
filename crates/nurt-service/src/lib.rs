// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Background service of Danaco Nurt.
//!
//! The service hosts the space module, which keeps free space above the
//! reserve on journal volumes, and the schedule module, which runs scheduled
//! programs. It installs itself as a systemd unit, a launchd agent or a Windows
//! service.

#![deny(missing_docs)]
#![forbid(unsafe_code)]
