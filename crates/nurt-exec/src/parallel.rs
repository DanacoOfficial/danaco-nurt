// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-exec::parallel
// Responsibility:   Parallel per-record work with a deterministic output order.
// Dependencies:     nurt_contracts, nurt_testkit, rayon
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Parallel per-record work with a deterministic output order.

use std::collections::VecDeque;
use std::sync::Arc;

use nurt_contracts::error::{Error, Result};
use nurt_contracts::stage::{ExecCtx, RecordStream};
use nurt_contracts::value::Record;
use rayon::ThreadPool;
use rayon::prelude::*;

/// Records pulled from the input per thread of the pool before the batch is
/// processed in parallel.
pub const BATCH_PER_THREAD: usize = 64;

/// Applies `map` to every record of `stream`, in parallel when the execution
/// has more than one thread, keeping the order of the input.
///
/// `map` returns the output record, `None` to drop the record, or an error
/// that ends the stream. With [`Limits::threads`] above 1 and a thread pool
/// registered in the [`ExecCtx::extensions`] of the context (the executor
/// registers one), the records are read in batches of
/// [`BATCH_PER_THREAD`] per thread and each batch is processed on the pool;
/// otherwise the records are processed one by one as they are pulled. The
/// output is the same in both cases: the records in input order, up to the
/// first error, which is the error of the earliest failing record.
///
/// [`Limits::threads`]: nurt_contracts::stage::Limits::threads
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
/// use nurt_contracts::stage::{ExecCtx, RecordStream};
/// use nurt_contracts::value::{Record, RecordKind, Value};
/// use nurt_exec::ordered_filter_map;
/// use nurt_testkit::{clock::VirtualClock, host::TestHost};
///
/// let ctx = ExecCtx::builder(Arc::new(VirtualClock::new()), Arc::new(TestHost::linux())).build();
/// let input = (0..4).map(|n| Record::new(RecordKind::Entry).with("n", Value::Int(n))).collect();
/// let output = ordered_filter_map(RecordStream::from_records(input), &ctx, |record| {
///     Ok(matches!(record.get("n"), Some(Value::Int(n)) if n % 2 == 0).then_some(record))
/// });
/// let kept: Vec<Option<Value>> = output.map(|item| item.ok().and_then(|r| r.get("n").cloned())).collect();
/// assert_eq!(kept, [Some(Value::Int(0)), Some(Value::Int(2))]);
/// ```
pub fn ordered_filter_map<'a, F>(
    stream: RecordStream<'a>,
    ctx: &ExecCtx,
    map: F,
) -> RecordStream<'a>
where
    F: Fn(Record) -> Result<Option<Record>> + Send + Sync + 'a,
{
    let threads = usize::try_from(ctx.limits().threads).unwrap_or(1);
    let pool = if threads > 1 {
        ctx.extensions().get::<ThreadPool>()
    } else {
        None
    };
    match pool {
        Some(pool) => RecordStream::new(Batched {
            input: stream,
            map,
            pool,
            batch: threads.saturating_mul(BATCH_PER_THREAD),
            ready: VecDeque::new(),
            pending: None,
            exhausted: false,
        }),
        None => RecordStream::new(Sequential {
            input: stream,
            map,
            done: false,
        }),
    }
}

struct Sequential<'a, F> {
    input: RecordStream<'a>,
    map: F,
    done: bool,
}

impl<F> Iterator for Sequential<'_, F>
where
    F: Fn(Record) -> Result<Option<Record>>,
{
    type Item = Result<Record>;

    fn next(&mut self) -> Option<Self::Item> {
        while !self.done {
            let item = match self.input.next() {
                None => None,
                Some(Ok(record)) => match (self.map)(record) {
                    Ok(Some(output)) => return Some(Ok(output)),
                    Ok(None) => continue,
                    Err(error) => Some(error),
                },
                Some(Err(error)) => Some(error),
            };
            self.done = true;
            return item.map(Err);
        }
        None
    }
}

struct Batched<'a, F> {
    input: RecordStream<'a>,
    map: F,
    pool: Arc<ThreadPool>,
    batch: usize,
    ready: VecDeque<Result<Record>>,
    pending: Option<Error>,
    exhausted: bool,
}

impl<F> Batched<'_, F>
where
    F: Fn(Record) -> Result<Option<Record>> + Send + Sync,
{
    fn fill(&mut self) {
        let mut batch = Vec::with_capacity(self.batch);
        while batch.len() < self.batch {
            match self.input.next() {
                Some(Ok(record)) => batch.push(record),
                Some(Err(error)) => {
                    self.pending = Some(error);
                    self.exhausted = true;
                    break;
                }
                None => {
                    self.exhausted = true;
                    break;
                }
            }
        }
        let map = &self.map;
        let results: Vec<Result<Option<Record>>> = self
            .pool
            .install(|| batch.into_par_iter().map(map).collect());
        for result in results {
            match result {
                Ok(Some(record)) => self.ready.push_back(Ok(record)),
                Ok(None) => {}
                Err(error) => {
                    self.ready.push_back(Err(error));
                    self.pending = None;
                    self.exhausted = true;
                    break;
                }
            }
        }
    }
}

