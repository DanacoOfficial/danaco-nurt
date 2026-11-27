// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-proc::spawn::batch
// Responsibility:   Runs over record streams: arguments split into batches and run one after another or several at a time, with results in batch order.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Runs over record streams: arguments split into batches and run one after
//! another or several at a time, with results in batch order.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};

/// Splits `args` into batches of up to `size` arguments; a size of 0 is
/// taken as 1.
pub(crate) fn split(args: &[String], size: usize) -> Vec<Vec<String>> {
    args.chunks(size.max(1)).map(<[String]>::to_vec).collect()
}

/// Runs `task` for every item on up to `parallel` threads and returns the
/// results in item order.
///
/// `task` returns its result and whether the item failed. With `bail`, no
/// item is started after the first failure; items already running finish,
/// and items never started are `None`.
pub(crate) fn run_ordered<T, R, F>(
    items: &[T],
    parallel: usize,
    bail: bool,
    task: F,
) -> Vec<Option<R>>
where
    T: Sync,
    R: Send,
    F: Fn(&T) -> (R, bool) + Sync,
{
    let next = AtomicUsize::new(0);
    let stopped = AtomicBool::new(false);
    let results: Mutex<Vec<Option<R>>> = Mutex::new(items.iter().map(|_| None).collect());
    let workers = parallel.clamp(1, items.len().max(1));
    let work = || {
        while !stopped.load(Ordering::SeqCst) {
            let index = next.fetch_add(1, Ordering::SeqCst);
            let Some(item) = items.get(index) else {
                break;
            };
            let (result, failed) = task(item);
            if failed && bail {
                stopped.store(true, Ordering::SeqCst);
            }
            results.lock().unwrap_or_else(PoisonError::into_inner)[index] = Some(result);
        }
    };
    if workers == 1 {
        work();
    } else {
        std::thread::scope(|scope| {
            for _ in 0..workers {
                scope.spawn(work);
            }
        });
    }
    results.into_inner().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use super::{run_ordered, split};

    fn texts(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    #[test]
    fn splits_arguments_into_batches_of_the_given_size() {
        let args = texts(&["a", "b", "c", "d", "e"]);
        assert_eq!(
            split(&args, 2),
            vec![texts(&["a", "b"]), texts(&["c", "d"]), texts(&["e"])]
        );
        assert_eq!(split(&args, 0).len(), 5);
        assert!(split(&[], 3).is_empty());
    }

    #[test]
    fn keeps_batch_order_when_later_batches_finish_first() {
        let items: Vec<u64> = vec![40, 30, 20, 10, 0];
        let results = run_ordered(&items, 5, false, |delay| {
            std::thread::sleep(Duration::from_millis(*delay));
            (*delay, false)
        });
        assert_eq!(
            results,
            items.iter().map(|delay| Some(*delay)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn limits_concurrent_batches_to_parallel() {
        let running = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let items: Vec<usize> = (0..8).collect();
        run_ordered(&items, 3, false, |_| {
            let now = running.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(now, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(20));
            running.fetch_sub(1, Ordering::SeqCst);
            ((), false)
        });
        assert!(peak.load(Ordering::SeqCst) <= 3);
        assert!(peak.load(Ordering::SeqCst) >= 2);
    }

    #[test]
    fn bail_starts_no_batch_after_a_failure() {
        let started = Mutex::new(Vec::new());
        let items: Vec<usize> = (0..5).collect();
        let results = run_ordered(&items, 1, true, |item| {
            started.lock().unwrap().push(*item);
            (*item, *item == 1)
        });
        assert_eq!(results, vec![Some(0), Some(1), None, None, None]);
        assert_eq!(*started.lock().unwrap(), vec![0, 1]);
    }

    #[test]
    fn runs_every_batch_after_a_failure_without_bail() {
        let items: Vec<usize> = (0..4).collect();
        let results = run_ordered(&items, 2, false, |item| (*item, true));
        assert!(results.iter().all(Option::is_some));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
