// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
package pl.danacogroup.nurt;

import java.time.Duration;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionException;
import java.util.concurrent.ExecutionException;
import java.util.concurrent.Flow;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.TimeoutException;
import pl.danacogroup.nurt.internal.EventStream;
import pl.danacogroup.nurt.spi.StartedCall;

/**
 * Request started with {@link Client#start(Request)}.
 *
 * <p>{@link #events()} publishes every event of the execution, also to
 * subscribers that subscribe after the start, and completes when the
 * response has arrived. Closing an execution that has not finished cancels
 * it.
 */
public final class Execution implements AutoCloseable {
    private final StartedCall call;
    private final EventStream<Event> events;
    private final CompletableFuture<Response> response;

    Execution(StartedCall call, EventStream<Event> events) {
        this.call = call;
        this.events = events;
        this.response = call.result().thenApply(Response::of);
        this.response.whenComplete((value, error) -> events.complete(unwrap(error)));
    }

    /**
     * Events of the execution in the order of the server.
     *
     * @return the publisher
     */
    public Flow.Publisher<Event> events() {
        return events;
    }

    /**
     * The response, when it arrives.
     *
     * @return the future response; it fails with a {@link CallException}
     */
    public CompletableFuture<Response> response() {
        return response;
    }

    /**
     * Whether the response has arrived or the call has failed.
     *
     * @return {@code true} when finished
     */
    public boolean isDone() {
        return response.isDone();
    }

    /**
     * Waits for the response.
     *
     * @return the response
     * @throws CallException when the call fails
     */
    public Response await() {
        try {
            return response.get();
        } catch (ExecutionException e) {
            throw callException(e.getCause());
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new CallException(CallException.Kind.CANCELLED, "waiting for the execution was interrupted", e);
        }
    }

    /**
     * Waits up to {@code timeout} for the response.
     *
     * @param timeout the limit
     * @return the response
     * @throws CallException of kind {@link CallException.Kind#TIMEOUT} when the
     *     execution is still running; the execution continues and can be
     *     awaited again
     */
    public Response await(Duration timeout) {
        try {
            return response.get(timeout.toMillis(), TimeUnit.MILLISECONDS);
        } catch (TimeoutException e) {
            throw new CallException(CallException.Kind.TIMEOUT,
                    "execution still running after " + timeout.toMillis() + " ms", e);
        } catch (ExecutionException e) {
            throw callException(e.getCause());
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new CallException(CallException.Kind.CANCELLED, "waiting for the execution was interrupted", e);
        }
    }

    /**
     * Asks the server to cancel the execution. The response then reports the
     * cancelled execution; changes already committed stay in the journal.
     */
    public void cancel() {
        call.cancel();
    }

    /** Cancels the execution when it has not finished. */
    @Override
    public void close() {
        if (!response.isDone()) {
            call.cancel();
        }
    }

    private static Throwable unwrap(Throwable error) {
        return error instanceof CompletionException && error.getCause() != null ? error.getCause() : error;
    }

    private static CallException callException(Throwable cause) {
        Throwable error = unwrap(cause);
        if (error instanceof CallException call) {
            return call;
        }
        return new CallException(CallException.Kind.INTERNAL, "execution failed: " + error, error);
    }
}