impl<F> Iterator for Batched<'_, F>
where
    F: Fn(Record) -> Result<Option<Record>> + Send + Sync,
{
    type Item = Result<Record>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(item) = self.ready.pop_front() {
                return Some(item);
            }
            if let Some(error) = self.pending.take() {
                return Some(Err(error));
            }
            if self.exhausted {
                return None;
            }
            self.fill();
        }
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::error::ErrorCode;
    use nurt_contracts::stage::Limits;
    use nurt_contracts::stage::{ExecCtx, Extensions};
    use nurt_contracts::value::{RecordKind, Value};
    use nurt_testkit::clock::VirtualClock;
    use nurt_testkit::host::TestHost;
    use rayon::ThreadPoolBuilder;

    use super::*;

    const RECORDS: i64 = 1000;
    const THREADS: u32 = 8;
    const FAILING: i64 = 700;

    fn context(threads: u32) -> ExecCtx {
        let mut extensions = Extensions::new();
        let pool = ThreadPoolBuilder::new()
            .num_threads(usize::try_from(threads).unwrap())
            .build()
            .unwrap();
        extensions.insert::<ThreadPool>(Arc::new(pool));
        ExecCtx::builder(Arc::new(VirtualClock::new()), Arc::new(TestHost::linux()))
            .limits(Limits {
                threads,
                ..Limits::default()
            })
            .extensions(extensions)
            .build()
    }

    fn numbers(count: i64) -> RecordStream<'static> {
        RecordStream::from_records(
            (0..count)
                .map(|n| Record::new(RecordKind::Entry).with("n", Value::Int(n)))
                .collect(),
        )
    }

    fn number(record: &Record) -> i64 {
        match record.get("n") {
            Some(Value::Int(n)) => *n,
            _ => -1,
        }
    }

    fn keep_odd(record: Record) -> Result<Option<Record>> {
        let n = number(&record);
        let spin = u64::try_from(n % 13).unwrap_or(0) * 200;
        let mut acc = 0_u64;
        for step in 0..spin {
            acc = std::hint::black_box(acc.wrapping_add(step));
        }
        Ok((n % 2 == 1).then(|| record.with("acc", Value::Int(i64::from(acc > 0)))))
    }

    fn run(threads: u32, count: i64) -> Vec<Result<i64>> {
        let ctx = context(threads);
        ordered_filter_map(numbers(count), &ctx, keep_odd)
            .map(|item| item.map(|record| number(&record)))
            .collect()
    }

    #[test]
    fn parallel_output_equals_sequential_output() {
        let sequential = run(1, RECORDS);
        let parallel = run(THREADS, RECORDS);
        assert_eq!(sequential.len(), usize::try_from(RECORDS / 2).unwrap());
        assert_eq!(sequential, parallel);
    }

    #[test]
    fn output_keeps_the_order_at_the_batch_boundaries() {
        let batch = usize::try_from(THREADS).unwrap() * BATCH_PER_THREAD;
        for count in [0, 1, batch - 1, batch, batch + 1] {
            let count = i64::try_from(count).unwrap();
            assert_eq!(run(1, count), run(THREADS, count), "{count} records");
        }
    }

    #[test]
    fn first_failing_record_ends_the_stream_in_input_order() {
        for threads in [1, THREADS] {
            let ctx = context(threads);
            let output: Vec<Result<Record>> =
                ordered_filter_map(numbers(RECORDS), &ctx, |record| {
                    if number(&record) >= FAILING {
                        Err(Error::new(
                            ErrorCode::Type,
                            format!("bad {}", number(&record)),
                        ))
                    } else {
                        Ok(Some(record))
                    }
                })
                .collect();
            assert_eq!(output.len(), usize::try_from(FAILING + 1).unwrap());
            let last = output.last().unwrap().as_ref().err().unwrap();
            assert_eq!(last.message, format!("bad {FAILING}"));
        }
    }

    #[test]
    fn upstream_error_follows_processed_records() {
        let input = RecordStream::new(
            (0..3)
                .map(|n| Ok(Record::new(RecordKind::Entry).with("n", Value::Int(n))))
                .chain(std::iter::once(Err(Error::new(ErrorCode::Io, "broken")))),
        );
        let ctx = context(THREADS);
        let output: Vec<Result<Record>> =
            ordered_filter_map(input, &ctx, |r| Ok(Some(r))).collect();
        assert_eq!(output.len(), 4);
        assert_eq!(
            output[3].as_ref().err().map(Error::code),
            Some(ErrorCode::Io)
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
