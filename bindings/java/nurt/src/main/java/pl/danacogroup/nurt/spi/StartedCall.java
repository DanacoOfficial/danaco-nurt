// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
package pl.danacogroup.nurt.spi;

import java.util.concurrent.CompletableFuture;

/** Request started with {@link Backend#start}. */
public interface StartedCall {
    /**
     * Result of the request: the response {@code nurt-response/1} as parsed
     * JSON, or a {@link pl.danacogroup.nurt.CallException}.
     *
     * @return the future result
     */
    CompletableFuture<Object> result();

    /** Asks for cancellation; idempotent. */
    void cancel();
}
