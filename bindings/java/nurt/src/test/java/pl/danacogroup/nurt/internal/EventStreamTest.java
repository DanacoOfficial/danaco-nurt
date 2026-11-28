// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/java/nurt/src/test/java/pl/danacogroup/nurt/internal/EventStreamTest.java
// Responsibility:   Source file bindings/java/nurt/src/test/java/pl/danacogroup/nurt/internal/EventStreamTest.java.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

package pl.danacogroup.nurt.internal;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertInstanceOf;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.util.List;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CopyOnWriteArrayList;
import java.util.concurrent.Flow;
import java.util.concurrent.ForkJoinPool;
import java.util.concurrent.TimeUnit;
import org.junit.jupiter.api.Test;

class EventStreamTest {
    /** Subscriber that requests {@code batch} items at a time and records them. */
    private static final class Recorder implements Flow.Subscriber<Integer> {
        final List<Integer> items = new CopyOnWriteArrayList<>();
        final CompletableFuture<Throwable> done = new CompletableFuture<>();
        private final long batch;
        private Flow.Subscription subscription;
        private long received;

        Recorder(long batch) {
            this.batch = batch;
        }

        @Override
        public void onSubscribe(Flow.Subscription subscription) {
            this.subscription = subscription;
            subscription.request(batch);
        }

        @Override
        public void onNext(Integer item) {
            items.add(item);
            if (++received % batch == 0) {
                subscription.request(batch);
            }
        }

        @Override
        public void onError(Throwable error) {
            done.complete(error);
        }

        @Override
        public void onComplete() {
            done.complete(null);
        }
    }

    @Test
    void lateSubscriberReceivesEveryEventInOrder() throws Exception {
        EventStream<Integer> stream = new EventStream<>(ForkJoinPool.commonPool());
        for (int i = 0; i < 100; i++) {
            stream.publish(i);
        }
        Recorder recorder = new Recorder(7);
        stream.subscribe(recorder);
        for (int i = 100; i < 200; i++) {
            stream.publish(i);
        }
        stream.complete(null);
        assertEquals(null, recorder.done.get(10, TimeUnit.SECONDS));
        assertEquals(200, recorder.items.size());
        for (int i = 0; i < 200; i++) {
            assertEquals(i, recorder.items.get(i));
        }
    }

    @Test
    void failureReachesTheSubscriberAfterTheEvents() throws Exception {
        EventStream<Integer> stream = new EventStream<>(ForkJoinPool.commonPool());
        stream.publish(1);
        stream.complete(new IllegalStateException("ended"));
        stream.publish(2);
        Recorder recorder = new Recorder(Long.MAX_VALUE);
        stream.subscribe(recorder);
        assertInstanceOf(IllegalStateException.class, recorder.done.get(10, TimeUnit.SECONDS));
        assertEquals(List.of(1), recorder.items);
        assertEquals(List.of(1), stream.snapshot());
    }

    @Test
    void nonPositiveDemandIsAnError() throws Exception {
        EventStream<Integer> stream = new EventStream<>(ForkJoinPool.commonPool());
        Recorder recorder = new Recorder(0);
        stream.subscribe(recorder);
        stream.publish(1);
        assertInstanceOf(IllegalArgumentException.class, recorder.done.get(10, TimeUnit.SECONDS));
        assertTrue(recorder.items.isEmpty());
    }

    @Test
    void cancelledSubscriberReceivesNothingMore() throws Exception {
        EventStream<Integer> stream = new EventStream<>(ForkJoinPool.commonPool());
        CompletableFuture<Flow.Subscription> subscribed = new CompletableFuture<>();
        List<Integer> items = new CopyOnWriteArrayList<>();
        stream.subscribe(new Flow.Subscriber<Integer>() {
            @Override
            public void onSubscribe(Flow.Subscription subscription) {
                subscription.cancel();
                subscribed.complete(subscription);
            }

            @Override
            public void onNext(Integer item) {
                items.add(item);
            }

            @Override
            public void onError(Throwable error) {
                items.add(-1);
            }

            @Override
            public void onComplete() {
                items.add(-2);
            }
        });
        subscribed.get(10, TimeUnit.SECONDS).request(5);
        stream.publish(1);
        stream.complete(null);
        ForkJoinPool.commonPool().awaitQuiescence(10, TimeUnit.SECONDS);
        assertTrue(items.isEmpty());
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
