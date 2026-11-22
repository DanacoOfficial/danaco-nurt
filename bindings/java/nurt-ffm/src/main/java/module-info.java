// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
/**
 * Library backend of the Danaco Nurt Java interface: calls {@code libnurt}
 * through the Foreign Function and Memory API.
 */
module pl.danacogroup.nurt.ffm {
    requires pl.danacogroup.nurt;

    provides pl.danacogroup.nurt.spi.BackendProvider
            with pl.danacogroup.nurt.ffm.LibraryBackendProvider;
}
