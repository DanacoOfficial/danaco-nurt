// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
/**
 * Java interface of Danaco Nurt, the command console for AI agents and people.
 *
 * <p>The package {@code pl.danacogroup.nurt} holds the client. The package
 * {@code pl.danacogroup.nurt.spi} is the service interface implemented by
 * additional backends, such as the library backend of the artifact
 * {@code nurt-ffm}.
 */
module pl.danacogroup.nurt {
    exports pl.danacogroup.nurt;
    exports pl.danacogroup.nurt.spi;
    exports pl.danacogroup.nurt.internal to pl.danacogroup.nurt.ffm;

    uses pl.danacogroup.nurt.spi.BackendProvider;
}
