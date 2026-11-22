// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
package pl.danacogroup.nurt.spi;

import java.util.Map;
import java.util.function.Consumer;

/**
 * Connection of a client to Danaco Nurt.
 *
 * <p>Methods and parameters follow the protocol {@code nurt-rpc/1}: the
 * method names are {@code run}, {@code check}, {@code parse}, {@code print},
 * {@code correct}, {@code page}, {@code describe}, {@code help},
 * {@code catalog}, {@code schema}, {@code history}, {@code space},
 * {@code info} and {@code cancel}; parameters and results are parsed JSON
 * values. Language errors are part of the results. Every method is
 * thread-safe. Failures of the call are reported as
 * {@link pl.danacogroup.nurt.CallException}.
 */
public interface Backend extends AutoCloseable {
    /**
     * Calls a method and waits for its result.
     *
     * @param method method name
     * @param params parameters
     * @return the result
     */
    Object call(String method, Map<String, Object> params);

    /**
     * Starts a request envelope and returns at once.
     *
     * @param request the envelope {@code nurt-request/1}
     * @param events receiver of the events of the execution, called in the
     *     order of the server
     * @return the started call
     */
    StartedCall start(Map<String, Object> request, Consumer<Map<String, Object>> events);

    /** Ends the connection; further calls fail. */
    @Override
    void close();
}
