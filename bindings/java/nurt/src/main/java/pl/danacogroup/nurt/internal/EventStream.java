// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/java/nurt/src/main/java/pl/danacogroup/nurt/internal/EventStream.java
// Responsibility:   Source file bindings/java/nurt/src/main/java/pl/danacogroup/nurt/internal/EventStream.java.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

package pl.danacogroup.nurt.internal;

import java.util.ArrayList;
import java.util.List;
import java.util.Objects;
import java.util.concurrent.Executor;
import java.util.concurrent.Flow;
import java.util.concurrent.atomic.AtomicBoolean;

/**
 * Publisher of the events of one execution.
 *
 * <p>Every subscriber receives every event from the first one, also when it
 * subscribes after the execution has started, and then completion. Events
 * are delivered on the executor, one at a time per subscriber and within the
 * demand of the subscriber, so a slow subscriber never blocks the producer.
 *
 * @param <T> type of the events
 */
public final class EventStream<T> implements Flow.Publisher<T> {
    private final Executor executor;
    private final List<T> items = new ArrayList<>();
    private boolean complete;
    private Throwable failure;
    private final List<Delivery> deliveries = new ArrayList<>();

    /**
     * Creates the publisher.
     *
     * @param executor executor that runs the deliveries
     */
    public EventStream(Executor executor) {
        this.executor = Objects.requireNonNull(executor, "executor");
    }

    /**
     * Adds an event; ignored after completion.
     *
     * @param item the event
     */
    public void publish(T item) {
        List<Delivery> targets;
        synchronized (this) {
            if (complete) {
                return;
            }
            items.add(item);
            targets = List.copyOf(deliveries);
        }
        targets.forEach(Delivery::schedule);
    }

    /**
     * Completes the stream; later calls are ignored.
     *
     * @param error failure of the execution, or {@code null} for normal completion
     */
    public void complete(Throwable error) {
        List<Delivery> targets;
        synchronized (this) {
            if (complete) {
                return;
            }
            complete = true;
            failure = error;
            targets = List.copyOf(deliveries);
        }
        targets.forEach(Delivery::schedule);
    }

    /**
     * Events published so far.
     *
     * @return a copy of the events
     */
    public synchronized List<T> snapshot() {
        return List.copyOf(items);
    }

    @Override
    public void subscribe(Flow.Subscriber<? super T> subscriber) {
        Delivery delivery = new Delivery(Objects.requireNonNull(subscriber, "subscriber"));
        executor.execute(delivery::start);
    }

    private final class Delivery implements Flow.Subscription {
        private final Flow.Subscriber<? super T> subscriber;
        private final AtomicBoolean scheduled = new AtomicBoolean();
        private int next;
        private long demand;
        private boolean started;
        private boolean finished;

        Delivery(Flow.Subscriber<? super T> subscriber) {
            this.subscriber = subscriber;
        }

        void start() {
            subscriber.onSubscribe(this);
            synchronized (EventStream.this) {
                started = true;
                if (!finished) {
                    deliveries.add(this);
                }
            }
            schedule();
        }

        @Override
        public void request(long n) {
            if (n <= 0) {
                synchronized (EventStream.this) {
                    if (finished) {
                        return;
                    }
                    finished = true;
                    deliveries.remove(this);
                }
                executor.execute(() -> subscriber.onError(
                        new IllegalArgumentException("demand must be positive, got " + n)));
                return;
            }
            synchronized (EventStream.this) {
                demand = demand + n < 0 ? Long.MAX_VALUE : demand + n;
            }
            schedule();
        }

        @Override
        public void cancel() {
            synchronized (EventStream.this) {
                finished = true;
                deliveries.remove(this);
            }
        }

        void schedule() {
            if (scheduled.compareAndSet(false, true)) {
                executor.execute(this::drain);
            }
        }

        private boolean hasWork() {
            synchronized (EventStream.this) {
                return started && !finished
                        && ((demand > 0 && next < items.size()) || (complete && next == items.size()));
            }
        }

        private void drain() {
            while (true) {
                T item = null;
                boolean terminate = false;
                Throwable error = null;
                synchronized (EventStream.this) {
                    if (!started || finished) {
                        break;
                    }
                    if (demand > 0 && next < items.size()) {
                        item = items.get(next++);
                        demand--;
                    } else if (complete && next == items.size()) {
                        finished = true;
                        terminate = true;
                        error = failure;
                        deliveries.remove(this);
                    } else {
                        break;
                    }
                }
                if (terminate) {
                    if (error == null) {
                        subscriber.onComplete();
                    } else {
                        subscriber.onError(error);
                    }
                    break;
                }
                subscriber.onNext(item);
            }
            scheduled.set(false);
            if (hasWork()) {
                schedule();
            }
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
